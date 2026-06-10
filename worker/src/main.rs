use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");

// Set when a 401/403 is received; main loop sleeps then exits.
static AUTH_HALT: AtomicBool = AtomicBool::new(false);
// Set when WS connection is active (informational only).
static WS_CONNECTED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
const DEFAULT_FFMPEG_PATH: &str = r"C:\msys64\mingw64\bin\ffmpeg.exe";
#[cfg(windows)]
const DEFAULT_FFPROBE_PATH: &str = r"C:\msys64\mingw64\bin\ffprobe.exe";
#[cfg(windows)]
const DEFAULT_WORK_DIR: &str = r"C:\transcode\jobs";
#[cfg(windows)]
const DEFAULT_LOG_DIR: &str = r"C:\transcode\logs";
#[cfg(windows)]
const DEFAULT_WORKER_ENV_FILE: &str = r"C:\transcode\worker.env";

#[cfg(not(windows))]
const DEFAULT_FFMPEG_PATH: &str = "ffmpeg";
#[cfg(not(windows))]
const DEFAULT_FFPROBE_PATH: &str = "ffprobe";
#[cfg(not(windows))]
const DEFAULT_WORK_DIR: &str = "/tmp/yulia-worker/jobs";
#[cfg(not(windows))]
const DEFAULT_LOG_DIR: &str = "~/.local/share/yulia-worker/logs";
#[cfg(not(windows))]
const DEFAULT_WORKER_ENV_FILE: &str = "~/.config/yulia-worker/worker.env";

const DEFAULT_ENCODER: &str = "";

// ── output codec ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum OutputCodec {
    Av1,
    Hevc,
    H264,
}

impl OutputCodec {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "hevc" | "h265" | "h.265" => OutputCodec::Hevc,
            "h264" | "avc" | "h.264" => OutputCodec::H264,
            _ => OutputCodec::Av1, // default
        }
    }

    fn preferred_encoders(&self) -> &'static [&'static str] {
        match self {
            OutputCodec::Av1 => &[
                "av1_qsv",
                "av1_nvenc",
                "av1_amf",
                "av1_vaapi",
                "libsvtav1",
            ],
            OutputCodec::Hevc => &[
                "hevc_qsv",
                "hevc_nvenc",
                "hevc_amf",
                "hevc_vaapi",
                "hevc_videotoolbox",
                "libx265",
            ],
            OutputCodec::H264 => &[
                "h264_qsv",
                "h264_nvenc",
                "h264_amf",
                "h264_vaapi",
                "h264_videotoolbox",
                "libx264",
            ],
        }
    }

    fn expected_codec_name(&self) -> &'static str {
        match self {
            OutputCodec::Av1 => "av1",
            OutputCodec::Hevc => "hevc",
            OutputCodec::H264 => "h264",
        }
    }
}

// ── config ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Config {
    queue_url: String,
    queue_token: Option<String>,
    ffmpeg: String,
    ffprobe: String,
    work_dir: PathBuf,
    log_dir: PathBuf,
    worker_name: String,
    worker_name_url: String,
    poll_secs: u64,
    encoder: String,
    encoder_av1: String,
    encoder_hevc: String,
    encoder_h264: String,
    encode_quality: String,
    encode_preset: String,
    audio_codec: String,
    audio_bitrate: String,
    vaapi_device: String,
    env_file_path: String,
    encoder_explicit: bool,
    encode_preset_explicit: bool,
    ws_enabled: bool,
}

fn sanitize_worker_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn expand_home(path: &str) -> String {
    if path.starts_with('~') {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        path.replacen('~', &home, 1)
    } else {
        path.to_string()
    }
}

/// Parse KEY=VALUE lines from .env file content. Env vars are NOT modified.
pub fn parse_env_file(contents: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim().to_string();
            let val = val.trim().trim_matches('"').trim_matches('\'').to_string();
            if !key.is_empty() {
                map.insert(key, val);
            }
        }
    }
    map
}

fn load_env_file(path: &str) -> HashMap<String, String> {
    match fs::read_to_string(path) {
        Ok(contents) => parse_env_file(&contents),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            eprintln!("WARNING: could not read env file {}: {}", path, e);
            HashMap::new()
        }
        _ => HashMap::new(),
    }
}

fn default_env_file_path() -> String {
    expand_home(DEFAULT_WORKER_ENV_FILE)
}

fn default_preset_for_encoder(encoder: &str) -> &'static str {
    match encoder {
        "libsvtav1" => "6",
        "av1_nvenc" | "hevc_nvenc" | "h264_nvenc" => "p4",
        _ => "medium",
    }
}

impl Config {
    fn from_env() -> Self {
        let env_file_path = expand_home(
            &std::env::var("WORKER_ENV_FILE").unwrap_or_else(|_| default_env_file_path()),
        );
        let file = load_env_file(&env_file_path);

        // env var wins over file value
        let get = |key: &str, default: &str| -> String {
            std::env::var(key).unwrap_or_else(|_| {
                file.get(key)
                    .cloned()
                    .unwrap_or_else(|| default.to_string())
            })
        };

        let configured = |key: &str| -> bool {
            std::env::var(key)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
                || file.get(key).map(|v| !v.trim().is_empty()).unwrap_or(false)
        };

        let queue_token = ["QUEUE_TOKEN", "AUTH_WORKER_TOKEN"]
            .iter()
            .filter_map(|&k| std::env::var(k).ok().or_else(|| file.get(k).cloned()))
            .map(|t| t.trim().to_string())
            .find(|t| !t.is_empty());

        let worker_name = get("WORKER_NAME", &hostname());
        let worker_name_url = sanitize_worker_name(&worker_name);
        let encoder = get("ENCODER", DEFAULT_ENCODER);
        let encoder_explicit = configured("ENCODER");
        let encode_preset_explicit = configured("ENCODE_PRESET");
        let default_preset = default_preset_for_encoder(&encoder);

        Self {
            queue_url: get("QUEUE_URL", "http://172.16.81.137:8090"),
            queue_token,
            ffmpeg: get("FFMPEG_PATH", DEFAULT_FFMPEG_PATH),
            ffprobe: get("FFPROBE_PATH", DEFAULT_FFPROBE_PATH),
            work_dir: PathBuf::from(expand_home(&get("WORK_DIR", DEFAULT_WORK_DIR))),
            log_dir: PathBuf::from(expand_home(&get("LOG_DIR", DEFAULT_LOG_DIR))),
            worker_name,
            worker_name_url,
            poll_secs: get("POLL_SECS", "10").parse().unwrap_or(10),
            encode_quality: get("ENCODE_QUALITY", "28"),
            encode_preset: get("ENCODE_PRESET", default_preset),
            audio_codec: get("AUDIO_CODEC", "aac"),
            audio_bitrate: get("AUDIO_BITRATE", "192k"),
            vaapi_device: get("VAAPI_DEVICE", "/dev/dri/renderD128"),
            encoder,
            encoder_av1: String::new(),
            encoder_hevc: String::new(),
            encoder_h264: String::new(),
            env_file_path,
            encoder_explicit,
            encode_preset_explicit,
            ws_enabled: true,
        }
    }

    fn apply_detected_encoder(&mut self, encoder: String) {
        self.encoder = encoder;
        if !self.encode_preset_explicit {
            self.encode_preset = default_preset_for_encoder(&self.encoder).to_string();
        }
    }
}

// ── logging ───────────────────────────────────────────────────────────────────

static LOG_FILE: Mutex<Option<fs::File>> = Mutex::new(None);
const LOG_ROTATE_BYTES: u64 = 50 * 1024 * 1024;
const IDLE_LOG_INTERVAL: Duration = Duration::from_secs(5 * 60);

fn init_log(log_dir: &PathBuf) {
    let _ = fs::create_dir_all(log_dir);
    if let Ok(f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("worker.log"))
    {
        *LOG_FILE.lock().unwrap() = Some(f);
    }
}

fn iso8601_utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    iso8601_utc_from_unix_secs(secs)
}

fn iso8601_utc_from_unix_secs(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let seconds_of_day = secs % 86_400;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u64, u64) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month as u64, day as u64)
}

fn format_log_line(
    timestamp: &str,
    level: &str,
    worker_name: &str,
    job_id: Option<&str>,
    msg: &str,
) -> String {
    let job = job_id.map(|id| format!(" job={}", id)).unwrap_or_default();
    let msg = msg.replace(['\r', '\n'], " ");
    format!(
        "[{}] {:<5} worker={}{} {}",
        timestamp, level, worker_name, job, msg
    )
}

fn log_with_level(level: &str, worker_name: &str, job_id: Option<&str>, msg: &str) {
    let line = format_log_line(&iso8601_utc_now(), level, worker_name, job_id, msg);
    eprintln!("{}", line);
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{}", line);
        }
    }
}

fn log_info(cfg: &Config, job_id: Option<&str>, msg: &str) {
    log_with_level("INFO", &cfg.worker_name, job_id, msg);
}

fn log_warn(cfg: &Config, job_id: Option<&str>, msg: &str) {
    log_with_level("WARN", &cfg.worker_name, job_id, msg);
}

fn log_error(cfg: &Config, job_id: Option<&str>, msg: &str) {
    log_with_level("ERROR", &cfg.worker_name, job_id, msg);
}

fn rotate_log_if_needed(cfg: &Config) {
    let log_path = cfg.log_dir.join("worker.log");
    let should_rotate = fs::metadata(&log_path)
        .map(|m| m.len() > LOG_ROTATE_BYTES)
        .unwrap_or(false);
    if !should_rotate {
        return;
    }

    let rotated_path = cfg.log_dir.join("worker.log.1");
    if let Ok(mut guard) = LOG_FILE.lock() {
        *guard = None;
        let _ = fs::remove_file(&rotated_path);
        match fs::rename(&log_path, &rotated_path) {
            Ok(_) => {}
            Err(e) => eprintln!("log rotation failed: {}", e),
        }
        if let Ok(f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            *guard = Some(f);
        }
    }
}

// ── API types ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Job {
    id: String,
    source_duration_secs: f64,
    #[serde(default)]
    output_codec: String, // "av1", "hevc", "h264" — defaults to "av1"
}

#[derive(Serialize)]
struct ProgressPayload {
    worker: String,
    phase: String,
    percent: f32,
    fps: f32,
    speed: String,
    frame: u64,
    bitrate: String,
    out_time: String,
}

#[derive(Serialize)]
struct DonePayload {
    worker: String,
    output_size: u64,
}

#[derive(Serialize)]
struct FailedPayload {
    worker: String,
    error: String,
}

#[derive(Serialize, Clone)]
struct HeartbeatPayload {
    status: String,
    current_job: Option<String>,
    current_file: Option<String>,
}

#[derive(Clone)]
struct HeartbeatState {
    status: String,
    current_job: Option<String>,
    current_file: Option<String>,
}

impl HeartbeatState {
    fn idle() -> Self {
        Self {
            status: "idle".into(),
            current_job: None,
            current_file: None,
        }
    }
    fn drain() -> Self {
        Self {
            status: "drain".into(),
            current_job: None,
            current_file: None,
        }
    }
    fn encoding(job_id: &str, file: &str) -> Self {
        Self {
            status: "encoding".into(),
            current_job: Some(job_id.into()),
            current_file: Some(file.into()),
        }
    }
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client")
}

fn http_large() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3600))
        .build()
        .expect("http client large")
}

fn with_auth(
    req: reqwest::blocking::RequestBuilder,
    cfg: &Config,
) -> reqwest::blocking::RequestBuilder {
    match cfg.queue_token.as_deref() {
        Some(t) if !t.trim().is_empty() => req.bearer_auth(t.trim()),
        _ => req,
    }
}

fn flag_auth_halt(cfg: &Config, status: u16, endpoint: &str) {
    AUTH_HALT.store(true, Ordering::SeqCst);
    log_error(
        cfg,
        None,
        &format!(
            "AUTH ERROR: {} returned {} — stopping. Check QUEUE_TOKEN in {} and restart.",
            endpoint, status, "worker.env"
        ),
    );
}

fn poll_job(cfg: &Config) -> Result<Option<Job>> {
    let url = format!("{}/jobs/next?worker={}", cfg.queue_url, cfg.worker_name_url);
    let resp = with_auth(http().get(&url), cfg)
        .send()
        .context("GET /jobs/next")?;
    match resp.status().as_u16() {
        204 => return Ok(None),
        401 | 403 => {
            flag_auth_halt(cfg, resp.status().as_u16(), "GET /jobs/next");
            bail!("auth error {} from queue", resp.status());
        }
        s if !(200..300).contains(&(s as i32)) => bail!("GET /jobs/next returned {}", s),
        _ => {}
    }
    Ok(Some(resp.json().context("parse job")?))
}

fn download_source(cfg: &Config, job: &Job, dest: &PathBuf) -> Result<()> {
    let url = format!("{}/jobs/{}/source", cfg.queue_url, job.id);
    let mut resp = with_auth(http_large().get(&url), cfg)
        .send()
        .context("GET source")?;
    if !resp.status().is_success() {
        bail!("download source HTTP {}", resp.status());
    }
    let mut f = fs::File::create(dest).context("create input file")?;
    resp.copy_to(&mut f).context("stream source to disk")?;
    Ok(())
}

fn upload_output(cfg: &Config, job: &Job, path: &PathBuf) -> Result<()> {
    let url = format!("{}/jobs/{}/output", cfg.queue_url, job.id);
    let file = fs::File::open(path).context("open output for upload")?;
    let size = file.metadata()?.len();
    let resp = with_auth(
        http_large()
            .put(&url)
            .header("Content-Length", size)
            .body(file),
        cfg,
    )
    .send()
    .context("PUT output")?;
    if !resp.status().is_success() {
        bail!("upload output HTTP {}", resp.status());
    }
    Ok(())
}

fn report_progress(
    cfg: &Config,
    job_id: &str,
    percent: f32,
    fps: f32,
    speed: &str,
    frame: u64,
    bitrate: &str,
    out_time: &str,
) {
    let url = format!("{}/jobs/{}/progress", cfg.queue_url, job_id);
    let _ = with_auth(
        http().post(&url).json(&ProgressPayload {
            worker: cfg.worker_name.clone(),
            phase: "encoding".to_string(),
            percent,
            fps,
            speed: speed.to_string(),
            frame,
            bitrate: bitrate.to_string(),
            out_time: out_time.to_string(),
        }),
        cfg,
    )
    .send();
}

fn report_phase(cfg: &Config, job_id: &str, phase: &str) {
    let url = format!("{}/jobs/{}/progress", cfg.queue_url, job_id);
    let _ = with_auth(
        http().post(&url).json(&ProgressPayload {
            worker: cfg.worker_name.clone(),
            phase: phase.to_string(),
            percent: 100.0,
            fps: 0.0,
            speed: String::new(),
            frame: 0,
            bitrate: String::new(),
            out_time: String::new(),
        }),
        cfg,
    )
    .send();
}

fn report_done(cfg: &Config, job_id: &str, output_size: u64) -> Result<()> {
    let url = format!("{}/jobs/{}/done", cfg.queue_url, job_id);
    let resp = with_auth(
        http().post(&url).json(&DonePayload {
            worker: cfg.worker_name.clone(),
            output_size,
        }),
        cfg,
    )
    .send()
    .context("POST /done")?;
    match resp.status().as_u16() {
        401 | 403 => {
            flag_auth_halt(cfg, resp.status().as_u16(), "POST /done");
            bail!(
                "POST /done auth error {} — local output retained",
                resp.status()
            );
        }
        s if !(200..300).contains(&(s as i32)) => {
            bail!(
                "POST /done returned {} — local output retained for operator retry",
                s
            );
        }
        _ => Ok(()),
    }
}

fn report_failed(cfg: &Config, job_id: &str, error: &str) {
    let url = format!("{}/jobs/{}/failed", cfg.queue_url, job_id);
    match with_auth(
        http().post(&url).json(&FailedPayload {
            worker: cfg.worker_name.clone(),
            error: error.to_string(),
        }),
        cfg,
    )
    .send()
    {
        Ok(r) if !r.status().is_success() => log_warn(
            cfg,
            Some(job_id),
            &format!("POST /failed returned {}", r.status()),
        ),
        Err(e) => log_warn(cfg, Some(job_id), &format!("POST /failed error: {}", e)),
        _ => {}
    }
}

fn send_heartbeat(cfg: &Config, state: &HeartbeatState) {
    let url = format!(
        "{}/workers/{}/heartbeat",
        cfg.queue_url, cfg.worker_name_url
    );
    let payload = HeartbeatPayload {
        status: state.status.clone(),
        current_job: state.current_job.clone(),
        current_file: state.current_file.clone(),
    };
    match with_auth(http().post(&url).json(&payload), cfg).send() {
        Ok(r) if r.status() == 401 || r.status() == 403 => {
            flag_auth_halt(cfg, r.status().as_u16(), "POST /heartbeat")
        }
        _ => {}
    }
}

fn cleanup_stale(cfg: &Config) {
    let url = format!(
        "{}/jobs/abandon?worker={}",
        cfg.queue_url, cfg.worker_name_url
    );
    let _ = with_auth(http().post(&url), cfg).send();
}

#[derive(Clone, PartialEq)]
enum ControlCmd {
    Run,
    Drain,
    Stop,
}

fn poll_control(cfg: &Config) -> ControlCmd {
    #[derive(Deserialize)]
    struct Resp {
        command: String,
    }
    let url = format!("{}/control", cfg.queue_url);
    match with_auth(http().get(&url), cfg)
        .send()
        .and_then(|r| r.json::<Resp>())
    {
        Ok(r) => match r.command.as_str() {
            "drain" => ControlCmd::Drain,
            "stop" => ControlCmd::Stop,
            _ => ControlCmd::Run,
        },
        Err(_) => ControlCmd::Run,
    }
}

// ── encode ────────────────────────────────────────────────────────────────────

pub fn quality_flag(encoder: &str) -> &'static str {
    match encoder {
        "av1_qsv" | "hevc_qsv" | "h264_qsv" => "-global_quality",
        "av1_nvenc" | "hevc_nvenc" | "h264_nvenc" => "-cq",
        "av1_vaapi" | "hevc_vaapi" | "h264_vaapi" => "-qp",
        "av1_amf" | "hevc_amf" | "h264_amf" => "-qp",
        "hevc_videotoolbox" | "h264_videotoolbox" => "-q:v",
        _ => "-crf",
    }
}

fn is_vaapi_encoder(encoder: &str) -> bool {
    encoder.ends_with("_vaapi")
}

/// Returns true if this encoder accepts a standard -preset argument.
/// AMF uses -quality {speed|balanced|quality} and VideoToolbox has no preset at all.
fn encoder_uses_preset(encoder: &str) -> bool {
    !encoder.ends_with("_vaapi")
        && !encoder.ends_with("_amf")
        && !encoder.ends_with("_videotoolbox")
}

fn build_encode_args(cfg: &Config, input: &str, output: &str) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if is_vaapi_encoder(&cfg.encoder) {
        a.extend(["-vaapi_device".into(), cfg.vaapi_device.clone()]);
    }
    a.extend(["-i".into(), input.into()]);
    if is_vaapi_encoder(&cfg.encoder) {
        a.extend(["-vf".into(), "format=nv12,hwupload".into()]);
    }
    a.extend([
        "-c:v".into(),
        cfg.encoder.clone(),
        quality_flag(&cfg.encoder).into(),
        cfg.encode_quality.clone(),
    ]);
    if encoder_uses_preset(&cfg.encoder) {
        a.extend(["-preset".into(), cfg.encode_preset.clone()]);
    }
    a.extend([
        "-c:a".into(),
        cfg.audio_codec.clone(),
        "-b:a".into(),
        cfg.audio_bitrate.clone(),
        "-movflags".into(),
        "+faststart".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-stats_period".into(),
        "2".into(),
        "-loglevel".into(),
        "error".into(),
        output.into(),
        "-y".into(),
    ]);
    a
}

fn build_encoder_test_args(cfg: &Config, encoder: &str) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if is_vaapi_encoder(encoder) {
        a.extend(["-vaapi_device".into(), cfg.vaapi_device.clone()]);
    }
    a.extend([
        "-hide_banner".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "testsrc=duration=1:size=64x64:rate=1".into(),
    ]);
    if is_vaapi_encoder(encoder) {
        a.extend(["-vf".into(), "format=nv12,hwupload".into()]);
    }
    a.extend([
        "-c:v".into(),
        encoder.into(),
        quality_flag(encoder).into(),
        cfg.encode_quality.clone(),
    ]);
    if encoder_uses_preset(encoder) {
        let preset = if cfg.encode_preset_explicit {
            cfg.encode_preset.clone()
        } else {
            default_preset_for_encoder(encoder).into()
        };
        a.extend(["-preset".into(), preset]);
    }
    a.extend([
        "-frames:v".into(),
        "1".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
        "-y".into(),
    ]);
    a
}

fn test_encoder_candidate(cfg: &Config, encoder: &str) -> Result<()> {
    let args = build_encoder_test_args(cfg, encoder);
    let out = Command::new(&cfg.ffmpeg)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("run {} encoder test", encoder))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let detail = err
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("encoder test failed");
        bail!("{}", detail);
    }
    Ok(())
}

fn detect_encoder(cfg: &Config, target_codec: &OutputCodec) -> String {
    if cfg.encoder_explicit {
        log_info(
            cfg,
            None,
            &format!(
                "Encoder explicitly configured as {}; skipping detection.",
                cfg.encoder
            ),
        );
        return cfg.encoder.clone();
    }

    for candidate in target_codec.preferred_encoders() {
        match test_encoder_candidate(cfg, candidate) {
            Ok(()) => {
                log_info(
                    cfg,
                    None,
                    &format!("Selected encoder {} for {:?}.", candidate, target_codec),
                );
                return candidate.to_string();
            }
            Err(e) => {
                log_warn(cfg, None, &format!("Skipping encoder {}: {}", candidate, e));
            }
        }
    }

    let fallback = target_codec
        .preferred_encoders()
        .last()
        .copied()
        .unwrap_or("libsvtav1");
    log_warn(
        cfg,
        None,
        &format!(
            "No encoder passed detection; falling back to {}",
            fallback
        ),
    );
    fallback.to_string()
}

fn transcode(
    cfg: &Config,
    job: &Job,
    input: &PathBuf,
    output: &PathBuf,
    ffmpeg_child: &Arc<Mutex<Option<Child>>>,
) -> Result<()> {
    let args = build_encode_args(cfg, input.to_str().unwrap(), output.to_str().unwrap());
    let mut child = Command::new(&cfg.ffmpeg)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn ffmpeg")?;

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let total = job.source_duration_secs;
    let mut last_report = Instant::now();

    *ffmpeg_child.lock().unwrap() = Some(child);

    let mut kv: HashMap<String, String> = HashMap::new();
    for line in reader.lines().map_while(Result::ok) {
        if let Some((k, v)) = line.split_once('=') {
            kv.insert(k.trim().to_string(), v.trim().to_string());
        }
        if line.starts_with("progress=") {
            if last_report.elapsed() >= Duration::from_secs(2) {
                let out_time_us: f64 = kv
                    .get("out_time_us")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.0);
                let pct = if total > 0.0 {
                    ((out_time_us / 1_000_000.0) / total * 100.0) as f32
                } else {
                    0.0
                };
                report_progress(
                    cfg,
                    &job.id,
                    pct.min(100.0),
                    kv.get("fps").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                    &kv.get("speed").cloned().unwrap_or_default(),
                    kv.get("frame").and_then(|v| v.parse().ok()).unwrap_or(0),
                    &kv.get("bitrate").cloned().unwrap_or_default(),
                    &kv.get("out_time").cloned().unwrap_or_default(),
                );
                last_report = Instant::now();
            }
            kv.clear();
        }
    }

    let status = {
        let mut guard = ffmpeg_child.lock().unwrap();
        guard.as_mut().unwrap().wait().context("wait ffmpeg")?
    };
    *ffmpeg_child.lock().unwrap() = None;

    if !status.success() {
        bail!("ffmpeg exited with {}", status);
    }
    Ok(())
}

// ── validate ──────────────────────────────────────────────────────────────────

fn validate(cfg: &Config, job: &Job, output: &PathBuf) -> Result<()> {
    if !output.exists() {
        bail!("output file missing after encode");
    }
    let codec = ffprobe_value(
        cfg,
        output,
        &[
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name",
        ],
    )
    .context("ffprobe codec")?;
    let expected = OutputCodec::from_str(&job.output_codec).expected_codec_name();
    if codec.trim() != expected {
        bail!(
            "output codec is '{}', expected '{}'",
            codec.trim(),
            expected
        );
    }
    let dur_str = ffprobe_value(cfg, output, &["-show_entries", "format=duration"])
        .context("ffprobe duration")?;
    let out_dur: f64 = dur_str.trim().parse().context("parse output duration")?;
    let diff = (out_dur - job.source_duration_secs).abs();
    if diff > 2.0 {
        bail!(
            "duration mismatch: source={:.1}s output={:.1}s diff={:.1}s",
            job.source_duration_secs,
            out_dur,
            diff
        );
    }
    Ok(())
}

fn ffprobe_value(cfg: &Config, path: &PathBuf, extra_args: &[&str]) -> Result<String> {
    let mut args = vec!["-v", "error", "-of", "default=noprint_wrappers=1:nokey=1"];
    args.extend_from_slice(extra_args);
    args.push(path.to_str().unwrap());
    let out = Command::new(&cfg.ffprobe)
        .args(&args)
        .output()
        .context("spawn ffprobe")?;
    if !out.status.success() {
        bail!("ffprobe: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ── diagnostics ───────────────────────────────────────────────────────────────

fn check_binaries(cfg: &Config) -> Result<()> {
    Command::new(&cfg.ffmpeg)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| {
            format!(
                "ffmpeg not found at '{}'. Install ffmpeg or set FFMPEG_PATH.",
                cfg.ffmpeg
            )
        })?;
    Command::new(&cfg.ffprobe)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| {
            format!(
                "ffprobe not found at '{}'. Install ffprobe or set FFPROBE_PATH.",
                cfg.ffprobe
            )
        })?;
    Ok(())
}

fn test_encoder(cfg: &Config) -> Result<()> {
    if cfg.encoder.trim().is_empty() {
        bail!("no encoder configured or detected");
    }
    test_encoder_candidate(cfg, &cfg.encoder)
}

fn check_queue_health(cfg: &Config) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let healthz = format!("{}/healthz", cfg.queue_url);
    if let Ok(r) = client.get(&healthz).send() {
        if r.status().is_success() {
            return Ok(format!("/healthz {}", r.status()));
        }
    }
    let status = format!("{}/status", cfg.queue_url);
    match client.get(&status).send() {
        Ok(r) if r.status().is_success() => Ok(format!("/status {}", r.status())),
        Ok(r) => bail!("queue returned {}", r.status()),
        Err(e) => bail!("queue unreachable: {}", e),
    }
}

fn check_auth_token(cfg: &Config) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let url = format!("{}/status", cfg.queue_url);
    let resp = with_auth(client.get(&url), cfg)
        .send()
        .context("auth check")?;
    match resp.status().as_u16() {
        200..=299 => Ok(()),
        401 => bail!("401 Unauthorized — token missing or invalid"),
        403 => bail!("403 Forbidden — token not authorized for this endpoint"),
        s => bail!("unexpected status {}", s),
    }
}

fn run_diagnostics(cfg: &Config) -> bool {
    let mut ok = true;

    println!("yulia-worker {} diagnostics", VERSION);
    println!("{}", "=".repeat(40));
    println!("worker name  : {}", cfg.worker_name);
    println!("queue url    : {}", cfg.queue_url);
    println!("encoder(av1) : {}", cfg.encoder_av1);
    println!("encoder(hevc): {}", cfg.encoder_hevc);
    println!("encoder(h264): {}", cfg.encoder_h264);
    println!("work dir     : {}", cfg.work_dir.display());
    println!("log dir      : {}", cfg.log_dir.display());
    println!("env file     : {}", cfg.env_file_path);
    println!(
        "token        : {}",
        if cfg.queue_token.is_some() {
            "set"
        } else {
            "unset"
        }
    );
    println!();

    macro_rules! check {
        ($label:expr, $expr:expr) => {{
            print!("{:<14}", $label);
            match $expr {
                Ok(msg) => println!("ok  {}", msg),
                Err(e) => {
                    println!("FAIL  {}", e);
                    ok = false;
                }
            }
        }};
    }

    check!(
        "ffmpeg",
        Command::new(&cfg.ffmpeg)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| anyhow::anyhow!("{} — check FFMPEG_PATH", e))
            .and_then(|s| if s.success() {
                Ok(format!("({})", cfg.ffmpeg))
            } else {
                bail!("exit {}", s)
            })
    );

    check!(
        "ffprobe",
        Command::new(&cfg.ffprobe)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| anyhow::anyhow!("{} — check FFPROBE_PATH", e))
            .and_then(|s| if s.success() {
                Ok(format!("({})", cfg.ffprobe))
            } else {
                bail!("exit {}", s)
            })
    );

    check!("encoder", test_encoder(cfg).map(|_| String::new()));
    check!("queue", check_queue_health(cfg));

    if cfg.queue_token.is_some() {
        check!(
            "auth",
            check_auth_token(cfg).map(|_| "token accepted".to_string())
        );
    } else {
        println!("{:<14}skipped (no token configured)", "auth");
    }

    println!();
    if ok {
        println!("All checks passed.");
    } else {
        println!("One or more checks FAILED — see above.");
    }
    ok
}

// ── file permissions check (Linux) ───────────────────────────────────────────

#[cfg(unix)]
fn warn_env_file_permissions(cfg: &Config, path: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mode = meta.permissions().mode();
        if mode & 0o044 != 0 {
            log_warn(
                cfg,
                None,
                &format!(
                "{} has permissions {:o} (expected 0600) — token may be exposed to other users.",
                path, mode & 0o777
            ),
            );
        }
    }
}
#[cfg(not(unix))]
fn warn_env_file_permissions(_: &Config, _: &str) {}

// ── job loop ──────────────────────────────────────────────────────────────────

fn process_job(cfg: &Config, job: &Job, ffmpeg_child: &Arc<Mutex<Option<Child>>>) -> Result<bool> {
    let work_dir = cfg.work_dir.join(&job.id);
    fs::create_dir_all(&work_dir).context("create work dir")?;

    let input = work_dir.join("input.mp4");
    let output = work_dir.join("output.mp4");

    // Select the pre-detected encoder for this job's target codec.
    let target_codec = OutputCodec::from_str(&job.output_codec);
    let selected_encoder = match &target_codec {
        OutputCodec::Av1 => &cfg.encoder_av1,
        OutputCodec::Hevc => &cfg.encoder_hevc,
        OutputCodec::H264 => &cfg.encoder_h264,
    };
    if selected_encoder.is_empty() {
        log_error(
            cfg,
            Some(&job.id),
            &format!(
                "No working encoder found for {:?}, failing job {}",
                target_codec, job.id
            ),
        );
        report_failed(
            cfg,
            &job.id,
            "No hardware or software encoder available for requested codec",
        );
        return Ok(false);
    }
    let mut job_cfg = cfg.clone();
    job_cfg.apply_detected_encoder(selected_encoder.clone());

    log_info(cfg, Some(&job.id), "Downloading source...");
    download_source(cfg, job, &input)?;
    log_info(
        cfg,
        Some(&job.id),
        &format!("{} bytes downloaded", input.metadata()?.len()),
    );

    log_info(
        cfg,
        Some(&job.id),
        &format!("encoder={} codec={:?} Transcoding...", job_cfg.encoder, target_codec),
    );
    transcode(&job_cfg, job, &input, &output, ffmpeg_child)?;
    log_info(cfg, Some(&job.id), "Transcode complete");

    log_info(cfg, Some(&job.id), "Validating output...");
    validate(cfg, job, &output)?;
    log_info(cfg, Some(&job.id), "Validation passed");

    let out_size = output.metadata()?.len();
    log_info(
        cfg,
        Some(&job.id),
        &format!(
            "Uploading output ({:.1} MB)...",
            out_size as f64 / 1_048_576.0
        ),
    );
    report_phase(&job_cfg, &job.id, "uploading");
    upload_output(&job_cfg, job, &output)?;
    report_phase(&job_cfg, &job.id, "verifying");

    // Only delete local output after a confirmed 2xx done response.
    match report_done(cfg, &job.id, out_size) {
        Ok(_) => {
            let _ = fs::remove_dir_all(&work_dir);
            log_info(
                cfg,
                Some(&job.id),
                &format!("Done. {:.1} MB", out_size as f64 / 1_048_576.0),
            );
            Ok(true)
        }
        Err(e) => {
            log_error(
                cfg,
                Some(&job.id),
                &format!(
                    "POST /done failed — output retained in {}: {}",
                    work_dir.display(),
                    e
                ),
            );
            // Return false so caller skips report_failed (upload already succeeded).
            Ok(false)
        }
    }
}

// ── WebSocket client ──────────────────────────────────────────────────────────

fn current_platform() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        "unknown"
    }
}

fn fetch_job_by_id(cfg: &Config, job_id: &str, output_codec: &str) -> Result<Job> {
    let url = format!("{}/jobs/{}", cfg.queue_url, job_id);
    let resp = with_auth(http().get(&url), cfg)
        .send()
        .context("fetch job by id")?;
    if !resp.status().is_success() {
        bail!("GET /jobs/{} returned {}", job_id, resp.status());
    }
    let mut job: Job = resp.json().context("parse job")?;
    if job.output_codec.is_empty() {
        job.output_codec = output_codec.to_string();
    }
    Ok(job)
}

fn ws_worker_loop(
    cfg: Config,
    job_tx: std::sync::mpsc::Sender<Job>,
    hb_state: Arc<Mutex<HeartbeatState>>,
) {
    use tungstenite::{connect, Message};

    loop {
        let server = &cfg.queue_url;
        let ws_url = if server.starts_with("https://") {
            format!(
                "wss://{}/ws/worker/{}",
                &server["https://".len()..],
                cfg.worker_name_url
            )
        } else {
            let base = server.trim_start_matches("http://");
            format!("ws://{}/ws/worker/{}", base, cfg.worker_name_url)
        };
        let ws_url = if let Some(t) = &cfg.queue_token {
            format!("{}?token={}", ws_url, t)
        } else {
            ws_url
        };

        let display_url = ws_url.split('?').next().unwrap_or(&ws_url).to_string();
        log_info(&cfg, None, &format!("WS connecting to {}", display_url));

        match connect(&ws_url) {
            Ok((mut socket, _)) => {
                WS_CONNECTED.store(true, Ordering::SeqCst);
                log_info(&cfg, None, "WS connected");

                // Send hello with capabilities.
                let hello = serde_json::json!({
                    "type": "hello",
                    "name": cfg.worker_name,
                    "platform": current_platform(),
                    "version": VERSION,
                    "capabilities": {
                        "encoders": {
                            "av1": &cfg.encoder_av1,
                            "hevc": &cfg.encoder_hevc,
                            "h264": &cfg.encoder_h264,
                        },
                        "decoders": ["av1", "hevc", "h264"],
                        "ffprobe_available": true,
                    }
                });
                if socket.send(Message::Text(hello.to_string())).is_err() {
                    WS_CONNECTED.store(false, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_secs(15));
                    continue;
                }

                // Set a read timeout so the heartbeat loop doesn't block indefinitely.
                // MaybeTlsStream wraps TcpStream; match on the Plain variant for the common
                // LAN (ws://) case; silently skip for TLS where we can't reach the inner stream.
                {
                    use tungstenite::stream::MaybeTlsStream;
                    if let MaybeTlsStream::Plain(ref tcp) = *socket.get_mut() {
                        tcp.set_read_timeout(Some(Duration::from_secs(5))).ok();
                    }
                }
                let mut last_heartbeat = Instant::now();

                loop {
                    match socket.read() {
                        Ok(Message::Text(text)) => {
                            if let Ok(msg) =
                                serde_json::from_str::<serde_json::Value>(&text)
                            {
                                match msg
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                {
                                    "welcome" => {
                                        log_info(&cfg, None, "WS welcome received");
                                    }
                                    "assign_encode" => {
                                        let job_id = msg["job_id"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string();
                                        let output_codec = msg["output_codec"]
                                            .as_str()
                                            .unwrap_or("av1")
                                            .to_string();
                                        log_info(
                                            &cfg,
                                            Some(&job_id),
                                            &format!(
                                                "WS assigned job (codec={})",
                                                output_codec
                                            ),
                                        );
                                        match fetch_job_by_id(
                                            &cfg,
                                            &job_id,
                                            &output_codec,
                                        ) {
                                            Ok(job) => {
                                                let _ = job_tx.send(job);
                                            }
                                            Err(e) => {
                                                log_warn(
                                                    &cfg,
                                                    Some(&job_id),
                                                    &format!(
                                                        "WS: failed to fetch job details: {}",
                                                        e
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                    "control" => {
                                        // Control messages handled by existing poll mechanism.
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Ok(Message::Ping(d)) => {
                            let _ = socket.send(Message::Pong(d));
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(ref e))
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            // Expected: read timeout, continue to heartbeat check.
                        }
                        Err(e) => {
                            log_warn(
                                &cfg,
                                None,
                                &format!("WS read error: {}", e),
                            );
                            break;
                        }
                    }

                    if last_heartbeat.elapsed().as_secs() >= 30 {
                        let state = hb_state.lock().unwrap().clone();
                        let hb = serde_json::json!({
                            "type": "heartbeat",
                            "status": state.status,
                            "current_job": state.current_job,
                        });
                        if socket
                            .send(Message::Text(hb.to_string()))
                            .is_err()
                        {
                            break;
                        }
                        last_heartbeat = Instant::now();
                    }
                }

                WS_CONNECTED.store(false, Ordering::SeqCst);
                log_info(&cfg, None, "WS disconnected");
            }
            Err(e) => {
                log_info(
                    &cfg,
                    None,
                    &format!("WS unavailable: {} — using HTTP polling", e),
                );
            }
        }

        std::thread::sleep(Duration::from_secs(15));
    }
}

fn should_log_idle_state(
    last_idle_log: &mut Option<Instant>,
    last_idle_state: &mut Option<String>,
    state: &str,
) -> bool {
    let state_changed = last_idle_state.as_deref() != Some(state);
    let interval_due = last_idle_log
        .map(|last| last.elapsed() >= IDLE_LOG_INTERVAL)
        .unwrap_or(true);
    if state_changed || interval_due {
        *last_idle_log = Some(Instant::now());
        *last_idle_state = Some(state.to_string());
        true
    } else {
        false
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(cmd) = args.get(1) {
        match cmd.as_str() {
            "--version" | "-V" => {
                println!("yulia-worker {}", VERSION);
                return;
            }
            "diagnostics" => {
                let mut cfg = Config::from_env();
                init_log(&cfg.log_dir);
                let mut test_cfg_av1 = cfg.clone();
                let encoder_av1 = detect_encoder(&test_cfg_av1, &OutputCodec::Av1);
                let mut test_cfg_hevc = cfg.clone();
                let encoder_hevc = detect_encoder(&test_cfg_hevc, &OutputCodec::Hevc);
                let mut test_cfg_h264 = cfg.clone();
                let encoder_h264 = detect_encoder(&test_cfg_h264, &OutputCodec::H264);
                let _ = (&mut test_cfg_av1, &mut test_cfg_hevc, &mut test_cfg_h264);
                cfg.encoder_av1 = encoder_av1.clone();
                cfg.encoder_hevc = encoder_hevc;
                cfg.encoder_h264 = encoder_h264;
                cfg.apply_detected_encoder(encoder_av1);
                std::process::exit(if run_diagnostics(&cfg) { 0 } else { 1 });
            }
            other => {
                eprintln!("Unknown argument: {}", other);
                eprintln!("Usage: yulia-worker [--version | diagnostics]");
                std::process::exit(1);
            }
        }
    }

    let mut cfg = Config::from_env();
    fs::create_dir_all(&cfg.work_dir).expect("create work dir");
    init_log(&cfg.log_dir);
    warn_env_file_permissions(&cfg, &cfg.env_file_path);

    if let Err(e) = check_binaries(&cfg) {
        log_error(&cfg, None, &format!("FATAL: {}", e));
        std::process::exit(1);
    }

    {
        let mut test_cfg_av1 = cfg.clone();
        let encoder_av1 = detect_encoder(&test_cfg_av1, &OutputCodec::Av1);
        let mut test_cfg_hevc = cfg.clone();
        let encoder_hevc = detect_encoder(&test_cfg_hevc, &OutputCodec::Hevc);
        let mut test_cfg_h264 = cfg.clone();
        let encoder_h264 = detect_encoder(&test_cfg_h264, &OutputCodec::H264);
        // Suppress unused_mut warnings — clones are passed as &mut to detect_encoder
        // but detection only reads cfg; assignments below ensure the names are used.
        let _ = (&mut test_cfg_av1, &mut test_cfg_hevc, &mut test_cfg_h264);
        cfg.encoder_av1 = encoder_av1.clone();
        cfg.encoder_hevc = encoder_hevc;
        cfg.encoder_h264 = encoder_h264;
        // Set cfg.encoder to the AV1 encoder so build_encode_args has a sane default
        // for legacy callers; the per-job path uses encoder_av1/hevc/h264 directly.
        cfg.apply_detected_encoder(encoder_av1);
    }

    log_info(
        &cfg,
        None,
        &format!(
            "yulia-worker {} starting — queue: {} encoder(av1)={} encoder(hevc)={} encoder(h264)={} token: {}",
            VERSION,
            cfg.queue_url,
            cfg.encoder_av1,
            cfg.encoder_hevc,
            cfg.encoder_h264,
            if cfg.queue_token.is_some() {
                "set"
            } else {
                "unset"
            }
        ),
    );

    cleanup_stale(&cfg);

    let ffmpeg_child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let hb_state: Arc<Mutex<HeartbeatState>> = Arc::new(Mutex::new(HeartbeatState::idle()));

    // heartbeat thread
    {
        let cfg = cfg.clone();
        let hb_state = Arc::clone(&hb_state);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(30));
            send_heartbeat(&cfg, &hb_state.lock().unwrap().clone());
        });
    }

    // control watcher — kills ffmpeg on stop
    {
        let cfg = cfg.clone();
        let ffmpeg_child = Arc::clone(&ffmpeg_child);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(5));
            if poll_control(&cfg) == ControlCmd::Stop {
                let mut guard = ffmpeg_child.lock().unwrap();
                if let Some(child) = guard.as_mut() {
                    let _ = child.kill();
                    log_warn(&cfg, None, "Stop command received — killed ffmpeg");
                }
            }
        });
    }

    // WS client thread + channel for WS-assigned jobs
    let (ws_job_tx, ws_job_rx) = std::sync::mpsc::channel::<Job>();
    if cfg.ws_enabled {
        let cfg_ws = cfg.clone();
        let hb_state_ws = Arc::clone(&hb_state);
        std::thread::spawn(move || {
            ws_worker_loop(cfg_ws, ws_job_tx, hb_state_ws);
        });
    }

    send_heartbeat(&cfg, &HeartbeatState::idle());

    let mut last_idle_log: Option<Instant> = None;
    let mut last_idle_state: Option<String> = None;

    loop {
        if AUTH_HALT.load(Ordering::SeqCst) {
            log_error(
                &cfg,
                None,
                "Authentication failure — sleeping 30s then exiting (code 2). Fix QUEUE_TOKEN and restart.",
            );
            eprintln!(
                "ERROR: Authentication failure. Check QUEUE_TOKEN in {} and restart.",
                cfg.env_file_path
            );
            std::thread::sleep(Duration::from_secs(30));
            std::process::exit(2);
        }

        let cmd = poll_control(&cfg);
        match cmd {
            ControlCmd::Stop | ControlCmd::Drain => {
                let label = if cmd == ControlCmd::Stop {
                    "stop"
                } else {
                    "drain"
                };
                if should_log_idle_state(&mut last_idle_log, &mut last_idle_state, label) {
                    log_warn(&cfg, None, &format!("Command is {} — worker idle", label));
                }
                *hb_state.lock().unwrap() = HeartbeatState::drain();
                send_heartbeat(&cfg, &HeartbeatState::drain());
                std::thread::sleep(Duration::from_secs(cfg.poll_secs));
                continue;
            }
            ControlCmd::Run => {}
        }

        // Check for WS-assigned jobs first.
        let ws_job = ws_job_rx.try_recv().ok();
        if let Some(job) = ws_job {
            rotate_log_if_needed(&cfg);
            last_idle_state = Some("encoding".to_string());
            log_info(&cfg, Some(&job.id), "WS-assigned job claimed");
            let hs = HeartbeatState::encoding(&job.id, &job.id);
            *hb_state.lock().unwrap() = hs.clone();
            send_heartbeat(&cfg, &hs);

            match process_job(&cfg, &job, &ffmpeg_child) {
                Ok(true) => {
                    log_info(&cfg, Some(&job.id), "Job complete");
                }
                Ok(false) => {
                    log_error(
                        &cfg,
                        Some(&job.id),
                        "Upload succeeded but done report failed — retained locally",
                    );
                }
                Err(e) => {
                    log_error(&cfg, Some(&job.id), &format!("Job FAILED: {:#}", e));
                    report_failed(&cfg, &job.id, &format!("{:#}", e));
                }
            }
            *hb_state.lock().unwrap() = HeartbeatState::idle();
            send_heartbeat(&cfg, &HeartbeatState::idle());
            if should_log_idle_state(&mut last_idle_log, &mut last_idle_state, "idle") {
                log_info(&cfg, None, "Worker idle");
            }
            continue;
        }

        match poll_job(&cfg) {
            Ok(Some(job)) => {
                rotate_log_if_needed(&cfg);
                last_idle_state = Some("encoding".to_string());
                log_info(&cfg, Some(&job.id), "Job claimed");
                let hs = HeartbeatState::encoding(&job.id, &job.id);
                *hb_state.lock().unwrap() = hs.clone();
                send_heartbeat(&cfg, &hs);

                match process_job(&cfg, &job, &ffmpeg_child) {
                    Ok(true) => {
                        log_info(&cfg, Some(&job.id), "Job complete");
                    }
                    Ok(false) => {
                        log_error(
                            &cfg,
                            Some(&job.id),
                            "Upload succeeded but done report failed — retained locally",
                        );
                    }
                    Err(e) => {
                        log_error(&cfg, Some(&job.id), &format!("Job FAILED: {:#}", e));
                        report_failed(&cfg, &job.id, &format!("{:#}", e));
                    }
                }
                *hb_state.lock().unwrap() = HeartbeatState::idle();
                send_heartbeat(&cfg, &HeartbeatState::idle());
                if should_log_idle_state(&mut last_idle_log, &mut last_idle_state, "idle") {
                    log_info(&cfg, None, "Worker idle");
                }
            }
            Ok(None) => {
                *hb_state.lock().unwrap() = HeartbeatState::idle();
                if should_log_idle_state(&mut last_idle_log, &mut last_idle_state, "idle") {
                    log_info(&cfg, None, "Worker idle; no jobs available");
                }
                std::thread::sleep(Duration::from_secs(cfg.poll_secs));
            }
            Err(e) => {
                if should_log_idle_state(&mut last_idle_log, &mut last_idle_state, "poll_error") {
                    log_warn(&cfg, None, &format!("Poll error: {}", e));
                }
                std::thread::sleep(Duration::from_secs(cfg.poll_secs));
            }
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            queue_url: "http://localhost:8090".into(),
            queue_token: None,
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            work_dir: PathBuf::from("/tmp/yulia-worker-test/jobs"),
            log_dir: PathBuf::from("/tmp/yulia-worker-test/logs"),
            worker_name: "test-worker".into(),
            worker_name_url: "test-worker".into(),
            poll_secs: 1,
            encoder: String::new(),
            encoder_av1: String::new(),
            encoder_hevc: String::new(),
            encoder_h264: String::new(),
            encode_quality: "28".into(),
            encode_preset: "medium".into(),
            audio_codec: "aac".into(),
            audio_bitrate: "192k".into(),
            vaapi_device: "/dev/dri/renderD128".into(),
            env_file_path: "/tmp/yulia-worker-test/worker.env".into(),
            encoder_explicit: false,
            encode_preset_explicit: false,
            ws_enabled: false,
        }
    }

    fn arg_after(args: &[String], flag: &str) -> Option<String> {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
    }

    #[test]
    fn test_sanitize_worker_name() {
        assert_eq!(sanitize_worker_name("my-worker_01"), "my-worker_01");
        assert_eq!(sanitize_worker_name("worker name"), "worker_name");
        assert_eq!(
            sanitize_worker_name("worker@host.local"),
            "worker_host.local"
        );
        assert_eq!(sanitize_worker_name(""), "");
    }

    #[test]
    fn test_iso8601_utc_from_unix_secs() {
        assert_eq!(iso8601_utc_from_unix_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            iso8601_utc_from_unix_secs(1_781_100_181),
            "2026-06-10T14:03:01Z"
        );
    }

    #[test]
    fn test_format_log_line_with_job() {
        assert_eq!(
            format_log_line(
                "2026-06-10T14:23:01Z",
                "INFO",
                "MY-PC",
                Some("abc123"),
                "encoder=av1_qsv Transcoding..."
            ),
            "[2026-06-10T14:23:01Z] INFO  worker=MY-PC job=abc123 encoder=av1_qsv Transcoding..."
        );
    }

    #[test]
    fn test_parse_env_file_basic() {
        let content = "# comment\nQUEUE_URL=http://localhost:8090\nQUEUE_TOKEN=mytoken\n\nENCODE_QUALITY=30\n";
        let map = parse_env_file(content);
        assert_eq!(map["QUEUE_URL"], "http://localhost:8090");
        assert_eq!(map["QUEUE_TOKEN"], "mytoken");
        assert_eq!(map["ENCODE_QUALITY"], "30");
        assert!(!map.contains_key("# comment"));
    }

    #[test]
    fn test_parse_env_file_quoted() {
        let content = "TOKEN=\"my secret token\"\nPRESET='fast'\n";
        let map = parse_env_file(content);
        assert_eq!(map["TOKEN"], "my secret token");
        assert_eq!(map["PRESET"], "fast");
    }

    #[test]
    fn test_parse_env_file_empty_value() {
        let map = parse_env_file("KEY=\n");
        assert_eq!(map["KEY"], "");
    }

    #[test]
    fn test_quality_flag() {
        // Intel QSV
        assert_eq!(quality_flag("av1_qsv"),  "-global_quality");
        assert_eq!(quality_flag("hevc_qsv"), "-global_quality");
        assert_eq!(quality_flag("h264_qsv"), "-global_quality");
        // NVIDIA NVENC
        assert_eq!(quality_flag("av1_nvenc"),  "-cq");
        assert_eq!(quality_flag("hevc_nvenc"), "-cq");
        assert_eq!(quality_flag("h264_nvenc"), "-cq");
        // AMD AMF
        assert_eq!(quality_flag("av1_amf"),  "-qp");
        assert_eq!(quality_flag("hevc_amf"), "-qp");
        assert_eq!(quality_flag("h264_amf"), "-qp");
        // VAAPI (Linux generic HW)
        assert_eq!(quality_flag("av1_vaapi"),  "-qp");
        assert_eq!(quality_flag("hevc_vaapi"), "-qp");
        assert_eq!(quality_flag("h264_vaapi"), "-qp");
        // Apple VideoToolbox
        assert_eq!(quality_flag("hevc_videotoolbox"), "-q:v");
        assert_eq!(quality_flag("h264_videotoolbox"), "-q:v");
        // Software fallback
        assert_eq!(quality_flag("libsvtav1"), "-crf");
        assert_eq!(quality_flag("libx265"),   "-crf");
        assert_eq!(quality_flag("libx264"),   "-crf");
    }

    #[test]
    fn test_encoder_uses_preset() {
        // These accept -preset
        assert!(encoder_uses_preset("av1_qsv"));
        assert!(encoder_uses_preset("av1_nvenc"));
        assert!(encoder_uses_preset("libsvtav1"));
        assert!(encoder_uses_preset("libx265"));
        assert!(encoder_uses_preset("libx264"));
        // These do NOT
        assert!(!encoder_uses_preset("av1_vaapi"));
        assert!(!encoder_uses_preset("hevc_vaapi"));
        assert!(!encoder_uses_preset("av1_amf"));
        assert!(!encoder_uses_preset("hevc_amf"));
        assert!(!encoder_uses_preset("h264_amf"));
        assert!(!encoder_uses_preset("hevc_videotoolbox"));
        assert!(!encoder_uses_preset("h264_videotoolbox"));
    }

    #[test]
    fn test_output_codec_from_str() {
        assert_eq!(OutputCodec::from_str("av1"), OutputCodec::Av1);
        assert_eq!(OutputCodec::from_str(""), OutputCodec::Av1);
        assert_eq!(OutputCodec::from_str("AV1"), OutputCodec::Av1);
        assert_eq!(OutputCodec::from_str("hevc"), OutputCodec::Hevc);
        assert_eq!(OutputCodec::from_str("h265"), OutputCodec::Hevc);
        assert_eq!(OutputCodec::from_str("H.265"), OutputCodec::Hevc);
        assert_eq!(OutputCodec::from_str("h264"), OutputCodec::H264);
        assert_eq!(OutputCodec::from_str("avc"), OutputCodec::H264);
        assert_eq!(OutputCodec::from_str("H.264"), OutputCodec::H264);
    }

    #[test]
    fn test_output_codec_expected_name() {
        assert_eq!(OutputCodec::Av1.expected_codec_name(), "av1");
        assert_eq!(OutputCodec::Hevc.expected_codec_name(), "hevc");
        assert_eq!(OutputCodec::H264.expected_codec_name(), "h264");
    }

    #[test]
    fn test_output_codec_preferred_encoders() {
        assert!(OutputCodec::Av1.preferred_encoders().contains(&"libsvtav1"));
        assert!(OutputCodec::Hevc.preferred_encoders().contains(&"libx265"));
        assert!(OutputCodec::H264.preferred_encoders().contains(&"libx264"));
    }

    #[test]
    fn test_duration_validation_logic() {
        let source = 100.0_f64;
        assert!((103.0_f64 - source).abs() > 2.0, "3s diff must fail");
        assert!((101.5_f64 - source).abs() <= 2.0, "1.5s diff must pass");
        assert!((98.1_f64 - source).abs() <= 2.0, "1.9s diff must pass");
    }

    #[test]
    fn test_build_encode_args_qsv() {
        let mut cfg = test_config();
        cfg.encoder = "av1_qsv".into();
        cfg.encode_quality = "28".into();
        cfg.encode_preset = "medium".into();
        let args = build_encode_args(&cfg, "in.mp4", "out.mp4");
        assert_eq!(arg_after(&args, "-c:v").as_deref(), Some("av1_qsv"));
        assert_eq!(arg_after(&args, "-global_quality").as_deref(), Some("28"));
        assert_eq!(arg_after(&args, "-preset").as_deref(), Some("medium"));
        assert!(!args.contains(&"-vaapi_device".to_string()));
    }

    #[test]
    fn test_build_encode_args_vaapi_no_preset() {
        let mut cfg = test_config();
        cfg.encoder = "av1_vaapi".into();
        let args = build_encode_args(&cfg, "in.mp4", "out.mp4");
        assert_eq!(
            arg_after(&args, "-vaapi_device").as_deref(),
            Some("/dev/dri/renderD128")
        );
        assert_eq!(
            arg_after(&args, "-vf").as_deref(),
            Some("format=nv12,hwupload")
        );
        assert_eq!(arg_after(&args, "-c:v").as_deref(), Some("av1_vaapi"));
        assert_eq!(arg_after(&args, "-qp").as_deref(), Some("28"));
        assert!(!args.contains(&"-preset".to_string()));
    }

    #[test]
    fn test_build_encode_args_nvenc() {
        let mut cfg = test_config();
        cfg.encoder = "av1_nvenc".into();
        cfg.encode_preset = "p4".into();
        let args = build_encode_args(&cfg, "in.mp4", "out.mp4");
        assert_eq!(arg_after(&args, "-c:v").as_deref(), Some("av1_nvenc"));
        assert_eq!(arg_after(&args, "-cq").as_deref(), Some("28"));
        assert_eq!(arg_after(&args, "-preset").as_deref(), Some("p4"));
        assert!(!args.contains(&"-vaapi_device".to_string()));
    }

    #[test]
    fn test_build_encode_args_libsvtav1() {
        let mut cfg = test_config();
        cfg.encoder = "libsvtav1".into();
        cfg.encode_preset = "6".into();
        let args = build_encode_args(&cfg, "in.mp4", "out.mp4");
        assert_eq!(arg_after(&args, "-c:v").as_deref(), Some("libsvtav1"));
        assert_eq!(arg_after(&args, "-crf").as_deref(), Some("28"));
        assert_eq!(arg_after(&args, "-preset").as_deref(), Some("6"));
        assert!(!args.contains(&"-vaapi_device".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn test_detect_encoder_falls_through_to_libsvtav1() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let ffmpeg = dir.path().join("fake-ffmpeg");
        fs::write(
            &ffmpeg,
            r#"#!/bin/sh
args="$*"
case "$args" in
  *av1_qsv*) echo "qsv unavailable" >&2; exit 1 ;;
  *av1_vaapi*) echo "vaapi unavailable" >&2; exit 1 ;;
  *av1_nvenc*) echo "nvenc unavailable" >&2; exit 1 ;;
  *libsvtav1*) exit 0 ;;
esac
echo "unexpected args: $args" >&2
exit 1
"#,
        )
        .unwrap();
        let mut perms = fs::metadata(&ffmpeg).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&ffmpeg, perms).unwrap();

        let mut cfg = test_config();
        cfg.ffmpeg = ffmpeg.to_string_lossy().into_owned();
        assert_eq!(detect_encoder(&cfg, &OutputCodec::Av1), "libsvtav1");
    }

    #[test]
    fn test_load_env_file_missing_is_empty() {
        let map = load_env_file("/nonexistent/path/worker.env");
        assert!(map.is_empty());
    }

    #[test]
    fn test_load_env_file_from_tempfile() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "MY_KEY=hello\nOTHER=world").unwrap();
        let map = load_env_file(f.path().to_str().unwrap());
        assert_eq!(map["MY_KEY"], "hello");
        assert_eq!(map["OTHER"], "world");
    }
}

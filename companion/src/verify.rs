use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

pub struct VideoInfo {
    pub codec: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub audio_codec: Option<String>,
}

pub fn probe(path: &Path) -> Result<VideoInfo> {
    let ffprobe = find_ffprobe();

    // Single call for all video stream fields
    let video_raw = run_ffprobe(&ffprobe, path, &[
        "-select_streams", "v:0",
        "-show_entries", "stream=codec_name,width,height,r_frame_rate",
    ])?;
    let mut vlines = video_raw.lines();
    let codec   = vlines.next().unwrap_or("").trim().to_string();
    let width:  u32 = vlines.next().unwrap_or("0").trim().parse().unwrap_or(0);
    let height: u32 = vlines.next().unwrap_or("0").trim().parse().unwrap_or(0);
    let fps = parse_fps(vlines.next().unwrap_or("0"));

    let duration_str = run_ffprobe(&ffprobe, path, &[
        "-show_entries", "format=duration",
    ])?;
    let duration: f64 = duration_str.trim().parse().context("parse duration")?;

    let audio_codec = run_ffprobe(&ffprobe, path, &[
        "-select_streams", "a:0",
        "-show_entries", "stream=codec_name",
    ]).ok().filter(|s| !s.is_empty());

    Ok(VideoInfo { codec, duration, width, height, fps, audio_codec })
}

fn parse_fps(s: &str) -> f64 {
    let s = s.trim();
    if let Some((n, d)) = s.split_once('/') {
        let n: f64 = n.trim().parse().unwrap_or(0.0);
        let d: f64 = d.trim().parse().unwrap_or(1.0);
        if d > 0.0 { n / d } else { 0.0 }
    } else {
        s.parse().unwrap_or(0.0)
    }
}

pub fn verify_output(output: &Path, source_duration: f64) -> Result<()> {
    let info = probe(output).context("ffprobe on output")?;
    if info.codec != "av1" {
        bail!("output codec is '{}', expected av1", info.codec);
    }
    let diff = (info.duration - source_duration).abs();
    if diff > 2.0 {
        bail!(
            "duration mismatch: source={:.1}s output={:.1}s diff={:.1}s",
            source_duration, info.duration, diff
        );
    }
    Ok(())
}

fn run_ffprobe(ffprobe: &str, path: &Path, extra_args: &[&str]) -> Result<String> {
    let mut args = vec!["-v", "error", "-of", "default=noprint_wrappers=1:nokey=1"];
    args.extend_from_slice(extra_args);
    args.push(path.to_str().unwrap());
    let out = Command::new(ffprobe)
        .args(&args)
        .output()
        .context("spawn ffprobe")?;
    if !out.status.success() {
        bail!("ffprobe: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn find_ffprobe() -> String {
    for candidate in &["ffprobe", "/opt/homebrew/bin/ffprobe", "/usr/local/bin/ffprobe"] {
        if Command::new(candidate).arg("-version").output().is_ok() {
            return candidate.to_string();
        }
    }
    "ffprobe".to_string()
}

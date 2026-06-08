use anyhow::{bail, Context, Result};
use indicatif::ProgressBar;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct UploadResponse {
    pub job_id: String,
    pub priority_position: u64,
}

#[derive(Debug, Deserialize)]
pub struct Job {
    pub id: String,
    pub status: String,
    pub percent: Option<f64>,
    pub fps: Option<f64>,
    pub speed: Option<String>,
    pub worker: Option<String>,
    pub error: Option<String>,
    pub output_size: Option<u64>,
    pub source_size: Option<u64>,
    pub verify_status: Option<String>,
    pub verify_detail: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QueueStatus {
    pub pending: u64,
    pub active: u64,
    pub done: u64,
    pub failed: u64,
}

struct ProgressReader<R> {
    inner: R,
    bar: ProgressBar,
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bar.inc(n as u64);
        Ok(n)
    }
}

fn client_upload() -> Client {
    Client::builder()
        .timeout(None)
        .build()
        .expect("http client")
}

fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client")
}

pub fn upload_file(server_url: &str, path: &Path, bar: &ProgressBar) -> Result<UploadResponse> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.mp4");
    let filepath = path.to_string_lossy();
    let file = File::open(path).context("open file for upload")?;
    let size = file.metadata()?.len();
    bar.set_length(size);

    let reader = ProgressReader {
        inner: file,
        bar: bar.clone(),
    };

    let resp = client_upload()
        .post(format!("{}/jobs/upload", server_url))
        .header("X-Filename", filename)
        .header("X-Filepath", filepath.as_ref())
        .header("Content-Length", size)
        .body(reqwest::blocking::Body::new(reader))
        .send()
        .context("POST /jobs/upload")?;

    if !resp.status().is_success() {
        bail!("upload failed: HTTP {}", resp.status());
    }
    resp.json::<UploadResponse>().context("parse upload response")
}

pub fn poll_job(server_url: &str, job_id: &str) -> Result<Job> {
    let resp = client()
        .get(format!("{}/jobs/{}", server_url, job_id))
        .send()
        .context("GET /jobs/{id}")?;
    if !resp.status().is_success() {
        bail!("poll failed: HTTP {}", resp.status());
    }
    resp.json::<Job>().context("parse job response")
}

pub fn download_output(server_url: &str, job_id: &str, dest: &Path, bar: &ProgressBar) -> Result<()> {
    let mut resp = client_upload()
        .get(format!("{}/jobs/{}/output", server_url, job_id))
        .send()
        .context("GET /jobs/{id}/output")?;

    if !resp.status().is_success() {
        bail!("download failed: HTTP {}", resp.status());
    }

    if let Some(len) = resp.content_length() {
        bar.set_length(len);
    }

    let file = File::create(dest).context("create output file")?;
    let mut writer = BufWriter::new(file);
    let mut buf = vec![0u8; 1 << 20];

    loop {
        let n = resp.read(&mut buf).context("read response")?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).context("write output")?;
        bar.inc(n as u64);
    }
    writer.flush()?;
    Ok(())
}

pub fn queue_status(server_url: &str) -> Result<QueueStatus> {
    let resp = client()
        .get(format!("{}/status", server_url))
        .send()
        .context("GET /status")?;
    resp.json::<QueueStatus>().context("parse status")
}

#[derive(Debug, Deserialize, Clone)]
pub struct LiveJob {
    pub file: String,
    #[serde(default)]
    pub percent: f64,
    #[serde(default)]
    pub fps: f64,
    #[serde(default)]
    pub speed: String,
    #[serde(default)]
    pub phase: String,
}

pub fn live_jobs(server_url: &str) -> Result<std::collections::HashMap<String, LiveJob>> {
    let resp = client()
        .get(format!("{}/jobs/live", server_url))
        .send()
        .context("GET /jobs/live")?;
    resp.json().context("parse live jobs")
}

pub fn control_status(server_url: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Ctrl { command: String }
    let resp = client()
        .get(format!("{}/control", server_url))
        .send()
        .context("GET /control")?;
    Ok(resp.json::<Ctrl>().context("parse control")?.command)
}

pub fn set_control(server_url: &str, cmd: &str) -> Result<()> {
    client()
        .post(format!("{}/control/{}", server_url, cmd))
        .send()
        .context("POST /control")?;
    Ok(())
}

pub fn post_queue_manifest(server_url: &str, files: &[String]) -> Result<()> {
    let body = serde_json::json!({ "files": files });
    client()
        .post(format!("{}/clients/queue-manifest", server_url))
        .json(&body)
        .send()
        .context("POST /clients/queue-manifest")?;
    Ok(())
}

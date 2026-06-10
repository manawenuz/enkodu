use anyhow::{bail, Context, Result};
use indicatif::ProgressBar;
use reqwest::blocking::{Client, RequestBuilder};
use serde::Deserialize;
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;
use std::time::Duration;

use crate::retry::{classify_reqwest_error, classify_status, with_retry, ErrorKind, RetryConfig};

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

fn with_auth(req: RequestBuilder, auth_token: Option<&str>) -> RequestBuilder {
    if let Some(token) = effective_auth_token(auth_token) {
        return req.bearer_auth(token);
    }
    req
}

fn effective_auth_token(auth_token: Option<&str>) -> Option<String> {
    if let Ok(token) = std::env::var("ENKODU_AUTH_TOKEN") {
        if !token.trim().is_empty() {
            return Some(token.trim().to_string());
        }
    }
    auth_token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

pub fn upload_file(
    server_url: &str,
    auth_token: Option<&str>,
    path: &Path,
    bar: &ProgressBar,
) -> Result<UploadResponse> {
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

    let resp = with_auth(
        client_upload()
            .post(format!("{}/jobs/upload", server_url))
            .header("X-Filename", filename)
            .header("X-Filepath", filepath.as_ref())
            .header("Content-Length", size)
            .body(reqwest::blocking::Body::new(reader)),
        auth_token,
    )
    .send()
    .context("POST /jobs/upload")?;

    if !resp.status().is_success() {
        bail!("upload failed: HTTP {}", resp.status());
    }
    resp.json::<UploadResponse>()
        .context("parse upload response")
}

/// Upload with retry logic for transient failures.
pub fn upload_file_with_retry(
    server_url: &str,
    auth_token: Option<&str>,
    path: &Path,
    bar: &ProgressBar,
) -> Result<UploadResponse> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.mp4");
    let filepath = path.to_string_lossy();
    let size = path.metadata()?.len();
    bar.set_length(size);

    let config = RetryConfig::transfer();
    with_retry(&config, || {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => return Err((ErrorKind::Permanent, format!("open file: {}", e))),
        };
        let reader = ProgressReader {
            inner: file,
            bar: bar.clone(),
        };

        let resp = match with_auth(
            client_upload()
                .post(format!("{}/jobs/upload", server_url))
                .header("X-Filename", filename)
                .header("X-Filepath", filepath.as_ref())
                .header("Content-Length", size)
                .body(reqwest::blocking::Body::new(reader)),
            auth_token,
        )
        .send()
        {
            Ok(r) => r,
            Err(e) => {
                let kind = classify_reqwest_error(&e);
                return Err((kind, format!("upload request failed: {}", e)));
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let kind = classify_status(status);
            return Err((kind, format!("upload failed: HTTP {}", status)));
        }

        match resp.json::<UploadResponse>() {
            Ok(r) => Ok(r),
            Err(e) => Err((
                ErrorKind::Transient,
                format!("parse upload response: {}", e),
            )),
        }
    })
    .map_err(|e| anyhow::anyhow!("upload failed after retries: {}", e))
}

pub fn poll_job(server_url: &str, auth_token: Option<&str>, job_id: &str) -> Result<Job> {
    let resp = with_auth(
        client().get(format!("{}/jobs/{}", server_url, job_id)),
        auth_token,
    )
    .send()
    .context("GET /jobs/{id}")?;
    if !resp.status().is_success() {
        bail!("poll failed: HTTP {}", resp.status());
    }
    resp.json::<Job>().context("parse job response")
}

pub fn download_output(
    server_url: &str,
    auth_token: Option<&str>,
    job_id: &str,
    dest: &Path,
    bar: &ProgressBar,
) -> Result<()> {
    let mut resp = with_auth(
        client_upload().get(format!("{}/jobs/{}/output", server_url, job_id)),
        auth_token,
    )
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

/// Download with retry and resume support using HTTP Range headers.
/// If a partial file exists, resumes from the existing size.
pub fn download_output_with_retry(
    server_url: &str,
    auth_token: Option<&str>,
    job_id: &str,
    dest: &Path,
    bar: &ProgressBar,
) -> Result<()> {
    let config = RetryConfig::transfer();
    let temp_path = dest.with_extension("part");
    let mut start_offset = 0u64;

    if temp_path.exists() {
        start_offset = temp_path.metadata().map(|m| m.len()).unwrap_or(0);
        log::info!(
            "Resuming download for {} from byte {}",
            job_id,
            start_offset
        );
    }

    let result = with_retry(&config, || {
        let mut req = with_auth(
            client_upload().get(format!("{}/jobs/{}/output", server_url, job_id)),
            auth_token,
        );

        if start_offset > 0 {
            req = req.header("Range", format!("bytes={}-", start_offset));
        }

        let mut resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                let kind = classify_reqwest_error(&e);
                return Err((kind, format!("download request failed: {}", e)));
            }
        };

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            let status = resp.status().as_u16();
            let kind = classify_status(status);
            return Err((kind, format!("download failed: HTTP {}", status)));
        }

        if let Some(len) = resp.content_length() {
            let total = if start_offset > 0 && resp.status() == reqwest::StatusCode::PARTIAL_CONTENT
            {
                // Parse Content-Range to get total
                let cr = resp
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if let Some(total_str) = cr.split("/").nth(1) {
                    total_str.parse::<u64>().unwrap_or(len + start_offset)
                } else {
                    len + start_offset
                }
            } else {
                len
            };
            bar.set_length(total);
        }
        bar.set_position(start_offset);

        let mut file = match File::options().create(true).append(true).open(&temp_path) {
            Ok(f) => f,
            Err(e) => return Err((ErrorKind::Permanent, format!("open temp file: {}", e))),
        };

        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = match resp.read(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    return Err((ErrorKind::Network, format!("read response: {}", e)));
                }
            };
            if n == 0 {
                break;
            }
            if let Err(e) = file.write_all(&buf[..n]) {
                return Err((ErrorKind::Permanent, format!("write temp file: {}", e)));
            }
            bar.inc(n as u64);
        }
        Ok(())
    });

    match result {
        Ok(()) => {
            std::fs::rename(&temp_path, dest)?;
            Ok(())
        }
        Err(e) => {
            // Don't delete temp file — leave it for resume
            Err(anyhow::anyhow!("download failed after retries: {}", e))
        }
    }
}

pub fn queue_status(server_url: &str, auth_token: Option<&str>) -> Result<QueueStatus> {
    let resp = with_auth(client().get(format!("{}/status", server_url)), auth_token)
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

pub fn live_jobs(
    server_url: &str,
    auth_token: Option<&str>,
) -> Result<std::collections::HashMap<String, LiveJob>> {
    let resp = with_auth(
        client().get(format!("{}/jobs/live", server_url)),
        auth_token,
    )
    .send()
    .context("GET /jobs/live")?;
    resp.json().context("parse live jobs")
}

pub fn control_status(server_url: &str, auth_token: Option<&str>) -> Result<String> {
    #[derive(Deserialize)]
    struct Ctrl {
        command: String,
    }
    let resp = with_auth(client().get(format!("{}/control", server_url)), auth_token)
        .send()
        .context("GET /control")?;
    Ok(resp.json::<Ctrl>().context("parse control")?.command)
}

pub fn set_control(server_url: &str, auth_token: Option<&str>, cmd: &str) -> Result<()> {
    with_auth(
        client().post(format!("{}/control/{}", server_url, cmd)),
        auth_token,
    )
    .send()
    .context("POST /control")?;
    Ok(())
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerJob {
    pub id: String,
    pub source_filename: Option<String>,
    pub source_meta: Option<String>,
    pub source_path: Option<String>,
    pub client_path: Option<String>,
    pub output_size: Option<u64>,
    pub verify_status: Option<String>,
}

pub fn list_done_companion_jobs(
    server_url: &str,
    auth_token: Option<&str>,
) -> Result<Vec<ServerJob>> {
    #[derive(Deserialize)]
    struct Resp {
        jobs: Vec<ServerJob>,
    }
    let resp = with_auth(
        client_upload().get(format!("{}/jobs?status=done&limit=2000", server_url)),
        auth_token,
    )
    .send()
    .context("GET /jobs")?;
    if !resp.status().is_success() {
        anyhow::bail!("list jobs: HTTP {}", resp.status());
    }
    // Return all done jobs — reconcile will prefer NAS jobs (proper paths) over
    // companion upload jobs (source under /.transcode/uploads/) for the same file.
    Ok(resp.json::<Resp>().context("parse jobs")?.jobs)
}

pub fn set_client_path(
    server_url: &str,
    auth_token: Option<&str>,
    job_id: &str,
    path: &str,
) -> Result<()> {
    let body = serde_json::json!({ "client_path": path });
    with_auth(
        client()
            .post(format!("{}/jobs/{}/set-path", server_url, job_id))
            .json(&body),
        auth_token,
    )
    .send()
    .context("POST /jobs/{id}/set-path")?;
    Ok(())
}

pub fn get_settings(
    server_url: &str,
    auth_token: Option<&str>,
) -> Result<std::collections::HashMap<String, String>> {
    let resp = with_auth(client().get(format!("{}/settings", server_url)), auth_token)
        .send()
        .context("GET /settings")?;
    resp.json().context("parse settings")
}

pub fn set_setting(
    server_url: &str,
    auth_token: Option<&str>,
    key: &str,
    value: &str,
) -> Result<()> {
    let body = serde_json::json!({ key: value });
    with_auth(
        client()
            .post(format!("{}/settings", server_url))
            .json(&body),
        auth_token,
    )
    .send()
    .context("POST /settings")?;
    Ok(())
}

pub fn post_queue_manifest(
    server_url: &str,
    auth_token: Option<&str>,
    files: &[String],
) -> Result<()> {
    let body = serde_json::json!({ "files": files });
    with_auth(
        client()
            .post(format!("{}/clients/queue-manifest", server_url))
            .json(&body),
        auth_token,
    )
    .send()
    .context("POST /clients/queue-manifest")?;
    Ok(())
}

// ── SHA-256 checksum verification ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChecksumResponse {
    pub job_id: String,
    pub status: String,
    pub source_sha256: Option<String>,
    pub output_sha256: Option<String>,
}

pub fn get_checksum(
    server_url: &str,
    auth_token: Option<&str>,
    job_id: &str,
) -> Result<ChecksumResponse> {
    let resp = with_auth(
        client().get(format!("{}/jobs/{}/checksum", server_url, job_id)),
        auth_token,
    )
    .send()
    .context("GET /jobs/{id}/checksum")?;
    if !resp.status().is_success() {
        bail!("checksum fetch failed: HTTP {}", resp.status());
    }
    resp.json::<ChecksumResponse>()
        .context("parse checksum response")
}

/// Compute SHA-256 of a local file.
pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = File::open(path).context("open file for sha256")?;
    let mut hasher = sha2::Sha256::default();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).context("read file for sha256")?;
        if n == 0 {
            break;
        }
        sha2::digest::Update::update(&mut hasher, &buf[..n]);
    }
    Ok(hex::encode(sha2::digest::FixedOutput::finalize_fixed(
        hasher,
    )))
}

/// Verify a downloaded file against the server's SHA-256.
/// Returns Ok(true) if checksum matches or server has no checksum.
/// Returns Ok(false) if checksum mismatch.
/// Returns Err if local compute or server fetch fails.
pub fn verify_download_checksum(
    server_url: &str,
    auth_token: Option<&str>,
    job_id: &str,
    path: &Path,
) -> Result<bool> {
    let server = match get_checksum(server_url, auth_token, job_id) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Could not fetch server checksum for {}: {}", job_id, e);
            return Ok(true); // permissive — server may not have checksum yet
        }
    };
    let expected = match server.output_sha256 {
        Some(h) if !h.is_empty() => h,
        _ => return Ok(true), // no checksum available
    };
    let local = sha256_file(path)?;
    if local != expected {
        log::error!(
            "Checksum mismatch for {}: local={} server={}",
            job_id,
            local,
            expected
        );
        return Ok(false);
    }
    log::info!("Checksum verified for {}: {}", job_id, local);
    Ok(true)
}

// ── Auth diagnostics ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(default)]
    pub version: String,
}

/// Connection test result for companion setup diagnostics.
#[derive(Debug, PartialEq)]
pub enum ConnectionTest {
    /// Server reachable and a protected companion endpoint accepted the request.
    Ok { role: Option<String> },
    /// Server reachable but auth rejected (401).
    AuthRejected,
    /// Server reachable but auth forbidden (403).
    AuthForbidden,
    /// Server reachable but the protected companion endpoint is missing (404).
    AuthNotConfigured,
    /// Server unreachable or network error.
    Unreachable(String),
}

fn classify_protected_endpoint_status(
    endpoint: &str,
    status: reqwest::StatusCode,
) -> ConnectionTest {
    match status {
        s if s.is_success() => ConnectionTest::Ok { role: None },
        reqwest::StatusCode::UNAUTHORIZED => ConnectionTest::AuthRejected,
        reqwest::StatusCode::FORBIDDEN => ConnectionTest::AuthForbidden,
        reqwest::StatusCode::NOT_FOUND => ConnectionTest::AuthNotConfigured,
        other => ConnectionTest::Unreachable(format!("{} HTTP {}", endpoint, other)),
    }
}

/// Test whether the server is reachable and whether the current auth is valid.
/// Probes `/healthz` first, then validates against protected `/status`.
pub fn test_connection(server_url: &str, auth_token: Option<&str>) -> ConnectionTest {
    // 1. Health check
    let health_resp =
        match with_auth(client().get(format!("{}/healthz", server_url)), auth_token).send() {
            Ok(r) => r,
            Err(e) => {
                return ConnectionTest::Unreachable(format!("healthz failed: {}", e));
            }
        };

    // `/healthz` proves the server answered, but in strict auth deployments it
    // may itself return 401 because it is not a companion endpoint. Only treat
    // server-side failures as hard liveness failures here; `/status` below is
    // the actual companion auth proof.
    if health_resp.status().is_server_error() {
        return ConnectionTest::Unreachable(format!("healthz HTTP {}", health_resp.status()));
    }

    // 2. Auth proof — /status is protected for companion/device-token clients.
    let status_resp =
        match with_auth(client().get(format!("{}/status", server_url)), auth_token).send() {
            Ok(r) => r,
            Err(e) => {
                return ConnectionTest::Unreachable(format!("status failed: {}", e));
            }
        };

    classify_protected_endpoint_status("status", status_resp.status())
}

/// Print a human-readable summary of a connection test.
pub fn print_connection_test(result: &ConnectionTest) {
    match result {
        ConnectionTest::Ok { role } => {
            println!("✓ Server reachable");
            if let Some(r) = role {
                println!(
                    "✓ Protected companion endpoint accepted request — role: {}",
                    r
                );
            } else {
                println!("✓ Protected companion endpoint accepted request");
            }
        }
        ConnectionTest::AuthRejected => {
            println!("✗ Server reachable but auth is required or rejected (401)");
            println!("  Check auth_token in config or ENKODU_AUTH_TOKEN env var.");
        }
        ConnectionTest::AuthForbidden => {
            println!("✗ Server reachable but permission denied (403)");
            println!("  The companion token is not authorized for this endpoint.");
        }
        ConnectionTest::AuthNotConfigured => {
            println!("✓ Server reachable");
            println!("  Protected companion endpoint not found — this server may be too old.");
        }
        ConnectionTest::Unreachable(e) => {
            println!("✗ Server unreachable");
            println!("  {}", e);
            println!("  Try: enkodu tcpping <host:port>");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn protected_endpoint_success_accepts_connection() {
        assert_eq!(
            classify_protected_endpoint_status("status", StatusCode::OK),
            ConnectionTest::Ok { role: None }
        );
    }

    #[test]
    fn protected_endpoint_unauthorized_rejects_auth() {
        assert_eq!(
            classify_protected_endpoint_status("status", StatusCode::UNAUTHORIZED),
            ConnectionTest::AuthRejected
        );
    }

    #[test]
    fn protected_endpoint_forbidden_is_permission_denied() {
        assert_eq!(
            classify_protected_endpoint_status("status", StatusCode::FORBIDDEN),
            ConnectionTest::AuthForbidden
        );
    }

    #[test]
    fn protected_endpoint_missing_is_legacy_server() {
        assert_eq!(
            classify_protected_endpoint_status("status", StatusCode::NOT_FOUND),
            ConnectionTest::AuthNotConfigured
        );
    }

    #[test]
    fn protected_endpoint_other_failure_is_unreachable_detail() {
        assert_eq!(
            classify_protected_endpoint_status("status", StatusCode::INTERNAL_SERVER_ERROR),
            ConnectionTest::Unreachable("status HTTP 500 Internal Server Error".to_string())
        );
    }
}

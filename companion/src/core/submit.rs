//! Background file submission and download flow.

use log::{error, info, warn};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crate::api;
use crate::config::Config;
use crate::core::now_secs;
use crate::state;
use crate::verify;

/// Submit a file for transcoding.
/// This runs in a background thread.
pub fn submit_bg(cfg: Config, path: PathBuf) {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    info!("Probing: {}", name);
    let source_info = match verify::probe(&path) {
        Ok(i) => i,
        Err(e) => {
            error!("Cannot probe {}: {}", name, e);
            // Platform-specific notification will be called by caller
            return;
        }
    };

    if source_info.codec == "av1" {
        info!("Skipping {} — already AV1", name);
        // Platform-specific notification will be called by caller
        return;
    }

    info!(
        "Uploading {} ({:.1}s, {})",
        name, source_info.duration, source_info.codec
    );

    let hidden_bar = indicatif::ProgressBar::hidden();
    let upload = match api::upload_file(&cfg.server_url, &path, &hidden_bar) {
        Ok(u) => u,
        Err(e) => {
            error!("Upload failed for {}: {}", name, e);
            // Platform-specific notification will be called by caller
            return;
        }
    };

    info!(
        "Queued {} -> job {} (position {})",
        name, upload.job_id, upload.priority_position
    );
    let key = path.to_string_lossy().to_string();
    let _ = state::upsert(&key, state::JobEntry {
        job_id: upload.job_id.clone(),
        submitted_at: now_secs(),
        status: "pending".to_string(),
        output_path: None,
    });

    // poll until done
    loop {
        thread::sleep(Duration::from_secs(5));
        let job = match api::poll_job(&cfg.server_url, &upload.job_id) {
            Ok(j) => j,
            Err(e) => {
                warn!("Poll error for {}: {}", upload.job_id, e);
                continue;
            }
        };
        match job.status.as_str() {
            "done" => {
                let vs = job.verify_status.as_deref().unwrap_or("");
                if vs == "fail" {
                    error!(
                        "Verify failed for {}: {}",
                        name,
                        job.verify_detail.as_deref().unwrap_or("?")
                    );
                    // Platform-specific notification will be called by caller
                    return;
                }
                if vs == "running" {
                    continue;
                }
                info!("Job {} done — downloading output", upload.job_id);
                break;
            }
            "failed" => {
                error!(
                    "Job {} failed: {}",
                    upload.job_id,
                    job.error.as_deref().unwrap_or("?")
                );
                // Platform-specific notification will be called by caller
                return;
            }
            _ => continue,
        }
    }

    let output_name = path.with_file_name(format!(
        "{}_av1.mp4",
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("output")
    ));
    info!("Downloading output to {}", output_name.display());
    let dl_bar = indicatif::ProgressBar::hidden();
    if let Err(e) = api::download_output(&cfg.server_url, &upload.job_id, &output_name, &dl_bar) {
        error!("Download failed for {}: {}", name, e);
        // Platform-specific notification will be called by caller
        return;
    }

    info!("Local verify for {}", output_name.display());
    if let Err(e) = verify::verify_output(&output_name, source_info.duration) {
        error!("Local verify failed for {}: {}", name, e);
        // Platform-specific notification will be called by caller
        return;
    }

    let final_path = match cfg.behavior.on_success.as_str() {
        "replace" => {
            let bak = format!("{}{}", path.display(), cfg.behavior.backup_suffix);
            let _ = std::fs::rename(&path, &bak);
            let _ = std::fs::rename(&output_name, &path);
            path.clone()
        }
        _ => output_name.clone(),
    };

    let src_sz = path.metadata().map(|m| m.len()).unwrap_or(0);
    let out_sz = final_path.metadata().map(|m| m.len()).unwrap_or(0);
    let ratio = if out_sz > 0 {
        src_sz as f64 / out_sz as f64
    } else {
        0.0
    };

    let _ = state::upsert(&key, state::JobEntry {
        job_id: upload.job_id,
        submitted_at: now_secs(),
        status: "done".to_string(),
        output_path: Some(final_path.to_string_lossy().to_string()),
    });

    info!(
        "Done: {} — {:.2} GB output, {:.1}x smaller",
        name,
        out_sz as f64 / 1e9,
        ratio
    );
}

/// Recover pending downloads from a previous session.
pub fn recover_pending_downloads(cfg: Config) {
    let st = match state::load() {
        Ok(s) => s,
        Err(e) => {
            warn!("Recovery: could not load state: {}", e);
            return;
        }
    };

    let pending: Vec<(String, state::JobEntry)> = st
        .into_iter()
        .filter(|(_, e)| !matches!(e.status.as_str(), "done" | "failed"))
        .collect();

    if pending.is_empty() {
        info!("Recovery: no pending jobs");
        return;
    }

    info!("Recovery: {} unfinished job(s) found — resuming", pending.len());

    for (file_path, entry) in pending {
        let cfg = cfg.clone();
        thread::spawn(move || recover_one(cfg, file_path, entry.job_id));
    }
}

/// Recover a single job by downloading and verifying its output.
pub fn recover_one(cfg: Config, file_path: String, job_id: String) {
    let path = std::path::PathBuf::from(&file_path);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    info!("Recovery: watching job {} ({})", job_id, name);

    loop {
        thread::sleep(Duration::from_secs(10));
        let job = match api::poll_job(&cfg.server_url, &job_id) {
            Ok(j) => j,
            Err(e) => {
                warn!("Recovery poll error for {}: {}", job_id, e);
                continue;
            }
        };
        match job.status.as_str() {
            "done" => {
                let vs = job.verify_status.as_deref().unwrap_or("");
                if vs == "running" {
                    continue;
                }
                if vs == "fail" {
                    error!("Recovery: server verify failed for {}", name);
                    let _ = state::upsert(&file_path, state::JobEntry {
                        job_id,
                        submitted_at: 0,
                        status: "failed".to_string(),
                        output_path: None,
                    });
                    return;
                }
                break;
            }
            "failed" => {
                error!(
                    "Recovery: job {} failed: {}",
                    job_id,
                    job.error.as_deref().unwrap_or("?")
                );
                let _ = state::upsert(&file_path, state::JobEntry {
                    job_id,
                    submitted_at: 0,
                    status: "failed".to_string(),
                    output_path: None,
                });
                return;
            }
            _ => continue,
        }
    }

    let output_name = path.with_file_name(format!(
        "{}_av1.mp4",
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("output")
    ));

    info!("Recovery: downloading {} to {}", job_id, output_name.display());

    let bar = indicatif::ProgressBar::hidden();
    if let Err(e) = api::download_output(&cfg.server_url, &job_id, &output_name, &bar) {
        error!("Recovery: download failed for {}: {}", name, e);
        return;
    }

    // Codec-only check — server already validated duration/frames
    match verify::probe(&output_name) {
        Ok(info) if info.codec != "av1" => {
            error!("Recovery: output codec is '{}', expected av1", info.codec);
            let _ = std::fs::remove_file(&output_name);
            return;
        }
        Err(e) => warn!("Recovery: probe warning for {}: {}", name, e),
        _ => {}
    }

    let out_sz = output_name.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = state::upsert(&file_path, state::JobEntry {
        job_id,
        submitted_at: 0,
        status: "done".to_string(),
        output_path: Some(output_name.to_string_lossy().to_string()),
    });

    info!("Recovery done: {} ({:.2} GB)", name, out_sz as f64 / 1e9);
}

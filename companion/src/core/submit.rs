//! Background file submission and download flow.

use log::{error, info, warn};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crate::api;
use crate::config::{Config, ReviewMode};
use crate::core::now_secs;
use crate::platform;
use crate::state;
use crate::verify;

/// Submit a file for transcoding.
/// This runs in a background thread.
pub fn submit_bg(cfg: Config, path: PathBuf) {
    let platform = platform::get_platform();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let submitted_at_ts = now_secs();

    info!("Probing: {}", name);
    let source_info = match verify::probe(&path) {
        Ok(i) => i,
        Err(e) => {
            error!("Cannot probe {}: {}", name, e);
            platform.notify("Enkodu ✗", &format!("Cannot probe {}: {}", name, e));
            return;
        }
    };

    if source_info.codec == "av1" {
        info!("Skipping {} — already AV1", name);
        platform.notify("Enkodu", &format!("{} is already AV1 — skipped", name));
        return;
    }

    info!(
        "Uploading {} ({:.1}s, {})",
        name, source_info.duration, source_info.codec
    );

    let hidden_bar = indicatif::ProgressBar::hidden();
    let upload = match api::upload_file_with_retry(
        &cfg.server_url,
        cfg.auth_token.as_deref(),
        &path,
        &hidden_bar,
    ) {
        Ok(u) => u,
        Err(e) => {
            error!("Upload failed for {}: {}", name, e);
            platform.notify("Enkodu ✗", &format!("Upload failed for {}", name));
            return;
        }
    };

    info!(
        "Queued {} -> job {} (position {})",
        name, upload.job_id, upload.priority_position
    );
    let key = path.to_string_lossy().to_string();
    let _ = state::upsert(
        &key,
        state::JobEntry {
            job_id: upload.job_id.clone(),
            submitted_at: submitted_at_ts,
            status: "pending".to_string(),
            output_path: None,
            source_path: Some(path.to_string_lossy().to_string()),
            source_stats: None,
            output_stats: None,
            encode_finished_at: None,
            encode_duration_secs: None,
        },
    );
    platform.notify(
        "Enkodu",
        &format!("{} queued — job {}", name, upload.job_id),
    );

    // poll until done; loop yields the encode-finished timestamp
    let encode_finished_ts: u64 = loop {
        thread::sleep(Duration::from_secs(5));
        let job = match api::poll_job(&cfg.server_url, cfg.auth_token.as_deref(), &upload.job_id) {
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
                    platform.notify("Enkodu ✗", &format!("Verify failed for {}", name));
                    let _ = state::upsert(
                        &key,
                        state::JobEntry {
                            job_id: upload.job_id,
                            submitted_at: submitted_at_ts,
                            status: "failed".to_string(),
                            output_path: None,
                            source_path: Some(path.to_string_lossy().to_string()),
                            source_stats: None,
                            output_stats: None,
                            encode_finished_at: None,
                            encode_duration_secs: None,
                        },
                    );
                    return;
                }
                if vs == "running" {
                    continue;
                }
                info!("Job {} done — downloading output", upload.job_id);
                break now_secs();
            }
            "failed" => {
                error!(
                    "Job {} failed: {}",
                    upload.job_id,
                    job.error.as_deref().unwrap_or("?")
                );
                platform.notify("Enkodu ✗", &format!("Job failed for {}", name));
                let _ = state::upsert(
                    &key,
                    state::JobEntry {
                        job_id: upload.job_id,
                        submitted_at: submitted_at_ts,
                        status: "failed".to_string(),
                        output_path: None,
                        source_path: Some(path.to_string_lossy().to_string()),
                        source_stats: None,
                        output_stats: None,
                        encode_finished_at: None,
                        encode_duration_secs: None,
                    },
                );
                return;
            }
            _ => continue,
        }
    };

    let output_name = path.with_file_name(format!(
        "{}_av1.mp4",
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
    ));
    info!("Downloading output to {}", output_name.display());
    let dl_bar = indicatif::ProgressBar::hidden();
    if let Err(e) = api::download_output_with_retry(
        &cfg.server_url,
        cfg.auth_token.as_deref(),
        &upload.job_id,
        &output_name,
        &dl_bar,
    ) {
        error!("Download failed for {}: {}", name, e);
        platform.notify("Enkodu ✗", &format!("Download failed for {}", name));
        return;
    }

    // Verify checksum if server provides one
    match api::verify_download_checksum(
        &cfg.server_url,
        cfg.auth_token.as_deref(),
        &upload.job_id,
        &output_name,
    ) {
        Ok(true) => info!("Checksum verified for {}", output_name.display()),
        Ok(false) => {
            error!("Checksum mismatch for {} — removing bad output", name);
            let _ = std::fs::remove_file(&output_name);
            let _ = std::fs::remove_file(output_name.with_extension("part"));
            platform.notify(
                "Enkodu ✗",
                &format!("Checksum mismatch for {} — removed", name),
            );
            return;
        }
        Err(e) => warn!("Checksum check skipped for {}: {}", name, e),
    }

    info!("Local verify for {}", output_name.display());
    if let Err(e) = verify::verify_output(&output_name, source_info.duration) {
        error!("Local verify failed for {}: {}", name, e);
        platform.notify("Enkodu ✗", &format!("Local verify failed for {}", name));
        return;
    }

    if cfg.behavior.review_mode == ReviewMode::Manual {
        // Probe stats for both files so the review UI can display them.
        let source_stats = verify::probe_stats(&path).ok();
        let output_stats = verify::probe_stats(&output_name).ok();
        let encode_dur = if encode_finished_ts > submitted_at_ts {
            Some(encode_finished_ts - submitted_at_ts)
        } else {
            None
        };
        let _ = state::upsert(
            &key,
            state::JobEntry {
                job_id: upload.job_id,
                submitted_at: submitted_at_ts,
                status: "pending_review".to_string(),
                output_path: Some(output_name.to_string_lossy().to_string()),
                source_path: Some(path.to_string_lossy().to_string()),
                source_stats,
                output_stats,
                encode_finished_at: Some(encode_finished_ts),
                encode_duration_secs: encode_dur,
            },
        );
        platform.notify(
            "Enkodu — Review needed",
            &format!("{} is ready for review", name),
        );
        return;
    }

    // Auto mode: apply on_success immediately.
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
    let ratio = if out_sz > 0 { src_sz as f64 / out_sz as f64 } else { 0.0 };

    let _ = state::upsert(
        &key,
        state::JobEntry {
            job_id: upload.job_id,
            submitted_at: submitted_at_ts,
            status: "done".to_string(),
            output_path: Some(final_path.to_string_lossy().to_string()),
            source_path: Some(path.to_string_lossy().to_string()),
            source_stats: None,
            output_stats: None,
            encode_finished_at: Some(encode_finished_ts),
            encode_duration_secs: if encode_finished_ts > submitted_at_ts {
                Some(encode_finished_ts - submitted_at_ts)
            } else {
                None
            },
        },
    );

    info!("Done: {} — {:.2} GB output, {:.1}x smaller", name, out_sz as f64 / 1e9, ratio);
    platform.notify("Enkodu ✓", &format!("{} done — {:.1}x smaller", name, ratio));
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
        .filter(|(_, e)| !matches!(e.status.as_str(), "done" | "failed" | "pending_review" | "rejected"))
        .collect();

    if pending.is_empty() {
        info!("Recovery: no pending jobs");
        return;
    }

    info!(
        "Recovery: {} unfinished job(s) found — resuming",
        pending.len()
    );

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
        let job = match api::poll_job(&cfg.server_url, cfg.auth_token.as_deref(), &job_id) {
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
                    let _ = state::upsert(
                        &file_path,
                        state::JobEntry {
                            job_id,
                            submitted_at: 0,
                            status: "failed".to_string(),
                            output_path: None,
                            source_path: None,
                            source_stats: None,
                            output_stats: None,
                            encode_finished_at: None,
                            encode_duration_secs: None,
                        },
                    );
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
                let _ = state::upsert(
                    &file_path,
                    state::JobEntry {
                        job_id,
                        submitted_at: 0,
                        status: "failed".to_string(),
                        output_path: None,
                        source_path: None,
                        source_stats: None,
                        output_stats: None,
                        encode_finished_at: None,
                        encode_duration_secs: None,
                    },
                );
                return;
            }
            _ => continue,
        }
    }

    let output_name = path.with_file_name(format!(
        "{}_av1.mp4",
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
    ));

    info!(
        "Recovery: downloading {} to {}",
        job_id,
        output_name.display()
    );

    let bar = indicatif::ProgressBar::hidden();
    if let Err(e) = api::download_output_with_retry(
        &cfg.server_url,
        cfg.auth_token.as_deref(),
        &job_id,
        &output_name,
        &bar,
    ) {
        error!("Recovery: download failed for {}: {}", name, e);
        return;
    }

    // Verify checksum
    match api::verify_download_checksum(
        &cfg.server_url,
        cfg.auth_token.as_deref(),
        &job_id,
        &output_name,
    ) {
        Ok(true) => info!("Recovery: checksum verified for {}", name),
        Ok(false) => {
            error!(
                "Recovery: checksum mismatch for {} — removing bad output",
                name
            );
            let _ = std::fs::remove_file(&output_name);
            let _ = std::fs::remove_file(output_name.with_extension("part"));
            return;
        }
        Err(e) => warn!("Recovery: checksum check skipped for {}: {}", name, e),
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
    let _ = state::upsert(
        &file_path,
        state::JobEntry {
            job_id,
            submitted_at: 0,
            status: "done".to_string(),
            output_path: Some(output_name.to_string_lossy().to_string()),
            source_path: Some(file_path.clone()),
            source_stats: None,
            output_stats: None,
            encode_finished_at: None,
            encode_duration_secs: None,
        },
    );

    info!("Recovery done: {} ({:.2} GB)", name, out_sz as f64 / 1e9);
}

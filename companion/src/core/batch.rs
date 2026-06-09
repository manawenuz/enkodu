//! Batch scanning and submission of files.

use log::{info, warn};

use crate::api;
use crate::config::Config;
use crate::scan;
use crate::state;
use crate::core::submit;

/// Perform a batch scan and submit eligible files.
/// This runs in a background thread.
pub fn batch_bg(cfg: Config, mac_drain: bool) {
    if mac_drain {
        info!("Batch scan skipped — Mac submissions paused");
        // Platform-specific notification
        return;
    }

    info!("Batch scan: scanning {} directory/ies", cfg.scan.directories.len());
    for d in &cfg.scan.directories {
        info!("  scanning {}", d);
    }

    let files = scan::scan(&cfg);
    info!("Scan found {} candidate files", files.len());

    let all_paths: Vec<String> = files
        .iter()
        .map(|f| f.path.to_string_lossy().to_string())
        .collect();

    let st = state::load().unwrap_or_default();
    let eligible: Vec<_> = files
        .into_iter()
        .filter(|f| {
            let key = f.path.to_string_lossy().to_string();
            !matches!(
                st.get(&key).map(|e| e.status.as_str()),
                Some("pending" | "active" | "done")
            )
        })
        .collect();

    if let Err(e) = api::post_queue_manifest(&cfg.server_url, &all_paths) {
        warn!("Failed to post queue manifest: {}", e);
    }

    info!("Batch: {} files eligible (not yet submitted)", eligible.len());

    if !eligible.is_empty() {
        for (i, f) in eligible.iter().enumerate() {
            info!("Batch [{}/{}]: {}", i + 1, eligible.len(), f.path.display());
            submit::submit_bg(cfg.clone(), f.path.clone());
        }
        info!("Batch complete");
    }
}

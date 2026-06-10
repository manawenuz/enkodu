//! Batch scanning and submission of files.

use log::{info, warn};

use crate::api;
use crate::config::Config;
use crate::core::submit;
use crate::platform;
use crate::scan;
use crate::state;

/// Perform a batch scan and submit eligible files.
/// This runs in a background thread.
pub fn batch_bg(cfg: Config, mac_drain: bool) {
    let platform = platform::get_platform();
    if mac_drain {
        info!("Batch scan skipped — Local submissions paused");
        platform.notify("Enkodu", "Local submissions paused — scan skipped");
        return;
    }

    info!(
        "Batch scan: scanning {} directory/ies",
        cfg.scan.directories.len()
    );
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

    if let Err(e) = api::post_queue_manifest(&cfg.server_url, cfg.auth_token.as_deref(), &all_paths)
    {
        warn!("Failed to post queue manifest: {}", e);
    }

    info!(
        "Batch: {} files eligible (not yet submitted)",
        eligible.len()
    );

    if !eligible.is_empty() {
        platform.notify(
            "Enkodu",
            &format!("Batch: submitting {} files", eligible.len()),
        );
        for (i, f) in eligible.iter().enumerate() {
            info!("Batch [{}/{}]: {}", i + 1, eligible.len(), f.path.display());
            submit::submit_bg(cfg.clone(), f.path.clone());
        }
        info!("Batch complete");
        platform.notify(
            "Enkodu",
            &format!("Batch complete: {} files submitted", eligible.len()),
        );
    } else {
        platform.notify("Enkodu", "No new eligible videos found");
    }
}

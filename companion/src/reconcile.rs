use crate::{api, state, verify};
use crate::config::Config;
use crate::core::now_secs;
use crate::scan::VideoFile;
use crate::platform;
use log::{info, warn};
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;

// ── public entry point ────────────────────────────────────────────────────────

pub fn reconcile(cfg: &Config, local_files: &[VideoFile]) {
    // Build filename → Vec<path> map
    let mut by_name: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for f in local_files {
        if let Some(name) = f.path.file_name().and_then(|n| n.to_str()) {
            by_name.entry(name.to_string()).or_default().push(f.path.clone());
        }
    }

    // Fetch ALL done jobs — we want to update existing ones, not create parallel flows
    let all_jobs = match api::list_done_companion_jobs(&cfg.server_url) {
        Ok(j) => j,
        Err(e) => { warn!("Reconcile: cannot fetch server jobs: {}", e); return; }
    };

    let local_state = state::load().unwrap_or_default();

    // Group jobs by source_filename, prefer NAS jobs (source under /data/Videos/)
    // over companion upload jobs (source under /data/.transcode/uploads/)
    let mut by_filename: HashMap<String, Vec<api::ServerJob>> = HashMap::new();
    for job in all_jobs {
        if let Some(fname) = &job.source_filename {
            by_filename.entry(fname.clone()).or_default().push(job);
        }
    }

    let mut to_process: Vec<(String, api::ServerJob)> = Vec::new();

    for (fname, mut jobs) in by_filename {
        // Skip if already locally done for all of these
        let already_done = local_state.values()
            .any(|e| jobs.iter().any(|j| j.id == e.job_id) && e.status == "done");
        if already_done {
            continue;
        }

        // Prefer NAS job (real output path beside original) over companion upload job
        jobs.sort_by_key(|j| {
            let src = j.source_path.as_deref().unwrap_or("");
            if src.contains("/.transcode/uploads/") { 1 } else { 0 }
        });

        // If the top pick (NAS job or only companion job) has an output that already
        // exists locally as a sibling file, we still want to update client_path.
        to_process.push((fname, jobs.into_iter().next().unwrap()));
    }

    if to_process.is_empty() {
        info!("Reconcile: all server jobs accounted for locally");
        return;
    }

    info!("Reconcile: {} job(s) to reconcile", to_process.len());

    let mut ambiguous_names: Vec<String> = Vec::new();

    for (fname, job) in to_process {
        let candidates = match by_name.get(&fname) {
            Some(c) if !c.is_empty() => c.clone(),
            _ => {
                info!("Reconcile: '{}' — no local file with that name", fname);
                continue;
            }
        };

        let server_meta = parse_server_meta(job.source_meta.as_deref().unwrap_or("{}"));
        let confident: Vec<PathBuf> = candidates.into_iter()
            .filter(|p| is_confident_match(p, &server_meta))
            .collect();

        match confident.len() {
            0 => info!("Reconcile: '{}' — found by name but metadata doesn't match", fname),
            1 => {
                let local_path = confident.into_iter().next().unwrap();
                let cfg2 = cfg.clone();
                thread::spawn(move || handle_match(cfg2, job, local_path));
            }
            _ => {
                warn!("Reconcile: '{}' — {} ambiguous local matches", fname, confident.len());
                for p in &confident { warn!("  {}", p.display()); }
                ambiguous_names.push(fname);
            }
        }
    }

    if !ambiguous_names.is_empty() {
        crate::platform::get_platform().notify(
            "Enkodu \u{26a0}",
            &format!(
                "{} file(s) ambiguous — set path manually in web UI: {}",
                ambiguous_names.len(),
                ambiguous_names.join(", ")
            ),
        );
    }
}

// ── handle a confident match ──────────────────────────────────────────────────

fn handle_match(cfg: Config, job: api::ServerJob, local_path: PathBuf) {
    let name = local_path.file_name().and_then(|n| n.to_str())
        .unwrap_or("file").to_string();
    let job_id = job.id.clone();

    // Update server record with correct Mac-side path
    if let Err(e) = api::set_client_path(&cfg.server_url, &job_id, local_path.to_str().unwrap_or("")) {
        warn!("Reconcile: set_client_path failed for {}: {}", job_id, e);
    }

    let output_path = local_path.with_file_name(format!(
        "{}_av1.mp4",
        local_path.file_stem().and_then(|s| s.to_str()).unwrap_or("output")
    ));

    // If the output already exists (e.g. NAS job wrote it to the mounted share),
    // just record it locally — no download needed.
    if output_path.exists() {
        info!("Reconcile: output already exists at {} — updating local state only", output_path.display());
        let _ = state::upsert(&local_path.to_string_lossy(), state::JobEntry {
            job_id,
            submitted_at: now_secs(),
            status: "done".to_string(),
            output_path: Some(output_path.to_string_lossy().to_string()),
        });
        crate::platform::get_platform().notify("Enkodu \u{2713}", &format!("Reconcile: linked {}  (already on disk)", name));
        return;
    }

    // Output not present locally — need to download from server
    info!("Reconcile: downloading {} → {}", job_id, output_path.display());
    crate::platform::get_platform().notify("Enkodu", &format!("Reconcile: downloading {}", name));

    // Wait if verify is still running
    loop {
        match api::poll_job(&cfg.server_url, &job_id) {
            Ok(j) if j.verify_status.as_deref() == Some("running") => {
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            _ => break,
        }
    }

    let bar = indicatif::ProgressBar::hidden();
    if let Err(e) = api::download_output(&cfg.server_url, &job_id, &output_path, &bar) {
        warn!("Reconcile: download failed for '{}': {}", name, e);
        crate::platform::get_platform().notify("Enkodu \u{2717}", &format!("Reconcile download failed: {}", name));
        return;
    }

    match verify::probe(&output_path) {
        Ok(info) if info.codec != "av1" => {
            warn!("Reconcile: bad codec '{}' for {}", info.codec, name);
            crate::platform::get_platform().notify("Enkodu \u{2717}", &format!("Bad codec after reconcile: {}", name));
            let _ = std::fs::remove_file(&output_path);
            return;
        }
        Err(e) => warn!("Reconcile: probe warning for {}: {}", name, e),
        _ => {}
    }

    let out_sz = output_path.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = state::upsert(&local_path.to_string_lossy(), state::JobEntry {
        job_id,
        submitted_at: now_secs(),
        status: "done".to_string(),
        output_path: Some(output_path.to_string_lossy().to_string()),
    });

    info!("Reconcile done: {} ({:.2} GB)", name, out_sz as f64 / 1e9);
    crate::platform::get_platform().notify("Enkodu \u{2713}", &format!("Reconcile: {}  ({:.2} GB)", name, out_sz as f64 / 1e9));
}

// ── confidence check ──────────────────────────────────────────────────────────

struct ServerMeta { duration: f64, width: u32, height: u32 }

fn parse_server_meta(json: &str) -> ServerMeta {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    ServerMeta {
        duration: v["duration"].as_f64().unwrap_or(0.0),
        width:    v["width"].as_u64().unwrap_or(0) as u32,
        height:   v["height"].as_u64().unwrap_or(0) as u32,
    }
}

fn is_confident_match(local_path: &PathBuf, server: &ServerMeta) -> bool {
    let info = match verify::probe(local_path) {
        Ok(i) => i,
        Err(e) => { warn!("Reconcile: probe failed for {}: {}", local_path.display(), e); return false; }
    };
    let dur_ok = server.duration > 0.0 && (info.duration - server.duration).abs() < 3.0;
    let res_ok = server.width > 0 && info.width == server.width && info.height == server.height;
    if !dur_ok || !res_ok {
        info!(
            "Reconcile: {} — no match (dur {:.1}s vs {:.1}s, {}x{} vs {}x{})",
            local_path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            info.duration, server.duration, info.width, info.height, server.width, server.height
        );
    }
    dur_ok && res_ok
}

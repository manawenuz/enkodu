mod api;
mod config;
mod scan;
mod state;
mod verify;

use anyhow::Result;
use config::Config;
use log::{error, info, warn};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use tray_icon::{TrayIcon, TrayIconBuilder};
use winit::event::Event;
use winit::event_loop::{ControlFlow, EventLoopBuilder};

#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

// ── shared state ──────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct ServerState {
    online: bool,
    pending: u64,
    active: u64,
    done: u64,
    failed: u64,
    encoding_file: Option<String>,
    encoding_pct: f64,
    encoding_speed: String,
    encoding_phase: String,
    control_cmd: String,
    prev_done: u64,
}

// ── menu item handles ─────────────────────────────────────────────────────────

struct Tray {
    _tray_icon: TrayIcon, // must be kept alive — dropping this removes the status bar item
    status_item: MenuItem,
    job_item: MenuItem,
    submit_item: MenuItem,
    batch_item: MenuItem,
    webui_item: MenuItem,
    drain_item: MenuItem,
    resume_item: MenuItem,
    login_item: CheckMenuItem,
    config_item: MenuItem,
    quit_item: MenuItem,
}

// ── entry point ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cfg = Config::load()?;
    info!("Enkodu starting — server: {} (build {})", cfg.server_url, env!("GIT_HASH"));

    let state: Arc<RwLock<ServerState>> = Arc::new(RwLock::new(ServerState::default()));

    let mut builder = EventLoopBuilder::new();
    #[cfg(target_os = "macos")]
    {
        builder.with_activation_policy(ActivationPolicy::Accessory);
        builder.with_default_menu(false);
    }
    let event_loop = builder.build().expect("event loop");
    info!("Event loop created");

    let tray = build_tray(&cfg)?;
    info!("Tray icon registered in menu bar");

    {
        let state = Arc::clone(&state);
        let cfg = cfg.clone();
        thread::spawn(move || poll_loop(cfg, state));
    }

    let menu_rx = MenuEvent::receiver();
    let mut last_update = Instant::now() - Duration::from_secs(10);
    let state_ref = Arc::clone(&state);

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(300),
        ));

        if let Event::NewEvents(_) = event {
            if last_update.elapsed() >= Duration::from_secs(2) {
                let s = state_ref.read().unwrap().clone();
                update_tray_menu(&tray, &s);
                last_update = Instant::now();
            }
        }

        while let Ok(ev) = menu_rx.try_recv() {
            handle_event(&ev, &tray, &cfg, &state_ref);
        }
    })?;

    Ok(())
}

// ── tray construction ─────────────────────────────────────────────────────────

fn build_tray(cfg: &Config) -> Result<Tray> {
    let status_item = MenuItem::new("○ Connecting...", false, None);
    let job_item    = MenuItem::new("No active jobs", false, None);
    let submit_item = MenuItem::new("Submit File…", true, None);
    let batch_item  = MenuItem::new("Batch Scan", true, None);
    let webui_item  = MenuItem::new("Open Web UI", true, None);
    let drain_item  = MenuItem::new("⏸  Drain Worker", true, None);
    let resume_item = MenuItem::new("▶  Resume Worker", false, None);
    let login_item  = CheckMenuItem::new("Start at Login", true, launch_agent_exists(), None);
    let config_item = MenuItem::new("Open Config…", true, None);
    let quit_item   = MenuItem::new("Quit", true, None);

    let menu = Menu::new();
    menu.append(&status_item).unwrap();
    menu.append(&job_item).unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&submit_item).unwrap();
    menu.append(&batch_item).unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&webui_item).unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&drain_item).unwrap();
    menu.append(&resume_item).unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&login_item).unwrap();
    menu.append(&config_item).unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&quit_item).unwrap();

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_title("✦")
        .with_tooltip(&format!("Enkodu — {}", cfg.server_url))
        .build()
        .expect("tray icon");

    Ok(Tray {
        _tray_icon: tray_icon,
        status_item,
        job_item,
        submit_item,
        batch_item,
        webui_item,
        drain_item,
        resume_item,
        login_item,
        config_item,
        quit_item,
    })
}

// ── menu updates ──────────────────────────────────────────────────────────────

fn update_tray_menu(tray: &Tray, s: &ServerState) {
    if s.online {
        tray.status_item.set_text(format!(
            "● Online  ·  {} pending  {}✓  {}✗",
            s.pending, s.done, s.failed
        ));
    } else {
        tray.status_item.set_text("○ Server offline");
    }

    match &s.encoding_file {
        Some(f) => {
            let phase = match s.encoding_phase.as_str() {
                "uploading" => "▲",
                "verifying" => "◎",
                _           => "⟳",
            };
            tray.job_item.set_text(format!(
                "{} {}  {:.0}%  {}",
                phase,
                truncate(f, 28),
                s.encoding_pct,
                s.encoding_speed
            ));
        }
        None if s.active > 0 => {
            tray.job_item.set_text(format!("{} active", s.active));
        }
        None => {
            tray.job_item.set_text("No active jobs");
        }
    }

    let drained = matches!(s.control_cmd.as_str(), "drain" | "stop");
    tray.drain_item.set_enabled(!drained && s.online);
    tray.resume_item.set_enabled(drained && s.online);
}

// ── menu event handling ───────────────────────────────────────────────────────

fn handle_event(ev: &muda::MenuEvent, tray: &Tray, cfg: &Config, _state: &Arc<RwLock<ServerState>>) {
    if ev.id == tray.quit_item.id() {
        info!("Quit requested");
        std::process::exit(0);

    } else if ev.id == tray.submit_item.id() {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select video to transcode")
            .add_filter("Video", &["mp4", "mov", "mkv", "avi", "m4v", "ts"])
            .pick_file()
        {
            info!("User selected: {}", path.display());
            let cfg = cfg.clone();
            thread::spawn(move || submit_bg(cfg, path));
        }

    } else if ev.id == tray.batch_item.id() {
        info!("Batch scan triggered");
        let cfg = cfg.clone();
        thread::spawn(move || batch_bg(cfg));

    } else if ev.id == tray.webui_item.id() {
        info!("Opening web UI: {}", cfg.server_url);
        let _ = open::that(&cfg.server_url);

    } else if ev.id == tray.drain_item.id() {
        info!("Draining worker");
        let url = cfg.server_url.clone();
        thread::spawn(move || { let _ = api::set_control(&url, "drain"); });

    } else if ev.id == tray.resume_item.id() {
        info!("Resuming worker");
        let url = cfg.server_url.clone();
        thread::spawn(move || { let _ = api::set_control(&url, "run"); });

    } else if ev.id == tray.config_item.id() {
        let _ = open::that(Config::path());

    } else if ev.id == tray.login_item.id() {
        toggle_launch_agent();
    }
}

// ── background submit flow ────────────────────────────────────────────────────

fn submit_bg(cfg: Config, path: PathBuf) {
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
            notify("Enkodu", &format!("Cannot probe {}: {}", name, e));
            return;
        }
    };

    if source_info.codec == "av1" {
        info!("Skipping {} — already AV1", name);
        notify("Enkodu", &format!("{} is already AV1, skipping", name));
        return;
    }

    info!("Uploading {} ({:.1}s, {})", name, source_info.duration, source_info.codec);
    notify("Enkodu", &format!("Uploading {}…", name));

    let hidden_bar = indicatif::ProgressBar::hidden();
    let upload = match api::upload_file(&cfg.server_url, &path, &hidden_bar) {
        Ok(u) => u,
        Err(e) => {
            error!("Upload failed for {}: {}", name, e);
            notify("Enkodu ✗", &format!("Upload failed: {}", e));
            return;
        }
    };

    info!("Queued {} → job {} (position {})", name, upload.job_id, upload.priority_position);
    let key = path.to_string_lossy().to_string();
    let _ = state::upsert(&key, state::JobEntry {
        job_id: upload.job_id.clone(),
        submitted_at: now_secs(),
        status: "pending".to_string(),
        output_path: None,
    });

    notify("Enkodu", &format!("Queued {} (position {})", name, upload.priority_position));

    // poll until done
    loop {
        thread::sleep(Duration::from_secs(5));
        let job = match api::poll_job(&cfg.server_url, &upload.job_id) {
            Ok(j) => j,
            Err(e) => { warn!("Poll error for {}: {}", upload.job_id, e); continue; }
        };
        match job.status.as_str() {
            "done" => {
                let vs = job.verify_status.as_deref().unwrap_or("");
                if vs == "fail" {
                    error!("Verify failed for {}: {}", name, job.verify_detail.as_deref().unwrap_or("?"));
                    notify("Enkodu ✗", &format!(
                        "Verify failed: {}",
                        job.verify_detail.as_deref().unwrap_or("?")
                    ));
                    return;
                }
                if vs == "running" { continue; }
                info!("Job {} done — downloading output", upload.job_id);
                break;
            }
            "failed" => {
                error!("Job {} failed: {}", upload.job_id, job.error.as_deref().unwrap_or("?"));
                notify("Enkodu ✗", &format!(
                    "{} failed: {}",
                    name,
                    job.error.as_deref().unwrap_or("?")
                ));
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
        notify("Enkodu ✗", &format!("Download failed: {}", e));
        return;
    }

    info!("Local verify for {}", output_name.display());
    if let Err(e) = verify::verify_output(&output_name, source_info.duration) {
        error!("Local verify failed for {}: {}", name, e);
        notify("Enkodu ✗", &format!("Local verify failed: {}", e));
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
    let ratio = if out_sz > 0 { src_sz as f64 / out_sz as f64 } else { 0.0 };

    let _ = state::upsert(&key, state::JobEntry {
        job_id: upload.job_id,
        submitted_at: now_secs(),
        status: "done".to_string(),
        output_path: Some(final_path.to_string_lossy().to_string()),
    });

    info!("Done: {} — {:.2} GB output, {:.1}x smaller", name, out_sz as f64 / 1e9, ratio);
    notify("Enkodu ✓", &format!(
        "{} → {:.2} GB  ({:.1}x smaller)",
        name,
        out_sz as f64 / 1e9,
        ratio
    ));
}

fn batch_bg(cfg: Config) {
    notify("Enkodu", "Scanning for eligible videos…");
    info!("Batch scan: scanning {} directory/ies", cfg.scan.directories.len());
    for d in &cfg.scan.directories {
        info!("  scanning {}", d);
    }

    let files = scan::scan(&cfg);
    info!("Scan found {} candidate files", files.len());

    // Collect paths before consuming `files` into the eligible filter.
    let all_paths: Vec<String> = files.iter()
        .map(|f| f.path.to_string_lossy().to_string())
        .collect();

    let st = state::load().unwrap_or_default();
    let eligible: Vec<_> = files.into_iter().filter(|f| {
        let key = f.path.to_string_lossy().to_string();
        !matches!(st.get(&key).map(|e| e.status.as_str()), Some("pending" | "active" | "done"))
    }).collect();

    // Post manifest so the server knows this client's full local queue depth.
    if let Err(e) = api::post_queue_manifest(&cfg.server_url, &all_paths) {
        warn!("Failed to post queue manifest: {}", e);
    }

    info!("Batch: {} files eligible (not yet submitted)", eligible.len());

    if eligible.is_empty() {
        notify("Enkodu", "No new eligible videos found");
        return;
    }
    notify("Enkodu", &format!("Batch: submitting {} files", eligible.len()));

    for (i, f) in eligible.iter().enumerate() {
        info!("Batch [{}/{}]: {}", i + 1, eligible.len(), f.path.display());
        submit_bg(cfg.clone(), f.path.clone());
    }

    info!("Batch complete");
}

// ── background server polling ─────────────────────────────────────────────────

fn poll_loop(cfg: Config, state: Arc<RwLock<ServerState>>) {
    info!("Poll loop started — polling every 5s");
    loop {
        thread::sleep(Duration::from_secs(5));

        match api::queue_status(&cfg.server_url) {
            Ok(s) => {
                let prev = state.read().unwrap().prev_done;
                if s.done > prev && prev > 0 {
                    let n = s.done - prev;
                    info!("{} new completion(s)", n);
                    notify("Enkodu ✓", &format!("{} job{} completed", n, if n > 1 { "s" } else { "" }));
                }
                let mut st = state.write().unwrap();
                let was_online = st.online;
                st.online = true;
                st.pending = s.pending;
                st.active = s.active;
                st.done = s.done;
                st.failed = s.failed;
                st.prev_done = s.done;
                if !was_online {
                    info!("Server online — pending={} active={} done={} failed={}", s.pending, s.active, s.done, s.failed);
                }
            }
            Err(e) => {
                let was_online = state.read().unwrap().online;
                if was_online {
                    warn!("Server unreachable: {}", e);
                }
                state.write().unwrap().online = false;
            }
        }

        if let Ok(live) = api::live_jobs(&cfg.server_url) {
            let mut st = state.write().unwrap();
            if let Some(job) = live.values().next() {
                st.encoding_file = Some(job.file.clone());
                st.encoding_pct = job.percent;
                st.encoding_speed = job.speed.clone();
                st.encoding_phase = job.phase.clone();
            } else {
                st.encoding_file = None;
                st.encoding_pct = 0.0;
                st.encoding_speed = String::new();
                st.encoding_phase = String::new();
            }
        }

        if let Ok(cmd) = api::control_status(&cfg.server_url) {
            state.write().unwrap().control_cmd = cmd;
        }
    }
}

// ── launch agent ──────────────────────────────────────────────────────────────

fn launch_agent_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/LaunchAgents/io.enkodu.companion.plist")
}

fn launch_agent_exists() -> bool {
    launch_agent_path().exists()
}

fn toggle_launch_agent() {
    let path = launch_agent_path();
    if path.exists() {
        let _ = std::fs::remove_file(&path);
        info!("LaunchAgent removed");
        notify("Enkodu", "Removed from login items");
    } else {
        let exe = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("/usr/local/bin/enkodu"));
        let plist = format!(
r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>io.enkodu.companion</string>
    <key>ProgramArguments</key>
    <array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key>
    <string>/tmp/enkodu.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/enkodu.log</string>
</dict>
</plist>"#,
            exe.display()
        );
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        if std::fs::write(&path, plist).is_ok() {
            info!("LaunchAgent written to {}", path.display());
            notify("Enkodu", "Will launch at login");
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn notify(title: &str, body: &str) {
    info!("[notify] {}: {}", title, body);
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show();
}

fn truncate(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        format!("{}…", chars[..n - 1].iter().collect::<String>())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

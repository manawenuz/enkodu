mod api;
mod config;
mod ipc;
mod reconcile;
mod scan;
mod state;
mod verify;
mod wanryo;

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
    mac_drain: bool,
    nas_drain: bool,
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
    mac_drain_item: CheckMenuItem,
    nas_drain_item: CheckMenuItem,
    reconcile_item: MenuItem,
    login_item: CheckMenuItem,
    config_item: MenuItem,
    quit_item: MenuItem,
}

// ── entry point ───────────────────────────────────────────────────────────────

// ── diagnostic commands (run directly, no IPC, no tray needed) ───────────────

fn cmd_tcpping(addr: &str) {
    use std::net::TcpStream;
    use std::time::Instant;
    println!("tcpping {} (stdlib TcpStream, 3 attempts)", addr);
    for i in 1..=3u8 {
        let t = Instant::now();
        match TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| {
                // handle bare host — shouldn't happen if user passes host:port
                "0.0.0.0:0".parse().unwrap()
            }),
            std::time::Duration::from_secs(5),
        ) {
            Ok(_)  => println!("  [{}] ok  {:.1}ms", i, t.elapsed().as_secs_f64() * 1000.0),
            Err(e) => println!("  [{}] err {}", i, e),
        }
    }
}

fn cmd_httping(url: &str) {
    use std::time::Instant;
    println!("httping {} (reqwest blocking, 3 attempts)", url);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("client");
    for i in 1..=3u8 {
        let t = Instant::now();
        match client.get(url).send() {
            Ok(r)  => println!("  [{}] ok  HTTP {}  {:.1}ms", i, r.status(), t.elapsed().as_secs_f64() * 1000.0),
            Err(e) => println!("  [{}] err {}", i, e),
        }
    }
}

pub fn batch_bg(cfg: Config, mac_drain: bool) {
    if mac_drain {
        info!("Batch scan skipped — Mac submissions paused");
        notify("Enkodu", "Mac submissions paused — scan skipped");
        return;
    }
    notify("Enkodu", "Scanning for eligible videos…");
    info!("Batch scan: scanning {} directory/ies", cfg.scan.directories.len());
    for d in &cfg.scan.directories {
        info!("  scanning {}", d);
    }

    let files = scan::scan(&cfg);
    info!("Scan found {} candidate files", files.len());

    let all_paths: Vec<String> = files.iter()
        .map(|f| f.path.to_string_lossy().to_string())
        .collect();

    let st = state::load().unwrap_or_default();
    let eligible: Vec<_> = files.into_iter().filter(|f| {
        let key = f.path.to_string_lossy().to_string();
        !matches!(st.get(&key).map(|e| e.status.as_str()), Some("pending" | "active" | "done"))
    }).collect();

    if let Err(e) = api::post_queue_manifest(&cfg.server_url, &all_paths) {
        warn!("Failed to post queue manifest: {}", e);
    }

    info!("Batch: {} files eligible (not yet submitted)", eligible.len());

    if !eligible.is_empty() {
        notify("Enkodu", &format!("Batch: submitting {} files", eligible.len()));
        for (i, f) in eligible.iter().enumerate() {
            info!("Batch [{}/{}]: {}", i + 1, eligible.len(), f.path.display());
            submit_bg(cfg.clone(), f.path.clone());
        }
        info!("Batch complete");
    } else {
        notify("Enkodu", "No new eligible videos found");
    }

    let all_files = scan::scan(&cfg);
    reconcile::reconcile(&cfg, &all_files);
}

fn main() -> Result<()> {
    // ── CLI mode ──────────────────────────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "tcpping" => {
                let addr = args.get(2).map(|s| s.as_str()).unwrap_or("172.16.81.137:443");
                cmd_tcpping(addr);
                return Ok(());
            }
            "httping" => {
                let url = args.get(2).map(|s| s.as_str()).unwrap_or("https://enkodu.manwe.qzz.io/status");
                cmd_httping(url);
                return Ok(());
            }
            "wanryo" => {
                use anyhow::Context as _;
                let cfg = Config::load().context("load config")?;
                wanryo::run(&cfg)?;
                return Ok(());
            }
            _ => {
                let cmd = args[1..].join(" ");
                match ipc::send_cmd(&cmd) {
                    Ok(resp) => { println!("{}", resp); return Ok(()); }
                    Err(e)   => { eprintln!("enkodu: {}", e); std::process::exit(1); }
                }
            }
        }
    }

    // ── tray mode ────────────────────────────────────────────────────────────
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    acquire_pid_lock()?;

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

    ipc::start_server(cfg.clone(), Arc::clone(&state));

    {
        let state = Arc::clone(&state);
        let cfg = cfg.clone();
        thread::spawn(move || poll_loop(cfg, state));
    }

    {
        let cfg = cfg.clone();
        thread::spawn(move || recover_pending_downloads(cfg));
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
    let drain_item     = MenuItem::new("⏸  Drain Worker", true, None);
    let resume_item    = MenuItem::new("▶  Resume Worker", false, None);
    let mac_drain_item = CheckMenuItem::new("⏸  Pause Mac Submissions", true, false, None);
    let nas_drain_item = CheckMenuItem::new("⏸  Pause NAS Scan", true, false, None);
    let reconcile_item = MenuItem::new("Reconcile Server Jobs", true, None);
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
    menu.append(&mac_drain_item).unwrap();
    menu.append(&nas_drain_item).unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&reconcile_item).unwrap();
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
        mac_drain_item,
        nas_drain_item,
        reconcile_item,
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
    tray.mac_drain_item.set_checked(s.mac_drain);
    tray.nas_drain_item.set_checked(s.nas_drain);
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
        let mac_drain = _state.read().unwrap().mac_drain;
        thread::spawn(move || batch_bg(cfg, mac_drain));

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

    } else if ev.id == tray.mac_drain_item.id() {
        let new_state = !_state.read().unwrap().mac_drain;
        _state.write().unwrap().mac_drain = new_state;
        tray.mac_drain_item.set_checked(new_state);
        info!("Mac submissions {}", if new_state { "paused" } else { "resumed" });
        notify("Enkodu", if new_state { "Mac submissions paused" } else { "Mac submissions resumed" });

    } else if ev.id == tray.nas_drain_item.id() {
        let new_state = !_state.read().unwrap().nas_drain;
        _state.write().unwrap().nas_drain = new_state;
        tray.nas_drain_item.set_checked(new_state);
        let url = cfg.server_url.clone();
        let val = if new_state { "true" } else { "false" };
        thread::spawn(move || { let _ = api::set_setting(&url, "nas_drain", val); });
        info!("NAS scan {}", if new_state { "paused" } else { "resumed" });
        notify("Enkodu", if new_state { "NAS scan paused" } else { "NAS scan resumed" });

    } else if ev.id == tray.reconcile_item.id() {
        info!("Reconcile triggered manually");
        let cfg = cfg.clone();
        thread::spawn(move || {
            notify("Enkodu", "Reconcile: scanning local files…");
            let files = scan::scan(&cfg);
            reconcile::reconcile(&cfg, &files);
        });

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
                warn!("Server unreachable: {:#}", e);
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

        if let Ok(settings) = api::get_settings(&cfg.server_url) {
            let nas_drain = settings.get("nas_drain").map(|v| v == "true").unwrap_or(false);
            state.write().unwrap().nas_drain = nas_drain;
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

// ── recovery: download completed jobs that were missed while app was closed ──

fn recover_pending_downloads(cfg: Config) {
    let st = match state::load() {
        Ok(s) => s,
        Err(e) => { warn!("Recovery: could not load state: {}", e); return; }
    };

    let pending: Vec<(String, state::JobEntry)> = st.into_iter()
        .filter(|(_, e)| !matches!(e.status.as_str(), "done" | "failed"))
        .collect();

    if pending.is_empty() {
        info!("Recovery: no pending jobs");
        return;
    }

    info!("Recovery: {} unfinished job(s) found — resuming", pending.len());
    notify("Enkodu", &format!("Resuming {} interrupted job(s)\u{2026}", pending.len()));

    for (file_path, entry) in pending {
        let cfg = cfg.clone();
        thread::spawn(move || recover_one(cfg, file_path, entry.job_id));
    }
}

fn recover_one(cfg: Config, file_path: String, job_id: String) {
    let path = std::path::PathBuf::from(&file_path);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    info!("Recovery: watching job {} ({})", job_id, name);

    loop {
        thread::sleep(Duration::from_secs(10));
        let job = match api::poll_job(&cfg.server_url, &job_id) {
            Ok(j) => j,
            Err(e) => { warn!("Recovery poll error for {}: {}", job_id, e); continue; }
        };
        match job.status.as_str() {
            "done" => {
                let vs = job.verify_status.as_deref().unwrap_or("");
                if vs == "running" { continue; }
                if vs == "fail" {
                    error!("Recovery: server verify failed for {}", name);
                    notify("Enkodu \u{2717}", &format!("Verify failed: {}", name));
                    let _ = state::upsert(&file_path, state::JobEntry {
                        job_id, submitted_at: 0, status: "failed".to_string(), output_path: None,
                    });
                    return;
                }
                break;
            }
            "failed" => {
                error!("Recovery: job {} failed: {}", job_id, job.error.as_deref().unwrap_or("?"));
                notify("Enkodu \u{2717}", &format!("{} failed on server", name));
                let _ = state::upsert(&file_path, state::JobEntry {
                    job_id, submitted_at: 0, status: "failed".to_string(), output_path: None,
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
    notify("Enkodu", &format!("Downloading: {}", name));

    let bar = indicatif::ProgressBar::hidden();
    if let Err(e) = api::download_output(&cfg.server_url, &job_id, &output_name, &bar) {
        error!("Recovery: download failed for {}: {}", name, e);
        notify("Enkodu \u{2717}", &format!("Download failed: {}", name));
        return;
    }

    // Codec-only check — server already validated duration/frames
    match verify::probe(&output_name) {
        Ok(info) if info.codec != "av1" => {
            error!("Recovery: output codec is '{}', expected av1", info.codec);
            notify("Enkodu \u{2717}", &format!("Bad codec after download: {}", name));
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
    notify("Enkodu \u{2713}", &format!("Recovered: {}  ({:.2} GB)", name, out_sz as f64 / 1e9));
}

// ── pid lock — prevent multiple instances ────────────────────────────────────

fn pid_lock_path() -> std::path::PathBuf {
    std::env::temp_dir().join("enkodu.lock")
}

fn acquire_pid_lock() -> Result<()> {
    use std::io::{Read, Write};
    let path = pid_lock_path();

    // Check if a lock file exists with a live PID
    if let Ok(mut f) = std::fs::File::open(&path) {
        let mut buf = String::new();
        let _ = f.read_to_string(&mut buf);
        if let Ok(pid) = buf.trim().parse::<u32>() {
            // On Unix, kill -0 checks if process exists without signalling it
            let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
            if alive {
                eprintln!("enkodu is already running (pid {}). Exiting.", pid);
                std::process::exit(0);
            }
        }
        // Stale lock — remove it
        let _ = std::fs::remove_file(&path);
    }

    // Write our PID
    let pid = std::process::id();
    std::fs::write(&path, format!("{}", pid))?;

    // Remove lock file on exit via atexit
    let path_clone = path.clone();
    unsafe {
        LOCK_PATH = Some(path_clone);
        libc::atexit(remove_lock_on_exit);
    }

    Ok(())
}

static mut LOCK_PATH: Option<std::path::PathBuf> = None;

extern "C" fn remove_lock_on_exit() {
    unsafe {
        if let Some(ref p) = LOCK_PATH {
            let _ = std::fs::remove_file(p);
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn notify(title: &str, body: &str) {
    info!("[notify] {}: {}", title, body);
    // Use osascript directly — notify_rust triggers a "Choose Application" dialog
    // on macOS because it registers a click-action handler named "use_default".
    let script = format!(
        "display notification {} with title {}",
        applescript_quote(body),
        applescript_quote(title),
    );
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output();
}

fn applescript_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
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

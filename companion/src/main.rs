//! Enkodu companion - main entry point.
//!
//! This module provides the tray UI and coordinates between platform-specific
//! adapters and the shared core logic.

mod api;
mod config;
mod ipc;
mod reconcile;
mod scan;
mod state;
mod verify;
mod wanryo;

// Core modules
mod core;
// Platform modules
mod platform;

use anyhow::Result;
use config::Config;
use core::{ServerState, commands, batch, poll, submit, truncate};
use log::info;
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

fn main() -> Result<()> {
    // ── CLI mode ──────────────────────────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let platform = platform::get_platform();
        match args[1].as_str() {
            "tcpping" => {
                let addr = args.get(2).map(|s| s.as_str()).unwrap_or("172.16.81.137:443");
                commands::cmd_tcpping(addr);
                return Ok(());
            }
            "httping" => {
                let url = args.get(2).map(|s| s.as_str()).unwrap_or("https://enkodu.manwe.qzz.io/status");
                commands::cmd_httping(url);
                return Ok(());
            }
            "wanryo" => {
                let cfg = Config::load()?;
                commands::cmd_wanryo(&cfg)?;
                return Ok(());
            }
            _ => {
                let cmd = args[1..].join(" ");
                match platform.send_ipc_command(&cmd) {
                    Ok(resp) => { println!("{}", resp); return Ok(()); }
                    Err(e)   => { eprintln!("enkodu: {}", e); std::process::exit(1); }
                }
            }
        }
    }

    // ── tray mode ────────────────────────────────────────────────────────────
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let platform = platform::get_platform();

    // Acquire single instance lock
    let _lock = platform.acquire_single_instance_lock()?;

    let cfg = Config::load()?;
    info!("Enkodu starting — server: {} (build {})", cfg.server_url, env!("GIT_HASH"));

    let state: Arc<RwLock<ServerState>> = Arc::new(RwLock::new(ServerState::default()));

    // Start IPC server
    platform.start_ipc_server(cfg.clone(), Arc::clone(&state));

    let mut builder = EventLoopBuilder::new();
    #[cfg(target_os = "macos")]
    {
        builder.with_activation_policy(ActivationPolicy::Accessory);
        builder.with_default_menu(false);
    }
    let event_loop = builder.build().expect("event loop");
    info!("Event loop created");

    let tray = build_tray(&cfg, &state)?;
    info!("Tray icon registered in menu bar");

    // Start background poll loop
    {
        let state = Arc::clone(&state);
        let cfg = cfg.clone();
        thread::spawn(move || poll::poll_loop(cfg, state));
    }

    // Start recovery of pending downloads
    {
        let cfg = cfg.clone();
        thread::spawn(move || submit::recover_pending_downloads(cfg));
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

fn build_tray(
    cfg: &Config,
    state: &Arc<RwLock<ServerState>>,
) -> Result<Tray> {
    let platform = platform::get_platform();
    let status_item = MenuItem::new("○ Connecting...", false, None);
    let job_item = MenuItem::new("No active jobs", false, None);
    let submit_item = MenuItem::new("Submit File...", true, None);
    let batch_item = MenuItem::new("Batch Scan", true, None);
    let webui_item = MenuItem::new("Open Web UI", true, None);
    let drain_item = MenuItem::new("⏸  Drain Worker", true, None);
    let resume_item = MenuItem::new("▶  Resume Worker", false, None);
    let mac_drain_item = CheckMenuItem::new("⏸  Pause Mac Submissions", true, false, None);
    let nas_drain_item = CheckMenuItem::new("⏸  Pause NAS Scan", true, false, None);
    let reconcile_item = MenuItem::new("Reconcile Server Jobs", true, None);
    let login_item = CheckMenuItem::new("Start at Login", true, platform.autostart_enabled(), None);
    let config_item = MenuItem::new("Open Config...", true, None);
    let quit_item = MenuItem::new("Quit", true, None);

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
                _ => "⟳",
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

fn handle_event(
    ev: &muda::MenuEvent,
    tray: &Tray,
    cfg: &Config,
    state: &Arc<RwLock<ServerState>>,
) {
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
            let path_clone = path.clone();
            thread::spawn(move || {
                submit::submit_bg(cfg, path_clone);
            });
        }

    } else if ev.id == tray.batch_item.id() {
        info!("Batch scan triggered");
        let cfg = cfg.clone();
        let mac_drain = state.read().unwrap().mac_drain;
        thread::spawn(move || {
            batch::batch_bg(cfg, mac_drain);
        });

    } else if ev.id == tray.webui_item.id() {
        info!("Opening web UI: {}", cfg.server_url);
        let _ = platform::get_platform().open_url(&cfg.server_url);

    } else if ev.id == tray.drain_item.id() {
        info!("Draining worker");
        let url = cfg.server_url.clone();
        thread::spawn(move || { let _ = api::set_control(&url, "drain"); });

    } else if ev.id == tray.resume_item.id() {
        info!("Resuming worker");
        let url = cfg.server_url.clone();
        thread::spawn(move || { let _ = api::set_control(&url, "run"); });

    } else if ev.id == tray.mac_drain_item.id() {
        let new_state = !state.read().unwrap().mac_drain;
        state.write().unwrap().mac_drain = new_state;
        tray.mac_drain_item.set_checked(new_state);
        info!("Mac submissions {}", if new_state { "paused" } else { "resumed" });
        platform::get_platform().notify("Enkodu", if new_state { "Mac submissions paused" } else { "Mac submissions resumed" });

    } else if ev.id == tray.nas_drain_item.id() {
        let new_state = !state.read().unwrap().nas_drain;
        state.write().unwrap().nas_drain = new_state;
        tray.nas_drain_item.set_checked(new_state);
        let url = cfg.server_url.clone();
        let val = if new_state { "true" } else { "false" };
        thread::spawn(move || { let _ = api::set_setting(&url, "nas_drain", val); });
        info!("NAS scan {}", if new_state { "paused" } else { "resumed" });
        platform::get_platform().notify("Enkodu", if new_state { "NAS scan paused" } else { "NAS scan resumed" });

    } else if ev.id == tray.reconcile_item.id() {
        info!("Reconcile triggered manually");
        let cfg = cfg.clone();
        thread::spawn(move || {
            platform::get_platform().notify("Enkodu", "Reconcile: scanning local files...");
            let files = scan::scan(&cfg);
            reconcile::reconcile(&cfg, &files);
        });

    } else if ev.id == tray.config_item.id() {
        let _ = platform::get_platform().open_path(&Config::path());

    } else if ev.id == tray.login_item.id() {
        let platform = platform::get_platform();
        let enabled = !platform.autostart_enabled();
        let _ = platform.set_autostart(enabled);
        tray.login_item.set_checked(enabled);
    }
}

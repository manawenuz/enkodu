//! Command dispatch for CLI and IPC.
//!
//! This module handles commands that can be sent to a running companion instance
//! via CLI arguments or IPC mechanism.

use anyhow::Result;
use std::sync::Arc;
use std::thread;

use crate::api;
use crate::config::Config;
use crate::core::ServerState;
use crate::scan;
use crate::core::batch;
use crate::reconcile;
use crate::platform;

/// Dispatch a command string to the appropriate handler.
pub fn dispatch(
    cmd: &str,
    cfg: &Config,
    state: &Arc<std::sync::RwLock<ServerState>>,
) -> String {
    let platform = crate::platform::get_platform();
    match cmd {
        "scan" => {
            let cfg2 = cfg.clone();
            let mac_drain = state.read().unwrap().mac_drain;
            let platform_ref = platform;
            thread::spawn(move || {
                if mac_drain {
                    platform_ref.notify("Enkodu", "Mac submissions paused — scan skipped");
                    return;
                }
                platform_ref.notify("Enkodu", "Scanning for eligible videos...");
                batch::batch_bg(cfg2, mac_drain);
            });
            "ok: batch scan triggered".to_string()
        }
        "reconcile" => {
            let cfg2 = cfg.clone();
            let platform_ref = platform;
            thread::spawn(move || {
                platform_ref.notify("Enkodu", "Reconcile: scanning local files...");
                let files = scan::scan(&cfg2);
                reconcile::reconcile(&cfg2, &files);
            });
            "ok: reconcile triggered".to_string()
        }
        "status" => {
            let s = state.read().unwrap();
            format!(
                "online={} pending={} active={} done={} failed={} mac_drain={} nas_drain={}",
                s.online, s.pending, s.active, s.done, s.failed, s.mac_drain, s.nas_drain
            )
        }
        "pause-nas" => {
            state.write().unwrap().nas_drain = true;
            let url = cfg.server_url.clone();
            thread::spawn(move || { let _ = api::set_setting(&url, "nas_drain", "true"); });
            platform.notify("Enkodu", "NAS scan paused");
            "ok: NAS scan paused".to_string()
        }
        "resume-nas" => {
            state.write().unwrap().nas_drain = false;
            let url = cfg.server_url.clone();
            thread::spawn(move || { let _ = api::set_setting(&url, "nas_drain", "false"); });
            platform.notify("Enkodu", "NAS scan resumed");
            "ok: NAS scan resumed".to_string()
        }
        "pause-mac" => {
            state.write().unwrap().mac_drain = true;
            platform.notify("Enkodu", "Mac submissions paused");
            "ok: Mac submissions paused".to_string()
        }
        "resume-mac" => {
            state.write().unwrap().mac_drain = false;
            platform.notify("Enkodu", "Mac submissions resumed");
            "ok: Mac submissions resumed".to_string()
        }
        other => format!(
            "err: unknown command '{}' — try: scan, reconcile, status, pause-nas, resume-nas, pause-mac, resume-mac",
            other
        ),
    }
}

/// Diagnostic command: TCP ping test.
pub fn cmd_tcpping(addr: &str) {
    use std::net::TcpStream;
    use std::time::Instant;
    println!("tcpping {} (stdlib TcpStream, 3 attempts)", addr);
    for i in 1..=3u8 {
        let t = Instant::now();
        match TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap()),
            std::time::Duration::from_secs(5),
        ) {
            Ok(_) => println!("  [{}] ok  {:.1}ms", i, t.elapsed().as_secs_f64() * 1000.0),
            Err(e) => println!("  [{}] err {}", i, e),
        }
    }
}

/// Diagnostic command: HTTP ping test.
pub fn cmd_httping(url: &str) {
    use std::time::Instant;
    println!("httping {} (reqwest blocking, 3 attempts)", url);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("client");
    for i in 1..=3u8 {
        let t = Instant::now();
        match client.get(url).send() {
            Ok(r) => println!(
                "  [{}] ok  HTTP {}  {:.1}ms",
                i,
                r.status(),
                t.elapsed().as_secs_f64() * 1000.0
            ),
            Err(e) => println!("  [{}] err {}", i, e),
        }
    }
}

/// WAN sync command.
pub fn cmd_wanryo(cfg: &Config) -> Result<()> {
    use crate::wanryo;
    wanryo::run(cfg)
}

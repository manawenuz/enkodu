//! Command dispatch for CLI and IPC.
//!
//! This module handles commands that can be sent to a running companion instance
//! via CLI arguments or IPC mechanism.

use anyhow::Result;
use std::sync::Arc;
use std::thread;

use crate::api;
use crate::config::Config;
use crate::core::batch;
use crate::core::ServerState;
use crate::reconcile;
use crate::scan;
/// Dispatch a command string to the appropriate handler.
pub fn dispatch(cmd: &str, cfg: &Config, state: &Arc<std::sync::RwLock<ServerState>>) -> String {
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
            let token = cfg.auth_token.clone();
            thread::spawn(move || { let _ = api::set_setting(&url, token.as_deref(), "nas_drain", "true"); });
            platform.notify("Enkodu", "NAS scan paused");
            "ok: NAS scan paused".to_string()
        }
        "resume-nas" => {
            state.write().unwrap().nas_drain = false;
            let url = cfg.server_url.clone();
            let token = cfg.auth_token.clone();
            thread::spawn(move || { let _ = api::set_setting(&url, token.as_deref(), "nas_drain", "false"); });
            platform.notify("Enkodu", "NAS scan resumed");
            "ok: NAS scan resumed".to_string()
        }
        "pause-local" | "pause-mac" => {
            state.write().unwrap().mac_drain = true;
            platform.notify("Enkodu", "Local submissions paused");
            "ok: Local submissions paused".to_string()
        }
        "resume-local" | "resume-mac" => {
            state.write().unwrap().mac_drain = false;
            platform.notify("Enkodu", "Local submissions resumed");
            "ok: Local submissions resumed".to_string()
        }
        other => format!(
            "err: unknown command '{}' — try: scan, reconcile, status, pause-nas, resume-nas, pause-local, resume-local",
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
            &addr
                .parse()
                .unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ServerState;
    use std::sync::{Arc, RwLock};

    fn test_state() -> Arc<RwLock<ServerState>> {
        Arc::new(RwLock::new(ServerState::default()))
    }

    fn test_cfg() -> Config {
        Config {
            server_url: "https://example.invalid".to_string(),
            auth_token: None,
            scan: crate::config::ScanConfig {
                directories: vec![],
                extensions: vec![],
            },
            behavior: crate::config::BehaviorConfig {
                mode: "batch".to_string(),
                on_success: "rename".to_string(),
                backup_suffix: ".bak".to_string(),
                skip_if_av1: true,
                min_duration_secs: 30,
                review_mode: crate::config::ReviewMode::Auto,
            },
            companion_id: String::new(),
        }
    }

    #[test]
    fn dispatch_status_returns_counts() {
        let cfg = test_cfg();
        let state = test_state();
        let resp = dispatch("status", &cfg, &state);
        assert!(resp.contains("online="));
        assert!(resp.contains("pending="));
        assert!(resp.contains("mac_drain="));
    }

    #[test]
    fn dispatch_status_reflects_current_flags_and_counts() {
        let cfg = test_cfg();
        let state = test_state();
        {
            let mut s = state.write().unwrap();
            s.online = true;
            s.pending = 3;
            s.active = 1;
            s.done = 8;
            s.failed = 2;
            s.mac_drain = true;
            s.nas_drain = true;
        }

        let resp = dispatch("status", &cfg, &state);
        assert!(resp.contains("online=true"));
        assert!(resp.contains("pending=3"));
        assert!(resp.contains("active=1"));
        assert!(resp.contains("done=8"));
        assert!(resp.contains("failed=2"));
        assert!(resp.contains("mac_drain=true"));
        assert!(resp.contains("nas_drain=true"));
    }

    #[test]
    fn dispatch_pause_local_sets_mac_drain() {
        let cfg = test_cfg();
        let state = test_state();
        let resp = dispatch("pause-local", &cfg, &state);
        assert!(resp.contains("Local submissions paused"));
        assert!(state.read().unwrap().mac_drain);
    }

    #[test]
    fn dispatch_pause_mac_alias() {
        let cfg = test_cfg();
        let state = test_state();
        let resp = dispatch("pause-mac", &cfg, &state);
        assert!(resp.contains("Local submissions paused"));
        assert!(state.read().unwrap().mac_drain);
    }

    #[test]
    fn dispatch_resume_local_clears_mac_drain() {
        let cfg = test_cfg();
        let state = test_state();
        state.write().unwrap().mac_drain = true;
        let resp = dispatch("resume-local", &cfg, &state);
        assert!(resp.contains("Local submissions resumed"));
        assert!(!state.read().unwrap().mac_drain);
    }

    #[test]
    fn dispatch_resume_mac_alias() {
        let cfg = test_cfg();
        let state = test_state();
        state.write().unwrap().mac_drain = true;
        let resp = dispatch("resume-mac", &cfg, &state);
        assert!(resp.contains("Local submissions resumed"));
        assert!(!state.read().unwrap().mac_drain);
    }

    #[test]
    fn dispatch_pause_nas_sets_nas_drain() {
        let cfg = test_cfg();
        let state = test_state();
        let resp = dispatch("pause-nas", &cfg, &state);
        assert!(resp.contains("NAS scan paused"));
        assert!(state.read().unwrap().nas_drain);
    }

    #[test]
    fn dispatch_resume_nas_clears_nas_drain() {
        let cfg = test_cfg();
        let state = test_state();
        state.write().unwrap().nas_drain = true;
        let resp = dispatch("resume-nas", &cfg, &state);
        assert!(resp.contains("NAS scan resumed"));
        assert!(!state.read().unwrap().nas_drain);
    }

    #[test]
    fn dispatch_unknown_shows_help() {
        let cfg = test_cfg();
        let state = test_state();
        let resp = dispatch("foobar", &cfg, &state);
        assert!(resp.contains("unknown command"));
        assert!(resp.contains("pause-local"));
        assert!(resp.contains("resume-local"));
    }
}

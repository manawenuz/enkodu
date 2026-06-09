use anyhow::{Context, Result};
use log::{info, warn};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, RwLock};
use std::thread;

use crate::api;
use crate::config::Config;
use crate::notify;

pub const SOCK_PATH: &str = "/tmp/enkodu.sock";

// ── server (runs inside the tray process) ────────────────────────────────────

pub fn start_server(cfg: Config, state: Arc<RwLock<crate::ServerState>>) {
    let _ = std::fs::remove_file(SOCK_PATH);
    let listener = match UnixListener::bind(SOCK_PATH) {
        Ok(l) => l,
        Err(e) => { warn!("IPC: cannot bind {}: {}", SOCK_PATH, e); return; }
    };
    info!("IPC: listening on {}", SOCK_PATH);

    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let cfg2 = cfg.clone();
                    let state2 = Arc::clone(&state);
                    thread::spawn(move || handle_conn(s, cfg2, state2));
                }
                Err(e) => warn!("IPC: accept error: {}", e),
            }
        }
    });
}

fn handle_conn(mut stream: UnixStream, cfg: Config, state: Arc<RwLock<crate::ServerState>>) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() { return; }
    let cmd = line.trim().to_string();
    info!("IPC: received command '{}'", cmd);

    let resp = dispatch(&cmd, &cfg, &state);
    let _ = stream.write_all(format!("{}\n", resp).as_bytes());
}

fn dispatch(cmd: &str, cfg: &Config, state: &Arc<RwLock<crate::ServerState>>) -> String {
    match cmd {
        "scan" => {
            let cfg2 = cfg.clone();
            let mac_drain = state.read().unwrap().mac_drain;
            thread::spawn(move || crate::batch_bg(cfg2, mac_drain));
            "ok: batch scan triggered".to_string()
        }
        "reconcile" => {
            let cfg2 = cfg.clone();
            thread::spawn(move || {
                notify("Enkodu", "Reconcile: scanning local files…");
                let files = crate::scan::scan(&cfg2);
                crate::reconcile::reconcile(&cfg2, &files);
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
            notify("Enkodu", "NAS scan paused");
            "ok: NAS scan paused".to_string()
        }
        "resume-nas" => {
            state.write().unwrap().nas_drain = false;
            let url = cfg.server_url.clone();
            thread::spawn(move || { let _ = api::set_setting(&url, "nas_drain", "false"); });
            notify("Enkodu", "NAS scan resumed");
            "ok: NAS scan resumed".to_string()
        }
        "pause-mac" => {
            state.write().unwrap().mac_drain = true;
            notify("Enkodu", "Mac submissions paused");
            "ok: Mac submissions paused".to_string()
        }
        "resume-mac" => {
            state.write().unwrap().mac_drain = false;
            notify("Enkodu", "Mac submissions resumed");
            "ok: Mac submissions resumed".to_string()
        }
        other => format!("err: unknown command '{}' — try: scan, reconcile, status, pause-nas, resume-nas, pause-mac, resume-mac", other),
    }
}

// ── client (runs when user types `enkodu <cmd>`) ──────────────────────────────

pub fn send_cmd(cmd: &str) -> Result<String> {
    let mut stream = UnixStream::connect(SOCK_PATH)
        .context("enkodu is not running (no socket at /tmp/enkodu.sock)")?;
    stream
        .write_all(format!("{}\n", cmd).as_bytes())
        .context("write to socket")?;
    let mut resp = String::new();
    BufReader::new(stream)
        .read_line(&mut resp)
        .context("read response")?;
    Ok(resp.trim().to_string())
}

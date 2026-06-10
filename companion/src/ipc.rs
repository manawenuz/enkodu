//! Unix domain socket IPC for inter-process communication.
//!
//! This module provides the IPC server that runs inside the tray process
//! and the client that CLI commands use to send commands to the running instance.
//!
//! Note: This module is Unix-only. For Windows, see platform/windows.rs IPC implementation.

#[cfg(unix)]
use anyhow::{Context, Result};
#[cfg(unix)]
use log::{info, warn};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::sync::{Arc, RwLock};
#[cfg(unix)]
use std::thread;

#[cfg(unix)]
use crate::config::Config;
#[cfg(unix)]
use crate::core::commands;
#[cfg(unix)]
use crate::core::ServerState;

#[cfg(unix)]
pub const SOCK_PATH: &str = "/tmp/enkodu.sock";

// ── server (runs inside the tray process) ────────────────────────────────────

#[cfg(unix)]
pub fn start_server(cfg: Config, state: Arc<RwLock<ServerState>>) {
    let _ = std::fs::remove_file(SOCK_PATH);
    let listener = match UnixListener::bind(SOCK_PATH) {
        Ok(l) => l,
        Err(e) => {
            warn!("IPC: cannot bind {}: {}", SOCK_PATH, e);
            return;
        }
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

#[cfg(unix)]
fn handle_conn(mut stream: UnixStream, cfg: Config, state: Arc<RwLock<ServerState>>) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let cmd = line.trim().to_string();
    info!("IPC: received command '{}'", cmd);

    let resp = commands::dispatch(&cmd, &cfg, &state);
    let _ = stream.write_all(format!("{}\n", resp).as_bytes());
}

// ── client (runs when user types `enkodu <cmd>`) ──────────────────────────────

#[cfg(unix)]
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

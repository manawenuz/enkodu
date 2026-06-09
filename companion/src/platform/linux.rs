//! Linux-specific platform adapter (stub for Phase 1).
//!
//! This is a stub implementation. Full Linux support will be implemented
//! in Phase 2 of the PRD.

use anyhow::Result;
use log::info;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::core::ServerState;
use crate::platform::{Platform, SingleInstanceGuard};

/// Linux platform implementation (stub).
pub struct LinuxPlatform;

impl Platform for LinuxPlatform {
    fn notify(&self, title: &str, body: &str) {
        info!("[notify] {}: {}", title, body);
        // TODO: Phase 2 - implement Linux desktop notifications
        // Use libnotify or similar
    }

    fn config_dir(&self) -> PathBuf {
        // XDG config home or fallback
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("enkodu")
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/enkodu")
        }
    }

    fn state_dir(&self) -> PathBuf {
        // XDG state home or fallback to config dir
        if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
            PathBuf::from(xdg).join("enkodu")
        } else {
            self.config_dir()
        }
    }

    fn open_path(&self, path: &std::path::Path) -> Result<()> {
        let _ = open::that(path);
        Ok(())
    }

    fn open_url(&self, url: &str) -> Result<()> {
        let _ = open::that(url);
        Ok(())
    }

    fn autostart_enabled(&self) -> bool {
        // TODO: Phase 2 - check XDG autostart or systemd user service
        false
    }

    fn set_autostart(&self, enabled: bool) -> Result<()> {
        // TODO: Phase 2 - create/remove XDG autostart desktop file
        // or systemd user service
        Ok(())
    }

    fn acquire_single_instance_lock(&self) -> Result<SingleInstanceGuard> {
        use std::io::{Read, Write};
        use std::os::unix::net::{UnixListener, UnixStream};
        let path = std::env::temp_dir().join("enkodu.lock");

        // Check if a lock file exists with a live PID
        if let Ok(mut f) = std::fs::File::open(&path) {
            let mut buf = String::new();
            let _ = f.read_to_string(&mut buf);
            if let Ok(pid) = buf.trim().parse::<u32>() {
                // On Unix, kill -0 checks if process exists without signalling it
                let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
                if alive {
                    anyhow::bail!("enkodu is already running (pid {})", pid);
                }
            }
            // Stale lock — remove it
            let _ = std::fs::remove_file(&path);
        }

        // Write our PID
        let pid = std::process::id();
        std::fs::write(&path, format!("{}", pid))?;

        Ok(SingleInstanceGuard { path })
    }

    fn start_ipc_server(&self, cfg: Config, state: Arc<std::sync::RwLock<ServerState>>) {
        // TODO: Phase 2 - use Unix socket IPC (same as macOS)
        // For now, reuse the macOS implementation
        use crate::ipc;
        ipc::start_server(cfg, state);
    }

    fn send_ipc_command(&self, cmd: &str) -> Result<String> {
        // TODO: Phase 2 - use Unix socket IPC (same as macOS)
        use crate::ipc;
        ipc::send_cmd(cmd)
    }
}

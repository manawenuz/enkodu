//! Windows-specific platform adapter (stub for Phase 1).
//!
//! This is a stub implementation. Full Windows support will be implemented
//! in Phase 3 of the PRD.

use anyhow::Result;
use log::info;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::core::ServerState;
use crate::platform::{Platform, SingleInstanceGuard};

/// Windows platform implementation (stub).
pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn notify(&self, title: &str, body: &str) {
        info!("[notify] {}: {}", title, body);
        // TODO: Phase 3 - implement Windows desktop notifications
        // Use winrt-notification or similar
    }

    fn config_dir(&self) -> PathBuf {
        // %APPDATA%\Enkodu
        std::env::var("APPDATA")
            .map(|p| PathBuf::from(p).join("Enkodu"))
            .unwrap_or_else(|_| PathBuf::from(".enkodu"))
    }

    fn state_dir(&self) -> PathBuf {
        // %LOCALAPPDATA%\Enkodu
        std::env::var("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Enkodu"))
            .unwrap_or_else(|_| PathBuf::from(".enkodu"))
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
        // TODO: Phase 3 - check registry Run key or Startup folder or Scheduled Task
        false
    }

    fn set_autostart(&self, enabled: bool) -> Result<()> {
        // TODO: Phase 3 - create/remove registry Run key or Scheduled Task
        Ok(())
    }

    fn acquire_single_instance_lock(&self) -> Result<SingleInstanceGuard> {
        // TODO: Phase 3 - use Windows named mutex for single instance
        // For now, use a file-based lock
        use std::io::{Read, Write};
        let path = std::env::temp_dir().join("enkodu.lock");

        if path.exists() {
            // Try to remove stale lock
            let _ = std::fs::remove_file(&path);
        }

        // Write our PID
        let pid = std::process::id();
        std::fs::write(&path, format!("{}", pid))?;

        Ok(SingleInstanceGuard { path })
    }

    fn start_ipc_server(&self, cfg: Config, state: Arc<std::sync::RwLock<ServerState>>) {
        // TODO: Phase 3 - use named pipe IPC on Windows
        // For now, this is a no-op stub
        info!("Windows IPC server stub - not implemented yet");
    }

    fn send_ipc_command(&self, cmd: &str) -> Result<String> {
        // TODO: Phase 3 - use named pipe IPC on Windows
        anyhow::bail!("Windows IPC client stub - not implemented yet");
    }
}

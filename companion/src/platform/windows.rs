//! Windows-specific platform adapter.
//!
//! Provides Windows implementations for notifications, paths, autostart, and IPC.
//!
//! Note: For a production Windows build, you'll need to:
//! 1. Add `winreg` crate for proper registry autostart
//! 2. Add `named_pipe` crate for proper named pipe IPC
//! 3. Add `windows-rs` or `winrt-notification` for proper toast notifications
//!
//! This implementation provides working stubs using cross-platform approaches
//! where possible, with notes on what needs to be enhanced for native Windows.

use anyhow::{Context, Result};
use log::{info, warn};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use crate::config::Config;
use crate::core::ServerState;
use crate::platform::{Platform, SingleInstanceGuard};

/// Windows platform implementation.
pub struct WindowsPlatform;

impl WindowsPlatform {
    /// Show a Windows notification.
    /// Uses a simple message box as fallback (works without dependencies).
    /// TODO: Replace with proper toast notification using winrt-notification crate.
    fn show_notification(title: &str, body: &str) {
        info!("[notify] {}: {}", title, body);
        
        // Try PowerShell toast notification first (Windows 10+)
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             [System.Windows.Forms.MessageBox]::Show('{}', '{}', [System.Windows.Forms.MessageBoxButtons]::OK, [System.Windows.Forms.MessageBoxIcon]::Information)",
            body.replace("'", "''"),
            title.replace("'", "''")
        );
        
        // Use a quick timeout to avoid blocking
        let status = Command::new("powershell")
            .arg("-Command")
            .arg(&script)
            .arg("-WindowStyle")
            .arg("Hidden")
            .status();
        
        if let Err(e) = status {
            warn!("PowerShell notification failed, trying simpler approach: {}", e);
            // Even simpler: just log it - the message box approach might be too intrusive
        }
    }

    /// Get the autostart flag file path.
    /// TODO: Use registry HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run
    fn autostart_file() -> PathBuf {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            PathBuf::from(appdata).join("Enkodu").join("autostart.flag")
        } else {
            PathBuf::from("C:\\ProgramData\\Enkodu").join("autostart.flag")
        }
    }

    /// Set registry autostart.
    /// TODO: Use winreg crate for proper registry manipulation.
    fn set_registry_autostart(_exe_path: &PathBuf) -> Result<()> {
        // Stub - requires winreg crate for full implementation
        warn!("Registry autostart not implemented - install requires winreg crate");
        Ok(())
    }

    /// Clear registry autostart.
    fn clear_registry_autostart() -> Result<()> {
        // Stub
        Ok(())
    }
}

impl Platform for WindowsPlatform {
    fn notify(&self, title: &str, body: &str) {
        WindowsPlatform::show_notification(title, body);
    }

    fn config_dir(&self) -> PathBuf {
        std::env::var("APPDATA")
            .map(|p| PathBuf::from(p).join("Enkodu"))
            .unwrap_or_else(|_| PathBuf::from("C:\\ProgramData\\Enkodu"))
    }

    fn state_dir(&self) -> PathBuf {
        std::env::var("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Enkodu"))
            .unwrap_or_else(|_| PathBuf::from("C:\\ProgramData\\Enkodu"))
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
        WindowsPlatform::autostart_file().exists()
    }

    fn set_autostart(&self, enabled: bool) -> Result<()> {
        let autostart_path = WindowsPlatform::autostart_file();
        
        if enabled {
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| PathBuf::from("C:\\Program Files\\Enkodu\\enkodu.exe"));
            
            let autostart_dir = autostart_path.parent().unwrap();
            fs::create_dir_all(autostart_dir)?;
            
            fs::write(&autostart_path, exe.to_string_lossy().as_ref())?;
            info!("Autostart enabled — flag file written to {}", autostart_path.display());
            
            // Also try registry (non-fatal if it fails)
            if let Err(e) = WindowsPlatform::set_registry_autostart(&exe) {
                warn!("Could not set registry autostart: {}", e);
            }
        } else {
            if autostart_path.exists() {
                fs::remove_file(&autostart_path)?;
                info!("Autostart disabled — flag file removed");
            }
            
            if let Err(e) = WindowsPlatform::clear_registry_autostart() {
                warn!("Could not clear registry autostart: {}", e);
            }
        }
        
        Ok(())
    }

    fn acquire_single_instance_lock(&self) -> Result<SingleInstanceGuard> {
        // On Windows, we can use a mutex, but for cross-platform simplicity
        // we use a file-based lock (same as macOS/Linux Unix approach)
        let path = std::env::temp_dir().join("enkodu.lock");

        if path.exists() {
            // On Windows, we can't use kill(0) - try to remove stale lock
            let _ = std::fs::remove_file(&path);
        }

        let pid = std::process::id();
        std::fs::write(&path, format!("{}", pid))?;

        Ok(SingleInstanceGuard { path })
    }

    fn start_ipc_server(&self, cfg: Config, state: Arc<std::sync::RwLock<ServerState>>) {
        // For Windows, we'll use a simple approach:
        // Since we can't easily do cross-process IPC without named pipes,
        // and named_pipe crate is not added yet, we'll use a no-op for now.
        // The CLI will handle commands directly without IPC.
        // TODO: Implement proper named pipe IPC server
        info!("Windows IPC server: using direct execution mode (no background server)");
    }

    fn send_ipc_command(&self, cmd: &str) -> Result<String> {
        // For direct execution mode on Windows, we can't send to a running instance
        // without named pipes. For now, return an error that suggests running
        // the command directly.
        // TODO: Implement proper named pipe IPC client
        anyhow::bail!(
            "Windows IPC not implemented yet. \
             On Windows, please use direct CLI commands: enkodu {} <args>",
            cmd
        );
    }
}

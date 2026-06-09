//! Linux-specific platform adapter.
//!
//! Provides Linux implementations for notifications, paths, autostart, and IPC.

use anyhow::{Context, Result};
use log::{info, warn};
use std::fs;
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use crate::config::Config;
use crate::core::ServerState;
use crate::ipc::{send_cmd as unix_send_cmd, start_server as unix_start_server, SOCK_PATH};
use crate::platform::{Platform, SingleInstanceGuard};

/// Linux platform implementation.
pub struct LinuxPlatform;

impl LinuxPlatform {
    /// Check if notify-send is available on the system.
    fn has_notify_send() -> bool {
        Command::new("notify-send").arg("--version").output().is_ok()
    }

    /// Get the XDG autostart directory path.
    fn xdg_autostart_dir() -> PathBuf {
        if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg_config).join("autostart")
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".config/autostart")
        }
    }

    /// Get the autostart desktop file path.
    fn autostart_file() -> PathBuf {
        Self::xdg_autostart_dir().join("enkodu.desktop")
    }

    /// Generate a desktop file for autostart.
    fn generate_desktop_file(exe_path: &str) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Enkodu\nExec={}\nOnlyShowIn=XFCE;GNOME;KDE;\nNoDisplay=false\nHidden=false\n",
            exe_path
        )
    }
}

impl Platform for LinuxPlatform {
    fn notify(&self, title: &str, body: &str) {
        info!("[notify] {}: {}", title, body);
        if LinuxPlatform::has_notify_send() {
            let _ = Command::new("notify-send")
                .arg(title)
                .arg(body)
                .arg("--app-name=enkodu")
                .status();
        } else {
            warn!("notify-send not found — notifications disabled");
        }
    }

    fn config_dir(&self) -> PathBuf {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("enkodu")
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/enkodu")
        }
    }

    fn state_dir(&self) -> PathBuf {
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
        LinuxPlatform::autostart_file().exists()
    }

    fn set_autostart(&self, enabled: bool) -> Result<()> {
        let autostart_path = LinuxPlatform::autostart_file();
        
        if enabled {
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| PathBuf::from("/usr/local/bin/enkodu"));
            
            let autostart_dir = autostart_path.parent().unwrap();
            fs::create_dir_all(autostart_dir)?;
            
            let desktop_content = LinuxPlatform::generate_desktop_file(&exe.display().to_string());
            fs::write(&autostart_path, desktop_content)?;
            
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&autostart_path)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&autostart_path, perms)?;
            }
            
            info!("Autostart enabled — desktop file written to {}", autostart_path.display());
        } else {
            if autostart_path.exists() {
                fs::remove_file(&autostart_path)?;
                info!("Autostart disabled — desktop file removed");
            }
        }
        
        Ok(())
    }

    fn acquire_single_instance_lock(&self) -> Result<SingleInstanceGuard> {
        let path = std::env::temp_dir().join("enkodu.lock");

        if let Ok(mut f) = std::fs::File::open(&path) {
            let mut buf = String::new();
            let _ = f.read_to_string(&mut buf);
            if let Ok(pid) = buf.trim().parse::<u32>() {
                let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
                if alive {
                    anyhow::bail!("enkodu is already running (pid {})", pid);
                }
            }
            let _ = std::fs::remove_file(&path);
        }

        let pid = std::process::id();
        std::fs::write(&path, format!("{}", pid))?;

        Ok(SingleInstanceGuard { path })
    }

    fn start_ipc_server(&self, cfg: Config, state: Arc<std::sync::RwLock<ServerState>>) {
        unix_start_server(cfg, state);
    }

    fn send_ipc_command(&self, cmd: &str) -> Result<String> {
        unix_send_cmd(cmd)
    }
}

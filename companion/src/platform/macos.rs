//! macOS-specific platform adapter.

use anyhow::Result;
use log::info;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use crate::config::Config;
use crate::core::ServerState;
use crate::ipc::{send_cmd as unix_send_cmd, start_server as unix_start_server, SOCK_PATH};
use crate::platform::{Platform, SingleInstanceGuard};

/// macOS platform implementation.
pub struct MacPlatform;

impl MacPlatform {
    fn applescript_quote(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

impl Platform for MacPlatform {
    fn notify(&self, title: &str, body: &str) {
        info!("[notify] {}: {}", title, body);
        // Use osascript directly — notify_rust triggers a "Choose Application" dialog
        // on macOS because it registers a click-action handler named "use_default".
        let script = format!(
            "display notification {} with title {}",
            MacPlatform::applescript_quote(body),
            MacPlatform::applescript_quote(title),
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output();
    }

    fn config_dir(&self) -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/enkodu")
    }

    fn state_dir(&self) -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/enkodu")
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
        launch_agent_exists()
    }

    fn set_autostart(&self, enabled: bool) -> Result<()> {
        toggle_launch_agent(enabled)
    }

    fn acquire_single_instance_lock(&self) -> Result<SingleInstanceGuard> {
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
                    anyhow::bail!("enkodu is already running (pid {})", pid);
                }
            }
            // Stale lock — remove it
            let _ = std::fs::remove_file(&path);
        }

        // Write our PID
        let pid = std::process::id();
        std::fs::write(&path, format!("{}", pid))?;

        // Set up atexit handler to remove lock on exit
        setup_lock_cleanup(&path);

        Ok(SingleInstanceGuard { path })
    }

    fn start_ipc_server(&self, cfg: Config, state: Arc<std::sync::RwLock<ServerState>>) {
        unix_start_server(cfg, state);
    }

    fn send_ipc_command(&self, cmd: &str) -> Result<String> {
        unix_send_cmd(cmd)
    }
}

// ── pid lock ───────────────────────────────────────────────────────────────────

fn pid_lock_path() -> PathBuf {
    std::env::temp_dir().join("enkodu.lock")
}

static mut LOCK_PATH: Option<PathBuf> = None;

extern "C" fn remove_lock_on_exit() {
    unsafe {
        if let Some(ref p) = LOCK_PATH {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn setup_lock_cleanup(path: &PathBuf) {
    unsafe {
        LOCK_PATH = Some(path.clone());
        libc::atexit(remove_lock_on_exit);
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

fn toggle_launch_agent(enabled: bool) -> Result<()> {
    let path = launch_agent_path();
    if enabled {
        if path.exists() {
            return Ok(());
        }
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
        std::fs::write(&path, plist)?;
        info!("LaunchAgent written to {}", path.display());
    } else {
        if path.exists() {
            std::fs::remove_file(&path)?;
            info!("LaunchAgent removed");
        }
    }
    Ok(())
}

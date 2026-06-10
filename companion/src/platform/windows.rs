//! Windows-specific platform adapter.
//!
//! Provides Windows implementations for notifications, paths, autostart, and IPC.

use anyhow::{Context, Result};
use log::{info, warn};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::core::commands;
use crate::core::ServerState;
use crate::platform::{Platform, SingleInstanceGuard};

/// Windows platform implementation.
pub struct WindowsPlatform;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "EnkoduCompanion";
const IPC_HOST: &str = "127.0.0.1";

#[derive(Debug, Serialize, Deserialize)]
struct WindowsIpcMetadata {
    port: u16,
    auth_token: String,
    pid: u32,
}

impl WindowsPlatform {
    /// Show a Windows 10+ toast notification via PowerShell WinRT bindings.
    /// Non-blocking: spawns PowerShell and returns immediately.
    fn show_notification(title: &str, body: &str) {
        info!("[notify] {}: {}", title, body);

        let title_esc = title.replace('\'', "\\'");
        let body_esc = body.replace('\'', "\\'");
        // Use Windows.UI.Notifications WinRT toast — works on Windows 10+ without extra modules.
        // 'Windows PowerShell' is a registered AUMID usable as notifier fallback.
        let script = format!(
            "$null=[Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime]; \
             $t=[Windows.UI.Notifications.ToastTemplateType]::ToastText02; \
             $x=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent($t); \
             $e=$x.GetElementsByTagName('text'); \
             $null=$e[0].AppendChild($x.CreateTextNode('{title}')); \
             $null=$e[1].AppendChild($x.CreateTextNode('{body}')); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Windows PowerShell').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
            title = title_esc,
            body = body_esc,
        );

        if let Err(e) = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
            .spawn()
        {
            warn!("Toast notification spawn failed: {}", e);
        }
    }

    /// Legacy autostart flag file path from the placeholder implementation.
    fn legacy_autostart_flag() -> PathBuf {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            PathBuf::from(appdata).join("Enkodu").join("autostart.flag")
        } else {
            PathBuf::from("C:\\ProgramData\\Enkodu").join("autostart.flag")
        }
    }

    fn state_dir_path() -> PathBuf {
        std::env::var("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Enkodu"))
            .unwrap_or_else(|_| PathBuf::from("C:\\ProgramData\\Enkodu"))
    }

    fn ipc_metadata_path() -> PathBuf {
        Self::state_dir_path().join("ipc.json")
    }

    fn generate_auth_token() -> String {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    }

    fn write_ipc_metadata(path: &Path, metadata: &WindowsIpcMetadata) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let body = serde_json::to_vec(metadata).context("serialize Windows IPC metadata")?;
        fs::write(path, body).with_context(|| format!("write {}", path.display()))
    }

    fn load_ipc_metadata() -> Result<WindowsIpcMetadata> {
        let path = Self::ipc_metadata_path();
        let body = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_slice(&body).context("parse Windows IPC metadata")
    }

    fn quoted_exe(exe_path: &Path) -> String {
        format!("\"{}\"", exe_path.display())
    }

    fn registry_autostart_enabled() -> bool {
        Command::new("reg")
            .args(["query", RUN_KEY, "/v", RUN_VALUE])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn set_registry_autostart(exe_path: &Path) -> Result<()> {
        let value = Self::quoted_exe(exe_path);
        let output = Command::new("reg")
            .args([
                "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &value, "/f",
            ])
            .output()
            .context("run reg add for autostart")?;

        if !output.status.success() {
            anyhow::bail!(
                "reg add failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        Ok(())
    }

    fn clear_registry_autostart() -> Result<()> {
        if !Self::registry_autostart_enabled() {
            return Ok(());
        }

        let output = Command::new("reg")
            .args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])
            .output()
            .context("run reg delete for autostart")?;

        if !output.status.success() {
            anyhow::bail!(
                "reg delete failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        Ok(())
    }

    fn handle_ipc_client(
        mut stream: TcpStream,
        expected_token: &str,
        cfg: &Config,
        state: &Arc<std::sync::RwLock<ServerState>>,
    ) {
        let mut auth_line = String::new();
        let mut cmd_line = String::new();

        let response = match stream.try_clone() {
            Ok(cloned) => {
                let mut reader = BufReader::new(cloned);
                match reader.read_line(&mut auth_line) {
                    Ok(0) => return,
                    Ok(_) => {}
                    Err(e) => {
                        warn!("Windows IPC auth read failed: {}", e);
                        return;
                    }
                }
                match reader.read_line(&mut cmd_line) {
                    Ok(0) => "err: missing command".to_string(),
                    Ok(_) => {
                        let auth = auth_line.trim_end();
                        let cmd = cmd_line.trim_end();
                        if auth != expected_token {
                            "err: unauthorized".to_string()
                        } else {
                            info!("Windows IPC: received command '{}'", cmd);
                            commands::dispatch(cmd, cfg, state)
                        }
                    }
                    Err(e) => {
                        warn!("Windows IPC command read failed: {}", e);
                        "err: failed to read command".to_string()
                    }
                }
            }
            Err(e) => {
                warn!("Windows IPC stream clone failed: {}", e);
                return;
            }
        };

        if let Err(e) = stream.write_all(format!("{}\n", response).as_bytes()) {
            warn!("Windows IPC write failed: {}", e);
        }
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
        WindowsPlatform::registry_autostart_enabled()
    }

    fn set_autostart(&self, enabled: bool) -> Result<()> {
        if enabled {
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| PathBuf::from("C:\\Program Files\\Enkodu\\enkodu.exe"));
            WindowsPlatform::set_registry_autostart(&exe)?;
            info!("Autostart enabled via HKCU Run for {}", exe.display());
        } else {
            WindowsPlatform::clear_registry_autostart()?;
            info!("Autostart disabled via HKCU Run");
        }

        let legacy_flag = WindowsPlatform::legacy_autostart_flag();
        if legacy_flag.exists() {
            if let Err(e) = fs::remove_file(&legacy_flag) {
                warn!(
                    "Could not remove legacy autostart flag {}: {}",
                    legacy_flag.display(),
                    e
                );
            } else {
                info!("Removed legacy autostart flag {}", legacy_flag.display());
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
        let listener = match TcpListener::bind((IPC_HOST, 0)) {
            Ok(listener) => listener,
            Err(e) => {
                warn!("Windows IPC: cannot bind {}: {}", IPC_HOST, e);
                return;
            }
        };

        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(e) => {
                warn!("Windows IPC: cannot read bound address: {}", e);
                return;
            }
        };

        let metadata = WindowsIpcMetadata {
            port,
            auth_token: WindowsPlatform::generate_auth_token(),
            pid: std::process::id(),
        };
        let metadata_path = WindowsPlatform::ipc_metadata_path();
        if let Err(e) = WindowsPlatform::write_ipc_metadata(&metadata_path, &metadata) {
            warn!(
                "Windows IPC: cannot write {}: {}",
                metadata_path.display(),
                e
            );
            return;
        }

        info!(
            "Windows IPC: listening on {}:{} (metadata {})",
            IPC_HOST,
            port,
            metadata_path.display()
        );

        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let cfg2 = cfg.clone();
                        let state2 = Arc::clone(&state);
                        let token = metadata.auth_token.clone();
                        thread::spawn(move || {
                            WindowsPlatform::handle_ipc_client(stream, &token, &cfg2, &state2);
                        });
                    }
                    Err(e) => warn!("Windows IPC accept error: {}", e),
                }
            }
        });
    }

    fn send_ipc_command(&self, cmd: &str) -> Result<String> {
        let metadata = WindowsPlatform::load_ipc_metadata()
            .context("enkodu companion is not running (missing Windows IPC metadata)")?;
        let addr = format!("{}:{}", IPC_HOST, metadata.port);
        let socket_addr = addr
            .parse()
            .with_context(|| format!("parse Windows IPC address {}", addr))?;
        let mut stream = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(2))
            .with_context(|| format!("connect to running companion at {}", addr))?;
        stream
            .write_all(format!("{}\n{}\n", metadata.auth_token, cmd).as_bytes())
            .context("write Windows IPC command")?;
        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .context("read Windows IPC response")?;
        Ok(response.trim().to_string())
    }
}

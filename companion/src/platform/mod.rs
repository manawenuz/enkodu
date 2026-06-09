//! Platform-specific adapters for the Enkodu companion.
//!
//! Each desktop platform (macOS, Linux, Windows) provides its own implementation
//! of the `Platform` trait.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use crate::core::ServerState;

pub mod macos;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

/// Single-instance lock guard.
/// Dropping this releases the lock.
pub struct SingleInstanceGuard {
    pub path: PathBuf,
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Platform-specific operations that vary by OS.
/// Implementations must be thread-safe (Send + Sync).
pub trait Platform: Send + Sync {
    /// Show a desktop notification.
    fn notify(&self, title: &str, body: &str);

    /// Return the directory for configuration files.
    fn config_dir(&self) -> PathBuf;

    /// Return the directory for state files.
    fn state_dir(&self) -> PathBuf;

    /// Open a file path with the default application.
    fn open_path(&self, path: &std::path::Path) -> Result<()>;

    /// Open a URL with the default browser.
    fn open_url(&self, url: &str) -> Result<()>;

    /// Check if autostart is enabled.
    fn autostart_enabled(&self) -> bool;

    /// Enable or disable autostart.
    fn set_autostart(&self, enabled: bool) -> Result<()>;

    /// Acquire a single-instance lock. Returns Err if another instance is running.
    fn acquire_single_instance_lock(&self) -> Result<SingleInstanceGuard>;

    /// Start the IPC command server (platform-specific transport).
    fn start_ipc_server(
        &self,
        cfg: crate::config::Config,
        state: Arc<std::sync::RwLock<ServerState>>,
    );

    /// Send a command to a running instance (platform-specific transport).
    fn send_ipc_command(&self, cmd: &str) -> Result<String>;
}

/// Get the platform-specific implementation.
pub fn get_platform() -> &'static dyn Platform {
    #[cfg(target_os = "macos")]
    return &macos::MacPlatform;

    #[cfg(target_os = "linux")]
    return &linux::LinuxPlatform;

    #[cfg(target_os = "windows")]
    return &windows::WindowsPlatform;

    // This shouldn't happen but provides a fallback
    static FALLBACK: macos::MacPlatform = macos::MacPlatform;
    &FALLBACK
}

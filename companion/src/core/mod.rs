//! Shared companion core logic.
//!
//! This module contains platform-independent logic for the Enkodu companion.

pub mod commands;
pub mod submit;
pub mod batch;
pub mod poll;

use std::collections::HashMap;

use crate::state::JobEntry;

/// Shared server state for the companion UI and background tasks.
#[derive(Default, Clone)]
pub struct ServerState {
    pub online: bool,
    pub pending: u64,
    pub active: u64,
    pub done: u64,
    pub failed: u64,
    pub encoding_file: Option<String>,
    pub encoding_pct: f64,
    pub encoding_speed: String,
    pub encoding_phase: String,
    pub control_cmd: String,
    pub prev_done: u64,
    pub mac_drain: bool,
    pub nas_drain: bool,
}

/// Local state for tracking submitted jobs.
pub type LocalState = HashMap<String, JobEntry>;

/// Helper to truncate strings for display.
pub fn truncate(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        format!("{}...", chars[..n - 1].iter().collect::<String>())
    }
}

/// Get current timestamp in seconds since epoch.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

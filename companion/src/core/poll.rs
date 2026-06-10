//! Background polling of queue status.

use log::{info, warn};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::api;
use crate::config::Config;
use crate::core::ServerState;

/// Poll the queue server for status updates.
/// This runs in a background thread and updates the shared state.
pub fn poll_loop(cfg: Config, state: Arc<RwLock<ServerState>>) {
    info!("Poll loop started — polling every 5s");
    loop {
        thread::sleep(Duration::from_secs(5));

        // Poll queue status
        match api::queue_status(&cfg.server_url, cfg.auth_token.as_deref()) {
            Ok(s) => {
                let prev = state.read().unwrap().prev_done;
                if s.done > prev && prev > 0 {
                    let n = s.done - prev;
                    info!("{} new completion(s)", n);
                    // Notification will be triggered by the main loop
                }
                let mut st = state.write().unwrap();
                let was_online = st.online;
                st.online = true;
                st.pending = s.pending;
                st.active = s.active;
                st.done = s.done;
                st.failed = s.failed;
                st.prev_done = s.done;
                if !was_online {
                    info!(
                        "Server online — pending={} active={} done={} failed={}",
                        s.pending, s.active, s.done, s.failed
                    );
                }
            }
            Err(e) => {
                warn!("Server unreachable: {:#}", e);
                state.write().unwrap().online = false;
            }
        }

        // Poll live jobs for active encoding info
        if let Ok(live) = api::live_jobs(&cfg.server_url, cfg.auth_token.as_deref()) {
            let mut st = state.write().unwrap();
            if let Some(job) = live.values().next() {
                st.encoding_file = Some(job.file.clone());
                st.encoding_pct = job.percent;
                st.encoding_speed = job.speed.clone();
                st.encoding_phase = job.phase.clone();
            } else {
                st.encoding_file = None;
                st.encoding_pct = 0.0;
                st.encoding_speed = String::new();
                st.encoding_phase = String::new();
            }
        }

        // Poll control status
        if let Ok(cmd) = api::control_status(&cfg.server_url, cfg.auth_token.as_deref()) {
            state.write().unwrap().control_cmd = cmd;
        }

        // Poll NAS drain setting from server
        if let Ok(settings) = api::get_settings(&cfg.server_url, cfg.auth_token.as_deref()) {
            let nas_drain = settings
                .get("nas_drain")
                .map(|v| v == "true")
                .unwrap_or(false);
            state.write().unwrap().nas_drain = nas_drain;
        }
    }
}

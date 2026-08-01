use crate::capabilities::Capabilities;
use crate::config::Config;
use crate::core::submit;
use crate::scan;
use log::{info, warn};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const RECONNECT_DELAY_SECS: u64 = 15;
const HEARTBEAT_INTERVAL_SECS: u64 = 30;
const FILE_LIST_INTERVAL_SECS: u64 = 300; // re-send file list every 5 min
/// If no pong is received within this many seconds, consider the connection dead.
const PONG_TIMEOUT_SECS: u64 = 60;

type WsStream = tungstenite::WebSocket<
    tungstenite::stream::MaybeTlsStream<std::net::TcpStream>,
>;

/// Starts the WebSocket client in a background thread.
/// Returns immediately; the WS connection is maintained in the background.
pub fn start(live_cfg: Arc<RwLock<Config>>, caps: Capabilities) {
    std::thread::spawn(move || {
        ws_loop(live_cfg, caps);
    });
}

fn ws_loop(live_cfg: Arc<RwLock<Config>>, caps: Capabilities) {
    loop {
        let cfg = live_cfg.read().unwrap().clone();
        let server_url = cfg.server_url.clone();
        let companion_id = cfg.companion_id.clone();

        // Convert http:// to ws:// and https:// to wss://
        let ws_url = if server_url.starts_with("https://") {
            format!(
                "wss://{}/ws/companion/{}",
                &server_url["https://".len()..],
                companion_id
            )
        } else {
            let base = server_url.trim_start_matches("http://");
            format!("ws://{}/ws/companion/{}", base, companion_id)
        };

        // Append token if configured
        let ws_url = if let Some(token) = &cfg.auth_token {
            format!("{}?token={}", ws_url, token)
        } else {
            ws_url
        };

        info!(
            "Connecting to WS: {}",
            ws_url.split('?').next().unwrap_or(&ws_url)
        );

        match tungstenite::connect(&ws_url) {
            Ok((mut socket, _)) => {
                info!("WebSocket connected");

                // Send hello
                let hello = json!({
                    "type": "hello",
                    "name": hostname(),
                    "platform": caps.platform,
                    "version": env!("CARGO_PKG_VERSION"),
                    "capabilities": {
                        "encoders": caps.encoders,
                        "decoders": caps.decoders,
                        "ffprobe_available": caps.ffprobe_available,
                    }
                });
                if let Err(e) =
                    socket.send(tungstenite::Message::Text(hello.to_string()))
                {
                    warn!("WS hello send error: {}", e);
                    std::thread::sleep(Duration::from_secs(RECONNECT_DELAY_SECS));
                    continue;
                }

                // FIX M6: collect file list in a background thread before the message
                // loop so that a large scan does not block heartbeats. The scan result
                // is stored in a shared slot; the message loop drains and sends it on
                // the next heartbeat tick rather than blocking inline.
                let pending_file_list: Arc<Mutex<Option<Vec<Value>>>> =
                    Arc::new(Mutex::new(None));

                {
                    // Kick off the initial scan right away.
                    let cfg_snap = live_cfg.read().unwrap().clone();
                    let slot = Arc::clone(&pending_file_list);
                    std::thread::spawn(move || {
                        let entries = collect_file_list(&cfg_snap);
                        *slot.lock().unwrap() = Some(entries);
                    });
                }

                let mut last_heartbeat = Instant::now();
                let mut last_file_list = Instant::now();

                // FIX H8: track last pong to detect dead TLS connections.
                let mut last_pong = Instant::now();

                // Set read timeout on the underlying TCP stream (Plain only; TLS
                // streams don't expose set_read_timeout directly).
                set_stream_read_timeout(socket.get_mut(), Duration::from_secs(5));

                // Message loop
                loop {
                    match socket.read() {
                        Ok(tungstenite::Message::Text(text)) => {
                            let msg: Value = match serde_json::from_str(&text) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };

                            // FIX H8: treat "pong" text messages as proof-of-life.
                            if msg
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                == "pong"
                            {
                                last_pong = Instant::now();
                            }

                            handle_server_message(
                                &msg,
                                &live_cfg,
                                &mut socket,
                            );
                        }
                        Ok(tungstenite::Message::Ping(data)) => {
                            let _ =
                                socket.send(tungstenite::Message::Pong(data));
                        }
                        // FIX H8: binary Pong frame also counts as proof-of-life.
                        Ok(tungstenite::Message::Pong(_)) => {
                            last_pong = Instant::now();
                        }
                        Ok(tungstenite::Message::Close(_)) => {
                            info!("WS server closed connection");
                            break;
                        }
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(ref e))
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            // Timeout is normal — continue to check heartbeat
                        }
                        Err(e) => {
                            warn!("WS read error: {}", e);
                            break;
                        }
                    }

                    // Heartbeat
                    if last_heartbeat.elapsed().as_secs() >= HEARTBEAT_INTERVAL_SECS {
                        // FIX H8: check for dead connection before sending the next
                        // heartbeat. Two missed heartbeat intervals without a pong
                        // means the TLS connection is likely silently dead.
                        if last_pong.elapsed().as_secs() > PONG_TIMEOUT_SECS {
                            warn!(
                                "WS dead (no pong in {}s), reconnecting",
                                PONG_TIMEOUT_SECS
                            );
                            break;
                        }

                        let hb = json!({"type": "heartbeat"});
                        if socket
                            .send(tungstenite::Message::Text(hb.to_string()))
                            .is_err()
                        {
                            break;
                        }
                        last_heartbeat = Instant::now();
                    }

                    // FIX M6: flush any pending file list collected by the background
                    // thread, or trigger a new background scan on the refresh interval.
                    if last_file_list.elapsed().as_secs() >= FILE_LIST_INTERVAL_SECS {
                        let cfg_snap = live_cfg.read().unwrap().clone();
                        let slot = Arc::clone(&pending_file_list);
                        std::thread::spawn(move || {
                            let entries = collect_file_list(&cfg_snap);
                            *slot.lock().unwrap() = Some(entries);
                        });
                        last_file_list = Instant::now();
                    }

                    // Send file list if the background thread has produced one.
                    if let Ok(mut guard) = pending_file_list.try_lock() {
                        if let Some(entries) = guard.take() {
                            send_file_list(&mut socket, &entries);
                        }
                    }
                }

                info!(
                    "WS disconnected — will retry in {}s",
                    RECONNECT_DELAY_SECS
                );
            }
            Err(e) => {
                // Connection failed — not an error, server may be offline
                info!(
                    "WS connection failed: {} — retry in {}s",
                    e, RECONNECT_DELAY_SECS
                );
            }
        }

        std::thread::sleep(Duration::from_secs(RECONNECT_DELAY_SECS));
    }
}

fn set_stream_read_timeout(
    stream: &mut tungstenite::stream::MaybeTlsStream<std::net::TcpStream>,
    timeout: Duration,
) {
    match stream {
        tungstenite::stream::MaybeTlsStream::Plain(tcp) => {
            let _ = tcp.set_read_timeout(Some(timeout));
        }
        // For TLS streams there's no direct way to set read timeout; rely on
        // the pong-timeout heartbeat mechanism to detect dead connections.
        _ => {}
    }
}

/// FIX M6: pure scan — no socket I/O.
fn collect_file_list(cfg: &Config) -> Vec<Value> {
    let files = scan::scan(cfg);
    files
        .iter()
        .map(|f| {
            json!({
                "path": f.path.to_string_lossy(),
                "size": f.size,
                "codec": f.codec,
                "duration": f.duration,
                "width": 0,
                "height": 0,
                "fps": 0.0,
                "bitrate": 0
            })
        })
        .collect()
}

/// FIX M6: write-only — accepts pre-collected entries.
fn send_file_list(socket: &mut WsStream, file_entries: &[Value]) {
    let msg = json!({
        "type": "file_list",
        "files": file_entries
    });

    if let Err(e) = socket.send(tungstenite::Message::Text(msg.to_string())) {
        warn!("WS file_list send error: {}", e);
    } else {
        info!("Sent file list: {} files", file_entries.len());
    }
}

fn handle_server_message(
    msg: &Value,
    live_cfg: &Arc<RwLock<Config>>,
    socket: &mut WsStream,
) {
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match msg_type {
        "welcome" => {
            if let Some(pending_cfg) =
                msg.get("pending_config").filter(|v| !v.is_null())
            {
                info!("Received pending config from server");
                apply_server_config(pending_cfg, live_cfg);
                // FIX H4: acknowledge so the server clears pending_config from DB.
                send_config_ack(socket);
            }
        }
        "config_update" => {
            if let Some(cfg_val) = msg.get("config") {
                info!("Received config update from server");
                apply_server_config(cfg_val, live_cfg);
                // FIX H4: acknowledge so the server clears pending_config from DB.
                send_config_ack(socket);
            }
        }
        "assign_upload" => {
            let job_id = msg
                .get("job_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // The server may send the field as "path" or "file_path".
            let file_path = msg
                .get("path")
                .or_else(|| msg.get("file_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if job_id.is_empty() || file_path.is_empty() {
                warn!(
                    "assign_upload: missing job_id or path in message: {}",
                    msg
                );
                return;
            }

            info!(
                "Received upload assignment: job={} file={}",
                job_id, file_path
            );

            // FIX H7: actually trigger the upload on a background thread.
            let cfg_snap = live_cfg.read().unwrap().clone();
            let path = std::path::PathBuf::from(file_path);

            // SECURITY (WS-1): the server fully controls `file_path`. Refuse any
            // path that does not resolve inside one of the locally-configured
            // scan directories, otherwise a malicious server could exfiltrate
            // (and, in replace mode, overwrite) arbitrary local files.
            if !path_in_scan_dirs(&path, &cfg_snap) {
                warn!(
                    "Rejecting assign_upload for path outside scan dirs: {}",
                    file_path
                );
                return;
            }

            std::thread::spawn(move || {
                submit::submit_bg(cfg_snap, path);
            });
        }
        "control" => {
            let cmd = msg
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("run");
            info!("WS control: {}", cmd);
        }
        "pong" => {
            // Already handled in the message loop for last_pong tracking.
        }
        _ => {}
    }
}

/// FIX H4: send a config_ack so the server knows to clear pending_config.
fn send_config_ack(socket: &mut WsStream) {
    let ack = serde_json::json!({"type": "config_ack"}).to_string();
    if let Err(e) = socket.send(tungstenite::Message::Text(ack)) {
        warn!("Failed to send config_ack: {}", e);
    }
}

/// Expand a leading `~/` to the user's home directory (mirrors scan.rs).
fn expand_tilde(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(rest)
    } else {
        std::path::PathBuf::from(path)
    }
}

/// SECURITY (WS-1): true only if `path` canonicalizes to a location inside one
/// of the configured scan directories. Fails closed on any canonicalize error
/// (e.g. the file does not exist) and on an empty scan-dir list.
fn path_in_scan_dirs(path: &std::path::Path, cfg: &Config) -> bool {
    let real = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    for dir_str in &cfg.scan.directories {
        let dir = expand_tilde(dir_str);
        if let Ok(real_dir) = dir.canonicalize() {
            if real.starts_with(&real_dir) {
                return true;
            }
        }
    }
    false
}

/// Reject scan directories that point at filesystem roots or bare home, which
/// would cause a batch scan to walk and upload arbitrary user files.
fn is_safe_scan_dir(dir: &str) -> bool {
    let trimmed = dir.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Refuse absolute roots and bare home references.
    if matches!(trimmed, "/" | "~" | "~/" | "." | "..") {
        return false;
    }
    // Refuse Windows drive roots like "C:\" / "C:/".
    let bytes = trimmed.as_bytes();
    if bytes.len() <= 3
        && bytes.get(1) == Some(&b':')
        && matches!(bytes.get(2), None | Some(b'\\') | Some(b'/'))
    {
        return false;
    }
    true
}

/// SECURITY (COMP-1 / WS-1): a server-pushed config is NOT trusted wholesale.
/// We never accept security-sensitive or destructive fields from the wire
/// (`server_url`, `auth_token`, `behavior.on_success`, `behavior.backup_suffix`,
/// `behavior.review_mode`) — those stay pinned to the locally-configured values
/// so a malicious/compromised server can neither redirect the bearer token to an
/// attacker host nor flip the companion into in-place "replace" mode. Only a
/// whitelist of non-destructive fields is merged, and pushed scan directories are
/// validated against filesystem roots before being persisted.
fn apply_server_config(cfg_val: &Value, live_cfg: &Arc<RwLock<Config>>) {
    // Start from the current live config and overlay only safe fields.
    let base = live_cfg.read().unwrap().clone();
    let merged = merge_server_config(base, cfg_val);

    if let Err(e) = merged.save() {
        warn!("Failed to save server-pushed config: {}", e);
    } else {
        *live_cfg.write().unwrap() = merged;
        info!("Applied server-pushed config (safe fields only)");
    }
}

/// SECURITY (COMP-1 / WS-1): pure merge of a server-pushed config onto `base`.
/// Only whitelisted, non-destructive fields are overlaid; security-sensitive and
/// destructive fields (`server_url`, `auth_token`, `companion_id`,
/// `behavior.on_success`, `behavior.backup_suffix`, `behavior.review_mode`) are
/// preserved from `base`. Pushed scan directories are filtered through
/// [`is_safe_scan_dir`]. No I/O — the caller persists the result.
fn merge_server_config(base: Config, cfg_val: &Value) -> Config {
    let mut merged = base;

    // scan.directories — validated; reject roots / bare home.
    if let Some(dirs) = cfg_val
        .get("scan")
        .and_then(|s| s.get("directories"))
        .and_then(|d| d.as_array())
    {
        let mut accepted = Vec::new();
        for v in dirs {
            if let Some(s) = v.as_str() {
                if is_safe_scan_dir(s) {
                    accepted.push(s.to_string());
                } else {
                    warn!("Ignoring unsafe server-pushed scan dir: {}", s);
                }
            }
        }
        merged.scan.directories = accepted;
    }

    // scan.extensions — non-destructive.
    if let Some(exts) = cfg_val
        .get("scan")
        .and_then(|s| s.get("extensions"))
        .and_then(|e| e.as_array())
    {
        merged.scan.extensions = exts
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
            .collect();
    }

    // behavior: only non-destructive knobs. on_success / backup_suffix /
    // review_mode are deliberately preserved from the local config.
    if let Some(b) = cfg_val.get("behavior") {
        if let Some(mode) = b.get("mode").and_then(|v| v.as_str()) {
            merged.behavior.mode = mode.to_string();
        }
        if let Some(skip) = b.get("skip_if_av1").and_then(|v| v.as_bool()) {
            merged.behavior.skip_if_av1 = skip;
        }
        if let Some(min) = b.get("min_duration_secs").and_then(|v| v.as_u64()) {
            merged.behavior.min_duration_secs = min;
        }
    }

    merged
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "enkodu-companion".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Create a unique real directory under the OS temp dir so canonicalize()
    /// succeeds. No external crate (tempfile is not a dependency).
    fn unique_tmp_dir(tag: &str) -> std::path::PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "enkodu_ws_test_{}_{}_{}_{}",
            tag,
            std::process::id(),
            nanos,
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn base_config(scan_dirs: Vec<String>) -> Config {
        Config {
            server_url: "https://local.example".to_string(),
            auth_token: Some("LOCAL-SECRET-TOKEN".to_string()),
            scan: crate::config::ScanConfig {
                directories: scan_dirs,
                extensions: vec!["mp4".to_string()],
            },
            behavior: crate::config::BehaviorConfig {
                mode: "interactive".to_string(),
                on_success: "rename".to_string(),
                backup_suffix: ".bak".to_string(),
                skip_if_av1: true,
                min_duration_secs: 30,
                review_mode: crate::config::ReviewMode::Manual,
            },
            companion_id: "local-companion-id".to_string(),
        }
    }

    // ── COMP-1 (a): is_safe_scan_dir rejects roots / bare home ──────────────

    #[test]
    fn is_safe_scan_dir_rejects_filesystem_and_home_roots() {
        // Before the fix these would all be accepted, letting a server push a
        // scan dir that walks the entire filesystem / home and uploads it.
        for bad in ["/", "~", "~/", ".", "..", "", "   "] {
            assert!(
                !is_safe_scan_dir(bad),
                "expected {:?} to be rejected as unsafe",
                bad
            );
        }
    }

    #[test]
    fn is_safe_scan_dir_rejects_windows_drive_roots() {
        for bad in ["C:\\", "C:/", "D:\\", "Z:"] {
            assert!(
                !is_safe_scan_dir(bad),
                "expected windows drive root {:?} to be rejected",
                bad
            );
        }
    }

    #[test]
    fn is_safe_scan_dir_accepts_normal_dirs() {
        let tmp = unique_tmp_dir("safe");
        assert!(is_safe_scan_dir(tmp.to_str().unwrap()));
        assert!(is_safe_scan_dir("~/Movies"));
        assert!(is_safe_scan_dir("/home/user/Videos"));
        // A Windows path deeper than a bare drive root is fine.
        assert!(is_safe_scan_dir("C:\\Users\\me\\Videos"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    // ── COMP-1 (b): apply_server_config / merge_server_config ignores
    //                security + destructive fields ─────────────────────────

    #[test]
    fn merge_server_config_ignores_security_and_destructive_fields() {
        let base = base_config(vec!["~/Movies".to_string()]);
        let pushed = json!({
            // Hostile server attempts to redirect the bearer token + flip mode.
            "server_url": "https://evil.example",
            "auth_token": "ATTACKER-TOKEN",
            "companion_id": "attacker-id",
            "scan": {
                "extensions": ["webm", "MKV"]
            },
            "behavior": {
                "mode": "batch",
                "on_success": "replace",
                "backup_suffix": ".attacker",
                "review_mode": "auto",
                "min_duration_secs": 99
            }
        });

        let merged = merge_server_config(base, &pushed);

        // Security-sensitive / destructive fields MUST stay at local values.
        // Before the fix apply_server_config deserialized the whole Config, so
        // these would have been overwritten with the attacker's values.
        assert_eq!(merged.server_url, "https://local.example");
        assert_eq!(
            merged.auth_token.as_deref(),
            Some("LOCAL-SECRET-TOKEN")
        );
        assert_eq!(merged.companion_id, "local-companion-id");
        assert_eq!(merged.behavior.on_success, "rename");
        assert_eq!(merged.behavior.backup_suffix, ".bak");
        assert_eq!(merged.behavior.review_mode, crate::config::ReviewMode::Manual);

        // Whitelisted non-destructive fields ARE updated.
        assert_eq!(merged.behavior.mode, "batch");
        assert_eq!(merged.behavior.min_duration_secs, 99);
        assert_eq!(
            merged.scan.extensions,
            vec!["webm".to_string(), "mkv".to_string()]
        );
    }

    #[test]
    fn merge_server_config_filters_unsafe_pushed_scan_dirs() {
        let safe = unique_tmp_dir("merge");
        let base = base_config(vec!["~/Movies".to_string()]);
        let pushed = json!({
            "scan": {
                "directories": ["/", "~", safe.to_str().unwrap()]
            }
        });
        let merged = merge_server_config(base, &pushed);
        // Roots dropped; only the safe dir survives. Before the fix the raw
        // server list (including "/") would have been persisted wholesale.
        assert_eq!(
            merged.scan.directories,
            vec![safe.to_str().unwrap().to_string()]
        );
        std::fs::remove_dir_all(&safe).ok();
    }

    #[test]
    fn merge_server_config_ignores_complete_attacker_config() {
        // A COMPLETE, fully-valid Config payload — every field of Config present,
        // exactly the kind a wholesale serde_json::from_value::<Config> would have
        // happily deserialized (and thus overwritten the token/url with) pre-fix.
        // The allowlist merge must still refuse the security/destructive fields.
        let safe = unique_tmp_dir("complete");
        let base = base_config(vec!["~/Movies".to_string()]);
        let pushed = json!({
            "server_url": "https://evil.example",
            "auth_token": "ATTACKER-TOKEN",
            "companion_id": "attacker-id",
            "scan": {
                "directories": [safe.to_str().unwrap()],
                "extensions": ["mkv"]
            },
            "behavior": {
                "mode": "batch",
                "on_success": "replace",
                "backup_suffix": ".attacker",
                "skip_if_av1": false,
                "min_duration_secs": 1,
                "review_mode": "auto"
            }
        });

        let merged = merge_server_config(base, &pushed);

        // Security / destructive fields stay at local values despite a full payload.
        assert_eq!(merged.server_url, "https://local.example");
        assert_eq!(merged.auth_token.as_deref(), Some("LOCAL-SECRET-TOKEN"));
        assert_eq!(merged.companion_id, "local-companion-id");
        assert_eq!(merged.behavior.on_success, "rename");
        assert_eq!(merged.behavior.backup_suffix, ".bak");
        assert_eq!(merged.behavior.review_mode, crate::config::ReviewMode::Manual);
        // Whitelisted non-destructive knobs ARE adopted.
        assert_eq!(merged.behavior.mode, "batch");
        assert!(!merged.behavior.skip_if_av1);
        assert_eq!(merged.behavior.min_duration_secs, 1);
        assert_eq!(merged.scan.directories, vec![safe.to_str().unwrap().to_string()]);
        assert_eq!(merged.scan.extensions, vec!["mkv".to_string()]);

        std::fs::remove_dir_all(&safe).ok();
    }

    // ── WS-1: path_in_scan_dirs + expand_tilde ──────────────────────────────

    #[test]
    fn path_in_scan_dirs_accepts_inside_and_rejects_outside() {
        let scan_dir = unique_tmp_dir("inside");
        let inside = scan_dir.join("clip.mp4");
        std::fs::write(&inside, b"x").unwrap();

        let outside_dir = unique_tmp_dir("outside");
        let outside = outside_dir.join("clip.mp4");
        std::fs::write(&outside, b"x").unwrap();

        let cfg = base_config(vec![scan_dir.to_str().unwrap().to_string()]);

        assert!(path_in_scan_dirs(&inside, &cfg));
        // A real file outside every scan dir must be rejected — this is the
        // exfiltration / overwrite guard a malicious server would try to evade.
        assert!(!path_in_scan_dirs(&outside, &cfg));

        std::fs::remove_dir_all(&scan_dir).ok();
        std::fs::remove_dir_all(&outside_dir).ok();
    }

    #[test]
    fn path_in_scan_dirs_fails_closed_on_empty_list() {
        let scan_dir = unique_tmp_dir("empty");
        let inside = scan_dir.join("clip.mp4");
        std::fs::write(&inside, b"x").unwrap();

        // No scan dirs configured ⇒ nothing is ever in-scope.
        let cfg = base_config(vec![]);
        assert!(!path_in_scan_dirs(&inside, &cfg));

        std::fs::remove_dir_all(&scan_dir).ok();
    }

    #[test]
    fn path_in_scan_dirs_fails_closed_on_nonexistent_path() {
        let scan_dir = unique_tmp_dir("nonexist");
        let cfg = base_config(vec![scan_dir.to_str().unwrap().to_string()]);
        // canonicalize() fails for a missing file ⇒ must return false (fail closed).
        let missing = scan_dir.join("does-not-exist.mp4");
        assert!(!path_in_scan_dirs(&missing, &cfg));
        std::fs::remove_dir_all(&scan_dir).ok();
    }

    #[test]
    fn expand_tilde_maps_under_home() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(expand_tilde("~/Movies"), home.join("Movies"));
        // No leading ~/ ⇒ returned unchanged.
        assert_eq!(
            expand_tilde("/abs/path"),
            std::path::PathBuf::from("/abs/path")
        );
    }
}

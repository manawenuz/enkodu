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

fn apply_server_config(cfg_val: &Value, live_cfg: &Arc<RwLock<Config>>) {
    match serde_json::from_value::<Config>(cfg_val.clone()) {
        Ok(new_cfg) => {
            // Preserve the companion_id (don't let server overwrite it)
            let current_id = live_cfg.read().unwrap().companion_id.clone();
            let mut merged = new_cfg;
            merged.companion_id = current_id;
            if let Err(e) = merged.save() {
                warn!("Failed to save server-pushed config: {}", e);
            } else {
                *live_cfg.write().unwrap() = merged;
                info!("Applied server-pushed config");
            }
        }
        Err(e) => {
            warn!("Failed to parse server-pushed config: {}", e);
        }
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "enkodu-companion".to_string())
}

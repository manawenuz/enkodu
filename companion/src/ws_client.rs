use crate::capabilities::Capabilities;
use crate::config::Config;
use crate::scan;
use log::{info, warn};
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const RECONNECT_DELAY_SECS: u64 = 15;
const HEARTBEAT_INTERVAL_SECS: u64 = 30;
const FILE_LIST_INTERVAL_SECS: u64 = 300; // re-send file list every 5 min

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

                // Send initial file list
                let cfg_snap = live_cfg.read().unwrap().clone();
                send_file_list(&mut socket, &cfg_snap);

                let mut last_heartbeat = Instant::now();
                let mut last_file_list = Instant::now();

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
                            handle_server_message(&msg, &live_cfg);
                        }
                        Ok(tungstenite::Message::Ping(data)) => {
                            let _ =
                                socket.send(tungstenite::Message::Pong(data));
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
                        let hb = json!({"type": "heartbeat"});
                        if socket
                            .send(tungstenite::Message::Text(hb.to_string()))
                            .is_err()
                        {
                            break;
                        }
                        last_heartbeat = Instant::now();
                    }

                    // Periodic file list refresh
                    if last_file_list.elapsed().as_secs() >= FILE_LIST_INTERVAL_SECS {
                        let cfg_snap = live_cfg.read().unwrap().clone();
                        send_file_list(&mut socket, &cfg_snap);
                        last_file_list = Instant::now();
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
        // the heartbeat interval to detect dead connections.
        _ => {}
    }
}

fn send_file_list(socket: &mut WsStream, cfg: &Config) {
    let files = scan::scan(cfg);
    let file_entries: Vec<Value> = files
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
        .collect();

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

fn handle_server_message(msg: &Value, live_cfg: &Arc<RwLock<Config>>) {
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match msg_type {
        "welcome" => {
            if let Some(pending_cfg) =
                msg.get("pending_config").filter(|v| !v.is_null())
            {
                info!("Received pending config from server");
                apply_server_config(pending_cfg, live_cfg);
            }
        }
        "config_update" => {
            if let Some(cfg_val) = msg.get("config") {
                info!("Received config update from server");
                apply_server_config(cfg_val, live_cfg);
            }
        }
        "assign_upload" => {
            let job_id = msg
                .get("job_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let file_path = msg
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            info!(
                "Received upload assignment: job={} file={}",
                job_id, file_path
            );
            let job_id = job_id.to_string();
            let file_path = file_path.to_string();
            std::thread::spawn(move || {
                // TODO: wire into submit::submit_bg for direct assignment support
                log::info!(
                    "Upload assignment received: job={} path={}",
                    job_id,
                    file_path
                );
            });
        }
        "control" => {
            let cmd = msg
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("run");
            info!("WS control: {}", cmd);
        }
        "pong" => {}
        _ => {}
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

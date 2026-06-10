//! Embedded local HTTP review server.
//!
//! Binds to 127.0.0.1 on a random port and serves:
//!   GET  /review            — ENKODU-themed review UI
//!   GET  /api/jobs          — pending_review jobs as JSON
//!   POST /api/jobs/{id}/accept  — accept with action
//!   POST /api/jobs/{id}/reject  — reject with optional delete
//!   POST /api/bulk/accept   — bulk accept
//!   POST /api/bulk/reject   — bulk reject
//!   GET  /api/jobs/{id}/open/source|output — open in system player
//!   GET  /api/config        — read current config as JSON
//!   POST /api/config        — update config on disk and in memory

use anyhow::Result;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::config::Config;
use crate::state;

static REVIEW_HTML: &str = include_str!("review_ui.html");

pub struct ReviewServer {
    pub port: u16,
}

impl ReviewServer {
    pub fn start(live_cfg: Arc<RwLock<Config>>) -> ReviewServer {
        let server = Server::http("127.0.0.1:0").expect("bind review server");
        let port = server
            .server_addr()
            .to_ip()
            .expect("server addr")
            .port();

        info!("Review server listening on http://127.0.0.1:{}/review", port);

        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                dispatch(req, &live_cfg);
            }
        });

        ReviewServer { port }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn content_type(ct: &str) -> Header {
    Header::from_bytes("Content-Type", ct).unwrap()
}

fn no_cache() -> Header {
    Header::from_bytes("Cache-Control", "no-store").unwrap()
}

fn cors() -> Header {
    Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap()
}

fn json_ok(body: impl Serialize) -> Response<std::io::Cursor<Vec<u8>>> {
    let s = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
    Response::from_data(s.into_bytes())
        .with_header(content_type("application/json"))
        .with_header(no_cache())
        .with_header(cors())
}

fn json_err(msg: &str, code: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    let s = format!("{{\"error\":\"{}\"}}", msg.replace('"', "'"));
    Response::from_data(s.into_bytes())
        .with_status_code(code)
        .with_header(content_type("application/json"))
        .with_header(no_cache())
}

fn read_body(req: &mut Request) -> Result<String> {
    let mut buf = String::new();
    req.as_reader().read_to_string(&mut buf)?;
    Ok(buf)
}

/// Strip query string: "/api/jobs?x=1" → "/api/jobs"
fn path_only(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

// ── Router ────────────────────────────────────────────────────────────────────

fn dispatch(mut req: Request, live_cfg: &Arc<RwLock<Config>>) {
    let url = req.url().to_string();
    let path = path_only(&url).to_string();
    let method = req.method().clone();

    let result = match (method, path.as_str()) {
        (Method::Get, "/") | (Method::Get, "") => {
            let _ = req.respond(
                Response::empty(302)
                    .with_header(Header::from_bytes("Location", "/review").unwrap()),
            );
            return;
        }
        (Method::Get, "/review") => req.respond(
            Response::from_data(REVIEW_HTML.as_bytes().to_vec())
                .with_header(content_type("text/html; charset=utf-8"))
                .with_header(no_cache()),
        ),
        (Method::Get, "/api/jobs") => req.respond(json_ok(get_pending_review())),
        (Method::Get, "/api/config") => req.respond(get_config(live_cfg)),
        (Method::Post, "/api/config") => {
            let resp = post_config(&mut req, live_cfg);
            req.respond(resp)
        }
        (Method::Post, "/api/bulk/accept") => {
            let resp = bulk_accept(&mut req, live_cfg);
            req.respond(resp)
        }
        (Method::Post, "/api/bulk/reject") => {
            let resp = bulk_reject(&mut req);
            req.respond(resp)
        }
        (Method::Post, p) if p.starts_with("/api/jobs/") && p.ends_with("/accept") => {
            let id = job_id_from(p, "/accept");
            let resp = accept_job(&mut req, &id, live_cfg);
            req.respond(resp)
        }
        (Method::Post, p) if p.starts_with("/api/jobs/") && p.ends_with("/reject") => {
            let id = job_id_from(p, "/reject");
            let resp = reject_job(&mut req, &id);
            req.respond(resp)
        }
        (Method::Get, p) if p.starts_with("/api/jobs/") && p.contains("/open/") => {
            open_file(p);
            req.respond(json_ok(serde_json::json!({"ok": true})))
        }
        _ => req.respond(json_err("not found", 404)),
    };

    if let Err(e) = result {
        warn!("Review server respond error: {}", e);
    }
}

fn job_id_from(path: &str, suffix: &str) -> String {
    path.strip_prefix("/api/jobs/")
        .unwrap_or(path)
        .strip_suffix(suffix)
        .unwrap_or("")
        .to_string()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

fn get_pending_review() -> Vec<state::JobEntry> {
    state::load()
        .unwrap_or_default()
        .into_values()
        .filter(|e| e.status == "pending_review")
        .collect()
}

fn get_config(live_cfg: &Arc<RwLock<Config>>) -> Response<std::io::Cursor<Vec<u8>>> {
    let cfg = live_cfg.read().unwrap().clone();
    json_ok(cfg)
}

#[derive(Deserialize)]
struct AcceptBody {
    action: String, // "keep" | "rename" | "replace"
}

#[derive(Deserialize)]
struct RejectBody {
    delete_output: bool,
}

#[derive(Deserialize)]
struct BulkAcceptBody {
    ids: Vec<String>,
    action: String,
}

#[derive(Deserialize)]
struct BulkRejectBody {
    ids: Vec<String>,
    delete_output: bool,
}

fn accept_job(
    req: &mut Request,
    job_id: &str,
    live_cfg: &Arc<RwLock<Config>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = match read_body(req) {
        Ok(b) => b,
        Err(e) => return json_err(&e.to_string(), 400),
    };
    let parsed: AcceptBody = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(e) => return json_err(&e.to_string(), 400),
    };

    let cfg = live_cfg.read().unwrap().clone();
    match apply_accept(job_id, &parsed.action, &cfg) {
        Ok(_) => json_ok(serde_json::json!({"ok": true})),
        Err(e) => json_err(&e.to_string(), 500),
    }
}

fn reject_job(req: &mut Request, job_id: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = match read_body(req) {
        Ok(b) => b,
        Err(e) => return json_err(&e.to_string(), 400),
    };
    let parsed: RejectBody = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(e) => return json_err(&e.to_string(), 400),
    };

    match apply_reject(job_id, parsed.delete_output) {
        Ok(_) => json_ok(serde_json::json!({"ok": true})),
        Err(e) => json_err(&e.to_string(), 500),
    }
}

fn bulk_accept(
    req: &mut Request,
    live_cfg: &Arc<RwLock<Config>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = match read_body(req) {
        Ok(b) => b,
        Err(e) => return json_err(&e.to_string(), 400),
    };
    let parsed: BulkAcceptBody = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(e) => return json_err(&e.to_string(), 400),
    };

    let cfg = live_cfg.read().unwrap().clone();
    let mut errors = vec![];
    for id in &parsed.ids {
        if let Err(e) = apply_accept(id, &parsed.action, &cfg) {
            errors.push(format!("{}: {}", id, e));
        }
    }

    if errors.is_empty() {
        json_ok(serde_json::json!({"ok": true, "count": parsed.ids.len()}))
    } else {
        json_err(&errors.join("; "), 207)
    }
}

fn bulk_reject(req: &mut Request) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = match read_body(req) {
        Ok(b) => b,
        Err(e) => return json_err(&e.to_string(), 400),
    };
    let parsed: BulkRejectBody = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(e) => return json_err(&e.to_string(), 400),
    };

    let mut errors = vec![];
    for id in &parsed.ids {
        if let Err(e) = apply_reject(id, parsed.delete_output) {
            errors.push(format!("{}: {}", id, e));
        }
    }

    if errors.is_empty() {
        json_ok(serde_json::json!({"ok": true, "count": parsed.ids.len()}))
    } else {
        json_err(&errors.join("; "), 207)
    }
}

fn open_file(path: &str) {
    // path = "/api/jobs/{id}/open/source" or ".../open/output"
    let parts: Vec<&str> = path.split('/').collect();
    let which = parts.last().copied().unwrap_or("");
    let id = parts.get(parts.len().saturating_sub(3)).copied().unwrap_or("");

    let st = state::load().unwrap_or_default();
    let entry = st.values().find(|e| e.job_id == id).cloned();
    if let Some(entry) = entry {
        let file_path = match which {
            "source" => entry.source_path.or(None),
            "output" => entry.output_path,
            _ => None,
        };
        if let Some(p) = file_path {
            info!("Opening {} in system player", p);
            let _ = open::that(p);
        }
    }
}

fn post_config(
    req: &mut Request,
    live_cfg: &Arc<RwLock<Config>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = match read_body(req) {
        Ok(b) => b,
        Err(e) => return json_err(&e.to_string(), 400),
    };
    let new_cfg: Config = match serde_json::from_str(&body) {
        Ok(c) => c,
        Err(e) => return json_err(&e.to_string(), 400),
    };
    if let Err(e) = new_cfg.save() {
        return json_err(&format!("save failed: {}", e), 500);
    }
    *live_cfg.write().unwrap() = new_cfg.clone();
    info!("Config updated via review UI");
    json_ok(new_cfg)
}

// ── Accept / Reject logic ─────────────────────────────────────────────────────

fn apply_accept(job_id: &str, action: &str, cfg: &Config) -> Result<()> {
    let mut st = state::load()?;

    // Find the state key (file path) for this job_id
    let key = st
        .iter()
        .find(|(_, e)| e.job_id == job_id && e.status == "pending_review")
        .map(|(k, _)| k.clone())
        .ok_or_else(|| anyhow::anyhow!("job {} not found in pending_review", job_id))?;

    let entry = st.get(&key).cloned().unwrap();

    let source = entry.source_path.as_deref().unwrap_or(&key);
    let output = entry.output_path.as_deref().unwrap_or("");

    match action {
        "replace" => {
            if !source.is_empty() && !output.is_empty() {
                let bak = format!("{}{}", source, cfg.behavior.backup_suffix);
                let _ = std::fs::rename(source, &bak);
                // Rename output to source name (only sensible when extensions match)
                let src_path = std::path::Path::new(source);
                let out_path = std::path::Path::new(output);
                if src_path.extension() == out_path.extension() {
                    let _ = std::fs::rename(output, source);
                } else {
                    warn!("replace: extension mismatch ({} vs {}), keeping output alongside", source, output);
                }
            }
        }
        "rename" => {
            if !source.is_empty() {
                let bak = format!("{}{}", source, cfg.behavior.backup_suffix);
                let _ = std::fs::rename(source, &bak);
            }
        }
        _ => {} // "keep": do nothing to source
    }

    if let Some(e) = st.get_mut(&key) {
        e.status = "done".to_string();
    }
    state::save(&st)?;
    info!("Review accepted ({}) for job {}", action, job_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_only_strips_query() {
        assert_eq!(path_only("/api/jobs?x=1"), "/api/jobs");
        assert_eq!(path_only("/api/jobs"), "/api/jobs");
        assert_eq!(path_only("/review?a=b&c=d"), "/review");
    }

    #[test]
    fn job_id_from_accept() {
        assert_eq!(job_id_from("/api/jobs/abc-123/accept", "/accept"), "abc-123");
    }

    #[test]
    fn job_id_from_reject() {
        assert_eq!(job_id_from("/api/jobs/xyz/reject", "/reject"), "xyz");
    }

    #[test]
    fn review_html_is_embedded() {
        assert!(REVIEW_HTML.contains("ENKODU"));
        assert!(REVIEW_HTML.contains("review_mode") || REVIEW_HTML.contains("Review Mode"));
    }
}

fn apply_reject(job_id: &str, delete_output: bool) -> Result<()> {
    let mut st = state::load()?;

    let key = st
        .iter()
        .find(|(_, e)| e.job_id == job_id && e.status == "pending_review")
        .map(|(k, _)| k.clone())
        .ok_or_else(|| anyhow::anyhow!("job {} not found in pending_review", job_id))?;

    if delete_output {
        if let Some(entry) = st.get(&key) {
            if let Some(out) = &entry.output_path {
                let _ = std::fs::remove_file(out);
                info!("Deleted output {} for rejected job {}", out, job_id);
            }
        }
    }

    if let Some(e) = st.get_mut(&key) {
        e.status = "rejected".to_string();
    }
    state::save(&st)?;
    info!("Review rejected (delete_output={}) for job {}", delete_output, job_id);
    Ok(())
}

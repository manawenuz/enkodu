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
                dispatch(req, &live_cfg, port);
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

fn json_ok(body: impl Serialize) -> Response<std::io::Cursor<Vec<u8>>> {
    // SECURITY (COMP-1): no Access-Control-Allow-Origin header. The review UI is
    // served same-origin from this very server, so cross-origin reads of /api
    // (which expose config) must be blocked.
    let s = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
    Response::from_data(s.into_bytes())
        .with_header(content_type("application/json"))
        .with_header(no_cache())
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

/// SECURITY (COMP-1): defeat DNS-rebinding by requiring the Host header to be a
/// loopback host on this exact port, and rejecting any cross-origin request.
/// Returns true if the request is allowed to proceed.
fn host_origin_ok(req: &Request, port: u16) -> bool {
    let mut host: Option<String> = None;
    let mut origin: Option<String> = None;
    for h in req.headers() {
        let field = h.field.as_str().as_str().to_ascii_lowercase();
        if field == "host" {
            host = Some(h.value.as_str().to_string());
        } else if field == "origin" {
            origin = Some(h.value.as_str().to_string());
        }
    }
    host_origin_allowed(host.as_deref(), origin.as_deref(), port)
}

/// Pure decision for [`host_origin_ok`]: the Host header must be a loopback host
/// on this exact `port`, and any present Origin must point at that same loopback
/// origin. A missing Host fails closed; a missing Origin is allowed (browsers
/// only send Origin on cross-origin / non-simple requests).
fn host_origin_allowed(host: Option<&str>, origin: Option<&str>, port: u16) -> bool {
    let allowed = [
        format!("127.0.0.1:{}", port),
        format!("localhost:{}", port),
    ];
    let host_ok = match host {
        Some(v) => allowed.iter().any(|a| a == v),
        None => false,
    };
    let origin_ok = match origin {
        Some(v) => allowed.iter().any(|a| v == format!("http://{}", a)),
        None => true,
    };
    host_ok && origin_ok
}

fn dispatch(mut req: Request, live_cfg: &Arc<RwLock<Config>>, port: u16) {
    if !host_origin_ok(&req, port) {
        let _ = req.respond(json_err("forbidden", 403));
        return;
    }

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

/// Sentinel returned in place of the real auth_token so the secret never leaves
/// the process. The UI only needs to know whether a token is set.
const TOKEN_SENTINEL: &str = "__SET__";

/// SECURITY (COMP-1): replace a set `auth_token` (the bearer credential for the
/// queue server) with the sentinel so the real secret never leaves the process.
/// A `None` token stays `None`.
fn redact_config(mut cfg: Config) -> Config {
    cfg.auth_token = cfg
        .auth_token
        .as_ref()
        .map(|_| TOKEN_SENTINEL.to_string());
    cfg
}

/// SECURITY (COMP-1): when a posted config echoes the redaction sentinel back as
/// its `auth_token`, keep the existing real token instead of clobbering it with
/// the placeholder. Any other value (including `None`) is taken verbatim.
fn resolve_posted_token(
    posted: Option<String>,
    existing: Option<String>,
) -> Option<String> {
    if posted.as_deref() == Some(TOKEN_SENTINEL) {
        existing
    } else {
        posted
    }
}

fn get_config(live_cfg: &Arc<RwLock<Config>>) -> Response<std::io::Cursor<Vec<u8>>> {
    // SECURITY (COMP-1): never serialize the real auth_token (the bearer
    // credential for the queue server). Redact it to a sentinel.
    let cfg = redact_config(live_cfg.read().unwrap().clone());
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
    let mut new_cfg: Config = match serde_json::from_str(&body) {
        Ok(c) => c,
        Err(e) => return json_err(&e.to_string(), 400),
    };
    // SECURITY (COMP-1): get_config redacts auth_token to a sentinel. If the
    // client echoes the sentinel back, preserve the existing real token instead
    // of clobbering it with the placeholder.
    new_cfg.auth_token = resolve_posted_token(
        new_cfg.auth_token.take(),
        live_cfg.read().unwrap().auth_token.clone(),
    );
    if let Err(e) = new_cfg.save() {
        return json_err(&format!("save failed: {}", e), 500);
    }
    *live_cfg.write().unwrap() = new_cfg.clone();
    info!("Config updated via review UI");
    // Re-redact before returning so the response never carries the real token.
    json_ok(redact_config(new_cfg))
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
    fn json_ok_emits_no_cors_header() {
        // COMP-1: the wildcard Access-Control-Allow-Origin header was removed so
        // the review UI's /api responses (which expose config) cannot be read
        // cross-origin. A regression re-adding any ACAO header must be caught here.
        let resp = json_ok(serde_json::json!({"ok": true}));
        let has_acao = resp.headers().iter().any(|h| {
            h.field.as_str().as_str().eq_ignore_ascii_case("Access-Control-Allow-Origin")
        });
        assert!(!has_acao, "json_ok must not set Access-Control-Allow-Origin");
    }

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

    fn cfg_with_token(token: Option<&str>) -> Config {
        let mut cfg = Config::default();
        cfg.auth_token = token.map(|s| s.to_string());
        cfg
    }

    // ── COMP-1 (a): get_config redacts the real token to the sentinel ───────

    #[test]
    fn redact_config_replaces_real_token_with_sentinel() {
        let redacted = redact_config(cfg_with_token(Some("REAL-BEARER-TOKEN")));
        // Before the fix get_config serialized the live config verbatim, leaking
        // the bearer credential to any /api/config reader.
        assert_eq!(redacted.auth_token.as_deref(), Some(TOKEN_SENTINEL));
        assert_ne!(redacted.auth_token.as_deref(), Some("REAL-BEARER-TOKEN"));
    }

    #[test]
    fn redact_config_leaves_unset_token_none() {
        let redacted = redact_config(cfg_with_token(None));
        assert_eq!(redacted.auth_token, None);
    }

    #[test]
    fn get_config_response_never_contains_real_token() {
        let live = Arc::new(RwLock::new(cfg_with_token(Some("REAL-BEARER-TOKEN"))));
        let resp = get_config(&live);
        // Drain the response body and assert the secret is absent / sentinel present.
        let mut reader = resp.into_reader();
        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut body).unwrap();
        let text = String::from_utf8(body).unwrap();
        assert!(
            !text.contains("REAL-BEARER-TOKEN"),
            "serialized config leaked the real token: {}",
            text
        );
        assert!(text.contains(TOKEN_SENTINEL));
    }

    // ── COMP-1 (b): post_config sentinel round-trip preserves the real token ─

    #[test]
    fn resolve_posted_token_preserves_real_token_on_sentinel_echo() {
        // Client echoes the sentinel back ⇒ keep the existing real token.
        // Before the fix the sentinel string would have been saved AS the token,
        // silently destroying the real bearer credential.
        let resolved = resolve_posted_token(
            Some(TOKEN_SENTINEL.to_string()),
            Some("REAL-BEARER-TOKEN".to_string()),
        );
        assert_eq!(resolved.as_deref(), Some("REAL-BEARER-TOKEN"));
    }

    #[test]
    fn resolve_posted_token_sets_new_distinct_token() {
        let resolved = resolve_posted_token(
            Some("BRAND-NEW-TOKEN".to_string()),
            Some("REAL-BEARER-TOKEN".to_string()),
        );
        assert_eq!(resolved.as_deref(), Some("BRAND-NEW-TOKEN"));
    }

    #[test]
    fn resolve_posted_token_allows_clearing_token() {
        let resolved =
            resolve_posted_token(None, Some("REAL-BEARER-TOKEN".to_string()));
        assert_eq!(resolved, None);
    }

    // ── COMP-1: host_origin_ok (via pure host_origin_allowed) ───────────────

    #[test]
    fn host_origin_allowed_accepts_loopback_hosts() {
        // Matching loopback Host, no Origin.
        assert!(host_origin_allowed(Some("127.0.0.1:8080"), None, 8080));
        assert!(host_origin_allowed(Some("localhost:8080"), None, 8080));
        // Matching loopback Host AND matching loopback Origin.
        assert!(host_origin_allowed(
            Some("127.0.0.1:8080"),
            Some("http://127.0.0.1:8080"),
            8080
        ));
        assert!(host_origin_allowed(
            Some("localhost:8080"),
            Some("http://localhost:8080"),
            8080
        ));
    }

    #[test]
    fn host_origin_allowed_rejects_foreign_host() {
        // DNS-rebinding: a foreign Host resolving to loopback must be rejected.
        assert!(!host_origin_allowed(Some("evil.com"), None, 8080));
        assert!(!host_origin_allowed(Some("evil.com:8080"), None, 8080));
        // Right host, wrong port.
        assert!(!host_origin_allowed(Some("127.0.0.1:9999"), None, 8080));
    }

    #[test]
    fn host_origin_allowed_rejects_missing_host() {
        // No Host header ⇒ fail closed.
        assert!(!host_origin_allowed(None, None, 8080));
        assert!(!host_origin_allowed(None, Some("http://127.0.0.1:8080"), 8080));
    }

    #[test]
    fn host_origin_allowed_rejects_cross_origin() {
        // Good loopback Host but a cross-origin Origin ⇒ rejected.
        assert!(!host_origin_allowed(
            Some("127.0.0.1:8080"),
            Some("http://evil.com"),
            8080
        ));
        assert!(!host_origin_allowed(
            Some("127.0.0.1:8080"),
            Some("https://127.0.0.1:8080"),
            8080
        ));
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

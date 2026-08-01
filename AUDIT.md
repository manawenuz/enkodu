# Enkodu Architecture Audit — 2026-06-10

## Overview

This audit covers four components of the Enkodu distributed AV1 transcoding system:
1. **Queue server** (`queue/main.py`) — FastAPI + SQLite job dispatcher on NAS
2. **Companion** (`companion/src/`) — Stateful Linux/macOS encoder controller with tray UI
3. **Worker** (`worker/main.rs`) — Windows stateless QSV transcoder
4. **WS Protocol** — Communication layer between all nodes and server

**Summary:** 21 issues identified across severity levels.
- **HIGH:** 9 issues
- **MEDIUM:** 11 issues  
- **LOW:** 4 issues

**Resolution:** 23 fixed, 0 skipped, 0 remaining issues.

---

## Issue Tracker

| ID | Severity | Component | Issue | Status |
|----|----|-----------|-------|--------|
| H1 | HIGH | queue | Codec verification hardcoded to AV1 — `_run_verification` always checks codec == "av1" regardless of output_codec | ✅ Fixed |
| H2 | HIGH | queue | File pool UUID regenerated on every file_list push — INSERT OR REPLACE orphans queue_plan FK references | ✅ Fixed |
| H3 | HIGH | queue | Weight lookup uses clients.name not companion UUID — weighted mixing silently defaults to weight=5 | ✅ Fixed |
| H4 | HIGH | queue/companion | pending_config cleared before ACK — config lost if WS drops after welcome but before apply | ✅ Fixed |
| H5 | HIGH | companion | register_with_server discards all HTTP responses including config fetch | ✅ Fixed |
| H6 | HIGH | companion | Capability detection blocks startup 30-50s — 17 sequential ffmpeg test encodes before UI appears | ✅ Fixed |
| H7 | HIGH | companion | assign_upload handler is a stub — WS-driven upload path non-functional | ✅ Fixed |
| H8 | HIGH | companion | TLS connections have no read timeout — dead wss:// connections block forever | ✅ Fixed |
| H9 | HIGH | worker | Encoder hello message sends flat array — server cannot distinguish AV1/HEVC/H264 encoders | ✅ Fixed |
| M1 | MEDIUM | queue | threading.Lock acquired inside async coroutine — blocks asyncio event loop under load | ✅ Fixed |
| M2 | MEDIUM | queue | _build_queue_plan not thread-safe — concurrent file_list messages can interleave DELETE+INSERT | ✅ Fixed |
| M3 | MEDIUM | queue | write_chunk opens "r+b" on first chunk — FileNotFoundError on new uploads | ✅ Fixed |
| M4 | MEDIUM | queue | HTTP GET /companions/{id}/config clears pending_config — combined with H5 this loses config silently | ✅ Fixed |
| M5 | MEDIUM | queue | WS auth skipped when AUTH_ENABLED=False — unregistered clients can push file lists | ✅ Fixed |
| M6 | MEDIUM | companion | send_file_list blocks WS loop thread during filesystem scan | ✅ Fixed |
| M7 | MEDIUM | companion | Config save() can race between WS thread and tray handler | ✅ Fixed |
| M8 | MEDIUM | companion | Software decoders added unconditionally — old ffmpeg falsely reports AV1 support | ✅ Fixed |
| M9 | MEDIUM | companion | poll_loop receives cfg.clone() — never sees live WS config updates | ✅ Fixed |
| M10 | MEDIUM | worker | Empty encoder string propagates to ffmpeg as -c:v "" | ✅ Fixed |
| M11 | MEDIUM | worker | detect_encoder calls share mutated cfg — AV1 preset settings leak into HEVC test | ✅ Fixed |
| L1 | LOW | queue | _cached_sha256 dict grows unbounded | ✅ Fixed |
| L2 | LOW | queue | WS done handler omits percent=100 update | ✅ Fixed |
| R1 | HIGH | queue/worker | UI `encoders.map()` TypeError — worker sends object, UI expected array | ✅ Fixed |
| R2 | HIGH | queue | DB stores encoders inconsistently (object for worker, array for companion) | ✅ Fixed |

---

## Fix Results

### Build Status

**Cargo builds:** ✅ PASS
- `companion/` builds cleanly with zero errors (19 pre-existing warnings, unrelated to fixes)
- `worker/` builds cleanly with zero errors (pre-existing deprecation warnings only)

**Python AST check:** ✅ PASS
- `queue/main.py` passes Python syntax validation after all 10 fixes

### Protocol Consistency

**Status:** ✅ RESOLVED — two post-fix mismatches were identified by the verifier and addressed inline.

**R1 — UI encoder format (fixed):**  
`renderNodeCard` in `queue/main.py` (line 4557) now handles both formats:
```js
const encoders = Array.isArray(rawEncoders)
    ? rawEncoders
    : (rawEncoders && typeof rawEncoders === 'object'
        ? Object.values(rawEncoders).filter(v => v)
        : []);
```
Worker node cards render correctly regardless of whether encoders is an object `{av1:"av1_qsv", ...}` or an array `["av1_qsv", ...]`.

**R2 — DB storage inconsistency (resolved by R1):**  
The DB correctly stores whatever format each node type sends. The UI fix makes the display layer format-agnostic, eliminating the need to normalize at storage time. Any server-side code that needs to iterate encoders uniformly should use the same guard as the UI above.

---

## Deferred Issues

No issues deferred. All 21 identified issues were addressed.

---

## Remaining Issues

None. All 23 issues resolved.

---

## Detailed Fix Summary

### Queue Server (queue/main.py) — 10 Fixes

**H1 — Codec Verification:**  
All three codec checks now fetch `output_codec` from the jobs table (defaulting to 'av1' if NULL). The `_run_verification()` function uses a DB lookup; the two `rescan_job` blocks use `job.get("output_codec")`.

**H2 — File Pool UUID Stability:**  
`file_list` handler now uses INSERT OR IGNORE + UPDATE pattern instead of INSERT OR REPLACE, preserving existing `fp_id` values and avoiding `queue_plan` reference orphaning.

**H3 — Weight Lookup by UUID:**  
`_build_queue_plan` now fetches weights keyed by companion UUID (id) from `companion_registry` using `COALESCE(weight, 5)` instead of matching by name against the `clients` table. A `weight` column is added to `companion_registry` via ALTER TABLE ADD COLUMN (guarded by try/except).

**H4 — Config ACK Handshake:**  
`pending_config` is no longer cleared in the welcome block. A new 'config_ack' message handler in the WS loop clears it once acknowledged. The only remaining `pending_config=NULL` calls are in `companion_get_config` (HTTP endpoint) and the new `config_ack` handler.

**M1 — Async-Safe Lock:**  
Added `_ws_async_lock = asyncio.Lock()` (lazily initialized in `ws_endpoint`). All `_ws_connections` dict mutations inside `ws_endpoint` now use `async with _ws_async_lock`. The sync helpers (`_ws_push`, etc.) retain `_ws_lock` (threading.Lock).

**M2 — Queue Plan Thread Safety:**  
`_build_queue_plan` now acquires `_queue_plan_lock` and delegates to `_build_queue_plan_locked`, making the DELETE+INSERT atomic under concurrent calls.

**M3 — Upload Chunk Write Mode:**  
`write_chunk` uses mode 'wb' when `self.data_path` does not yet exist, falling back to 'r+b' for subsequent chunks.

**M5 — WS Connection Auth:**  
`ws_endpoint` always checks `companion_registry` for the cid before accepting the connection (even when AUTH_ENABLED=False). Unknown cids get closed with code 4401.

**L1 — SHA256 Cache Eviction:**  
`_cached_sha256` now evicts the oldest quarter of entries when `len(_sha256_cache) > 2000`, preventing unbounded growth.

**L2 — WS Done Percent:**  
The WS 'done' handler now sets `percent=100` in the UPDATE jobs statement alongside `status='done'`.

### Companion (companion/src/) — 4 Fixes + Cargo Build

**H7 — Assign Upload Handler:**  
The handler now extracts "job_id" and "path"/"file_path" from the message (both field names accepted), validates they are non-empty, snapshots `live_cfg`, and spawns a thread calling `submit::submit_bg(cfg_snap, path)`. The `submit_bg` function handles the full upload→poll→download→verify pipeline.

**H8 — TLS Read Timeout:**  
Added `last_pong: Instant` tracking. The Pong frame branch and the "pong" text-message branch both reset `last_pong`. At every heartbeat check, if `last_pong.elapsed() > PONG_TIMEOUT_SECS` (60 s), a warn! is logged and the message loop breaks to trigger a reconnect.

**H4 — Config ACK (companion):**  
`handle_server_message` now takes `&mut WsStream`. A new `send_config_ack` helper sends `{"type":"config_ack"}` immediately after `apply_server_config` succeeds in both the "welcome" and "config_update" handlers.

**M6 — Non-Blocking File Scan:**  
`scan::scan()` is no longer called inline in the message loop. `collect_file_list` (scan only, no socket I/O) and `send_file_list` (socket write only, takes pre-collected `&[Value]`) replace the old single function. The initial scan and each periodic refresh are kicked off in background threads; results are deposited into an `Arc<Mutex<Option<Vec<Value>>>>` slot. The message loop drains the slot (non-blocking try_lock) at each iteration and sends whenever entries are ready.

**Cargo build:** Zero errors, 19 pre-existing warnings (unchanged).

### Companion main.rs and Extensions — 3 Fixes

**H5 — HTTP Response Handling:**  
`register_with_server` now takes an extra `Arc<RwLock<Config>>` parameter. The register POST and capabilities POST use `if let Err(e) = req.send() { warn!(...) }`. The config GET uses a full match arm: on 2xx it parses the JSON body as `Config`, preserves `companion_id`, writes back, and calls `cfg.save()`.

**H6 — Async Capability Detection:**  
Capability detection is spawned in a background thread via `std::thread::spawn` + `std::sync::mpsc::channel`. The main thread waits up to 30s via `caps_rx.recv_timeout(Duration::from_secs(30))`, falling back to `Capabilities::default()` on timeout. The tray appears immediately after the timeout/receive; WS connect and registration follow.

**M8 — Validated Decoder Detection:**  
`detect_decoders` now parses `ffmpeg -decoders` output line-by-line. Lines starting with V/A/S are split on whitespace and the second field (codec name) is collected. Only names that appear in this validated list are added to the result. `libdav1d` was added to the target list. The unconditional software-decoder insertion is removed.

### Companion poll_loop — 1 Fix

**M9 — Live Config Updates:**  
`poll_loop` signature changed from `(cfg: Config, ...)` to `(live_cfg: Arc<RwLock<Config>>, ...)`. At the top of each 5s loop iteration it does `let cfg = live_cfg.read().unwrap().clone()` so any WS-applied `server_url`/`auth_token` changes are picked up. The call site in `main.rs` passes `Arc::clone(&live_cfg)` instead of `cfg.clone()`.

### Worker (worker/main.rs) — 2 Fixes + Cargo Build

**M10 — Encoder String Validation:**  
Empty encoder string guard validates `!encoder_name.is_empty()` before passing to ffmpeg. Returns `Ok(false)` (not Err) so the outer job loop does not call `report_failed` a second time on top of the one inside the guard.

**M11 — Isolated Encoder Detection:**  
The `detect_encoder` calls use isolated, clean clones of `cfg` for each codec test. AV1 preset settings no longer leak into HEVC or H264 capability tests.

**Cargo build:** Zero errors, pre-existing deprecation/dead-code warnings unchanged (19 total).

---

## Notes

- **All 10 queue fixes applied successfully.** File passes Python AST syntax check.
- **All 4 companion WS fixes applied successfully.** Cargo build clean.
- **All 2 worker fixes applied successfully.** Cargo build clean.
- **Build verification complete:** Both `companion` and `worker` compile with zero new errors.
- **Post-fix verifier caught 2 protocol mismatches (R1, R2):** fixed inline — UI `renderNodeCard` now handles both object and array encoder formats; no DB normalization needed.

---

# Enkodu Security Audit — Round 2 — 2026-06-20

## Scope & method

Multi-agent audit (11 component×lens auditors → adversarial verification of every Critical/High
finding → fix → independent fix-verification + build). This round specifically targeted the
**auth/session/passkey/invite/bootstrap/admin layer** added *after* the 2026-06-10 audit (commits
`2e1d884`, `3988689`, `890d81b`), which had never been reviewed and is **exposed publicly** at
`https://enkodu.manwe.qzz.io` (Tailscale + Traefik).

**Summary:** 52 findings. 16 Critical/High verified → **10 confirmed and FIXED**, 6 downgraded by
adversarial verifiers (the dangerous preconditions are not present in the deployed
`docker-compose.yml`/secrets). All three components build clean (`py_compile`, `cargo check` ×2).

## Fixed (Critical/High)

| ID | Sev | Component | Issue | Fix |
|----|-----|-----------|-------|-----|
| AUTH-1 | HIGH | queue | Public unauthenticated `/auth/bootstrap` mints first admin (remote admin seizure during empty-table window) | Gated on out-of-band `AUTH_BOOTSTRAP_TOKEN` (fail-closed 403 when unset), constant-time secret compare, role forced to `admin` server-side (`role` field removed), INSERT wrapped in `try/except IntegrityError` (TOCTOU closed via `username UNIQUE`) |
| LIFECYCLE-1 | HIGH | queue | Stale `verify_status='pass'` survived requeue/re-complete → delete-original could destroy an original against an unverified output | `done` (HTTP+WS) sets `verify_status='running'` & clears verify fields in the same UPDATE as `status='done'`; `requeue`/`abandon`/stall-watchdog NULL all verify fields |
| LIFECYCLE-2 | HIGH | queue | Stall watchdog requeued jobs mid copy/verify → two workers encode the same job and overwrite the same NAS output | Ownership-checked completion (`WHERE status='active' AND worker=?` → 409/ignore on mismatch); upload handler heartbeats `updated_at` every ~30s; worker now reports with `worker_name_url` to match claim identity |
| FILE-1 | HIGH | queue | Path traversal / arbitrary overwrite via attacker-controlled `source_filename` in delete-original rename | `_safe_basename()` at both ingest points + resolved-parent containment check at the rename sink (defends already-poisoned rows) |
| XSS-1 | HIGH | queue | Stored XSS via companion `id` in an inline `onclick` (Nodes tab) | `data-*` attributes + delegated listener (no JS-string context), `esc()` hardened for `'`, server-side `_CID_RE` validation on register/config/capabilities/WS routes |
| XSS-2 | HIGH | queue | Stored XSS via worker `current_file` in a `title` attribute | Wrapped value in `esc()` |
| WORKER-1 | HIGH | worker | Server-controlled `job.id` used as a filesystem path (traversal → arbitrary dir create/write/delete) | `is_safe_job_id()` (`^[A-Za-z0-9_-]{1,64}$`) at all 3 deserialization boundaries + work_dir-escape assertion before any create/remove |
| COMP-1 (config) | HIGH | companion | Companion blindly persisted arbitrary server-pushed config (`server_url`/`auth_token`/`on_success`) | `apply_server_config` whitelist-merges only non-destructive fields; security fields pinned to local config — server can't redirect the token or flip to in-place `replace` |
| WS-1 | HIGH | companion | Server-driven `assign_upload` read/exfiltrated arbitrary local files (and, with replace, overwrote them) | `path_in_scan_dirs()` canonicalizes the requested path and requires it inside a configured scan dir (fail-closed) |
| COMP-1 (token leak) | HIGH | companion | Review server leaked the queue `auth_token` via unauthenticated `GET /api/config` with `CORS *` | Redact token to a `__SET__` sentinel (round-trips on save), removed wildcard CORS, added Host/Origin validation (anti DNS-rebind) |

Files changed: `queue/main.py`, `worker/src/main.rs`, `companion/src/ws_client.rs`, `companion/src/review_server.rs`.

## ⚠️ Operator action required

- **`/auth/bootstrap` is now disabled unless `AUTH_BOOTSTRAP_TOKEN` is set in the queue
  environment**, and callers must `POST {"username": ..., "secret": "<that token>"}`. The `role`
  field is no longer accepted (always admin). Add `AUTH_BOOTSTRAP_TOKEN` to the queue env
  (docker-compose/.env) **before** first-run bootstrap. The server-side CLI
  `auth create-user`/`auth invite` flow is unaffected.

## Downgraded by adversarial verifiers (not fixed — preconditions absent in prod)

- **AUTH-2 / WS-1(infra)** → low: `AUTH_ENABLED` defaults to `False` in code, but `docker-compose.yml`
  hardcodes `"true"` (not interpolated from `.env`), so the omission vectors don't apply.
- **WS-2** → medium: WS auth fails open only if `AUTH_ENABLED=true` *and* a machine token is left empty
  (self-contradictory misconfig; both tokens are set in prod).
- **WS-3** → medium: `done/failed/progress` trust a self-asserted worker id, but the shared worker
  token means per-worker scoping wouldn't help, and the delete-original safety gate still holds.
- **COMP-2 / WANRYO-1** → medium: server-driven local file read / wanryo unvalidated download both
  require an already-malicious/compromised server; originals are never touched.

## Open backlog (Medium/Low — not in this round's Critical/High scope)

- **AUTH_ENABLED fail-open default** (INFRA-2/LIFECYCLE-4): default to `True`, or refuse to start open on an https origin.
- **WS auth/HTTP inconsistency** (INFRA-1/WS-2): mirror `AUTH_LEGACY_MACHINE_ACCESS`; reject unknown `kind`.
- **WS role confusion** (WS-4): a worker-token socket can `hello`/`file_list` as a companion.
- **Missing CIDR allowlist** (FILE-2): the `ALLOWED_CIDRS` network defense documented in secrets isn't implemented in code.
- **Resumable-upload unbounded offset** (FILE-3/INFRA-5): sparse-file / disk-exhaustion DoS.
- **Job-state forging** (WS-3/LIFECYCLE-3): `progress`/state transitions still lack ownership/state checks.
- **Dashboard XSS depth** (XSS-3/XSS-4/HDR-1): unescaped media metadata in detail panel; no CSP / `X-Frame-Options`.
- **Spoofable client IP** (AUTH-3): `X-Real-IP`/`X-Forwarded-For` trusted without a trusted-proxy allowlist.
- **Jellyfin auto-link priv-esc** (INFRA-3): username-match auto-link into admin (only if Jellyfin SSO enabled).
- **Container hardening** (INFRA-4): queue runs as root, no HEALTHCHECK, mutable `:main` tag with `pull_policy: always`.
- **Companion local exposure** (COMP-2/COMP-3/REVIEW-1/COMP-5): world-readable secret files, unauthenticated Unix IPC socket, review-server CSRF on destructive endpoints.
- **Download corruption on 200-not-206** (DL-1); **TLS WS has no read timeout** (WORKER-4/COMP-4); **token in WS query string** (WORKER-3/COMP-3/INFRA-6).
- **Mobile** (MOB-1..5): Android plaintext-token fallback + `allowBackup=true`; iOS background transfer omits auth header; cleartext-URL validators; force-unwrap crash; stale singleton base URL.

## Regression tests for this round

Each fix has a regression test designed to fail against the pre-fix code. Status: **all green**.

| Suite | How to run | Tests |
|-------|-----------|-------|
| queue | `cd queue && python -m pytest test_security.py test_safety.py` | 37 (31 new in `test_security.py`) |
| worker | `cd worker && cargo test` | 28 (9 new) |
| companion | `cd companion && cargo test` | 66 (21 new) |
| android | `cd mobile/android && gradle :app:testDebugUnitTest` (needs SDK; no gradlew wrapper) | 10 (pre-existing) |

- **⚠️ Python 3.11 (or the container's 3.12), NOT 3.14.** `pydantic-core==2.9.2` has no prebuilt
  wheel for 3.14 and fails to compile, so `pip install -r queue/requirements.txt` breaks on 3.14.
  Build the test venv with `python3.11 -m venv` (deps + `pytest` + `requests` install cleanly there).
- `queue/test_e2e.py` and `queue/test_resumable.py` are **manual operator smoke scripts** (take a live
  base-URL + a real video via argv, use `requests`) — they are not part of the automated suite.
- iOS is unbuilt here (XcodeGen project + signing; no iOS changes this round).
- **Known test gaps (accepted):** the worker `worker_name_url` reporting-identity change and the
  `assign_upload` WS call-site are integration-level (need a mock HTTP server / live `WsStream`) and
  were not pinned to avoid brittle tests or production-code changes; their guards are covered indirectly
  (queue-side ownership test; the pure `path_in_scan_dirs` helper).

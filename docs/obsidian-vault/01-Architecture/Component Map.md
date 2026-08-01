---
tags:
  - architecture
  - components
---

# Component Map

## Queue Service

Source: `queue/main.py`

Responsibilities:

- SQLite schema migration for `jobs`, `clients`, `settings`, auth tables, and telemetry.
- NAS library scan with codec/size/resolution/bitrate filters.
- Companion uploads, resumable upload sessions, dedupe against verified outputs, and client registration.
- Weighted fair queueing between clients.
- Worker dispatch and heartbeats.
- Progress storage and live dashboard state.
- Server-side ffprobe verification.
- Output download/checksum/delete gates that require `verify_status=pass`.
- Optional auth: local passkeys, Authentik OIDC, Jellyfin login, worker tokens, companion tokens, and API tokens.
- Health/version probes and telemetry ingestion.
- Dashboard, install page, companion binary download.
- Companion registry, capability reporting, pending remote configuration, and live WebSocket updates for worker control, job progress, and file manifests.
- File-pool discovery and explicit queue-plan build/reorder operations for companion-managed libraries.
- Operational actions: scan, force encode, requeue, rescan, clear failed/pending, backfill metadata, delete originals, control run/drain/stop.

Runtime:

- FastAPI app.
- SQLite WAL.
- Background scanner thread.
- Background stall watchdog thread.
- Background resumable-upload cleanup thread.
- Optional Telegram notification env vars.

## Worker

Source: `worker/src/main.rs`

Responsibilities:

- Poll `GET /jobs/next`.
- Respect `/control` commands.
- Send heartbeat every 30 seconds.
- Download source to a local work directory.
- Detect or use configured AV1 encoder: `av1_qsv`, `av1_vaapi`, `av1_nvenc`, or `libsvtav1`.
- Parse ffmpeg progress output.
- Validate codec and duration locally with `ffprobe`.
- Upload output and report done/failed.
- Clean stale active jobs on startup.
- Read `worker.env` plus environment overrides.
- Provide `--version` and `diagnostics`.
- Halt clearly on queue auth failures.

Current assumptions:

- Windows and Linux defaults are first-class; macOS worker remains out of scope.
- Hardware encoding is preferred; SVT-AV1 is a slow fallback.
- One encode at a time per worker process.
- Windows uses Scheduled Task deployment; Linux uses a user systemd unit.

## Desktop Companion

Source: `companion/src/main.rs`

Responsibilities:

- Run as a tray app on macOS, Linux, and Windows adapters.
- Submit a selected file.
- Batch scan configured directories.
- Maintain local state.
- Poll queue and live progress.
- Download complete outputs, including retry/resume support.
- Verify downloaded output.
- Reconcile completed server jobs with local files.
- Toggle NAS scan pause and global worker drain/resume.
- Provide CLI commands through platform IPC.
- Send companion bearer tokens from config or `ENKODU_AUTH_TOKEN`.
- Provide connection/auth diagnostics through `enkodu test`.

Current assumptions:

- macOS notification path uses `osascript`.
- Autostart uses LaunchAgent.
- Linux uses `notify-send`, XDG autostart, and Unix-socket IPC.
- Windows uses `%APPDATA%`/`%LOCALAPPDATA%`, HKCU Run autostart, and localhost loopback IPC with a per-run token.
- `ffprobe` must be available in PATH, Homebrew, or `/usr/local/bin`.
- `enkodu --version`/`-V` reports the packaged companion version.

### Live coordination

The companion first registers its stable ID and capabilities, then connects to `WS /ws/companion/{id}`. It sends a hello message, periodic heartbeats, and file manifests discovered by local scanning. The server sends welcome/config/control/job messages; configuration is acknowledged before the server clears a pending configuration update. WebSocket auth uses the companion bearer token when strict auth is enabled.

## Mobile Companions

Sources: `mobile/android/`, `mobile/ios/`

Responsibilities being scaffolded:

- First-run server/auth setup.
- AV1 hardware decode gate before upgrade/download actions.
- Resumable upload client against `POST /jobs/upload/resumable/*`.
- Ranged download client against `GET /jobs/{id}/output`.
- Local transfer state persistence.
- Telemetry posting.
- User-visible save/share flow after checksum and `verify_status=pass`.

Current assumptions:

- Mobile devices submit to the queue; they do not transcode locally.
- Android uses Kotlin, Compose, Room, WorkManager, EncryptedSharedPreferences, and Retrofit.
- iOS uses SwiftUI, Core Data, Keychain, URLSession/background task pieces, and VideoToolbox.
- Both need real-device transfer, auth, and save/share verification.

## Dashboard

Source: rendered inline from `queue/main.py`

Provides:

- Queue tab.
- Report tab.
- Settings tab.
- Live progress card.
- Worker badges.
- Client filtering and weights.
- Bulk delete/rescan controls.
- Install link for macOS companion.

Risk:

- The dashboard is embedded as a large string in the FastAPI file. That is fast to iterate on, but makes testing and UI evolution harder.

---
tags:
  - operations
  - runbook
---

# Runbook

This runbook intentionally omits credentials and private deploy values. Use `~/.agentSecrets/enkodu/secrets.md` for non-git deployment context.

## Build Queue Image

From `queue/`, build and push the container image according to the current registry/deploy pattern in the secrets file.

Release need: replace ad hoc image names with a versioned tag, for example `enkodu:0.1.0`.

## Deploy Queue

The queue runs as a container with `/data` mounted to the NAS dataset. Required runtime values:

- `VIDEOS_ROOT`
- `NAS_UNC_ROOT`
- `DB_PATH`
- `SCAN_INTERVAL`
- Optional notification env vars.
- Optional `COMPANION_BIN`.

After deploy:

1. Open `/status`.
2. Open `/`.
3. Trigger `/scan` or wait for scanner loop.
4. Confirm logs show scanner startup and no ffprobe failures.

## Deploy Windows Worker

Current helpers:

- `build.sh` from repo root: cross-compiles the worker for Windows and deploys to the configured Windows host.
- `worker/install-windows.ps1` on Windows: installs or updates `C:\transcode\yulia-worker.exe`, keeps/creates `worker.env`, registers the `AV1Worker` Scheduled Task, runs diagnostics, then starts the task if diagnostics pass.

High-level steps:

1. Cross-compile `worker` for `x86_64-pc-windows-gnu`.
2. Copy `yulia-worker.exe` to the worker host.
3. Create or keep `C:\transcode\worker.env`; set `QUEUE_URL` and `QUEUE_TOKEN` if strict auth is enabled.
4. Run `yulia-worker.exe diagnostics`.
5. Install or update the Scheduled Task.
6. Start the task.
7. Check `C:\transcode\logs\worker.log`.
8. Confirm `/workers` shows the worker online.

Release need: replace private host assumptions in `build.sh` with a documented release/deploy input, or prefer the Windows-side installer for public docs.

## Install Linux Worker

Prerequisites: `ffmpeg`, `ffprobe`, user systemd, and encoder/driver support. See `worker/docs/LINUX.md`.

Steps:

1. Build: `cd worker && cargo build --release`, or pass a prebuilt binary to the installer.
2. Install: `./install-linux.sh` or `./install-linux.sh /path/to/yulia-worker`.
3. Edit `~/.config/yulia-worker/worker.env`.
4. Run `yulia-worker diagnostics`.
5. Check `journalctl --user -u yulia-worker -f`.

Release need: capture a real Linux fixture transcode run for each intended encoder class.

## Install Linux Companion

Prerequisites: Rust, `notify-send`, `xdg-utils`, `ffprobe` (see `companion/docs/LINUX.md`)

Steps:

1. Build: `cd companion && cargo build --release`
2. Install binary: `sudo cp target/release/enkodu /usr/local/bin/`
3. Or use build script: `./build-linux.sh` produces tarball
4. Run: `enkodu` (tray mode) or `enkodu status`, `enkodu scan`, etc. (CLI)
5. Config is created at `$XDG_CONFIG_HOME/enkodu/config.toml` or `~/.config/enkodu/config.toml`
6. Enable autostart via tray menu "Start at Login" or manual XDG desktop file

Verification:
- `enkodu status` shows online/offline state
- `enkodu scan` triggers batch scan of configured directories
- Submit a test file, monitor job, download output

Known issues (pre-Phase 0 completion):
- Needs real Linux desktop validation across at least one common environment
- Needs fixture submit/download/reconcile run

## Install Windows Companion

Prerequisites: Rust (MSVC target), Visual Studio 2022 C++ workload, `ffprobe` (see `companion/docs/WINDOWS.md`)

Steps:

1. Build: `cd companion && cargo build --release --target x86_64-pc-windows-msvc`
2. Or use build script: `.\build-windows.ps1` produces distribution in `target\windows-release\`
3. Copy `enkodu.exe` to installation directory (e.g., `C:\Program Files\Enkodu\`)
4. Run: `enkodu.exe` (tray mode) or `enkodu.exe status`, `enkodu.exe scan`, etc. (CLI)
5. Config is created at `%APPDATA%\Enkodu\config.toml`
6. State is stored at `%LOCALAPPDATA%\Enkodu\state.json`
7. Enable autostart via tray menu "Start at Login" or manual Startup shortcut/registry entry

Verification:
- `enkodu.exe status` works while tray is running
- `enkodu.exe scan` and `enkodu.exe reconcile` trigger running companion or execute directly
- Submit a test file, monitor job, download output

Known issues (pre-Phase 0 completion):
- Notifications currently use a PowerShell/System.Windows.Forms message box fallback rather than proper toast notifications
- Windows command bridge uses localhost IPC and needs real-host verification
- Submit/download/reconcile fixture run still needed

## Coexistence: Companion + Worker on Same Machine

Windows worker (`yulia-worker.exe`) and Windows companion (`enkodu.exe`) can run simultaneously:
- Different config directories: worker uses its own, companion uses `%APPDATA%\Enkodu`
- Different state files: companion uses `%LOCALAPPDATA%\Enkodu`
- Different process names and working directories
- No shared resources or conflicts

## Install macOS Companion

Current queue exposes:

- `/install`
- `/download/enkodu`

Current install requires manual quarantine removal and moving the binary to PATH.

Release need:

- Versioned binary.
- Signed/notarized app or documented trusted-test exception.
- Clear uninstall command.
- First-run check for `ffprobe`.

## Pause and Resume

Global worker control:

- `POST /control/run`
- `POST /control/drain`
- `POST /control/stop`

NAS scanner pause:

- `POST /settings` with `nas_drain=true`

Companion-side Mac submission pause:

- Local companion state only; exposed through tray/IPC.

## Failure Recovery

Worker died mid-job:

- Worker calls `/jobs/abandon` on startup for its own active jobs.
- Queue stall watchdog requeues active jobs older than `STALL_TIMEOUT`.

Failed job:

- Inspect error in dashboard or `/jobs/{id}`.
- Use `/jobs/{id}/requeue` after fixing the cause.

Bad metadata:

- Use `/jobs/{id}/rescan` or `/jobs/bulk-rescan`.
- Use `/jobs/backfill-meta` for old jobs.

Companion missed download:

- Companion startup recovery watches pending local state.
- Manual `enkodu reconcile` asks the running tray app to scan and reconcile.
- `enkodu wanryo` creates a CSV checklist for manual review.

Resumable mobile upload interrupted:

- If the queue process stays up, resume from the last confirmed byte offset using the same `upload_id`.
- If the queue process restarted and returns `404 upload session not found`, start a fresh resumable upload session.
- Resumable upload directories older than 24 hours are cleaned up by the queue.

Auth failure:

- Worker exits with code `2` after logging an auth error. Fix `QUEUE_TOKEN` in `worker.env`, then restart the service/task.
- Desktop/mobile companions should treat `401` as token/session rejected and `403` as permission denied; do not retry auth failures as transient network errors.

## Backup and Restore

Minimum release backup set:

- SQLite DB.
- `control.json`.
- Upload directory if companion jobs are in flight.
- NAS video library is outside the queue app and should have its own storage backup/snapshot policy.

## Test and Verification Commands

Queue safety unit tests:

```bash
python3 queue/test_safety.py
```

Resumable upload/download integration against a running queue:

```bash
python3 queue/test_resumable.py http://localhost:8090
```

Full queue/worker/client-path fixture against a running queue and worker:

```bash
python3 queue/test_e2e.py http://localhost:8090 /path/to/small-test-video.mp4
```

Worker checks:

```bash
yulia-worker --version
yulia-worker diagnostics
```

Desktop companion auth/connectivity check:

```bash
enkodu test
```

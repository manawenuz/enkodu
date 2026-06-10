# yulia-worker — Windows Install Guide

## Prerequisites

- Windows 10 or 11 (x86_64)
- Intel GPU with QSV AV1 support (Arc, 12th-gen Core or later) — required for the default `av1_qsv` encoder
- [msys2](https://www.msys2.org/) with ffmpeg installed (see **Dependencies** below)
- PowerShell 5.1 or later (included with Windows 10/11)
- The Enkodu queue service running and reachable over the network

## Dependencies

Install ffmpeg via msys2 (one-time):

```powershell
# In msys2 MINGW64 shell
pacman -S mingw-w64-x86_64-ffmpeg
```

This places `ffmpeg.exe` and `ffprobe.exe` at `C:\msys64\mingw64\bin\`, which is the default path the worker expects.

To verify:

```powershell
C:\msys64\mingw64\bin\ffmpeg.exe -version
C:\msys64\mingw64\bin\ffprobe.exe -version
```

## Install

Run in an elevated PowerShell (Run as Administrator):

```powershell
.\install-windows.ps1
```

What the script does:

1. Checks for a running job (asks before stopping).
2. Creates `C:\transcode\`, `C:\transcode\logs\`, and `C:\transcode\jobs\`.
3. Downloads `yulia-worker.exe` to `C:\transcode\yulia-worker.exe`.
4. Creates `C:\transcode\worker.env` if it does not exist.
5. Registers the `AV1Worker` Scheduled Task to run as SYSTEM on startup with automatic restart.
6. Runs `yulia-worker diagnostics` to verify the install.
7. Starts the task if diagnostics pass.

To install without downloading (use an existing binary):

```powershell
.\install-windows.ps1 -BinaryUrl ""
```

## Configuration

All settings live in `C:\transcode\worker.env`. Edit this file to configure the worker.
Environment variables override file values — useful for secrets in managed installs.

```env
# Queue service base URL
QUEUE_URL=http://172.16.81.137:8090

# Bearer token — must match AUTH_WORKER_TOKEN on the queue server.
# Leave empty if queue auth is disabled.
QUEUE_TOKEN=

# Worker name shown in the queue dashboard
WORKER_NAME=MY-MACHINE

# ffmpeg/ffprobe paths (defaults shown)
FFMPEG_PATH=C:\msys64\mingw64\bin\ffmpeg.exe
FFPROBE_PATH=C:\msys64\mingw64\bin\ffprobe.exe

# Working and log directories
WORK_DIR=C:\transcode\jobs
LOG_DIR=C:\transcode\logs

# Encoder selection: av1_qsv (Intel QSV), av1_nvenc (NVIDIA), av1_vaapi (Linux only), libsvtav1 (software)
ENCODER=av1_qsv

# Quality value: -global_quality for QSV, -cq for NVENC, -crf for SVT-AV1 (lower = better quality / larger file)
ENCODE_QUALITY=28

# Encoder preset (QSV/NVENC: veryfast/faster/fast/medium/slow/slower/veryslow; SVT-AV1: 0-12)
ENCODE_PRESET=medium

AUDIO_CODEC=aac
AUDIO_BITRATE=192k

# Seconds between queue polls when idle
POLL_SECS=10
```

**Note**: `worker.env` is read once at startup. A config change requires restarting the worker service (see **Restart** below).

## Verify the Install

```powershell
yulia-worker.exe --version
yulia-worker.exe diagnostics
```

`diagnostics` checks:
- `ffmpeg` and `ffprobe` are reachable at the configured paths
- The selected encoder (`av1_qsv` by default) can encode a 1-second test clip
- The queue is reachable at `QUEUE_URL`
- The bearer token is accepted (if `QUEUE_TOKEN` is set)

Exit code 0 = all checks passed. Exit code 1 = one or more checks failed.

## Authenticated Queue

If the queue has `AUTH_ENABLED=true` and `AUTH_LEGACY_MACHINE_ACCESS=false`:

1. On the queue server, set `AUTH_WORKER_TOKEN=<secret>`.
2. In `C:\transcode\worker.env`, set `QUEUE_TOKEN=<secret>`.
3. Run `yulia-worker.exe diagnostics` — the `auth` line should show `ok token accepted`.

To test that an invalid token is correctly rejected:

```powershell
$env:QUEUE_TOKEN="wrong"; yulia-worker.exe diagnostics
```

Expected: `auth  FAIL  401 Unauthorized — token missing or invalid`

## Check Worker Status

```powershell
# Is the task running?
Get-ScheduledTask -TaskName AV1Worker | Select-Object TaskName, State

# View live logs
Get-Content C:\transcode\logs\worker.log -Tail 50 -Wait
```

## Restart

After editing `worker.env`:

```powershell
Stop-ScheduledTask -TaskName AV1Worker
Start-Sleep -Seconds 3
Start-ScheduledTask -TaskName AV1Worker
```

Or from Task Scheduler UI: right-click `AV1Worker` → End → Run.

## Update

Re-run the install script. It will:
- Check for a running job before proceeding
- Replace the binary
- Keep your existing `worker.env`
- Update the Scheduled Task definition

```powershell
.\install-windows.ps1
```

## Rotate the Worker Token

The worker reads `QUEUE_TOKEN` only at startup. Rotating the token requires a restart.

Safe procedure:
1. On the queue server, update `AUTH_WORKER_TOKEN` to the new value.
2. On the worker machine, update `QUEUE_TOKEN` in `C:\transcode\worker.env`.
3. Wait for the current job to finish (check logs or queue dashboard).
4. Restart the worker (see **Restart** above).

Do not restart mid-encode — wait for the current job to reach `done` or `failed` first.

## Uninstall

```powershell
# Stop and remove the task
Stop-ScheduledTask -TaskName AV1Worker -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName AV1Worker -Confirm:$false

# Remove files (keeps C:\transcode\logs\ for audit)
Remove-Item -Force C:\transcode\yulia-worker.exe
Remove-Item -Force C:\transcode\worker.env
```

To also remove logs and working files:

```powershell
Remove-Item -Recurse -Force C:\transcode
```

## Coexistence with the enkodu Companion

`yulia-worker.exe` and `enkodu.exe` (the desktop companion) can run on the same Windows machine. They use different Scheduled Task names, different binary paths, and different working directories. No conflicts are expected.

If both are running on the same machine, ensure:
- The companion's `ENKODU_AUTH_TOKEN` and the worker's `QUEUE_TOKEN` are separate values (or both empty).
- The companion's `state.json` and the worker's `logs/` are in separate directories (they are by default).

## Troubleshooting

**`ffmpeg not found` at startup**
: Check `FFMPEG_PATH` in `worker.env`. Run `yulia-worker.exe diagnostics` to confirm the path.

**`encoder FAIL` in diagnostics**
: The `av1_qsv` encoder requires an Intel Arc or 12th-gen+ iGPU with the Intel GPU driver installed. Run `ffmpeg -encoders | findstr av1` to see available AV1 encoders. Set `ENCODER=libsvtav1` in `worker.env` to use software encode (slow but always available).

**`auth FAIL 401`**
: `QUEUE_TOKEN` in `worker.env` does not match `AUTH_WORKER_TOKEN` on the queue server. Update `QUEUE_TOKEN` and restart.

**`auth FAIL 403`**
: Token is accepted but the worker's token is not authorized for the endpoint the worker is calling. Check queue server configuration — the worker token must have the `worker` role.

**Worker starts then exits immediately (exit code 2)**
: Auth failure. Check `C:\transcode\logs\worker.log` for details. Fix `QUEUE_TOKEN` and restart.

**Logs filling disk**
: The log file is append-only. To rotate manually: stop the worker, rename `worker.log`, start the worker. Log rotation is planned for a future release.

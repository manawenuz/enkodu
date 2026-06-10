# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Secrets & Non-Git Config

All credentials, server URLs, deploy commands, and sensitive context live in:
```
~/.agentSecrets/enkodu/secrets.md
```
Read that file at the start of any session. Never commit anything from `~/.agentSecrets/`.

## Project Purpose

Distributed AV1 transcoding system. Logic lives on a TrueNAS NAS (172.16.81.137); Windows machines are stateless QSV worker nodes. The NAS dispatches one job at a time to an available worker, the worker transcodes locally, validates, then copies the result back to the NAS.

## Architecture

```
NAS (TrueNAS 172.16.81.137)         Windows worker (100.65.174.104)
├── docker-compose.yml               └── C:\transcode\
├── queue/  ← FastAPI + SQLite           ├── worker.py   ← polls queue API
│   ├── main.py                          └── (Task Scheduler autostart)
│   └── scanner.py
└── /mnt/<pool>/yulia/
    ├── .transcode/queue/            ← job state on disk (fallback/audit)
    └── Videos/                      ← source files + _av1 outputs alongside
```

**Communication:** Windows worker polls `http://172.16.81.137:<port>/jobs/next` over LAN. No inbound connections required to the Windows machine.

**File flow:**
1. NAS scanner finds h264/h265 files in `Videos/` with no `_av1` sibling → creates job in SQLite
2. Windows worker claims job via `GET /jobs/next`, mounts NAS share (`net use`), copies source to `C:\transcode\<job-id>\input.mp4`
3. ffmpeg QSV encode: `ffmpeg -i input.mp4 -c:v av1_qsv -global_quality 28 -preset medium -c:a copy output_av1.mp4`
4. Validation: ffprobe checks duration match (±2s) and codec = av1
5. Pass → copy output to NAS alongside original (e.g. `video_av1.mp4`) → `POST /jobs/<id>/done`
6. Fail → `POST /jobs/<id>/failed` with error log, leave source untouched

**Output naming:** `original.mp4` → `original_av1.mp4` in the same directory.

## Infrastructure

- **NAS share:** `\\172.16.81.137\yulia` — credentials: user `yulia`, password in env var `NAS_PASSWORD`
- **Windows ffmpeg:** `C:\msys64\mingw64\bin\ffmpeg.exe` and `ffprobe.exe` (installed via msys2 pacman: `mingw-w64-x86_64-ffmpeg`)
- **Windows Python:** `C:\msys64\mingw64\bin\python.exe` (Python 3.14 via msys2)
- **Windows SSH access:** `ssh -i ~/CascadeProjects/wzp manwe_gdqikx2@100.65.174.104`
- **Windows default shell:** PowerShell (set in `HKLM:\SOFTWARE\OpenSSH\DefaultShell`)
- **Windows SSH key:** `~/CascadeProjects/wzp` (ed25519, same key used for all servers)

## Key Constraints

- **Never replace originals.** Output always goes alongside the source with `_av1` suffix.
- **Validate before writing to NAS.** Duration must match within 2 seconds; output codec must be av1.
- **Worker is stateless.** All job state lives in the NAS queue service. Worker can be rebooted freely.
- **One job at a time per worker.** Worker reports busy via `GET /status`; dispatcher won't send another job until it's idle.
- **Windows background processes** must be launched via `schtasks` (not `Start-Process`), because SSH-spawned processes die when the SSH session ends.

## NAS Docker Compose

Deploy via TrueNAS SCALE "Custom App" or direct `docker compose up -d` if SSH is available. The yulia dataset is at `/mnt/pool1/pool1_data/home/yulia` on the NAS, mounted as `/data` in the container.

## Running the Worker on Windows

The worker runs as a scheduled task on Windows startup:
```powershell
schtasks /create /tn "AV1Worker" /tr "C:\msys64\mingw64\bin\python.exe C:\transcode\worker.py" /sc onstart /ru SYSTEM /f
schtasks /run /tn "AV1Worker"
```

Check worker logs: `C:\transcode\logs\worker.log`

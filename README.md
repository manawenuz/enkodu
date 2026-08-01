# Enkodu

Enkodu is a distributed video-transcoding system for converting a NAS video library to AV1 without replacing the originals. A queue service on the NAS discovers work, Rust workers transcode it on available machines, and companion clients submit local files, monitor jobs, and retrieve completed outputs.

## Architecture

```text
NAS / queue service                 Worker host                 Companion clients
┌──────────────────────┐            ┌──────────────────┐         ┌─────────────────┐
│ FastAPI + SQLite     │◄──────────►│ Rust yulia-worker│         │ macOS/Linux/Win │
│ scanner + dashboard  │            │ ffmpeg/ffprobe   │         │ desktop clients  │
│ /data video library  │            │ one job at a time│         │ Android/iOS apps │
└──────────────────────┘            └──────────────────┘         └─────────────────┘
```

The queue owns job state. Workers are disposable: they claim a job, download or stream the source, encode locally, validate the result, upload it, and report completion. The current worker supports QSV, VAAPI, NVENC, and SVT-AV1 where the host provides the required encoder.

## Repository layout

- `queue/` — FastAPI queue, scanner, dashboard, SQLite persistence, and tests.
- `worker/` — Rust transcoding worker and Linux/Windows installers.
- `companion/` — Rust desktop tray app and CLI for local scanning, submission, monitoring, and reconciliation.
- `mobile/` — Android and iOS companion clients.
- `docs/obsidian-vault/` — architecture maps, flows, operations notes, and product decisions.
- `docker-compose.yml` — queue service container definition.

## Safety guarantees

- Originals are never replaced.
- Outputs use an `_av1` suffix alongside the source file.
- A worker validates the output codec and duration before it is accepted by the queue.
- Jobs are processed one at a time per worker.
- Tokens and deployment credentials belong in environment files or local configuration, never in Git.

## Queue quick start

The queue requires Python 3.11+ and the dependencies in `queue/requirements.txt`.

```bash
cd queue
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
uvicorn main:app --host 0.0.0.0 --port 8090
```

For a NAS deployment, use the root `docker-compose.yml`. Configure the data path, queue database, scanner settings, and authentication through environment variables. Keep credentials outside the repository.

## Worker quick start

Build the worker from `worker/`:

```bash
cargo build --release
./target/release/yulia-worker diagnostics
```

Configure `QUEUE_URL`, `QUEUE_TOKEN`, `WORKER_NAME`, encoder settings, and working/log directories in the platform-specific worker environment file. See:

- [Linux worker installation](worker/docs/LINUX.md)
- [Windows worker installation](worker/docs/WINDOWS.md)

Windows workers run through the `AV1Worker` Scheduled Task. Linux workers use a user-level systemd service.

## Companion quick start

Build the desktop companion from `companion/`:

```bash
cargo build --release
./target/release/enkodu diagnostics
```

Platform setup and configuration are documented in:

- [Linux companion installation](companion/docs/LINUX.md)
- [Windows companion installation](companion/docs/WINDOWS.md)
- [iOS companion notes](mobile/ios/README.md)
- [Android companion notes](mobile/android/README.md)

## Tests

Run queue safety tests directly:

```bash
python3 queue/test_safety.py
python3 queue/test_security.py
```

Integration tests require a running queue and are documented in the [operations runbook](docs/obsidian-vault/04-Operations/Runbook.md).

## Documentation map

- [Architecture map](docs/obsidian-vault/00-Maps/Architecture%20Map.md)
- [System overview](docs/obsidian-vault/01-Architecture/System%20Overview.md)
- [Worker transcode flow](docs/obsidian-vault/02-Flows/Worker%20Transcode%20Flow.md)
- [Verification and safety flow](docs/obsidian-vault/02-Flows/Verification%20and%20Safety%20Flow.md)
- [Operations runbook](docs/obsidian-vault/04-Operations/Runbook.md)

## License

No license has been declared yet. Until one is added, the repository should be treated as source-available rather than generally licensed for redistribution.

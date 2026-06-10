---
tags:
  - architecture
  - overview
---

# System Overview

Enkodu is a distributed AV1 transcoding pipeline. The queue service owns all durable job state. Workers are disposable executors. Companions are user-facing clients that submit files, monitor progress, and retrieve verified AV1 outputs.

## Components

| Component | Status | Role | Source |
|---|---:|---|---|
| Queue service | Present | Scans NAS videos, stores jobs in SQLite, serves dashboard/API, accepts uploads, dispatches work, verifies outputs, enforces optional auth | `queue/main.py` |
| Worker | Present, needs fixture evidence | Polls for one job, downloads source over HTTP, probes encoder support, encodes AV1, validates locally, uploads output | `worker/src/main.rs` |
| macOS companion | Present, needs packaging | Tray app, file submit, batch scan, queue controls, reconcile/download, LaunchAgent toggle | `companion/src/main.rs`, `companion/src/platform/macos.rs` |
| Linux companion | Present, needs real-desktop verification | Desktop adapter for notifications, XDG paths, autostart, and Unix-socket IPC | `companion/src/platform/linux.rs` |
| Windows companion | Present, needs real-Windows verification | Desktop adapter for config/state paths, HKCU Run autostart, loopback IPC, and notification fallback | `companion/src/platform/windows.rs` |
| Android companion | Scaffolded, not release-ready | Native app shell with auth storage, AV1 gate, Retrofit API, Room transfer state, WorkManager transfer pieces | `mobile/android/` |
| iOS companion | Scaffolded, not release-ready | Native SwiftUI shell with Keychain auth, AV1 gate, Core Data transfer state, URLSession/background pieces | `mobile/ios/` |

## Design Principles

- Originals are preserved by default.
- `_av1.mp4` outputs are written beside sources when possible.
- Workers poll out; no inbound worker networking is required.
- The queue is the source of truth for job state.
- Hardware encoding is preferred for throughput.
- Release safety matters more than broad platform coverage.

## Current Data Paths

NAS-origin job:

1. Queue scanner finds an eligible file under `/data/Videos`.
2. Queue inserts a `pending` job with `source_path` and `output_path`.
3. Worker claims the job and streams source via `GET /jobs/{id}/source`.
4. Worker encodes locally and streams output back via `PUT /jobs/{id}/output`.
5. Queue writes output to the configured `output_path`.

Companion-origin job:

1. Desktop companion streams a local file via `POST /jobs/upload`; mobile clients use the resumable protocol.
2. Resumable clients call `POST /jobs/upload/resumable/start`, upload `Content-Range` chunks, then call `/finish`.
3. Queue writes input under `/data/.transcode/uploads/` and creates an output path in the same upload directory.
4. Worker processes it like any other job.
5. Client polls `GET /jobs/{id}` until `status=done` and `verify_status=pass`.
6. Client downloads the output via `GET /jobs/{id}/output`, optionally with `Range`, then verifies checksum via `/jobs/{id}/checksum`.

## Current Safety Gap

The worker validates codec and duration before upload. The queue then marks the job `done`, sends notification, and starts server verification asynchronously. If server verification fails, `verify_status` becomes `fail`, but `status` remains `done`.

Current guardrails:

- Output download requires `status=done` and `verify_status=pass`.
- Output checksum requires verified output.
- Delete-original endpoints require `verify_status=pass`.
- Mobile and desktop clients should keep treating `done` without `verify_status=pass` as not yet safe.

For a limited release, decide whether to:

- Fix the queue so `done` means verified-good.
- Or keep current semantics and make every UI/action visibly require `verify_status=pass`.

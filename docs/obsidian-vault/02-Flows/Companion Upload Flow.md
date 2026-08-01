---
tags:
  - flow
  - companion
---

# Companion Upload Flow

```mermaid
sequenceDiagram
  participant User
  participant C as Desktop companion
  participant Q as Queue
  participant W as Worker
  participant S as Local state

  User->>C: Select file or trigger batch scan
  C->>C: ffprobe source
  C->>Q: POST /jobs/upload<br/>X-Filename + X-Filepath
  Q->>Q: Save source under uploads/job_id
  Q->>Q: Create pending job with priority 10
  Q-->>C: job_id + priority_position
  C->>S: Save pending entry
  W->>Q: Claim and process job
  loop every 5s
    C->>Q: GET /jobs/{id}
    Q-->>C: status + verify_status
  end
  C->>C: Require status=done + verify_status=pass
  C->>Q: GET /jobs/{id}/output
  Q-->>C: output stream
  C->>Q: GET /jobs/{id}/checksum
  C->>C: Local verify codec + duration
  C->>S: Save done entry
  C-->>User: Notification
```

At startup the desktop companion registers its stable ID and capabilities, fetches any pending remote configuration, and opens `WS /ws/companion/{id}`. The connection carries live control/configuration and progress messages. Local scanning periodically publishes a file manifest to the queue's file pool; operators can inspect, exclude, reorder, and build that pool into jobs through the queue-plan API.

## Mobile / Resumable Variant

```mermaid
sequenceDiagram
  participant M as Mobile companion
  participant Q as Queue
  participant S as Local transfer state

  M->>M: Check AV1 hardware decode gate
  M->>Q: POST /jobs/upload/resumable/start
  Q-->>M: upload_id + chunk_size
  M->>S: Persist upload_id and byte offset
  loop chunks
    M->>Q: PUT /jobs/upload/resumable/{upload_id}/chunk<br/>Content-Range
    Q-->>M: received bytes
    M->>S: Persist confirmed offset
  end
  M->>Q: POST /jobs/upload/resumable/{upload_id}/finish
  Q-->>M: job_id
  M->>Q: GET /jobs/{id}
  Q-->>M: status + verify_status
  M->>Q: GET /jobs/{id}/output<br/>Range
  M->>Q: GET /jobs/{id}/checksum
  M->>S: Mark done only after checksum + verify_status=pass
```

## Modes Present In Code

- Tray submit through file picker.
- Batch scan over configured directories.
- IPC commands to trigger `scan`, `reconcile`, `status`, NAS pause/resume, Mac pause/resume.
- Recovery of unfinished downloads on startup.
- Reconcile mode that matches server-done jobs back to local files by filename plus metadata.
- `wanryo` helper that emits a CSV checklist for manual review/deletion decisions.
- Bearer-token support through `auth_token` or `ENKODU_AUTH_TOKEN`.
- Connection/auth diagnostics through `enkodu test`.
- Retry/backoff and checksum verification for transfer paths.

## Companion Config

Default path: `~/.config/enkodu/config.toml`

Important fields:

- `server_url`
- `scan.directories`
- `scan.extensions`
- `behavior.mode`
- `behavior.on_success`
- `behavior.backup_suffix`
- `behavior.skip_if_av1`
- `behavior.min_duration_secs`

## Release Gaps

- macOS binary is downloaded unsigned and requires quarantine removal.
- No versioning or update channel.
- No notarized `.app`, `.pkg`, Homebrew tap, or Sparkle-style updater.
- `replace` mode exists and should stay off by default for limited release.
- Linux and Windows companions have platform adapters, but need real-platform tray, notification, IPC, submit/download, and reconcile verification.
- Mobile clients are scaffolded, but need real-device auth, background transfer, and save/share verification.

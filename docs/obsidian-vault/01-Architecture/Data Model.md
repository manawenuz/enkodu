---
tags:
  - architecture
  - data
---

# Data Model

The queue service uses SQLite at `DB_PATH` with WAL enabled. The default path is under `/data/.transcode/queue.db`; concrete deployment paths belong in the external secrets file.

```mermaid
erDiagram
  JOBS {
    text id PK
    text source_path UK
    text output_path
    text source_unc
    text output_unc
    integer source_size
    real source_duration_secs
    text status
    text worker
    real percent
    real fps
    text speed
    integer output_size
    text error
    real created_at
    real updated_at
    integer priority
    text source_filename
    text verify_status
    text verify_detail
    text source_meta
    text output_meta
    text verify_checks
    text client_name
    text client_path
  }

  CLIENTS {
    text ip PK
    text name
    text color
    real first_seen
    real last_seen
    integer uploads
    integer weight
    text queue_manifest
  }

  SETTINGS {
    text key PK
    text value
  }

  AUTH_USERS {
    text id PK
    text username UK
    text display_name
    text email
    text role
    text source
    text external_subject
    integer enabled
    real created_at
    real updated_at
    real last_login
  }

  AUTH_PASSKEYS {
    text id PK
    text user_id FK
    text credential_id UK
    blob credential_public_key
    integer sign_count
    text transports
    text name
    real created_at
    real last_used
  }

  AUTH_CHALLENGES {
    text id PK
    text user_id FK
    text kind
    text challenge
    text token
    real expires_at
    real created_at
    real consumed_at
  }

  AUTH_SESSIONS {
    text token_hash PK
    text user_id FK
    real created_at
    real expires_at
    real last_seen
    text user_agent
    text ip
  }

  TELEMETRY {
    integer id PK
    text client_id
    text event_type
    text event_detail
    text job_id
    text platform
    integer success
    integer duration_ms
    integer bytes_transferred
    real created_at
  }

  CLIENTS ||--o{ JOBS : submits
  AUTH_USERS ||--o{ AUTH_PASSKEYS : owns
  AUTH_USERS ||--o{ AUTH_CHALLENGES : receives
  AUTH_USERS ||--o{ AUTH_SESSIONS : owns
```

## Job Statuses

| Status | Meaning |
|---|---|
| `pending` | Waiting to be claimed by a worker |
| `active` | Claimed by a worker |
| `done` | Worker reported output uploaded; server verification may still be running |
| `failed` | Worker or operator marked failure |

## Verification Statuses

| Status | Meaning |
|---|---|
| `running` | Server-side verification thread is probing output |
| `pass` | Server-side checks passed |
| `fail` | Server-side checks failed |
| null | Not verified, old job, or not yet started |

## In-Memory State

The service also keeps:

- `_live`: live job progress, lost on process restart.
- `_workers`: worker heartbeat/status registry, lost on process restart.
- `_control`: run/drain/stop command, persisted to `control.json`.
- `_resumable_uploads`: active upload sessions, lost on process restart. Session metadata and partial bytes are also written under `/data/.transcode/uploads/resumable_<upload_id>/`, but the in-memory map is authoritative until resumable sessions are made durable.
- `_sha256_cache`: process-local cache keyed by output path and mtime.

## Client State

Desktop companion state is keyed by local file path. It prevents duplicate submissions and supports interrupted download recovery.

Platform defaults:

| Client | Config | State |
|---|---|---|
| macOS desktop | `~/.config/enkodu/config.toml` | `~/.config/enkodu/state.json` |
| Linux desktop | `$XDG_CONFIG_HOME/enkodu/config.toml` or `~/.config/enkodu/config.toml` | `$XDG_STATE_HOME/enkodu/state.json` when set, otherwise config dir |
| Windows desktop | `%APPDATA%\Enkodu\config.toml` | `%LOCALAPPDATA%\Enkodu\state.json` |
| Android | EncryptedSharedPreferences / Room | Room transfer database |
| iOS | UserDefaults + Keychain | Core Data transfer state |

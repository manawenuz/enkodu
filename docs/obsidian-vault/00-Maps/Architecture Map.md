---
tags:
  - architecture
  - moc
---

# Architecture Map

```mermaid
graph LR
  subgraph NAS["Linux/NAS host"]
    Queue["FastAPI queue service<br/>queue/main.py"]
    DB[("SQLite<br/>queue.db")]
    Control["control.json<br/>run | drain | stop"]
    Videos["/data/Videos<br/>NAS library"]
    Uploads["/data/.transcode/uploads<br/>companion uploads"]
    Auth["Auth tables<br/>users/passkeys/sessions"]
    Telemetry["Telemetry table"]
    FilePool["Companion file pool"]
    QueuePlan["Queue plan / ordering"]
    Dashboard["Web dashboard<br/>/"]
  end

  subgraph WorkerHost["Worker host<br/>Windows or Linux"]
    Worker["Rust worker<br/>yulia-worker"]
    FFMPEG["ffmpeg / ffprobe<br/>QSV | VAAPI | NVENC | SVT-AV1"]
    WorkDir["Local job scratch"]
    Service["Scheduled Task<br/>or systemd user service"]
    WorkerEnv["worker.env<br/>QUEUE_TOKEN"]
  end

  subgraph DesktopCompanion["Desktop companion<br/>macOS / Linux / Windows"]
    Tray["Tray app"]
    CLI["CLI commands via IPC"]
    Config["Platform config<br/>auth_token"]
    State["Platform state"]
    Autostart["LaunchAgent<br/>XDG autostart<br/>HKCU Run"]
  end

  subgraph MobileCompanion["Mobile companion<br/>Android / iOS scaffold"]
    MobileUI["Native app"]
    MobileAuth["Secure token store"]
    TransferState["Transfer state<br/>Room / Core Data"]
  end

  Queue <--> DB
  Queue <--> Auth
  Queue <--> Telemetry
  Queue <--> FilePool
  Queue <--> QueuePlan
  Queue <--> Control
  Queue --> Videos
  Queue --> Uploads
  Dashboard --> Queue

  Service --> Worker
  WorkerEnv --> Worker
  Worker --> FFMPEG
  Worker --> WorkDir
  Worker -- "GET /jobs/next" --> Queue
  Worker -- "GET /jobs/{id}/source" --> Queue
  Worker -- "POST /jobs/{id}/progress" --> Queue
  Worker -- "PUT /jobs/{id}/output" --> Queue
  Worker -- "POST /jobs/{id}/done|failed" --> Queue
  Worker -- "POST /workers/{id}/heartbeat" --> Queue

  Tray --> CLI
  CLI --> Config
  CLI --> State
  Tray --> Autostart
  Tray -- "POST /jobs/upload" --> Queue
  Tray -- "GET /jobs/{id}" --> Queue
  Tray -- "GET /jobs/{id}/output" --> Queue
  Tray -- "POST /clients/queue-manifest" --> Queue
  Tray -- "GET /jobs/{id}/checksum" --> Queue
  Tray -- "WS /ws/companion/{id}" --> Queue

  MobileUI --> MobileAuth
  MobileUI --> TransferState
  MobileUI -- "POST /jobs/upload/resumable/*" --> Queue
  MobileUI -- "GET /jobs/{id}/output<br/>Range" --> Queue
  MobileUI -- "POST /telemetry" --> Queue

  Queue -. "live config, control, progress, file manifests" .-> Tray
```

## Maps

- [[01-Architecture/System Overview|System Overview]]
- [[01-Architecture/Component Map|Component Map]]
- [[01-Architecture/Data Model|Data Model]]
- [[01-Architecture/API Surface|API Surface]]
- [[02-Flows/NAS Scan Flow|NAS Scan Flow]]
- [[02-Flows/Worker Transcode Flow|Worker Transcode Flow]]
- [[02-Flows/Companion Upload Flow|Companion Upload Flow]]
- [[02-Flows/Verification and Safety Flow|Verification and Safety Flow]]

## Main Drift From Older Notes

The `AGENTS.md` and `CLAUDE.md` sketches describe a Python worker that mounts an SMB share and copies files directly. Current code uses a Rust worker that streams source and output through the queue API. Audio is re-encoded to AAC in the worker, not copied.

The worker is no longer Windows/QSV-only in code: Windows remains the deployment target with real host access, but the same binary now has Linux defaults, encoder probing for QSV/VAAPI/NVENC/SVT-AV1, `worker.env`, `diagnostics`, and `--version`.

The desktop companion is no longer macOS-only in code. macOS remains the first practical release target, but Linux and Windows adapters exist and need real-platform verification.

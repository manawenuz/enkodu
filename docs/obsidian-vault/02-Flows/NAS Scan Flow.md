---
tags:
  - flow
  - nas
---

# NAS Scan Flow

```mermaid
flowchart TD
  A["Scanner wakes every SCAN_INTERVAL"] --> B{"settings.nas_drain == true?"}
  B -->|"yes"| Z["Skip scan"]
  B -->|"no"| C["Walk VIDEOS_ROOT recursively"]
  C --> D{"Extension in VIDEO_EXTS?"}
  D -->|"no"| C
  D -->|"yes"| E{"Filename already contains _av1?"}
  E -->|"yes"| C
  E -->|"no"| F{"_av1.mp4 sibling exists?"}
  F -->|"yes"| C
  F -->|"no"| G{"Already in jobs.source_path?"}
  G -->|"yes"| C
  G -->|"no"| H["Probe source with ffprobe"]
  H --> I{"Pass filters?<br/>size, height, bitrate, skip hevc, skip av1"}
  I -->|"no"| C
  I -->|"yes"| J["Insert pending job<br/>client_name = NAS"]
  J --> C
```

## Eligibility Rules

Scanner currently skips:

- Non-video extensions.
- Files whose stem contains `_av1`.
- Files with an existing `_av1.mp4` sibling.
- Files already present in `jobs.source_path`.
- AV1 and HEVC when the corresponding settings are enabled.
- Files below configured minimum size, height, or bitrate.

## Dispatch Rule

`/jobs/next` selects a client using weighted fair queueing, then picks the highest priority and largest source file within that client. Companion uploads default to priority `10`; NAS jobs default lower.

## Release Notes

- The scanner mutates only SQLite, not source files.
- The `source_path` uniqueness constraint prevents duplicate NAS jobs.
- There is no durable scan history beyond jobs and logs.
- If `ffprobe` is absent or broken in the queue container, scanning and verification quality decline.


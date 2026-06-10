---
tags:
  - risks
  - open-questions
---

# Open Questions

## Safety

- Should `done` mean "worker uploaded output" or "server verified output"? Current code uses the former; release safety wants the latter.
- Should bad outputs be deleted immediately, moved to quarantine, or kept with a warning?
- How many retries should a job get before becoming permanent failure?
- Should quality failures trigger automatic drain/circuit breaker as described in the PRD?
- Is `verify_status=pass` enough for delete-original endpoints, or should destructive actions also require a second confirmation token?

## Product

- Is the limited release for one household/operator or multiple trusted users?
- Is unsigned macOS install acceptable for the first release?
- Is the product name officially Enkodu, with repo rename later?
- Is companion `replace` mode needed in limited release, or should it be hidden?
- Should NAS-origin jobs be visible and claimable by companions, or only administrable in dashboard?

## Platform

- Is Linux worker required for first release, or can Windows QSV carry the first version?
- Which Linux encoder target matters first: QSV, VAAPI, NVENC, or software?
- Should macOS ever be a worker, or only a companion?
- Is Android companion worth a native app, or would a web/PWA companion be enough?
- Should iOS be a first-class companion or a later share-sheet workflow?

## Operations

- What is the official registry/image tag strategy?
- What is the official rollback path for queue service updates?
- Where should structured logs live for queue and worker?
- What is the backup cadence for SQLite and in-flight uploads?
- What telemetry retention policy is acceptable for limited release?

## Codebase Drift

- `AGENTS.md` and `CLAUDE.md` still describe the older Python/SMB worker design.
- `companion/PRD.md` says no GUI and future menu bar app, but the tray app exists.
- The PRD describes stronger server verification behavior than current code implements.
- Docker image names differ between older secrets/deploy notes and current `docker-compose.yml`.

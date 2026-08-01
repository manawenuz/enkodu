---
tags:
  - roadmap
  - release
---

# Roadmap

## 0.1 Limited Trusted Release

Goal: one queue, one Windows worker, one macOS companion, private network only.

Must include:

- Safe completion semantics or a strictly enforced `verify_status=pass` UI/action contract.
- Verified-output guardrails before destructive actions, downloads, and checksums.
- Repeatable deploy steps.
- Worker and companion version visibility.
- Fixture-based smoke test run against a real worker.
- Clear "known limitations" note.

## 0.2 Operator Hardening

Goal: make day-two operations predictable.

Candidates:

- Telemetry retention policy.
- Structured logs.
- Backup/restore runbook tested.
- Queue DB migration notes.
- Retry/dead-letter tracking.
- Dashboard warning banners for unsafe states.
- Auth smoke tests for passkey, Authentik, Jellyfin, and strict machine tokens.

## 0.3 Platform Expansion

Goal: first non-current platform additions.

Likely order:

1. Finish desktop companion auth/test polish from [[Missing Companion Clients PRD]] Phase 0.
2. Verify Linux companion on an actual Linux desktop.
3. Verify Windows companion IPC/autostart/notification/WebSocket behavior on a real Windows host.
4. Verify Linux worker on a real Linux host with at least one hardware encoder and SVT-AV1 fallback.
5. Continue native Android companion from scaffold to real-device transfer flow.
6. Continue native iOS companion from scaffold to real-device transfer flow.

Primary PRD: [[Missing Companion Clients PRD]]

## 0.4 Distribution

Goal: make installation boring.

Candidates:

- Signed/notarized macOS app.
- Worker installer.
- Versioned downloads.
- Release notes.
- Update channel.
- Checksums.

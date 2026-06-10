---
tags:
  - enkodu
  - moc
  - release
created: 2026-06-09
---

# Enkodu Project Vault

Enkodu, in this repo still named `YuliaAV1`, is a distributed AV1 transcoding system. The current spine is:

- A Linux/NAS-hosted FastAPI queue service in `queue/main.py`.
- A stateless Rust worker in `worker/src/main.rs` with Windows and Linux defaults, encoder probing, diagnostics, and bearer-token auth.
- A Rust desktop companion in `companion/src/main.rs` with macOS, Linux, and Windows platform adapters.
- Native Android and iOS companion scaffolds under `mobile/`, both still pre-release.

## Start Here

- [[00-Maps/Architecture Map|Architecture Map]]
- [[00-Maps/Release Map|Release Map]]
- [[01-Architecture/System Overview|System Overview]]
- [[01-Architecture/Authentication|Authentication]]
- [[02-Flows/Worker Transcode Flow|Worker Transcode Flow]]
- [[02-Flows/Companion Upload Flow|Companion Upload Flow]]
- [[03-Platforms/Platform Matrix|Platform Matrix]]
- [[05-Product/Missing Companion Clients PRD|Missing Companion Clients PRD]]
- [[05-Product/Limited Release Checklist|Limited Release Checklist]]
- [[05-Product/Roadmap|Roadmap]]
- [[06-Risks/Risk Register|Risk Register]]
- [[06-Risks/Open Questions|Open Questions]]

There is also an Obsidian canvas at [[00-Maps/Architecture.canvas|Architecture.canvas]] for a visual project map.

## Current Limited-Release Thesis

The smallest coherent release is:

> Linux queue service + Windows worker + macOS companion, for trusted users on a private network/Tailscale, with originals never replaced automatically unless explicitly configured.

The queue now has opt-in auth, verified-output gates on download/checksum/delete actions, resumable uploads, ranged downloads, telemetry, and health/version probes. The remaining core safety decision is semantic: `status=done` still means "worker uploaded output"; clients must continue requiring `verify_status=pass` before download, checksum, delete, replacement, or save/share actions.

Linux companion, Linux worker, Windows companion, Android companion, and iOS companion all have implementation scaffolding. They are useful follow-ons, but they should not block the first limited release unless the release audience requires them and there is real-platform verification.

## Source Anchors

- Queue service: `queue/main.py`
- Queue container: `queue/Dockerfile`, `docker-compose.yml`
- Worker: `worker/src/main.rs`, `worker/docs/`
- Desktop companion app: `companion/src/main.rs`, `companion/src/platform/`, `companion/docs/`
- Android companion: `mobile/android/`
- iOS companion: `mobile/ios/`
- Companion API client: `companion/src/api.rs`
- Companion scan/reconcile: `companion/src/scan.rs`, `companion/src/reconcile.rs`
- Older companion PRD: `companion/PRD.md`
- Agent notes: `AGENTS.md`, `CLAUDE.md`

## Naming

The repository and older docs use `YuliaAV1`; the code and UI now mostly use `Enkodu`. Treat `Enkodu` as the product name and `YuliaAV1` as the historical repo/project name until renamed.

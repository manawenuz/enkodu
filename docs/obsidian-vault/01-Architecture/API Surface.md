---
tags:
  - architecture
  - api
---

# API Surface

The queue service is the only HTTP server. Workers and companions both call it.

When `AUTH_ENABLED=true`, browser/operator endpoints require a local passkey, Authentik-backed session, or Jellyfin-backed session. Worker and companion endpoints accept bearer tokens when `AUTH_WORKER_TOKEN` or `AUTH_COMPANION_TOKEN` are configured. The Rust worker reads `QUEUE_TOKEN` or `AUTH_WORKER_TOKEN`; the companion reads `auth_token` from config or `ENKODU_AUTH_TOKEN` from the environment. If queue tokens are unset and `AUTH_LEGACY_MACHINE_ACCESS=true`, machine endpoints remain compatible with current untokened clients.

## Authentication API

| Method | Path | Purpose |
|---|---|---|
| GET | `/login` | Passkey/Authentik/Jellyfin login entry page |
| GET | `/auth/setup?token=...` | One-time passkey registration page |
| GET | `/auth/me` | Return current session user |
| POST | `/auth/logout` | Revoke current session |
| POST | `/auth/admin/invite` | Create an invite/setup URL for an existing user |
| POST | `/auth/bootstrap` | Create the initial admin setup flow when no users exist |
| GET | `/auth/jellyfin/login` | Jellyfin login page |
| POST | `/auth/jellyfin/login` | Verify Jellyfin credentials and create Enkodu session |
| POST | `/auth/passkey/register/options` | Generate WebAuthn registration options |
| POST | `/auth/passkey/register/verify` | Verify and store passkey |
| POST | `/auth/passkey/login/options` | Generate WebAuthn authentication options |
| POST | `/auth/passkey/login/verify` | Verify passkey and create session |
| GET | `/auth/authentik/login` | Start Authentik OIDC flow |
| GET | `/auth/authentik/callback` | Complete Authentik OIDC flow |

## Status and Control

| Method | Path | Caller | Purpose |
|---|---|---|---|
| GET | `/status` | Companion, dashboard | Queue counts |
| GET | `/healthz` | Clients, deploy checks | DB-backed liveness check |
| GET | `/version` | Clients, deploy checks | Static queue version/build metadata |
| GET | `/stats` | Dashboard | Aggregate reporting |
| GET | `/control` | Worker, companion | Current command: run/drain/stop |
| POST | `/control/{cmd}` | Companion, dashboard | Set command |
| GET | `/settings` | Companion, dashboard | Current settings |
| POST | `/settings` | Companion, dashboard | Update settings |
| POST | `/scan` | Dashboard | Trigger NAS scan |

## Clients and Telemetry

| Method | Path | Purpose |
|---|---|---|
| GET | `/clients` | List known companion/upload clients and queued manifest counts |
| POST | `/clients/weights` | Update client scheduling weights |
| POST | `/clients/queue-manifest` | Report companion scan queue manifest |
| POST | `/companions/{cid}/register` | Register or refresh a desktop companion |
| GET | `/companions/{cid}/config` | Read pending/current companion configuration |
| PUT | `/companions/{cid}/config` | Set companion configuration |
| POST | `/companions/{cid}/capabilities` | Publish platform and codec capabilities |
| GET | `/companions` | List registered companions and last-seen state |
| POST | `/companions/{cid}/promote` | Promote a companion into the active coordination role |
| WS | `/ws/{kind}/{cid}` | Live worker/companion connection for hello, heartbeats, config, control, progress, and file manifests |
| POST | `/telemetry` | Accept bounded client events; rejects likely secrets and path-heavy detail |
| GET | `/telemetry/summary?days=7` | Aggregate event totals by event type and platform |

## Worker API

| Method | Path | Purpose |
|---|---|---|
| GET | `/jobs/next?worker=<name>` | Atomically claim next pending job |
| POST | `/jobs/abandon?worker=<name>` | Requeue active jobs for a worker |
| GET | `/jobs/{id}/source` | Stream source bytes |
| POST | `/jobs/{id}/progress` | Update percent, phase, fps, speed |
| PUT | `/jobs/{id}/output` | Stream encoded output back |
| POST | `/jobs/{id}/done` | Mark output uploaded |
| POST | `/jobs/{id}/failed` | Mark failure |
| POST | `/workers/{id}/heartbeat` | Advertise worker status |

## Companion API

| Method | Path | Purpose |
|---|---|---|
| POST | `/jobs/upload` | Upload local source file |
| POST | `/jobs/upload/resumable/start` | Start a chunked upload session |
| PUT | `/jobs/upload/resumable/{upload_id}/chunk` | Upload one chunk with `Content-Range: bytes start-end/total` |
| POST | `/jobs/upload/resumable/{upload_id}/finish` | Finalize upload and create a job |
| GET | `/jobs/{id}` | Poll job state |
| GET | `/jobs/{id}/output` | Download verified output; supports `Range` and returns `X-SHA256` on full downloads |
| POST | `/jobs/{id}/set-path` | Record client-side source path |
| GET | `/jobs?status=done&limit=...` | Reconcile local files with done jobs |
| GET | `/jobs/live` | Show active worker progress |
| GET | `/jobs/{id}/checksum` | Verify downloaded output checksum |

Notes:

- `/jobs/{id}/output` and `/jobs/{id}/checksum` require `status=done` and `verify_status=pass`.
- Resumable upload sessions are currently process-memory state with partial bytes and metadata on disk. A queue restart invalidates active session IDs; clients should start a new session from byte zero when they receive `404 upload session not found`.

## Operator API

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | Dashboard |
| GET | `/install` | macOS companion install guide |
| GET | `/download/enkodu` | Download companion binary |
| POST | `/jobs/{id}/force-encode` | Promote a pending job to priority `999` and set control to run |
| POST | `/jobs/{id}/requeue` | Requeue job |
| POST | `/jobs/{id}/rescan` | Re-probe one job |
| POST | `/jobs/bulk-rescan` | Re-probe many jobs |
| POST | `/jobs/backfill-meta` | Probe missing source/output metadata |
| POST | `/jobs/{id}/delete-original` | Delete original only when output is verified pass |
| POST | `/jobs/bulk-delete-original` | Delete originals per job, requiring verified pass for each |
| POST | `/jobs/clear-pending` | Delete pending jobs |
| POST | `/jobs/clear-failed` | Delete failed jobs |

## File pool and queue planning

| Method | Path | Purpose |
|---|---|---|
| GET | `/file-pool` | List companion-discovered files |
| POST | `/file-pool/exclude/{pool_id}` | Exclude a discovered file from planning |
| GET | `/queue-plan` | Read the current planned order |
| POST | `/queue-plan/reorder` | Reorder planned files |
| POST | `/file-pool/build-queue` | Turn planned files into queue jobs |

## Release API Gaps

- Strict machine auth still needs a deployment dry-run with `AUTH_LEGACY_MACHINE_ACCESS=false`.
- `done` does not guarantee `verify_status=pass`.
- Delete-original, output download, and checksum endpoints are guarded by verified-good output; keep tests around this because it is a data-loss boundary.
- Upload size, disk-space, and rate-limit protections are not explicit.
- Long-running operations are background threads with no durable task tracking.
- Resumable upload sessions are not durable across queue process restarts.
- Telemetry has guardrails but no retention policy yet.

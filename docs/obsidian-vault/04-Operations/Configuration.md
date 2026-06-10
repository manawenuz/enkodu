---
tags:
  - operations
  - config
---

# Configuration

## Queue Environment

| Variable | Purpose | Default in code |
|---|---|---|
| `VIDEOS_ROOT` | NAS video directory to scan | `/data/Videos` |
| `NAS_UNC_ROOT` | Windows/NAS UNC prefix retained for compatibility | `\\172.16.81.137\yulia` |
| `DB_PATH` | SQLite database path | `/data/.transcode/queue.db` |
| `SCAN_INTERVAL` | Scanner interval in seconds | `300` |
| `FFPROBE` | ffprobe executable inside container | `ffprobe` |
| `STALL_TIMEOUT` | Active-job stale timeout in seconds | `900` |
| `TELEGRAM_BOT_TOKEN` | Optional notification token | empty |
| `TELEGRAM_CHAT_ID` | Optional notification target | empty |
| `COMPANION_BIN` | File served by `/download/enkodu` | `/app/enkodu-macos` |

Notes:

- Companion uploads are stored under `/data/.transcode/uploads/` in current code; this path is not yet configurable.
- Resumable upload sessions expire after 24 hours via an hourly cleanup thread.
- `/healthz` proves the queue process can talk to SQLite. It is not a full API compatibility check.

## Authentication Environment

See [[01-Architecture/Authentication|Authentication]] for architecture and flows.

| Variable | Purpose | Default in code |
|---|---|---|
| `AUTH_ENABLED` | Enable dashboard/API auth middleware | `false` |
| `AUTH_PUBLIC_ORIGIN` | Public HTTPS origin used for passkeys and setup URLs | derived from request |
| `AUTH_RP_ID` | WebAuthn relying party ID | derived from origin host |
| `AUTH_RP_NAME` | WebAuthn relying party display name | `Enkodu` |
| `AUTH_SESSION_SECRET` | Starlette session secret for OIDC state cookies | generated per process if unset |
| `AUTH_SESSION_TTL` | Enkodu session lifetime in seconds | `2592000` |
| `AUTH_COOKIE_SECURE` | Force secure cookies | true when origin is HTTPS |
| `AUTH_API_TOKEN` | Optional bearer token for admin/API automation | empty |
| `AUTH_WORKER_TOKEN` | Bearer token accepted by worker endpoints | empty |
| `AUTH_COMPANION_TOKEN` | Bearer token accepted by companion endpoints | empty |
| `AUTH_LEGACY_MACHINE_ACCESS` | Allow untokened worker/companion endpoints when tokens are unset | `true` |
| `AUTHENTIK_ENABLED` | Enable Authentik OIDC login button/routes | `false` |
| `AUTHENTIK_DISCOVERY_URL` | Authentik OIDC discovery URL | empty |
| `AUTHENTIK_CLIENT_ID` | Authentik OAuth2/OIDC client ID | empty |
| `AUTHENTIK_CLIENT_SECRET` | Authentik OAuth2/OIDC client secret | empty |
| `AUTHENTIK_ALLOWED_EMAIL_DOMAIN` | Optional email-domain allow-list | empty |
| `AUTHENTIK_AUTO_CREATE_USERS` | Auto-create users on first Authentik login | `false` |
| `AUTHENTIK_DEFAULT_ROLE` | Role for auto-created Authentik users | `operator` |
| `JELLYFIN_ENABLED` | Enable Jellyfin login button/routes | `false` |
| `JELLYFIN_URL` | Base URL for Jellyfin, no trailing slash required | empty |
| `JELLYFIN_CLIENT_NAME` | Client name sent in Jellyfin auth headers | `Enkodu` |
| `JELLYFIN_DEVICE_NAME` | Device name sent in Jellyfin auth headers | `Enkodu Queue` |
| `JELLYFIN_DEVICE_ID` | Stable Jellyfin device ID for this queue service | `enkodu-queue` |
| `JELLYFIN_APP_VERSION` | Version sent in Jellyfin auth headers | `0.1.0` |
| `JELLYFIN_ALLOW_EMPTY_PASSWORD` | Permit Jellyfin passwordless users to log into Enkodu | `false` |
| `JELLYFIN_REQUIRE_ADMIN` | Require Jellyfin `Policy.IsAdministrator` | `false` |
| `JELLYFIN_ALLOWED_USERS` | Optional comma-separated Jellyfin usernames or IDs | empty |
| `JELLYFIN_AUTO_CREATE_USERS` | Auto-create Enkodu users after successful Jellyfin login | `false` |
| `JELLYFIN_AUTO_LINK_LOCAL_USERS` | Link matching local usernames to Jellyfin on first login | `true` |
| `JELLYFIN_DEFAULT_ROLE` | Role for auto-created Jellyfin users | `operator` |

## Authentication CLI

Run inside the queue container or on a host with the same `DB_PATH`:

```bash
python main.py auth status
python main.py auth create-user alice --display-name Alice --email alice@example.com --role admin
python main.py auth invite alice
python main.py auth reset-passkeys alice
python main.py auth list-users
```

Recovery is intentionally command-line only for the first release. There is no password reset flow.

For Jellyfin-backed users with `JELLYFIN_AUTO_CREATE_USERS=false`, pre-provision a local user with the same username and keep `JELLYFIN_AUTO_LINK_LOCAL_USERS=true` so first successful Jellyfin login links that row.

## Queue Settings Table

| Key | Purpose |
|---|---|
| `min_size_mb` | Skip small files |
| `min_height` | Skip low-resolution files |
| `min_bitrate_kbps` | Skip low-bitrate files |
| `skip_hevc` | Skip HEVC inputs |
| `skip_av1` | Skip AV1 inputs |
| `nas_drain` | Pause NAS scanner |
| `nas_data_root` | Dashboard display/help value |

## Worker Environment

The worker reads an optional `.env` file once at startup, then overlays real environment variables. Keep the file mode private because it can contain `QUEUE_TOKEN`.

Default env file:

| Platform | Default file |
|---|---|
| Windows | `C:\transcode\worker.env` |
| Linux | `~/.config/yulia-worker/worker.env` |

Config keys:

| Variable | Purpose | Windows default | Linux/default elsewhere |
|---|---|---|---|
| `WORKER_ENV_FILE` | Override env-file path | `C:\transcode\worker.env` | `~/.config/yulia-worker/worker.env` |
| `QUEUE_URL` | Queue API base URL | private LAN default in code | private LAN default in code |
| `QUEUE_TOKEN` | Worker bearer token sent to queue API | empty | empty |
| `AUTH_WORKER_TOKEN` | Compatibility alias; `QUEUE_TOKEN` wins | empty | empty |
| `FFMPEG_PATH` | ffmpeg path | `C:\msys64\mingw64\bin\ffmpeg.exe` | `ffmpeg` |
| `FFPROBE_PATH` | ffprobe path | `C:\msys64\mingw64\bin\ffprobe.exe` | `ffprobe` |
| `WORK_DIR` | Local scratch directory | `C:\transcode\jobs` | `/tmp/yulia-worker/jobs` |
| `LOG_DIR` | Worker log directory | `C:\transcode\logs` | `~/.local/share/yulia-worker/logs` |
| `WORKER_NAME` | Worker identifier | hostname | hostname |
| `POLL_SECS` | Poll interval | `10` | `10` |
| `ENCODER` | Encoder override; empty means auto-detect | empty or installer sets `av1_qsv` | empty |
| `ENCODE_QUALITY` | Quality value passed as encoder-specific quality flag | `28` | `28` |
| `ENCODE_PRESET` | Encoder preset | `medium` or encoder default | encoder default |
| `AUDIO_CODEC` | Output audio codec | `aac` | `aac` |
| `AUDIO_BITRATE` | Output audio bitrate | `192k` | `192k` |
| `VAAPI_DEVICE` | VAAPI render node path | `/dev/dri/renderD128` | `/dev/dri/renderD128` |

Encoder auto-detection order when `ENCODER` is empty:

1. `av1_qsv`
2. `av1_vaapi`
3. `av1_nvenc`
4. `libsvtav1`

## Companion Config File

Desktop config paths:

| Platform | Config path | State path |
|---|---|---|
| macOS | `~/.config/enkodu/config.toml` | `~/.config/enkodu/state.json` |
| Linux | `$XDG_CONFIG_HOME/enkodu/config.toml` or `~/.config/enkodu/config.toml` | `$XDG_STATE_HOME/enkodu/state.json` when set, otherwise config dir |
| Windows | `%APPDATA%\Enkodu\config.toml` | `%LOCALAPPDATA%\Enkodu\state.json` |

```toml
server_url = "https://example.invalid"
auth_token = "same-value-as-AUTH_COMPANION_TOKEN"

[scan]
directories = ["~/Movies", "~/Downloads"]
extensions = ["mp4", "mov", "mkv", "avi", "m4v", "ts"]

[behavior]
mode = "interactive"
on_success = "rename"
backup_suffix = ".bak"
skip_if_av1 = true
min_duration_secs = 30
```

The companion can also read `ENKODU_AUTH_TOKEN` from the environment. When set, the environment value takes precedence over `auth_token`.

## Secret Handling

Do not put these in the repo:

- API keys.
- Tokens.
- Private registry credentials.
- Private SSH usernames/hosts if they are not already intentionally documented.
- Production deploy commands that embed credentials.
- DNS provider tokens.

Reference the external secrets file when needed.

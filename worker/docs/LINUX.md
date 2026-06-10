# yulia-worker Linux Install Guide

## Prerequisites

- x86_64 Linux
- systemd with user services enabled
- `ffmpeg` and `ffprobe` with AV1 encoder support
- Network access to the Enkodu queue service

Install ffmpeg from your distribution package manager, then check available AV1 encoders:

```bash
ffmpeg -hide_banner -encoders 2>/dev/null | grep av1
ffprobe -version
```

## Encoder Requirements

The Linux worker can auto-detect encoders in this priority order unless `ENCODER` is set.

| Encoder | Config value | Requirements | Notes |
|---|---|---|---|
| Intel QSV | `av1_qsv` | Intel GPU with VA-API and the Intel iHD driver | Uses `-global_quality` and `-preset`; common on Intel Arc and newer Intel iGPUs. |
| VAAPI | `av1_vaapi` | AMD or Intel GPU with Mesa/VA-API support | Uses `VAAPI_DEVICE`, usually `/dev/dri/renderD128`, plus `-qp`. |
| NVIDIA NVENC | `av1_nvenc` | NVIDIA RTX 4000 series or later with a driver/ffmpeg build that exposes AV1 NVENC | Uses `-cq` and `-preset`; AV1 NVENC requires Lovelace-generation hardware. |
| SVT-AV1 software | `libsvtav1` | No GPU required; ffmpeg must include libsvtav1 | Slow but universal fallback; uses `-crf` and an SVT preset from `0` to `12`. |

For QSV or VAAPI, the worker user must be able to read/write the render node:

```bash
ls -l /dev/dri/renderD*
groups
sudo usermod -aG render "$USER"
# If your distro uses video instead:
sudo usermod -aG video "$USER"
```

Log out and back in after changing groups. If running inside a container or confined service, pass `/dev/dri/renderD*` or NVIDIA devices through and allow access in SELinux/AppArmor policy.

## Install

From the `worker/` directory:

```bash
chmod +x ./install-linux.sh
./install-linux.sh
```

The installer:

1. Installs `yulia-worker` to `/usr/local/bin/yulia-worker` when the current user can write there or passwordless `sudo` is available.
2. Falls back to `~/bin/yulia-worker` when `/usr/local/bin` cannot be used.
3. Creates `~/.config/yulia-worker/worker.env` if it does not exist and sets mode `0600`.
4. Writes `~/.config/systemd/user/yulia-worker.service`.
5. Runs `systemctl --user daemon-reload && systemctl --user enable --now yulia-worker`.
6. Runs `yulia-worker diagnostics` and exits `1` if diagnostics fail.

To install a specific binary:

```bash
./install-linux.sh /path/to/yulia-worker
```

If no binary is provided, the script looks for a release binary in `worker/target/release/`, a binary next to the script, then `yulia-worker` on `PATH`. If none exists and Cargo is installed, it builds `cargo build --release`.

## Configuration

Edit `~/.config/yulia-worker/worker.env`. Environment variables override file values. The worker reads the file once at startup, so restart after changes.

```env
QUEUE_URL=http://172.16.81.137:8090
QUEUE_TOKEN=
WORKER_NAME=my-linux-worker

FFMPEG_PATH=ffmpeg
FFPROBE_PATH=ffprobe

WORK_DIR=/tmp/yulia-worker/jobs
LOG_DIR=/home/my-user/.local/share/yulia-worker/logs
WORKER_ENV_FILE=/home/my-user/.config/yulia-worker/worker.env

POLL_SECS=10

# Leave empty for auto-detection, or set av1_qsv, av1_vaapi, av1_nvenc, or libsvtav1.
ENCODER=
ENCODE_QUALITY=28
ENCODE_PRESET=medium
AUDIO_CODEC=aac
AUDIO_BITRATE=192k
VAAPI_DEVICE=/dev/dri/renderD128
```

Config reference:

| Variable | Default | Description |
|---|---|---|
| `QUEUE_URL` | `http://172.16.81.137:8090` | Queue service base URL. |
| `QUEUE_TOKEN` | empty | Bearer token sent to authenticated worker endpoints. |
| `AUTH_WORKER_TOKEN` | empty | Compatibility alias for `QUEUE_TOKEN`; `QUEUE_TOKEN` wins when both are set. |
| `WORKER_NAME` | `$HOSTNAME` | Worker identifier shown by the queue. |
| `FFMPEG_PATH` | `ffmpeg` | Path or command name for ffmpeg. |
| `FFPROBE_PATH` | `ffprobe` | Path or command name for ffprobe. |
| `WORK_DIR` | `/tmp/yulia-worker/jobs` | Per-job working files. |
| `LOG_DIR` | `~/.local/share/yulia-worker/logs` | Worker log directory. |
| `WORKER_ENV_FILE` | `~/.config/yulia-worker/worker.env` | Optional `.env` file path. |
| `POLL_SECS` | `10` | Seconds between idle queue polls. |
| `ENCODER` | auto-detected | Override encoder: `av1_qsv`, `av1_vaapi`, `av1_nvenc`, or `libsvtav1`. |
| `ENCODE_QUALITY` | `28` | Quality value passed as `-global_quality`, `-cq`, `-qp`, or `-crf` depending on encoder. Lower usually means better quality and larger files. |
| `ENCODE_PRESET` | `medium` | Encoder preset. For SVT-AV1 use `0` to `12`; lower is slower and higher quality. |
| `AUDIO_CODEC` | `aac` | Audio codec for output. |
| `AUDIO_BITRATE` | `192k` | Audio bitrate for output. |
| `VAAPI_DEVICE` | `/dev/dri/renderD128` | VAAPI render node path. |

Keep `worker.env` at mode `0600` because it may contain `QUEUE_TOKEN`:

```bash
chmod 600 ~/.config/yulia-worker/worker.env
```

## Diagnostics

Run:

```bash
yulia-worker --version
yulia-worker diagnostics
```

`diagnostics` checks:

- `ffmpeg` and `ffprobe` are reachable
- the selected or auto-detected AV1 encoder can encode a one-second test clip
- queue health/version/status endpoints are reachable where supported
- bearer-token auth is accepted when queue auth is enabled
- worker name, queue URL, work dir, log path, and token state are printable without exposing the token value

Exit code `0` means all checks passed. Exit code `1` means one or more checks failed.

## Service Operations

Check status:

```bash
systemctl --user status yulia-worker
journalctl --user -u yulia-worker -f
```

Restart after config changes:

```bash
systemctl --user restart yulia-worker
```

Stop without disabling autostart:

```bash
systemctl --user stop yulia-worker
```

Enable lingering if the worker must start before the user logs in:

```bash
loginctl enable-linger "$USER"
```

## Update

Wait for the current job to finish, then re-run the installer with the new binary:

```bash
./install-linux.sh /path/to/new/yulia-worker
```

The installer replaces only the binary and service unit. It keeps the existing `worker.env`.

## Rotate the Worker Token

1. On the queue server, update `AUTH_WORKER_TOKEN` to the new value.
2. On the worker, edit `QUEUE_TOKEN` in `~/.config/yulia-worker/worker.env`.
3. Wait for any active job to finish.
4. Restart and verify:

```bash
systemctl --user restart yulia-worker
yulia-worker diagnostics
```

Do not put the token on the command line. Command lines can be visible to other local users.

## Uninstall

```bash
systemctl --user disable --now yulia-worker
rm -f ~/.config/systemd/user/yulia-worker.service
systemctl --user daemon-reload

rm -f /usr/local/bin/yulia-worker
rm -f ~/bin/yulia-worker
rm -f ~/.config/yulia-worker/worker.env
```

To also remove logs and work files:

```bash
rm -rf ~/.local/share/yulia-worker
rm -rf /tmp/yulia-worker
```

Use `sudo rm -f /usr/local/bin/yulia-worker` if the binary was installed with root ownership.

## Troubleshooting

**`ffmpeg not found` or `ffprobe not found`**
: Install ffmpeg/ffprobe or set `FFMPEG_PATH` and `FFPROBE_PATH` in `worker.env`.

**`encoder FAIL` in diagnostics**
: Run `ffmpeg -hide_banner -encoders 2>/dev/null | grep av1`. If hardware encode is unavailable, set `ENCODER=libsvtav1` for the software fallback.

**QSV or VAAPI encoder exists but the null encode fails**
: Check `/dev/dri/renderD*` permissions and add the worker user to `render` or `video`. Confirm Intel iHD or Mesa VA-API drivers are installed.

**`av1_nvenc` is missing**
: AV1 NVENC requires NVIDIA RTX 4000 series or later, a recent NVIDIA driver, and an ffmpeg build compiled with NVENC support.

**`auth FAIL 401`**
: `QUEUE_TOKEN` is missing or does not match `AUTH_WORKER_TOKEN` on the queue server.

**`auth FAIL 403`**
: The token is accepted but not authorized for worker endpoints. Check queue auth configuration.

**Service starts then exits repeatedly**
: Inspect `journalctl --user -u yulia-worker -n 100`. Auth failures exit non-zero and systemd waits `30` seconds before restart.

**`systemctl --user` cannot connect to the bus**
: Log in as the worker user and retry. For unattended startup, run `loginctl enable-linger "$USER"`.

**Logs fill disk**
: Stop the service, rotate or remove the log file under `LOG_DIR`, then start the service again. Journal logs can be managed with normal systemd journal retention settings.

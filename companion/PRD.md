# Enkodu Companion — PRD

## Purpose

A macOS-native CLI tool that lets a user submit local videos to the Enkodu transcoding server, monitor progress, verify quality, and safely replace the original with the AV1 output.

---

## Core Flow

```
scan dir → pick video → upload → poll status → server verifies → download → local verify → rename/replace
```

---

## Scope

- **Platform**: macOS (native Rust binary, arm64 + x86_64 universal)
- **Server dependency**: Enkodu queue API (HTTP)
- **No GUI** — CLI first. Menu bar app is a future consideration.

---

## Config

**Location**: `~/.config/enkodu/config.toml`

```toml
server_url = "https://enkodu.manwe.qzz.io"

[scan]
directories = [
  "~/Movies",
  "~/Downloads",
]
extensions = ["mp4", "mov", "mkv", "avi", "m4v", "ts"]

[behavior]
mode = "interactive"        # interactive | batch
on_success = "rename"       # rename | replace
backup_suffix = ".bak"      # only used when on_success = "replace"
skip_if_av1 = true          # skip files already encoded as AV1
min_duration_secs = 30      # skip very short clips
```

Generated with defaults on first run if missing.

---

## Modes

### Interactive (default)
1. Scan configured directories
2. Present numbered list of eligible videos with size and duration
3. User selects one (or a range: `1,3,5` or `1-5` or `all`)
4. Confirm before upload
5. Show live progress (polling every 3s, progress bar in terminal)
6. On completion: show verification result, prompt before replacing

### Batch (`--batch` flag or `mode = "batch"` in config)
- Processes all eligible videos sequentially
- No prompts — uses config defaults for all decisions
- Logs results to `~/.config/enkodu/batch.log`
- Skips files already in-flight or done (tracked in local state file)

---

## Eligibility Check (client-side, before upload)

A video is eligible if:
- Extension matches config list
- Duration > `min_duration_secs`
- Not already AV1 (ffprobe codec check)
- No `_av1` sibling exists alongside it
- Not currently tracked as in-flight in local state

---

## Upload

`POST /jobs/upload`
- Multipart or streaming body
- Returns `{ job_id, priority_position }`
- Local uploads get `priority = 10` — jump ahead of NAS scanner jobs
- Server stores file at `/data/.transcode/uploads/<job_id>/input.<ext>`

---

## Progress Polling

`GET /jobs/<id>` every 3s

Terminal output:
```
⟳  Запись МК Диагностика.mp4
   Uploading ████████████████░░░░  78%
   Encoding  ████░░░░░░░░░░░░░░░░  21%  |  143 fps  |  1.8x  |  ETA 4m32s
```

---

## Server-Side Verification

Triggered automatically after worker `PUT /jobs/<id>/output`. Blocks `done` status until passed.

### Checks (in order)

| Check | Method | Threshold |
|---|---|---|
| Duration | ffprobe | ±2s |
| Frame count | ffprobe stream | ±2% of expected |
| Codec | ffprobe | must be `av1` |
| Audio codec | ffprobe | must be `aac` |
| Frame sampling | ffmpeg `-vf fps=1/60` | extract 1 frame/min |
| Perceptual hash | phash per frame pair | avg similarity ≥ 90% |
| SSIM | ffmpeg ssim filter (sampled) | avg ≥ 0.92 |

### On failure
- Job → `failed` with detailed reason (which check, which timestamp)
- Output file deleted
- Job requeued automatically (up to 2 retries, then `failed_permanent`)
- Client notified via status poll

### Verification status field
```json
{ "verify_status": "pass|fail|pending", "verify_score": 0.97, "verify_detail": "..." }
```

---

## Client-Side Download & Local Verification

Once server reports `verify_status: pass`:

1. `GET /jobs/<id>/output` — streaming download
2. Local ffprobe checks:
   - codec = av1
   - duration matches original ±2s
3. If local check fails: warn user, do not replace, mark in local state

---

## Rename / Replace Behavior

### Default: `rename`
```
original.mp4          → untouched
original_av1.mp4      → downloaded output (temp name)
                         after local verify passes:
original.mp4          → original.mp4  (untouched)
original_av1.mp4      → stays as-is
```
User ends up with both. Safe, no data loss.

### `replace` mode
```
original.mp4          → original.mp4.bak  (renamed before overwrite)
original_av1.mp4      → original.mp4      (renamed to original name)
```
`--no-backup` flag skips the `.bak` step (dangerous, requires explicit flag).

---

## Local State

`~/.config/enkodu/state.json` — tracks submitted jobs keyed by file path:

```json
{
  "/Users/yulia/Movies/video.mp4": {
    "job_id": "abc-123",
    "submitted_at": 1234567890,
    "status": "done",
    "output_path": "/Users/yulia/Movies/video_av1.mp4"
  }
}
```

Prevents re-submitting the same file. User can `enkodu forget <path>` to clear an entry.

---

## CLI Reference

```
enkodu                        # interactive scan + pick
enkodu --batch                # process all eligible files
enkodu submit <file>          # submit a specific file directly
enkodu status                 # show all in-flight and recent jobs
enkodu status <job_id>        # show one job
enkodu forget <path>          # remove file from local state
enkodu config                 # print current config + path
enkodu config init            # write default config if missing
enkodu server                 # show server status (queue counts)
```

---

## Error Handling

| Scenario | Behavior |
|---|---|
| Server unreachable | Retry 3× with backoff, then exit with clear message |
| Upload interrupted | Job abandoned server-side on next startup (via `/jobs/abandon`) |
| Verification fail | Requeued up to 2×, then reported as permanent failure |
| Local verify fail | Output kept, original untouched, warning shown |
| Disk full (client) | Check free space before download, abort cleanly |

---

## Consecutive Quality Failure Circuit Breaker

If the server sees N consecutive jobs fail verification (not download errors — specifically SSIM/phash failures), it assumes something is wrong with the encoder and:

1. Sets control command to `drain` automatically
2. Sends a Telegram notification (see below)
3. Flags the queue status with `{ "circuit_breaker": true, "reason": "3 consecutive quality failures" }`
4. Dashboard shows a prominent warning banner
5. Operator must manually set control back to `run` after investigating

**Threshold**: 3 consecutive quality failures (configurable via env var `QUALITY_FAIL_THRESHOLD`, default 3).

Resets to 0 on any successful verification pass.

---

## Notifications (Telegram)

All server-side events of note push a message to a configured Telegram bot.

### Config (env vars in docker-compose)
```yaml
TELEGRAM_BOT_TOKEN: "..."
TELEGRAM_CHAT_ID: "..."
```

If not set, notifications are silently skipped.

### Events that trigger a notification

| Event | Message |
|---|---|
| Job done | `✅ Done: <filename> — <source>GB → <output>MB (<ratio>x smaller)` |
| Job failed (verify) | `⚠️ Quality fail: <filename> — SSIM <score>, phash <score>` |
| Job failed (error) | `❌ Error: <filename> — <error>` |
| Circuit breaker triggered | `🚨 Circuit breaker: 3 consecutive quality failures. Worker set to DRAIN. Check encoder.` |
| Queue empty | `🎉 Queue empty — all <N> jobs done` |
| Worker went idle (drain/stop) | `⏸ Worker idle — command: <drain\|stop>` |

### Dashboard notification log
Last 20 notifications shown in a collapsible panel on the dashboard (in-memory, lost on restart — Telegram is the persistent record).

---

## Future
- Menu bar app (shows active job count, progress)
- Watch mode (`enkodu watch ~/Movies` — auto-submits new files)
- Multi-server support
- Windows / Linux companion

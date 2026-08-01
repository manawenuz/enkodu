---
tags:
  - platforms
  - release
---

# Platform Matrix

Legend:

- `+` present or mostly present.
- `-` missing.
- `~` partially present, needs release work.
- `?` technically possible but not decided.

| Platform | Queue | Companion | Worker | Current Read |
|---|---:|---:|---:|---|
| Linux / NAS | + | ~ | ~ | Queue, auth, file-pool planning, and WebSocket coordination are real; Linux companion adapter and worker install/docs/encoder profiles exist, but need real host diagnostics and fixture run |
| macOS | N/A | ~ | - | Companion exists but needs packaging/signing; worker is not implemented |
| Windows | N/A | ~ | ~ | Worker install/docs/diagnostics and Scheduled Task flow exist; companion adapter has paths, HKCU autostart, loopback IPC, and WebSocket coordination; both need real Windows fixture verification |
| Android | N/A | ~ | - | Native companion scaffold exists with AV1 gate, auth storage, Retrofit, Room, WorkManager transfer pieces; not release-ready |
| iOS | N/A | ~ | - | Native companion scaffold exists with AV1 gate, Keychain, Core Data, URLSession transfer pieces; not release-ready |

Implementation PRD: [[05-Product/Missing Companion Clients PRD|Missing Companion Clients PRD]]

## Linux

Companion status (Phase 1-2 by vibe-cli):

- Platform adapter exists in `companion/src/platform/linux.rs`
- Notifications use `notify-send`
- Autostart writes XDG desktop entry
- Docs and build script exist (`companion/docs/LINUX.md`, `companion/build-linux.sh`)

Still needs:

- Real Linux desktop build/run verification
- Tray validation under common desktop environments
- Fixture submit/download/reconcile verification
- Config/state path confirmation on actual desktops

Worker status:

- Linux defaults are in place for ffmpeg/ffprobe, work paths, logs, and `worker.env`.
- Encoder profiles support QSV, VAAPI, NVENC, and SVT-AV1 fallback.
- User systemd install flow exists in `worker/install-linux.sh`.
- `yulia-worker --version` and `yulia-worker diagnostics` exist.
- Still needs real Linux host diagnostics and fixture transcode run.

## macOS

Companion somewhat present:

- Tray app exists.
- File picker exists.
- Batch scan exists.
- Upload/download/reconcile exists.
- LaunchAgent toggle exists.

Missing:

- Signed/notarized distribution.
- `.app` bundle or `.pkg`.
- Versioned download endpoint.
- Better first-run dependency check for `ffprobe`.
- Update/uninstall story.

Worker missing:

- AV1 hardware encode is not generally available across Macs.
- Software encode could be supported, but may be too slow/noisy for the product goal.
- VideoToolbox abstraction would matter only if target hardware supports AV1 encode.

## Windows

Worker status:

- Rust binary exists (`yulia-worker.exe`)
- `--version`, `diagnostics`, token auth, auth-failure halt behavior, log rotation, and `worker.env` support exist
- QSV encoder command exists
- Scheduled Task installer/update flow exists in `worker/install-windows.ps1`
- Config file support exists via `C:\transcode\worker.env`
- Logs are written under `C:\transcode\logs`

Still needs:

- Real Windows fixture transcode run
- Dependency bootstrap for ffmpeg/ffprobe beyond documented msys2 install
- Service manager UI, if desired

Companion status (Phase 3 by vibe-cli):

- Platform adapter exists in `companion/src/platform/windows.rs`
- Docs and build script exist (`companion/docs/WINDOWS.md`, `companion/build-windows.ps1`)
- Windows targets typecheck
- Config path uses `%APPDATA%\Enkodu`
- State path uses `%LOCALAPPDATA%\Enkodu`
- Autostart uses HKCU Run
- CLI commands route through localhost IPC with a per-run token

Still needs:

- Proper toast notifications (not blocking PowerShell message box)
- Real Windows command verification: `status`, `scan`, `reconcile`, `pause-nas`, `resume-nas`, `pause-local`, `resume-local`
- Submit/download/reconcile fixture run

## Android

Companion scaffolded:

- Kotlin + Jetpack Compose project exists.
- AV1 hardware decode checker uses `MediaCodecList` / `MediaCodecInfo` and blocks unsupported devices.
- Auth storage uses EncryptedSharedPreferences with a fallback SharedPreferences path.
- API client supports health/version/status, resumable upload, ranged download, checksum, control, and telemetry.
- Transfer manager uses Room transfer state, chunked upload, ranged download, retry/backoff, and telemetry hooks.

Still needs:

- Real-device auth setup and token validation.
- File picker and user-approved MediaStore/save flow verification.
- WorkManager/background behavior verification under Doze, network loss, battery, and process death.
- Output save only after checksum and `verify_status=pass`.

Worker out of scope:

- Mobile devices submit to Enkodu workers, not transcode locally

## iOS

Companion scaffolded:

- Swift + SwiftUI project exists with iOS 16 deployment target.
- AV1 hardware decode checker uses VideoToolbox `VTIsHardwareDecodeSupported`.
- Auth storage uses UserDefaults for non-secret state and Keychain for companion token.
- API client supports status, job polling, resumable upload, ranged download, checksum, health, and telemetry.
- Transfer manager uses Core Data state, explicit chunked upload, ranged download, retry/backoff, and telemetry hooks.

Still needs:

- Real-device auth setup and Keychain/background transfer verification.
- Photos/Files picker and save/share UX verification.
- Background URLSession behavior that fully matches the server chunk protocol.
- Output save/share only after checksum and `verify_status=pass`.

Worker out of scope:

- Not a realistic target for durable background transcoding

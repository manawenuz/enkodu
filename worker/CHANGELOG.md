# Changelog

## 0.1.0 - 2026-06-10

Phase 1-3 worker release prep.

- Added the Rust `yulia-worker` binary with queue polling, source download, AV1 transcode, ffprobe validation, output upload, progress reporting, heartbeat reporting, and safe local-output retention when final job completion reporting fails.
- Added worker configuration through environment variables and `worker.env`, including bearer-token queue auth, worker naming, configurable ffmpeg/ffprobe paths, work/log directories, poll interval, encoder quality, preset, audio settings, and VAAPI device path.
- Added cross-platform worker defaults for Windows and Linux, Linux `~` path expansion, log rotation, idle log throttling, auth-failure halt behavior, queue control handling, and `--version` / `diagnostics` commands.
- Added AV1 encoder selection for Intel QSV, VAAPI, NVIDIA NVENC, and SVT-AV1 software fallback, with per-encoder ffmpeg argument handling and one-second encoder probe diagnostics.
- Added Linux distribution assets: `install-linux.sh`, user systemd service setup, `worker.env` creation with `0600` permissions, diagnostics gate, and `worker/docs/LINUX.md`.
- Added Windows distribution assets: `install-windows.ps1`, `C:\transcode` layout, Scheduled Task install/update flow, `worker.env` creation, and diagnostics gate.
- Added unit coverage for env parsing, worker-name sanitization, log formatting, UTC timestamp formatting, duration threshold expectations, encoder argument generation, encoder fallback detection, and missing env-file handling.

Release verification:

- `cargo test` passed on `aarch64-apple-darwin`.
- `cargo build --release --target x86_64-pc-windows-gnu` passed on `aarch64-apple-darwin`.
- `cargo zigbuild --release --target x86_64-unknown-linux-gnu` passed on `aarch64-apple-darwin`.
- Plain `cargo build --release --target x86_64-unknown-linux-gnu` from this macOS host still needs a Linux GNU C linker/sysroot. Without one, `ring` fails looking for `x86_64-linux-gnu-gcc`. The working setup used the already-installed Zig toolchain through `cargo-zigbuild`.

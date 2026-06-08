#!/usr/bin/env bash
# Cross-compile the worker for Windows from macOS.
# Prerequisites (first run only):
#   rustup target add x86_64-pc-windows-gnu
#   brew install mingw-w64

set -e
cd "$(dirname "$0")/worker"

echo "Building yulia-worker for Windows..."
cargo build --release --target x86_64-pc-windows-gnu

EXE="target/x86_64-pc-windows-gnu/release/yulia-worker.exe"
echo "Built: $EXE ($(du -sh $EXE | cut -f1))"

# Deploy to Windows machine
WINDOWS_HOST="manwe_gdqikx2@100.65.174.104"
SSH_KEY="$HOME/CascadeProjects/wzp"

echo "Deploying to $WINDOWS_HOST ..."
scp -i "$SSH_KEY" "$EXE" "$WINDOWS_HOST:C:/transcode/yulia-worker.exe"

echo "Installing as Windows scheduled task (runs on boot)..."
ssh -i "$SSH_KEY" "$WINDOWS_HOST" '
  schtasks /delete /tn "YuliaWorker" /f 2>nul
  schtasks /create /tn "YuliaWorker" `
    /tr "C:\transcode\yulia-worker.exe" `
    /sc onstart /ru SYSTEM /f
  schtasks /run /tn "YuliaWorker"
  Write-Host "Worker installed and started."
'
echo "Done."

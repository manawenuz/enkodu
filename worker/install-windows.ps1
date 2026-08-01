#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Install or update yulia-worker on Windows.
.DESCRIPTION
    Downloads the latest yulia-worker.exe, places it at C:\transcode\yulia-worker.exe,
    creates a worker.env config file if one does not exist, and creates or updates
    the "AV1Worker" Scheduled Task.

    Does not interrupt a job that is currently running.
.PARAMETER BinaryUrl
    URL to download yulia-worker.exe from. Default is the GitHub Releases URL.
    Set to "" to skip download and use an existing binary at C:\transcode\yulia-worker.exe.
.PARAMETER WorkerName
    Worker name reported to the queue. Defaults to the machine hostname.
.EXAMPLE
    .\install-windows.ps1
    .\install-windows.ps1 -BinaryUrl "" -WorkerName "transcode-rig-2"
#>

param(
    [string]$BinaryUrl = "https://github.com/manawenuz/enkodu/releases/latest/download/yulia-worker.exe",
    [string]$WorkerName = $env:COMPUTERNAME
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$InstallDir = "C:\transcode"
$BinaryPath = "$InstallDir\yulia-worker.exe"
$EnvFile    = "$InstallDir\worker.env"
$TaskName   = "AV1Worker"
$PythonPath = "C:\msys64\mingw64\bin\python.exe"

# ── helpers ───────────────────────────────────────────────────────────────────

function Write-Step([string]$msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }
function Write-OK([string]$msg)   { Write-Host "    OK  $msg" -ForegroundColor Green }
function Write-Warn([string]$msg) { Write-Host "    WARN $msg" -ForegroundColor Yellow }

# ── check for running job ─────────────────────────────────────────────────────

Write-Step "Checking for active job"
$taskState = (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue).State
if ($taskState -eq "Running") {
    Write-Warn "AV1Worker task is currently Running."
    $answer = Read-Host "    A job may be in progress. Stop it now and continue? [y/N]"
    if ($answer -notmatch "^[Yy]$") {
        Write-Host "Aborted. Re-run after the current job finishes." -ForegroundColor Red
        exit 0
    }
    Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 3
}
Write-OK "Safe to proceed"

# ── create install directory ──────────────────────────────────────────────────

Write-Step "Creating directory $InstallDir"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\logs" | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\jobs" | Out-Null
Write-OK $InstallDir

# ── download binary ───────────────────────────────────────────────────────────

Write-Step "Installing binary"
if ($BinaryUrl -ne "") {
    Write-Host "    Downloading from $BinaryUrl ..."
    Invoke-WebRequest -Uri $BinaryUrl -OutFile "$BinaryPath.tmp" -UseBasicParsing
    Move-Item -Force "$BinaryPath.tmp" $BinaryPath
    Write-OK "Downloaded to $BinaryPath"
} elseif (Test-Path $BinaryPath) {
    Write-OK "Using existing binary at $BinaryPath (no download)"
} else {
    Write-Host "    ERROR: No binary at $BinaryPath and no BinaryUrl provided." -ForegroundColor Red
    Write-Host "    Copy yulia-worker.exe to $BinaryPath manually, then re-run with -BinaryUrl ''." -ForegroundColor Red
    exit 1
}

# ── print version ─────────────────────────────────────────────────────────────

try {
    $ver = & $BinaryPath --version 2>&1
    Write-OK "Binary version: $ver"
} catch {
    Write-Warn "Could not run --version: $_"
}

# ── create worker.env if missing ──────────────────────────────────────────────

Write-Step "Config file ($EnvFile)"
if (-not (Test-Path $EnvFile)) {
    @"
# yulia-worker configuration
# Lines starting with # are comments. Environment variables override these values.

QUEUE_URL=http://172.16.81.137:8090

# Worker bearer token. Must match AUTH_WORKER_TOKEN on the queue server.
# Leave empty if queue auth is disabled.
QUEUE_TOKEN=

WORKER_NAME=$WorkerName

FFMPEG_PATH=C:\msys64\mingw64\bin\ffmpeg.exe
FFPROBE_PATH=C:\msys64\mingw64\bin\ffprobe.exe

WORK_DIR=C:\transcode\jobs
LOG_DIR=C:\transcode\logs

# Encoder and quality settings
ENCODER=av1_qsv
ENCODE_QUALITY=28
ENCODE_PRESET=medium
AUDIO_CODEC=aac
AUDIO_BITRATE=192k

POLL_SECS=10
"@ | Set-Content -Path $EnvFile -Encoding UTF8
    Write-OK "Created $EnvFile (edit to set QUEUE_URL and QUEUE_TOKEN)"
} else {
    Write-OK "Existing $EnvFile kept unchanged"
}

# ── create / update Scheduled Task ───────────────────────────────────────────

Write-Step "Scheduled Task ($TaskName)"

$action  = New-ScheduledTaskAction -Execute $BinaryPath
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet `
    -ExecutionTimeLimit (New-TimeSpan -Days 365) `
    -RestartCount 5 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -MultipleInstances IgnoreNew

# Run as SYSTEM so it starts before any user logs in
$principal = New-ScheduledTaskPrincipal -UserId "SYSTEM" -RunLevel Highest

$existing = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
if ($existing) {
    Set-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal | Out-Null
    Write-OK "Updated existing task"
} else {
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal | Out-Null
    Write-OK "Registered new task"
}

# ── run diagnostics ───────────────────────────────────────────────────────────

Write-Step "Running diagnostics"
& $BinaryPath diagnostics
$diagExit = $LASTEXITCODE

# ── start worker ──────────────────────────────────────────────────────────────

if ($diagExit -eq 0) {
    Write-Step "Starting worker"
    Start-ScheduledTask -TaskName $TaskName
    Start-Sleep -Seconds 2
    $state = (Get-ScheduledTask -TaskName $TaskName).State
    Write-OK "Task state: $state"
    Write-Host "`nInstall complete. Logs at $InstallDir\logs\worker.log" -ForegroundColor Green
} else {
    Write-Host "`nDiagnostics failed — worker NOT started. Fix the issues above and run:" -ForegroundColor Yellow
    Write-Host "    Start-ScheduledTask -TaskName $TaskName" -ForegroundColor Yellow
}

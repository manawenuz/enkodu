<#
.SYNOPSIS
    Build script for Enkodu companion on Windows.

.DESCRIPTION
    This script builds the Enkodu companion for Windows and creates a distribution
    package. It requires Rust and the Windows SDK to be installed.

.PREREQUISITES
    - Rust (stable toolchain) - https://rustup.rs/
    - Visual Studio 2022 with Windows SDK (for Rust Windows targets)
    - PowerShell 5.1+

.EXAMPLE
    .\build-windows.ps1
    .\build-windows.ps1 -Release
#>

param (
    [switch]$Release = $true,
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$OutputDir = "target\windows-release"
)

$ErrorActionPreference = "Stop"

# Configuration
$BinName = "enkodu"
$ProjectDir = $PSScriptRoot
$TargetDir = if ($Release) { "release" } else { "debug" }
$FullTargetDir = "target\$TargetDir"

Write-Host "Building Enkodu companion for Windows..." -ForegroundColor Cyan

# Build the project
if ($Release) {
    Write-Host "Building in release mode..." -ForegroundColor Green
    & cargo build --release --target $Target
} else {
    Write-Host "Building in debug mode..." -ForegroundColor Yellow
    & cargo build --target $Target
}

if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed!"
    exit $LASTEXITCODE
}

Write-Host "Build succeeded!" -ForegroundColor Green

# Create output directory
if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Path $OutputDir | Out-Null
}

# Copy the binary
$SourcePath = "target\$TargetDir\$($Target.Replace('-', '\'))\$BinName.exe"
if (-not (Test-Path $SourcePath)) {
    Write-Error "Binary not found at: $SourcePath"
    exit 1
}

Copy-Item $SourcePath "$OutputDir\$BinName.exe" -Force

# Create a simple README
$ReadmeContent = @"
Enkodu Companion for Windows
==============================

INSTALLATION:
1. Copy `enkodu.exe` to a permanent location (e.g., `C:\Program Files\Enkodu\`)
2. Add the directory to your PATH, or create a shortcut

RUNNING:
- Double-click `enkodu.exe` to start the companion with tray icon
- Or run from command line: `enkodu.exe`

CLI COMMANDS:
- `enkodu.exe status` - Check queue status
- `enkodu.exe scan` - Trigger batch scan
- `enkodu.exe reconcile` - Reconcile server jobs
- `enkodu.exe pause-nas` - Pause NAS scanning
- `enkodu.exe resume-nas` - Resume NAS scanning
- `enkodu.exe tcpping <host:port>` - TCP connectivity test
- `enkodu.exe httping <url>` - HTTP connectivity test

REQUIREMENTS:
- PowerShell 5.1+ (for notifications)
- ffprobe (for video file verification - part of ffmpeg)

TROUBLESHOOTING:
- If tray icon doesn't appear, try running as Administrator
- If notifications don't work, ensure PowerShell is available
- Check logs in %LOCALAPPDATA%\Enkodu\ for debugging

UNINSTALL:
- Delete the `enkodu.exe` file
- Delete %APPDATA%\Enkodu\ for configuration
- Delete %LOCALAPPDATA%\Enkodu\ for state files
"@

$ReadmeContent | Out-File "$OutputDir\README.txt" -Encoding UTF8

Write-Host "" -ForegroundColor Cyan
Write-Host "Distribution created in: $OutputDir" -ForegroundColor Green
Write-Host "Contents:" -ForegroundColor Cyan
Get-ChildItem $OutputDir | ForEach-Object { Write-Host "  - $($_.Name)" }

Write-Host "" -ForegroundColor Cyan
Write-Host "To install:" -ForegroundColor Yellow
Write-Host "  1. Copy $BinName.exe to C:\Program Files\Enkodu\ (or any PATH directory)"
Write-Host "  2. Run $BinName.exe"
Write-Host ""
Write-Host "Note: Ensure PowerShell and ffprobe are available on your system."

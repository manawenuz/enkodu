# Enkodu Companion - Windows Installation

## Prerequisites

Ensure the following are installed on your Windows system:

### Required Dependencies
- **Rust** (stable toolchain with MSVC target) - <https://rustup.rs/>
  - Install with: `rustup default stable-msvc`
- **Visual Studio 2022** with Windows SDK
  - Install "Desktop development with C++" workload
- **PowerShell 5.1+** - Included with Windows 10/11 by default
- **ffprobe** - For video file verification (part of ffmpeg)
  - Download from: <https://ffmpeg.org/>
  - Or install via Chocolatey: `choco install ffmpeg`

### Install Dependencies via Chocolatey (Recommended)

```powershell
# Install Chocolatey (if not already installed)
Set-ExecutionPolicy Bypass -Scope Process -Force; [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor 3072; iex ((New-Object System.Net.WebClient).DownloadString('https://community.chocolatey.org/install.ps1'))

# Install build dependencies
choco install -y rust visualstudio2022community ffmpeg
```

## Building

### From Source

```powershell
# Clone the repository (if not already done)
git clone <repository-url>
cd YuliaAV1\companion

# Build for Windows (MSVC target)
cargo build --release --target x86_64-pc-windows-msvc

# The binary will be at target\x86_64-pc-windows-msvc\release\enkodu.exe
```

### Using the Build Script

```powershell
# Run the build script
.\build-windows.ps1
```

This will create a distribution in `target\windows-release\` containing:
- `enkodu.exe` - The companion binary
- `README.txt` - Installation instructions

## Installation

### Option 1: System-wide Installation

```powershell
# Create installation directory
New-Item -ItemType Directory -Path "C:\Program Files\Enkodu" -Force

# Copy the binary
Copy-Item target\x86_64-pc-windows-msvc\release\enkodu.exe "C:\Program Files\Enkodu\enkodu.exe"

# Add to PATH (optional - requires Admin)
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "Machine") + ";C:\Program Files\Enkodu", "Machine")
```

### Option 2: User Installation

```powershell
# Create user directory
$enkoduDir = "$env:USERPROFILE\AppData\Local\Programs\Enkodu"
New-Item -ItemType Directory -Path $enkoduDir -Force

# Copy the binary
Copy-Item target\x86_64-pc-windows-msvc\release\enkodu.exe "$enkoduDir\enkodu.exe"

# Add to user PATH
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$enkoduDir", "User")
```

### Option 3: Portable Installation

Simply copy `enkodu.exe` to any directory and run it directly.

## Configuration

On first run, Enkodu will create a default configuration file at:
- `%APPDATA%\Enkodu\config.toml`

### Example Configuration

```toml
server_url = "https://enkodu.manwe.qzz.io"

[scan]
directories = ["C:\\Users\\<user>\\Videos", "C:\\Users\\<user>\\Downloads"]
extensions = ["mp4", "mov", "mkv", "avi", "m4v", "ts"]

[behavior]
mode = "interactive"
on_success = "rename"
backup_suffix = ".bak"
skip_if_av1 = true
min_duration_secs = 30
```

Edit the configuration file to match your server URL and scan directories.

## Running

### Start the Companion

```powershell
# From the installation directory
.\enkodu.exe

# Or from anywhere if in PATH
enkodu.exe
```

The companion will appear as a tray icon in the system notification area.

### CLI Commands

```powershell
# Check status
enkodu.exe status

# Trigger a batch scan
enkodu.exe scan

# Trigger reconciliation
enkodu.exe reconcile

# Pause NAS scanning
enkodu.exe pause-nas

# Resume NAS scanning
enkodu.exe resume-nas

# TCP ping test
enkodu.exe tcpping <host:port>

# HTTP ping test
enkodu.exe httping <url>
```

### Running Alongside Windows Worker

The Enkodu companion and the Windows worker (`yulia-worker.exe`) can run on the same machine without conflicts. They use different:
- Configuration directories (companion: `%APPDATA%`, worker: `%LOCALAPPDATA%`)
- State files
- Process names
- Task names (if using Scheduled Tasks)

## Autostart

The companion supports autostart via the Startup folder. Use the "Start at Login" option in the tray menu to enable/disable autostart.

Alternatively, manually create a shortcut:

### Method 1: Startup Folder

```powershell
# Create a shortcut in the Startup folder
$shortcutPath = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup\Enkodu.lnk"
$targetPath = "C:\Program Files\Enkodu\enkodu.exe"

# Create shortcut using PowerShell
$WshShell = New-Object -ComObject WScript.Shell
$Shortcut = $WshShell.CreateShortcut($shortcutPath)
$Shortcut.TargetPath = $targetPath
$Shortcut.WorkingDirectory = Split-Path $targetPath
$Shortcut.Save()
```

### Method 2: Registry (Advanced)

```powershell
# Set registry Run key (requires Admin)
$regPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$valueName = "EnkoduCompanion"
$exePath = "C:\Program Files\Enkodu\enkodu.exe"

New-ItemProperty -Path $regPath -Name $valueName -Value $exePath -PropertyType String -Force
```

To disable:

```powershell
Remove-ItemProperty -Path $regPath -Name $valueName -ErrorAction SilentlyContinue
```

## Troubleshooting

### Tray Icon Not Appearing

- **Run as Administrator**: Some Windows versions require elevated privileges for tray icons
- **Check compatibility**: Right-click the executable → Properties → Compatibility → Try different settings
- **Taskbar issues**: Try restarting Explorer: `Stop-Process -Name explorer -Force`

### Notifications Not Working

- Ensure PowerShell 5.1+ is installed: `$PSVersionTable.PSVersion`
- Test PowerShell notifications: `Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show('Test', 'Title')`
- Check if notifications are disabled in Windows Settings

### "API not implemented" Error

This means the Rust Windows API being used requires the MSVC target. Ensure you're building with:
```powershell
cargo build --release --target x86_64-pc-windows-msvc
```

### Build Fails with Linker Error

Install Visual Studio 2022 with the "Desktop development with C++" workload, which includes the MSVC linker.

### ffprobe Not Found

Ensure ffmpeg is installed and in your PATH:
```powershell
# Test if ffprobe is available
ffprobe -version

# If not, install via Chocolatey
choco install ffmpeg
```

### Permission Errors

If you see permission errors when saving configuration or state files:
```powershell
# Create directories manually
New-Item -ItemType Directory -Path "$env:APPDATA\Enkodu" -Force
New-Item -ItemType Directory -Path "$env:LOCALAPPDATA\Enkodu" -Force
```

## Coexistence with Windows Worker

The companion and worker can run on the same machine. They:

- **Don't share configuration**: Companion uses `%APPDATA%\Enkodu`, worker uses its own config
- **Don't share state files**: Companion uses `%LOCALAPPDATA%\Enkodu` for state
- **Don't share working directories**: Worker uses `C:\transcode\`, companion uses temp directories
- **Use different process names**: `enkodu.exe` vs `yulia-worker.exe`

To verify both are running:
```powershell
Get-Process | Where-Object { $_.ProcessName -match "enkodu|yulia" }
```

## Known Limitations

- **IPC on Windows**: Currently uses a stub implementation. Full named pipe IPC requires the `named_pipe` crate. CLI commands work in direct mode.
- **Registry autostart**: Uses a file-based flag as fallback. Proper registry manipulation requires the `winreg` crate.
- **Toast notifications**: Uses a simple PowerShell approach. Proper Windows toast notifications require the `winrt-notification` crate.
- **Single instance**: Uses file-based lock. A proper Windows mutex would be more robust.

For production use, consider adding these crates to Cargo.toml:
```toml
[target.'cfg(target_os = "windows")'.dependencies]
winreg = "1.0"          # For registry manipulation
named_pipe = "0.3"     # For named pipe IPC
winrt-notification = "0.3"  # For proper toast notifications
```

## Uninstallation

```powershell
# Remove the binary
Remove-Item "C:\Program Files\Enkodu\enkodu.exe" -Force -ErrorAction SilentlyContinue

# Remove configuration
Remove-Item "$env:APPDATA\Enkodu" -Recurse -Force -ErrorAction SilentlyContinue

# Remove state files
Remove-Item "$env:LOCALAPPDATA\Enkodu" -Recurse -Force -ErrorAction SilentlyContinue

# Remove autostart shortcut
Remove-Item "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup\Enkodu.lnk" -Force -ErrorAction SilentlyContinue

# Remove registry autostart (if used)
Remove-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "EnkoduCompanion" -ErrorAction SilentlyContinue
```

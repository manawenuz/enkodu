# Enkodu Companion - Linux Installation

## Prerequisites

Ensure the following are installed on your Linux system:

### Required Dependencies
- **Rust** (stable toolchain) - <https://rustup.rs/>
- **notify-send** - For desktop notifications (part of `libnotify-bin` on Debian/Ubuntu)
- **xdg-utils** - For opening URLs and files (standard on most distributions)
- **ffprobe** - For video file verification (part of `ffmpeg` package)

### Install Dependencies on Debian/Ubuntu

```bash
sudo apt update
sudo apt install -y rustc cargo libnotify-bin xdg-utils ffmpeg
```

### Install Dependencies on Fedora/RHEL

```bash
sudo dnf install -y rust cargo libnotify xdg-utils ffmpeg
```

### Install Dependencies on Arch Linux

```bash
sudo pacman -S --needed rust libnotify xdg-utils ffmpeg
```

## Building

### From Source

```bash
# Clone the repository (if not already done)
git clone https://github.com/manawenuz/enkodu.git
cd enkodu/companion

# Build in release mode
cargo build --release

# The binary will be at target/release/enkodu
```

### Using the Build Script

```bash
./build-linux.sh
```

This will create a tarball `enkodu-linux-x86_64.tar.gz` containing the binary.

## Installation

### Manual Installation

```bash
# Copy the binary to a system directory
sudo cp target/release/enkodu /usr/local/bin/

# Or copy to your home directory
cp target/release/enkodu ~/.local/bin/
```

### Using the Tarball

```bash
# Extract the tarball
tar xzf enkodu-linux-x86_64.tar.gz

# Install the binary
sudo cp enkodu /usr/local/bin/
```

## Configuration

On first run, Enkodu will create a default configuration file at:
- `$XDG_CONFIG_HOME/enkodu/config.toml` (if `XDG_CONFIG_HOME` is set)
- or `~/.config/enkodu/config.toml` (fallback)

### Example Configuration

```toml
server_url = "https://enkodu.manwe.qzz.io"

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

Edit the configuration file to match your server URL and scan directories.

## Running

### Start the Companion

```bash
enkodu
```

The companion will appear as a tray icon in your desktop environment's notification area.

### CLI Commands

```bash
# Check status
enkodu status

# Trigger a batch scan
enkodu scan

# Trigger reconciliation
enkodu reconcile

# Pause NAS scanning
enkodu pause-nas

# Resume NAS scanning
enkodu resume-nas

# TCP ping test
enkodu tcpping <host:port>

# HTTP ping test
enkodu httping <url>
```

## Autostart

The companion supports autostart via XDG autostart. Use the "Start at Login" option in the tray menu to enable/disable autostart.

Alternatively, manually create a desktop file:

```bash
mkdir -p ~/.config/autostart
cat > ~/.config/autostart/enkodu.desktop <<EOF
[Desktop Entry]
Type=Application
Name=Enkodu
Exec=/usr/local/bin/enkodu
OnlyShowIn=XFCE;GNOME;KDE;
NoDisplay=false
Hidden=false
EOF
chmod +x ~/.config/autostart/enkodu.desktop
```

## Troubleshooting

### Tray Icon Not Appearing

Some desktop environments may require additional dependencies:

- **GNOME:** Ensure `gnome-shell` and `gnome-tweaks` are installed
- **KDE:** Ensure `plasma-workspace` is installed
- **Wayland:** The tray icon may not work on all Wayland compositors. Consider using X11 or a Wayland-compatible tray implementation.

### Notifications Not Working

Ensure `notify-send` is installed and your notification daemon is running:

```bash
# Check if notify-send is available
which notify-send

# Test notifications
notify-send "Test" "This is a test notification"
```

### Permissions Error

If you see permission errors when saving configuration or state files, ensure the directories exist:

```bash
mkdir -p ~/.config/enkodu
mkdir -p ~/.local/state/enkodu
```

## Known Limitations

- Wayland support may be limited depending on your desktop environment
- Some minimal Linux installations may not have all required dependencies
- The companion has only been tested on GNOME, KDE, and XFCE desktop environments

## Uninstallation

```bash
# Remove the binary
sudo rm /usr/local/bin/enkodu

# Remove configuration and state
rm -rf ~/.config/enkodu
rm -rf ~/.local/state/enkodu

# Remove autostart file
rm ~/.config/autostart/enkodu.desktop
```

# 🪟 Window Manager CLI

A Rust command-line tool for **capturing and restoring window layouts** on Linux. Supports both **X11** and **Wayland** windowing systems.

## Features

- **Automatic display server detection** — detects X11 or Wayland and uses the appropriate backend
- **Full window info capture** — positions (x, y), sizes (width, height), titles, application names, workspace number, and window state (normal, minimized, maximized, fullscreen)
- **JSON storage** — saves layouts to `~/.config/window-manager/windows.json`
- **Multiple Wayland compositors** — supports Sway, Hyprland, KDE Plasma, and GNOME
- **Graceful error handling** — informative error messages with suggestions

## Installation

### Prerequisites

- **Rust** (1.70+): [Install Rust](https://rustup.rs/)
- **X11 development libraries** (for X11 support):
  ```bash
  # Ubuntu/Debian
  sudo apt install libxcb1-dev libxcb-ewmh-dev libxcb-icccm4-dev libxcb-randr0-dev pkg-config

  # Fedora
  sudo dnf install libxcb-devel xcb-util-wm-devel xcb-util-devel

  # Arch Linux
  sudo pacman -S libxcb xcb-util-wm
  ```

- **Wayland development libraries** (for Wayland support):
  ```bash
  # Ubuntu/Debian
  sudo apt install libwayland-dev

  # Fedora
  sudo dnf install wayland-devel

  # Arch Linux
  sudo pacman -S wayland
  ```

### Build

```bash
git clone <repo-url>
cd window-manager
cargo build --release
```

The binary will be at `target/release/window-manager`.

### Install (optional)

```bash
cargo install --path .
```

## Usage

### Capture Current Layout

```bash
window-manager capture
```

Captures all open windows and saves their positions, sizes, titles, app names, workspace, and state to `~/.config/window-manager/windows.json`.

**Example output:**
```
🔍 Capturing window layout...
  Display server: X11
  Using X11/XCB backend
  Captured 5 window(s)

✅ Saved to: /home/user/.config/window-manager/windows.json
```

### List Saved Layout

```bash
window-manager list
```

Displays the saved window layout in a human-readable format.

**Example output:**
```
📋 Saved Window Layout

  Captured at: 2026-05-23 10:30:00 UTC
  Display server:  X11
  Hostname:  my-laptop
  Total windows:  5

  Windows:

  1. 🪟 Terminal — /home/user
     App:       Alacritty
     ID:        0x02c00003
     Position:  (100, 50)
     Size:      800 × 600
     Workspace: 0
     State:     Normal

  2. 🪟 Mozilla Firefox
     App:       firefox
     ID:        0x03400006
     Position:  (900, 50)
     Size:      1020 × 800
     Workspace: 1
     State:     Maximized
```

### Restore Layout

```bash
window-manager restore
```

Reads the saved layout and moves/resizes windows to their captured positions.

**Notes:**
- On **X11**, restore uses `wmctrl` or `xdotool` (install one of them):
  ```bash
  sudo apt install wmctrl   # Recommended
  sudo apt install xdotool  # Alternative
  ```
- Windows are matched by application name (WM_CLASS)
- If the display server changed since capture, a warning is shown

## How It Works

### X11 Backend

Uses the **XCB** library to communicate directly with the X11 server:

1. Connects to the X server via XCB
2. Reads `_NET_CLIENT_LIST` from the root window to get all managed windows
3. For each window, queries:
   - `_NET_WM_NAME` / `WM_NAME` for the title
   - `WM_CLASS` for the application name
   - `GetGeometry` + `TranslateCoordinates` for absolute position
   - `_NET_WM_DESKTOP` for workspace number
   - `_NET_WM_STATE` for window state (hidden, maximized, fullscreen)
4. Restore uses `wmctrl` (EWMH-compliant) or `xdotool` for positioning

### Wayland Backend

Wayland's security model prevents direct cross-client window inspection. Instead, we use **compositor-specific IPC**:

| Compositor | Capture Tool | Restore Tool |
|-----------|-------------|-------------|
| **Sway** | `swaymsg -t get_tree` | `swaymsg` IPC commands |
| **Hyprland** | `hyprctl clients -j` | `hyprctl dispatch` |
| **KDE Plasma** | `kdotool` | `kdotool` |
| **GNOME** | `gdbus` (Shell.Eval) | Limited support |

## JSON Format

The saved layout file (`~/.config/window-manager/windows.json`) looks like:

```json
{
  "captured_at": "2026-05-23T10:30:00Z",
  "display_server": "x11",
  "hostname": "my-laptop",
  "windows": [
    {
      "window_id": "0x02c00003",
      "title": "Terminal",
      "app_name": "Alacritty",
      "x": 100,
      "y": 50,
      "width": 800,
      "height": 600,
      "workspace": 0,
      "state": "normal"
    }
  ]
}
```

## Project Structure

```
window-manager/
├── Cargo.toml          # Dependencies and project metadata
├── README.md           # This file
└── src/
    ├── main.rs         # CLI entry point, command handlers
    ├── types.rs        # Core data types (WindowInfo, WindowLayout, etc.)
    ├── storage.rs      # JSON file I/O (~/.config/window-manager/)
    ├── detect.rs       # Display server detection (X11 vs Wayland)
    ├── x11.rs          # X11 capture/restore using XCB
    └── wayland.rs      # Wayland capture/restore (Sway, Hyprland, KDE, GNOME)
```

## Autostart on Login

To automatically restore your window layout on login, add the restore command to your session startup:

### X11 (e.g., i3, Openbox)
```bash
# In ~/.config/i3/config or ~/.xinitrc:
exec --no-startup-id window-manager restore
```

### Sway
```bash
# In ~/.config/sway/config:
exec window-manager restore
```

### Systemd user service
```ini
# ~/.config/systemd/user/window-restore.service
[Unit]
Description=Restore window layout
After=graphical-session.target

[Service]
Type=oneshot
ExecStartPre=/bin/sleep 5
ExecStart=%h/.cargo/bin/window-manager restore

[Install]
WantedBy=graphical-session.target
```

```bash
systemctl --user enable window-restore.service
```

## License

MIT

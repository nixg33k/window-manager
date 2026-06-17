//! Wayland window capture and restore implementation.
//!
//! Wayland's security model is fundamentally different from X11 — client applications
//! cannot enumerate or manipulate other applications' windows directly. Instead, we
//! rely on compositor-specific tools and protocols:
//!
//! - **Capture**: Uses `kdotool` (KDE), `swaymsg` (Sway/i3-like), or `hyprctl` (Hyprland)
//!   to query the compositor for window information.
//! - **Restore**: Uses the same tools to reposition windows.
//!
//! This approach is necessary because Wayland deliberately prevents the kind of
//! cross-client window inspection that X11 allows via EWMH.

use anyhow::{Context, Result};
use std::process::Command;

use crate::types::{WindowInfo, WindowState};

/// Supported Wayland compositors for window management.
#[derive(Debug)]
enum WaylandCompositor {
    Sway,
    Hyprland,
    KdePlasma,
    GnomeMutter,
    Unknown,
}

/// Detect which Wayland compositor is running.
fn detect_compositor() -> WaylandCompositor {
    // Check for Sway
    if std::env::var("SWAYSOCK").is_ok() || is_process_running("sway") {
        return WaylandCompositor::Sway;
    }

    // Check for Hyprland
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() || is_process_running("Hyprland") {
        return WaylandCompositor::Hyprland;
    }

    // Check for KDE Plasma (Wayland session)
    if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
        let desktop_lower = desktop.to_lowercase();
        if desktop_lower.contains("kde") || desktop_lower.contains("plasma") {
            return WaylandCompositor::KdePlasma;
        }
        if desktop_lower.contains("gnome") {
            return WaylandCompositor::GnomeMutter;
        }
    }

    WaylandCompositor::Unknown
}

/// Check if a process with the given name is running.
fn is_process_running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ─── Sway/i3-compatible capture ──────────────────────────────────────────────

/// Capture windows on Sway using `swaymsg -t get_tree`.
///
/// Sway's IPC returns a tree of containers. We traverse it to find all
/// application windows (nodes with an `app_id` or `window_properties`).
fn capture_sway() -> Result<Vec<WindowInfo>> {
    let output = Command::new("swaymsg")
        .args(["-t", "get_tree", "--raw"])
        .output()
        .context("Failed to run 'swaymsg'. Is Sway running?")?;

    if !output.status.success() {
        anyhow::bail!(
            "swaymsg failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Failed to parse swaymsg output")?;

    let mut windows = Vec::new();
    collect_sway_windows(&json, &mut windows, 0);
    Ok(windows)
}

/// Recursively traverse the Sway container tree and collect window info.
fn collect_sway_windows(node: &serde_json::Value, windows: &mut Vec<WindowInfo>, workspace: i32) {
    // Determine current workspace number
    let current_workspace = if node.get("type").and_then(|t| t.as_str()) == Some("workspace") {
        node.get("num")
            .and_then(|n| n.as_i64())
            .map(|n| n as i32)
            .unwrap_or(workspace)
    } else {
        workspace
    };

    // Check if this node is an application window
    let is_window = node.get("pid").is_some()
        && (node.get("app_id").is_some()
            || node.get("window_properties").is_some());

    if is_window {
        let rect = node.get("rect").unwrap_or(&serde_json::Value::Null);
        let x = rect.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = rect.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let width = rect.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let height = rect.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        let title = node
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("(untitled)")
            .to_string();

        let app_name = node
            .get("app_id")
            .and_then(|a| a.as_str())
            .or_else(|| {
                node.get("window_properties")
                    .and_then(|wp| wp.get("class"))
                    .and_then(|c| c.as_str())
            })
            .unwrap_or("(unknown)")
            .to_string();

        let window_id = node
            .get("id")
            .and_then(|id| id.as_u64())
            .map(|id| format!("{}", id))
            .unwrap_or_else(|| "0".to_string());

        // Determine state
        let state = if node.get("fullscreen_mode").and_then(|f| f.as_u64()) == Some(1) {
            WindowState::Fullscreen
        } else if node.get("visible").and_then(|v| v.as_bool()) == Some(false) {
            // In Sway, non-visible windows on inactive workspaces aren't quite "minimized"
            // but we treat hidden windows as minimized for our purposes
            WindowState::Minimized
        } else {
            WindowState::Normal
        };

        windows.push(WindowInfo {
            window_id,
            title,
            app_name,
            x,
            y,
            width,
            height,
            workspace: current_workspace,
            state,
        });
    }

    // Recurse into child nodes and floating nodes
    if let Some(nodes) = node.get("nodes").and_then(|n| n.as_array()) {
        for child in nodes {
            collect_sway_windows(child, windows, current_workspace);
        }
    }
    if let Some(floating) = node.get("floating_nodes").and_then(|n| n.as_array()) {
        for child in floating {
            collect_sway_windows(child, windows, current_workspace);
        }
    }
}

// ─── Hyprland capture ────────────────────────────────────────────────────────

/// Capture windows on Hyprland using `hyprctl clients -j`.
fn capture_hyprland() -> Result<Vec<WindowInfo>> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .context("Failed to run 'hyprctl'. Is Hyprland running?")?;

    if !output.status.success() {
        anyhow::bail!(
            "hyprctl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let clients: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).context("Failed to parse hyprctl output")?;

    let mut windows = Vec::new();

    for client in &clients {
        let at = client.get("at").unwrap_or(&serde_json::Value::Null);
        let size = client.get("size").unwrap_or(&serde_json::Value::Null);

        let x = at.get(0).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = at.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let width = size.get(0).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let height = size.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        let title = client
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("(untitled)")
            .to_string();

        let app_name = client
            .get("class")
            .and_then(|c| c.as_str())
            .unwrap_or("(unknown)")
            .to_string();

        let window_id = client
            .get("address")
            .and_then(|a| a.as_str())
            .unwrap_or("0x0")
            .to_string();

        let workspace = client
            .get("workspace")
            .and_then(|w| w.get("id"))
            .and_then(|id| id.as_i64())
            .unwrap_or(-1) as i32;

        let fullscreen = client
            .get("fullscreen")
            .and_then(|f| f.as_bool())
            .unwrap_or(false);

        let state = if fullscreen {
            WindowState::Fullscreen
        } else {
            WindowState::Normal
        };

        windows.push(WindowInfo {
            window_id,
            title,
            app_name,
            x,
            y,
            width,
            height,
            workspace,
            state,
        });
    }

    Ok(windows)
}

// ─── KDE Plasma Wayland capture ──────────────────────────────────────────────

/// Capture windows on KDE Plasma Wayland using `kdotool`.
fn capture_kde() -> Result<Vec<WindowInfo>> {
    // kdotool search --name "" returns all windows
    let output = Command::new("kdotool")
        .args(["search", "--name", ""])
        .output()
        .context(
            "Failed to run 'kdotool'. Install it with:\n  \
             pip install kdotool\n  \
             or check https://github.com/jinliu/kdotool",
        )?;

    let wid_str = String::from_utf8_lossy(&output.stdout);
    let wids: Vec<&str> = wid_str.lines().filter(|l| !l.is_empty()).collect();

    let mut windows = Vec::new();

    for wid in wids {
        // Get window geometry
        let geom_output = Command::new("kdotool")
            .args(["getwindowgeometry", "--shell", wid])
            .output();

        let (x, y, width, height) = if let Ok(ref out) = geom_output {
            parse_kdotool_geometry(&String::from_utf8_lossy(&out.stdout))
        } else {
            (0, 0, 0, 0)
        };

        // Get window name
        let name_output = Command::new("kdotool")
            .args(["getwindowname", wid])
            .output();

        let title = name_output
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "(untitled)".to_string());

        windows.push(WindowInfo {
            window_id: wid.to_string(),
            title,
            app_name: "(kde-window)".to_string(),
            x,
            y,
            width,
            height,
            workspace: 0,
            state: WindowState::Normal,
        });
    }

    Ok(windows)
}

/// Parse kdotool geometry output (shell-variable format).
fn parse_kdotool_geometry(output: &str) -> (i32, i32, u32, u32) {
    let mut x = 0i32;
    let mut y = 0i32;
    let mut w = 0u32;
    let mut h = 0u32;

    for line in output.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("X=") {
            x = val.parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("Y=") {
            y = val.parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("WIDTH=") {
            w = val.parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("HEIGHT=") {
            h = val.parse().unwrap_or(0);
        }
    }

    (x, y, w, h)
}

// ─── GNOME / generic Wayland ─────────────────────────────────────────────────

/// Attempt a generic Wayland capture using `wlr-randr` or DBus.
///
/// For GNOME, we try to use `gdbus` to communicate with the window manager.
/// This is a best-effort approach since GNOME doesn't expose a stable window
/// enumeration API.
fn capture_gnome() -> Result<Vec<WindowInfo>> {
    // Try using `gdbus` to call GNOME Shell's Eval method to list windows
    let js_code = r#"
        global.get_window_actors().map(a => {
            let w = a.get_meta_window();
            return JSON.stringify({
                title: w.get_title(),
                wm_class: w.get_wm_class(),
                x: w.get_frame_rect().x,
                y: w.get_frame_rect().y,
                width: w.get_frame_rect().width,
                height: w.get_frame_rect().height,
                workspace: w.get_workspace().index(),
                minimized: w.minimized,
                maximized: w.get_maximized() > 0,
                fullscreen: w.is_fullscreen()
            });
        }).join('\n')
    "#;

    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.gnome.Shell",
            "--object-path",
            "/org/gnome/Shell",
            "--method",
            "org.gnome.Shell.Eval",
            js_code,
        ])
        .output()
        .context(
            "Failed to query GNOME Shell via gdbus.\n\
             GNOME Wayland has limited support for window enumeration.\n\
             Consider using an X11 session or a compositor with better IPC (Sway, Hyprland).",
        )?;

    if !output.status.success() {
        anyhow::bail!(
            "GNOME Shell Eval failed. This may require enabling 'unsafe-mode' in GNOME Shell.\n\
             Error: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Parse the gdbus output — it returns (true, 'json_string_here')
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_start = stdout.find('\'').unwrap_or(0) + 1;
    let json_end = stdout.rfind('\'').unwrap_or(stdout.len());
    let json_str = &stdout[json_start..json_end];

    let mut windows = Vec::new();
    let mut id_counter = 0u64;

    for line in json_str.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let state = if val.get("fullscreen").and_then(|f| f.as_bool()) == Some(true) {
                WindowState::Fullscreen
            } else if val.get("minimized").and_then(|m| m.as_bool()) == Some(true) {
                WindowState::Minimized
            } else if val.get("maximized").and_then(|m| m.as_bool()) == Some(true) {
                WindowState::Maximized
            } else {
                WindowState::Normal
            };

            windows.push(WindowInfo {
                window_id: format!("gnome-{}", id_counter),
                title: val
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("(untitled)")
                    .to_string(),
                app_name: val
                    .get("wm_class")
                    .and_then(|c| c.as_str())
                    .unwrap_or("(unknown)")
                    .to_string(),
                x: val.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                y: val.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                width: val.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                height: val.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                workspace: val.get("workspace").and_then(|w| w.as_i64()).unwrap_or(0) as i32,
                state,
            });

            id_counter += 1;
        }
    }

    Ok(windows)
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Capture all windows on the current Wayland session.
///
/// Automatically detects the running compositor and uses the appropriate
/// tool/protocol for window enumeration.
pub fn capture_windows() -> Result<Vec<WindowInfo>> {
    let compositor = detect_compositor();
    println!("  Detected Wayland compositor: {:?}", compositor);

    match compositor {
        WaylandCompositor::Sway => capture_sway(),
        WaylandCompositor::Hyprland => capture_hyprland(),
        WaylandCompositor::KdePlasma => capture_kde(),
        WaylandCompositor::GnomeMutter => capture_gnome(),
        WaylandCompositor::Unknown => {
            // Try each compositor tool in order
            if let Ok(windows) = capture_sway() {
                return Ok(windows);
            }
            if let Ok(windows) = capture_hyprland() {
                return Ok(windows);
            }
            if let Ok(windows) = capture_kde() {
                return Ok(windows);
            }
            capture_gnome().context(
                "Could not detect a supported Wayland compositor.\n\
                 Supported compositors: Sway, Hyprland, KDE Plasma, GNOME.\n\n\
                 For best results, please use one of these compositors or\n\
                 switch to an X11 session.",
            )
        }
    }
}

/// Restore windows on the current Wayland session.
///
/// Uses compositor-specific tools to move and resize windows.
pub fn restore_windows(windows: &[WindowInfo]) -> Result<()> {
    let compositor = detect_compositor();
    println!("  Detected Wayland compositor: {:?}", compositor);

    match compositor {
        WaylandCompositor::Sway => restore_sway(windows),
        WaylandCompositor::Hyprland => restore_hyprland(windows),
        WaylandCompositor::KdePlasma => restore_kde(windows),
        WaylandCompositor::GnomeMutter | WaylandCompositor::Unknown => {
            anyhow::bail!(
                "Window restore is not fully supported on this Wayland compositor.\n\
                 Supported compositors for restore: Sway, Hyprland, KDE Plasma."
            )
        }
    }
}

/// Restore windows on Sway using `swaymsg`.
fn restore_sway(windows: &[WindowInfo]) -> Result<()> {
    for win in windows {
        println!("  Restoring: {} ({})", win.title, win.app_name);

        // Move the container to the target workspace
        if win.workspace >= 0 {
            let criteria = format!("[con_id={}]", win.window_id);
            let _ = Command::new("swaymsg")
                .args([
                    &criteria,
                    "move",
                    "container",
                    "to",
                    "workspace",
                    "number",
                    &win.workspace.to_string(),
                ])
                .output();
        }

        // Set floating and move/resize
        let criteria = format!("[con_id={}]", win.window_id);
        let resize_cmd = format!(
            "floating enable, move position {} {}, resize set {} {}",
            win.x, win.y, win.width, win.height
        );
        let _ = Command::new("swaymsg")
            .args([&criteria, &resize_cmd])
            .output();

        // Handle fullscreen
        if win.state == WindowState::Fullscreen {
            let _ = Command::new("swaymsg")
                .args([&criteria, "fullscreen", "enable"])
                .output();
        }
    }
    Ok(())
}

/// Restore windows on Hyprland using `hyprctl dispatch`.
fn restore_hyprland(windows: &[WindowInfo]) -> Result<()> {
    for win in windows {
        println!("  Restoring: {} ({})", win.title, win.app_name);

        // Move to workspace
        if win.workspace >= 0 {
            let _ = Command::new("hyprctl")
                .args([
                    "dispatch",
                    "movetoworkspacesilent",
                    &format!("{},address:{}", win.workspace, win.window_id),
                ])
                .output();
        }

        // Move and resize
        let _ = Command::new("hyprctl")
            .args([
                "dispatch",
                "movewindowpixel",
                &format!("exact {} {},address:{}", win.x, win.y, win.window_id),
            ])
            .output();

        let _ = Command::new("hyprctl")
            .args([
                "dispatch",
                "resizewindowpixel",
                &format!("exact {} {},address:{}", win.width, win.height, win.window_id),
            ])
            .output();

        // Handle fullscreen
        if win.state == WindowState::Fullscreen {
            let _ = Command::new("hyprctl")
                .args(["dispatch", "fullscreen", "0"])
                .output();
        }
    }
    Ok(())
}

/// Restore windows on KDE Plasma using `kdotool`.
fn restore_kde(windows: &[WindowInfo]) -> Result<()> {
    for win in windows {
        println!("  Restoring: {} ({})", win.title, win.app_name);

        let _ = Command::new("kdotool")
            .args([
                "windowmove",
                &win.window_id,
                &win.x.to_string(),
                &win.y.to_string(),
            ])
            .output();

        let _ = Command::new("kdotool")
            .args([
                "windowsize",
                &win.window_id,
                &win.width.to_string(),
                &win.height.to_string(),
            ])
            .output();
    }
    Ok(())
}

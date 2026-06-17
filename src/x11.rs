//! X11 window capture and restore implementation using the `xcb` crate.
//!
//! This module communicates directly with the X11 server via the XCB protocol
//! to enumerate windows, read their properties (title, class, geometry, state),
//! and reposition/resize them during restore.

use anyhow::{Context, Result};
use std::process::Command;
use xcb::Xid;

use crate::types::{WindowInfo, WindowState};

// ─── EWMH atom names used for querying window properties ─────────────────────

const NET_CLIENT_LIST: &str = "_NET_CLIENT_LIST";
const NET_WM_NAME: &str = "_NET_WM_NAME";
const NET_WM_DESKTOP: &str = "_NET_WM_DESKTOP";
const NET_WM_STATE: &str = "_NET_WM_STATE";
const NET_WM_STATE_HIDDEN: &str = "_NET_WM_STATE_HIDDEN";
const NET_WM_STATE_MAXIMIZED_HORZ: &str = "_NET_WM_STATE_MAXIMIZED_HORZ";
const NET_WM_STATE_MAXIMIZED_VERT: &str = "_NET_WM_STATE_MAXIMIZED_VERT";
const NET_WM_STATE_FULLSCREEN: &str = "_NET_WM_STATE_FULLSCREEN";
const WM_NAME: &str = "WM_NAME";
const WM_CLASS: &str = "WM_CLASS";
const UTF8_STRING: &str = "UTF8_STRING";
const STRING: &str = "STRING";

/// Helper: intern an atom name and return its ID.
fn intern_atom(conn: &xcb::Connection, name: &str) -> Result<xcb::x::Atom> {
    let cookie = conn.send_request(&xcb::x::InternAtom {
        only_if_exists: true,
        name: name.as_bytes(),
    });
    let reply = conn.wait_for_reply(cookie)
        .with_context(|| format!("Failed to intern atom '{}'", name))?;
    Ok(reply.atom())
}

/// Helper: get a property from a window as raw bytes.
fn get_property_bytes(
    conn: &xcb::Connection,
    window: xcb::x::Window,
    property: xcb::x::Atom,
    prop_type: xcb::x::Atom,
    length: u32,
) -> Result<Vec<u8>> {
    let cookie = conn.send_request(&xcb::x::GetProperty {
        delete: false,
        window,
        property,
        r#type: prop_type,
        long_offset: 0,
        long_length: length,
    });
    let reply = conn.wait_for_reply(cookie)
        .context("Failed to get property")?;
    Ok(reply.value::<u8>().to_vec())
}

/// Helper: get a property as a list of u32 values.
fn get_property_u32(
    conn: &xcb::Connection,
    window: xcb::x::Window,
    property: xcb::x::Atom,
    length: u32,
) -> Result<Vec<u32>> {
    let cookie = conn.send_request(&xcb::x::GetProperty {
        delete: false,
        window,
        property,
        r#type: xcb::x::ATOM_ATOM,
        long_offset: 0,
        long_length: length,
    });
    let reply = conn.wait_for_reply(cookie)
        .context("Failed to get property (u32)")?;
    Ok(reply.value::<u32>().to_vec())
}

/// Helper: get a cardinal (single u32) property.
fn get_cardinal(
    conn: &xcb::Connection,
    window: xcb::x::Window,
    property: xcb::x::Atom,
) -> Result<Option<u32>> {
    let cookie = conn.send_request(&xcb::x::GetProperty {
        delete: false,
        window,
        property,
        r#type: xcb::x::ATOM_CARDINAL,
        long_offset: 0,
        long_length: 1,
    });
    let reply = conn.wait_for_reply(cookie)
        .context("Failed to get cardinal property")?;
    let values = reply.value::<u32>();
    Ok(values.first().copied())
}

/// Get the window title, trying _NET_WM_NAME (UTF-8) first, then WM_NAME.
fn get_window_title(
    conn: &xcb::Connection,
    window: xcb::x::Window,
    net_wm_name: xcb::x::Atom,
    wm_name: xcb::x::Atom,
    utf8_string: xcb::x::Atom,
    string_atom: xcb::x::Atom,
) -> String {
    // Try _NET_WM_NAME (UTF-8 encoded)
    if let Ok(bytes) = get_property_bytes(conn, window, net_wm_name, utf8_string, 256) {
        if !bytes.is_empty() {
            return String::from_utf8_lossy(&bytes).to_string();
        }
    }

    // Fallback to WM_NAME (Latin-1 encoded)
    if let Ok(bytes) = get_property_bytes(conn, window, wm_name, string_atom, 256) {
        if !bytes.is_empty() {
            return String::from_utf8_lossy(&bytes).to_string();
        }
    }

    "(untitled)".to_string()
}

/// Get the application name from WM_CLASS property.
/// WM_CLASS contains two null-terminated strings: instance name and class name.
/// We return the class name (second one) as it's more user-friendly.
fn get_app_name(
    conn: &xcb::Connection,
    window: xcb::x::Window,
    wm_class: xcb::x::Atom,
    string_atom: xcb::x::Atom,
) -> String {
    if let Ok(bytes) = get_property_bytes(conn, window, wm_class, string_atom, 256) {
        if !bytes.is_empty() {
            // WM_CLASS is "instance\0class\0"
            let parts: Vec<&str> = std::str::from_utf8(&bytes)
                .unwrap_or("")
                .split('\0')
                .filter(|s| !s.is_empty())
                .collect();

            // Return the class name (second part) or instance name (first part)
            if parts.len() >= 2 {
                return parts[1].to_string();
            } else if !parts.is_empty() {
                return parts[0].to_string();
            }
        }
    }
    "(unknown)".to_string()
}

/// Determine the window state from _NET_WM_STATE atoms.
fn get_window_state(
    conn: &xcb::Connection,
    window: xcb::x::Window,
    net_wm_state: xcb::x::Atom,
    hidden_atom: xcb::x::Atom,
    max_horz_atom: xcb::x::Atom,
    max_vert_atom: xcb::x::Atom,
    fullscreen_atom: xcb::x::Atom,
) -> WindowState {
    if let Ok(atoms) = get_property_u32(conn, window, net_wm_state, 32) {
        let has_hidden = atoms.contains(&(hidden_atom.resource_id()));
        let has_max_h = atoms.contains(&(max_horz_atom.resource_id()));
        let has_max_v = atoms.contains(&(max_vert_atom.resource_id()));
        let has_fullscreen = atoms.contains(&(fullscreen_atom.resource_id()));

        if has_fullscreen {
            return WindowState::Fullscreen;
        }
        if has_hidden {
            return WindowState::Minimized;
        }
        if has_max_h && has_max_v {
            return WindowState::Maximized;
        }
    }
    WindowState::Normal
}

/// Capture all visible windows on X11.
///
/// Connects to the X server, queries _NET_CLIENT_LIST for all managed windows,
/// then reads each window's geometry, title, class, desktop, and state.
pub fn capture_windows() -> Result<Vec<WindowInfo>> {
    // Connect to the X server
    let (conn, screen_num) = xcb::Connection::connect(None)
        .context("Failed to connect to X11 server. Is DISPLAY set?")?;

    let setup = conn.get_setup();
    let screen = setup
        .roots()
        .nth(screen_num as usize)
        .context("Failed to get the default screen")?;

    let root = screen.root();

    // Intern all atoms we'll need
    let net_client_list = intern_atom(&conn, NET_CLIENT_LIST)?;
    let net_wm_name = intern_atom(&conn, NET_WM_NAME)?;
    let net_wm_desktop = intern_atom(&conn, NET_WM_DESKTOP)?;
    let net_wm_state = intern_atom(&conn, NET_WM_STATE)?;
    let hidden_atom = intern_atom(&conn, NET_WM_STATE_HIDDEN)?;
    let max_horz_atom = intern_atom(&conn, NET_WM_STATE_MAXIMIZED_HORZ)?;
    let max_vert_atom = intern_atom(&conn, NET_WM_STATE_MAXIMIZED_VERT)?;
    let fullscreen_atom = intern_atom(&conn, NET_WM_STATE_FULLSCREEN)?;
    let wm_name = intern_atom(&conn, WM_NAME)?;
    let wm_class = intern_atom(&conn, WM_CLASS)?;
    let utf8_string = intern_atom(&conn, UTF8_STRING)?;
    let string_atom = intern_atom(&conn, STRING)?;

    // Get the list of client windows from _NET_CLIENT_LIST on the root window
    let client_list_cookie = conn.send_request(&xcb::x::GetProperty {
        delete: false,
        window: root,
        property: net_client_list,
        r#type: xcb::x::ATOM_WINDOW,
        long_offset: 0,
        long_length: 1024,
    });
    let client_list_reply = conn
        .wait_for_reply(client_list_cookie)
        .context("Failed to get _NET_CLIENT_LIST from root window")?;

    let window_ids: Vec<xcb::x::Window> = client_list_reply.value::<xcb::x::Window>().to_vec();

    let mut windows = Vec::new();

    for &win in &window_ids {
        // Get window geometry
        let geom_cookie = conn.send_request(&xcb::x::GetGeometry {
            drawable: xcb::x::Drawable::Window(win),
        });
        let geom = match conn.wait_for_reply(geom_cookie) {
            Ok(g) => g,
            Err(_) => continue, // Window may have been destroyed
        };

        // Translate coordinates to root window (absolute screen coordinates)
        let translate_cookie = conn.send_request(&xcb::x::TranslateCoordinates {
            src_window: win,
            dst_window: root,
            src_x: 0,
            src_y: 0,
        });
        let (abs_x, abs_y) = match conn.wait_for_reply(translate_cookie) {
            Ok(t) => (t.dst_x() as i32, t.dst_y() as i32),
            Err(_) => (geom.x() as i32, geom.y() as i32),
        };

        // Get window title
        let title = get_window_title(&conn, win, net_wm_name, wm_name, utf8_string, string_atom);

        // Get application name (WM_CLASS)
        let app_name = get_app_name(&conn, win, wm_class, string_atom);

        // Get workspace/desktop number
        let workspace = get_cardinal(&conn, win, net_wm_desktop)?
            .map(|v| v as i32)
            .unwrap_or(-1);

        // Get window state
        let state = get_window_state(
            &conn,
            win,
            net_wm_state,
            hidden_atom,
            max_horz_atom,
            max_vert_atom,
            fullscreen_atom,
        );

        windows.push(WindowInfo {
            window_id: format!("0x{:08x}", win.resource_id()),
            title,
            app_name,
            x: abs_x,
            y: abs_y,
            width: geom.width() as u32,
            height: geom.height() as u32,
            workspace,
            state,
        });
    }

    Ok(windows)
}

/// Restore windows to their saved positions on X11.
///
/// For each saved window, we try to find a matching window by app_name (WM_CLASS),
/// then use `wmctrl` or direct XCB calls to reposition and resize it.
pub fn restore_windows(windows: &[WindowInfo]) -> Result<()> {
    // Check if wmctrl is available — it's the most reliable way to move/resize
    // windows on X11 since it handles EWMH hints properly.
    let has_wmctrl = which::which("wmctrl").is_ok();

    if !has_wmctrl {
        // Fallback: try using xdotool
        let has_xdotool = which::which("xdotool").is_ok();
        if !has_xdotool {
            anyhow::bail!(
                "Neither 'wmctrl' nor 'xdotool' found.\n\
                 Please install one of them:\n\
                 \n\
                 Ubuntu/Debian: sudo apt install wmctrl\n\
                 Fedora:        sudo dnf install wmctrl\n\
                 Arch:          sudo pacman -S wmctrl"
            );
        }
        return restore_with_xdotool(windows);
    }

    restore_with_wmctrl(windows)
}

/// Restore windows using `wmctrl`.
///
/// wmctrl provides the `-r` flag to select a window by name/class and `-e` to
/// set its geometry: `-e gravity,x,y,width,height`.
fn restore_with_wmctrl(windows: &[WindowInfo]) -> Result<()> {
    for win in windows {
        println!("  Restoring: {} ({})", win.title, win.app_name);

        // First, move the window to the correct workspace
        if win.workspace >= 0 {
            let _ = Command::new("wmctrl")
                .args(["-r", &win.app_name, "-t", &win.workspace.to_string()])
                .output();
        }

        // Handle window state before moving
        match win.state {
            WindowState::Minimized => {
                // wmctrl can't easily minimize; skip geometry for minimized windows
                println!("    (skipping geometry for minimized window)");
                continue;
            }
            WindowState::Maximized => {
                // Remove maximized state first, then set geometry, then re-maximize
                let _ = Command::new("wmctrl")
                    .args([
                        "-r",
                        &win.app_name,
                        "-b",
                        "remove,maximized_vert,maximized_horz",
                    ])
                    .output();
            }
            WindowState::Fullscreen => {
                let _ = Command::new("wmctrl")
                    .args(["-r", &win.app_name, "-b", "remove,fullscreen"])
                    .output();
            }
            WindowState::Normal => {}
        }

        // Set the window geometry: -e gravity,x,y,w,h (gravity 0 = use default)
        let geometry = format!("0,{},{},{},{}", win.x, win.y, win.width, win.height);
        let status = Command::new("wmctrl")
            .args(["-r", &win.app_name, "-e", &geometry])
            .output()
            .with_context(|| format!("Failed to run wmctrl for '{}'", win.app_name))?;

        if !status.status.success() {
            eprintln!(
                "    Warning: wmctrl failed for '{}': {}",
                win.app_name,
                String::from_utf8_lossy(&status.stderr)
            );
        }

        // Re-apply maximized or fullscreen state
        match win.state {
            WindowState::Maximized => {
                let _ = Command::new("wmctrl")
                    .args([
                        "-r",
                        &win.app_name,
                        "-b",
                        "add,maximized_vert,maximized_horz",
                    ])
                    .output();
            }
            WindowState::Fullscreen => {
                let _ = Command::new("wmctrl")
                    .args(["-r", &win.app_name, "-b", "add,fullscreen"])
                    .output();
            }
            _ => {}
        }
    }

    Ok(())
}

/// Restore windows using `xdotool` as a fallback.
fn restore_with_xdotool(windows: &[WindowInfo]) -> Result<()> {
    for win in windows {
        println!("  Restoring: {} ({})", win.title, win.app_name);

        // Search for the window by class name
        let search_output = Command::new("xdotool")
            .args(["search", "--class", &win.app_name])
            .output()
            .context("Failed to run xdotool search")?;

        let wid_str = String::from_utf8_lossy(&search_output.stdout);
        let wid = wid_str.lines().next().unwrap_or("").trim();

        if wid.is_empty() {
            eprintln!("    Warning: No window found for class '{}'", win.app_name);
            continue;
        }

        // Move and resize the window
        let _ = Command::new("xdotool")
            .args([
                "windowmove",
                "--sync",
                wid,
                &win.x.to_string(),
                &win.y.to_string(),
            ])
            .output();

        let _ = Command::new("xdotool")
            .args([
                "windowsize",
                "--sync",
                wid,
                &win.width.to_string(),
                &win.height.to_string(),
            ])
            .output();

        // Move to workspace
        if win.workspace >= 0 {
            let _ = Command::new("xdotool")
                .args([
                    "set_desktop_for_window",
                    wid,
                    &win.workspace.to_string(),
                ])
                .output();
        }
    }

    Ok(())
}

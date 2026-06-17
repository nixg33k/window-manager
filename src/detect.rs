//! Display server detection module.
//!
//! Determines whether the current session is running X11 or Wayland
//! by inspecting environment variables and session metadata.

use crate::types::DisplayServer;

/// Detect the active display server (X11 or Wayland).
///
/// Detection strategy:
/// 1. Check `XDG_SESSION_TYPE` environment variable (most reliable on modern distros)
/// 2. Check `WAYLAND_DISPLAY` — if set, Wayland is in use
/// 3. Check `DISPLAY` — if set, X11 is in use
/// 4. Fall back to Unknown
pub fn detect_display_server() -> DisplayServer {
    // Method 1: XDG_SESSION_TYPE is the standard way on systemd-based systems
    if let Ok(session_type) = std::env::var("XDG_SESSION_TYPE") {
        match session_type.to_lowercase().as_str() {
            "wayland" => return DisplayServer::Wayland,
            "x11" => return DisplayServer::X11,
            _ => {} // Continue to other checks
        }
    }

    // Method 2: WAYLAND_DISPLAY is set when a Wayland compositor is running
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        return DisplayServer::Wayland;
    }

    // Method 3: DISPLAY is set when an X11 server is running
    if std::env::var("DISPLAY").is_ok() {
        return DisplayServer::X11;
    }

    // Could not determine the display server
    DisplayServer::Unknown
}

/// Get the hostname of the current machine.
pub fn get_hostname() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_returns_valid_enum() {
        let server = detect_display_server();
        // Should return one of the valid enum variants without panicking
        match server {
            DisplayServer::X11 | DisplayServer::Wayland | DisplayServer::Unknown => {}
        }
    }
}

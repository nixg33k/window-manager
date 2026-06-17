//! Core data types for window layout capture and restoration.
//!
//! This module defines the serializable structures used to represent
//! window information, layout snapshots, and window state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents the state of a window (normal, minimized, maximized, fullscreen).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
    Fullscreen,
}

impl fmt::Display for WindowState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowState::Normal => write!(f, "Normal"),
            WindowState::Minimized => write!(f, "Minimized"),
            WindowState::Maximized => write!(f, "Maximized"),
            WindowState::Fullscreen => write!(f, "Fullscreen"),
        }
    }
}

/// The windowing system backend detected on the current session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DisplayServer {
    X11,
    Wayland,
    Unknown,
}

impl fmt::Display for DisplayServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisplayServer::X11 => write!(f, "X11"),
            DisplayServer::Wayland => write!(f, "Wayland"),
            DisplayServer::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Information about a single window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    /// Window ID (X11 window ID or Wayland-specific identifier)
    pub window_id: String,

    /// Window title / name
    pub title: String,

    /// Application name (WM_CLASS on X11, app_id on Wayland)
    pub app_name: String,

    /// X position of the window (pixels from left edge of screen)
    pub x: i32,

    /// Y position of the window (pixels from top edge of screen)
    pub y: i32,

    /// Width of the window in pixels
    pub width: u32,

    /// Height of the window in pixels
    pub height: u32,

    /// Workspace/desktop number the window is on (0-indexed)
    pub workspace: i32,

    /// Current state of the window
    pub state: WindowState,
}

impl fmt::Display for WindowInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] \"{}\" ({}) - pos: ({}, {}), size: {}x{}, workspace: {}, state: {}",
            self.window_id,
            self.title,
            self.app_name,
            self.x,
            self.y,
            self.width,
            self.height,
            self.workspace,
            self.state
        )
    }
}

/// A snapshot of all window layouts at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowLayout {
    /// Timestamp when this layout was captured
    pub captured_at: DateTime<Utc>,

    /// Which display server was active during capture
    pub display_server: DisplayServer,

    /// The hostname of the machine
    pub hostname: String,

    /// All captured windows
    pub windows: Vec<WindowInfo>,
}

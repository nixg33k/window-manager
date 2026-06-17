//! Storage module for persisting window layouts as JSON.
//!
//! Handles reading and writing the `~/.config/window-manager/windows.json` file,
//! including creating the config directory if it doesn't exist.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use crate::types::WindowLayout;

/// Default config directory name under ~/.config/
const CONFIG_DIR: &str = "window-manager";

/// Default filename for the saved layout
const CONFIG_FILE: &str = "windows.json";

/// Returns the path to the configuration directory (~/.config/window-manager/).
///
/// Creates the directory if it doesn't exist.
pub fn get_config_dir() -> Result<PathBuf> {
    let config_base = dirs::config_dir()
        .context("Could not determine the user's config directory (~/.config)")?;

    let config_dir = config_base.join(CONFIG_DIR);

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir).with_context(|| {
            format!("Failed to create config directory: {}", config_dir.display())
        })?;
    }

    Ok(config_dir)
}

/// Returns the full path to the JSON storage file.
pub fn get_storage_path() -> Result<PathBuf> {
    let dir = get_config_dir()?;
    Ok(dir.join(CONFIG_FILE))
}

/// Save a window layout snapshot to disk as pretty-printed JSON.
pub fn save_layout(layout: &WindowLayout) -> Result<()> {
    let path = get_storage_path()?;

    let json = serde_json::to_string_pretty(layout)
        .context("Failed to serialize window layout to JSON")?;

    fs::write(&path, &json).with_context(|| {
        format!("Failed to write layout to {}", path.display())
    })?;

    Ok(())
}

/// Load a previously saved window layout from disk.
///
/// Returns `None` if the file doesn't exist.
pub fn load_layout() -> Result<Option<WindowLayout>> {
    let path = get_storage_path()?;

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path).with_context(|| {
        format!("Failed to read layout from {}", path.display())
    })?;

    let layout: WindowLayout = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON from {}", path.display()))?;

    Ok(Some(layout))
}

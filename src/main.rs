//! # Window Manager CLI
//!
//! A command-line tool for capturing and restoring window layouts on Linux.
//!
//! Supports both **X11** and **Wayland** windowing systems, automatically detecting
//! which one is active and using the appropriate backend.
//!
//! ## Usage
//!
//! ```bash
//! # Capture the current window layout
//! window-manager capture
//!
//! # List saved window information
//! window-manager list
//!
//! # Restore windows to saved positions
//! window-manager restore
//! ```
//!
//! ## How It Works
//!
//! - On **X11**: Uses the XCB library to communicate directly with the X server,
//!   reading EWMH properties to enumerate windows and their attributes.
//! - On **Wayland**: Uses compositor-specific IPC tools (swaymsg, hyprctl, kdotool,
//!   or gdbus for GNOME) since Wayland's security model prevents direct cross-client
//!   window inspection.

mod detect;
mod storage;
mod types;
mod x11;
mod wayland;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::detect::{detect_display_server, get_hostname};
use crate::storage::{get_storage_path, load_layout, save_layout};
use crate::types::{DisplayServer, WindowLayout};

/// Window Manager — Capture and restore window layouts on Linux.
///
/// Supports both X11 and Wayland windowing systems.
#[derive(Parser, Debug)]
#[command(
    name = "window-manager",
    version,
    about = "Capture and restore window layouts on Linux (X11 & Wayland)",
    long_about = "A CLI tool that captures all open windows and their positions, sizes, \
                  states, and workspaces, saving them to a JSON file. You can later restore \
                  windows to their saved positions — perfect for after a reboot or session restart."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Available CLI commands.
#[derive(Subcommand, Debug)]
enum Commands {
    /// Capture the current window layout and save to disk.
    ///
    /// Detects the active display server (X11 or Wayland) and captures all
    /// open windows with their positions, sizes, titles, and states.
    Capture,

    /// Restore windows to their previously saved positions.
    ///
    /// Reads the saved layout from disk and attempts to move/resize all
    /// matching windows to their captured positions.
    Restore,

    /// Display the saved window layout information.
    ///
    /// Reads and pretty-prints the saved JSON file showing all captured
    /// window details.
    List,
}

fn main() {
    let cli = Cli::parse();

    // Run the selected command and handle any errors
    let result = match cli.command {
        Commands::Capture => cmd_capture(),
        Commands::Restore => cmd_restore(),
        Commands::List => cmd_list(),
    };

    if let Err(err) = result {
        eprintln!("{} {:#}", "Error:".red().bold(), err);
        std::process::exit(1);
    }
}

/// Execute the `capture` command.
///
/// 1. Detect the display server
/// 2. Capture all windows using the appropriate backend
/// 3. Save the layout to JSON
fn cmd_capture() -> Result<()> {
    println!("{}", "🔍 Capturing window layout...".cyan().bold());

    // Detect display server
    let display_server = detect_display_server();
    println!(
        "  Display server: {}",
        display_server.to_string().green()
    );

    // Capture windows using the appropriate backend
    let windows = match display_server {
        DisplayServer::X11 => {
            println!("  Using X11/XCB backend");
            x11::capture_windows().context("Failed to capture X11 windows")?
        }
        DisplayServer::Wayland => {
            println!("  Using Wayland backend");
            wayland::capture_windows().context("Failed to capture Wayland windows")?
        }
        DisplayServer::Unknown => {
            // Try X11 first (more tools available), then Wayland
            println!(
                "  {}",
                "Could not auto-detect display server, trying X11 first...".yellow()
            );
            match x11::capture_windows() {
                Ok(w) => w,
                Err(_) => {
                    println!("  X11 failed, trying Wayland...");
                    wayland::capture_windows()
                        .context("Failed to capture windows on both X11 and Wayland")?
                }
            }
        }
    };

    println!("  Captured {} window(s)", windows.len().to_string().green());

    if windows.is_empty() {
        println!(
            "  {}",
            "No windows found. This could mean no windows are open, \
             or the window manager doesn't support the required protocols."
                .yellow()
        );
    }

    // Build the layout snapshot
    let layout = WindowLayout {
        captured_at: Utc::now(),
        display_server,
        hostname: get_hostname(),
        windows,
    };

    // Save to disk
    save_layout(&layout)?;
    let path = get_storage_path()?;
    println!(
        "\n{} Saved to: {}",
        "✅".green(),
        path.display().to_string().cyan()
    );

    Ok(())
}

/// Execute the `restore` command.
///
/// 1. Load the saved layout
/// 2. Detect the current display server
/// 3. Restore windows using the appropriate backend
fn cmd_restore() -> Result<()> {
    println!("{}", "🔄 Restoring window layout...".cyan().bold());

    // Load saved layout
    let layout = load_layout()?
        .context("No saved layout found. Run 'window-manager capture' first.")?;

    println!(
        "  Loaded layout captured at: {}",
        layout.captured_at.format("%Y-%m-%d %H:%M:%S UTC").to_string().green()
    );
    println!(
        "  Original display server: {}",
        layout.display_server.to_string().green()
    );
    println!("  Windows to restore: {}", layout.windows.len());

    // Detect current display server
    let current_server = detect_display_server();
    println!(
        "  Current display server: {}",
        current_server.to_string().green()
    );

    // Warn if display servers don't match
    if layout.display_server != current_server && current_server != DisplayServer::Unknown {
        println!(
            "\n  {} Layout was captured on {} but current session is {}.",
            "⚠️  Warning:".yellow().bold(),
            layout.display_server,
            current_server
        );
        println!(
            "  Window IDs may not match. Restore will attempt to match by app name."
        );
    }

    // Restore windows
    match current_server {
        DisplayServer::X11 => {
            x11::restore_windows(&layout.windows)
                .context("Failed to restore X11 windows")?;
        }
        DisplayServer::Wayland => {
            wayland::restore_windows(&layout.windows)
                .context("Failed to restore Wayland windows")?;
        }
        DisplayServer::Unknown => {
            // Try X11 first
            println!(
                "  {}",
                "Could not detect display server, trying X11...".yellow()
            );
            match x11::restore_windows(&layout.windows) {
                Ok(()) => {}
                Err(_) => {
                    println!("  X11 failed, trying Wayland...");
                    wayland::restore_windows(&layout.windows)?;
                }
            }
        }
    }

    println!(
        "\n{} Window layout restored successfully!",
        "✅".green()
    );

    Ok(())
}

/// Execute the `list` command.
///
/// Load and display the saved window layout in a human-readable format.
fn cmd_list() -> Result<()> {
    println!("{}", "📋 Saved Window Layout".cyan().bold());

    let layout = match load_layout()? {
        Some(layout) => layout,
        None => {
            let path = get_storage_path()?;
            println!(
                "\n  {} No saved layout found at {}",
                "ℹ️ ".blue(),
                path.display()
            );
            println!("  Run '{}' first to capture your window layout.", "window-manager capture".green());
            return Ok(());
        }
    };

    // Display metadata
    println!();
    println!(
        "  {}: {}",
        "Captured at".bold(),
        layout
            .captured_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string()
            .green()
    );
    println!(
        "  {}:  {}",
        "Display server".bold(),
        layout.display_server.to_string().green()
    );
    println!(
        "  {}:  {}",
        "Hostname".bold(),
        layout.hostname.green()
    );
    println!(
        "  {}:  {}",
        "Total windows".bold(),
        layout.windows.len().to_string().green()
    );

    // Display each window
    if layout.windows.is_empty() {
        println!(
            "\n  {}",
            "(No windows were captured)".yellow()
        );
    } else {
        println!("\n  {}", "Windows:".bold().underline());
        println!();

        for (i, win) in layout.windows.iter().enumerate() {
            println!(
                "  {}. {} {}",
                (i + 1).to_string().cyan().bold(),
                "🪟".to_string(),
                win.title.bold()
            );
            println!("     App:       {}", win.app_name.green());
            println!("     ID:        {}", win.window_id.dimmed());
            println!("     Position:  ({}, {})", win.x, win.y);
            println!("     Size:      {} × {}", win.width, win.height);
            println!("     Workspace: {}", win.workspace);
            println!(
                "     State:     {}",
                match win.state {
                    crate::types::WindowState::Normal => "Normal".to_string(),
                    crate::types::WindowState::Minimized => "Minimized".yellow().to_string(),
                    crate::types::WindowState::Maximized => "Maximized".blue().to_string(),
                    crate::types::WindowState::Fullscreen => "Fullscreen".magenta().to_string(),
                }
            );
            println!();
        }
    }

    // Show storage path
    let path = get_storage_path()?;
    println!(
        "  {}: {}",
        "Storage file".dimmed(),
        path.display().to_string().dimmed()
    );

    Ok(())
}

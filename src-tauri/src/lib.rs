//! Open ScreenStudio - Beautiful screen recordings, made simple.
//!
//! This is the main library crate for the Open ScreenStudio application.
//! It provides the Tauri application setup and all backend functionality.

pub mod capture;
pub mod commands;
pub mod project;
pub mod recorder;
pub mod utils;

// These modules will be implemented in later phases
// pub mod processing;
// pub mod export;

use commands::recording::RecorderState;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize the application
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing/logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "open_screenstudio=debug,tauri=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Open ScreenStudio v{}", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(RecorderState::default())
        .invoke_handler(tauri::generate_handler![
            // Project commands
            commands::project::create_project,
            commands::project::open_project,
            commands::project::save_project,
            commands::project::get_project,
            // System commands
            commands::system::get_system_info,
            // Recording commands
            commands::recording::get_displays,
            commands::recording::get_audio_devices,
            commands::recording::check_screen_permission,
            commands::recording::request_screen_permission,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::recording::pause_recording,
            commands::recording::resume_recording,
            commands::recording::get_recording_state,
            commands::recording::get_recording_duration,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

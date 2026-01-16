//! Processing-related Tauri commands
//!
//! These commands expose cursor smoothing and other post-processing
//! functionality to the frontend.

use crate::capture::input::types::MouseMove;
use crate::processing::cursor_smoothing::{smooth_cursor_data, SmoothedMouseMove};
use crate::project::schema::SpringConfig;
use std::path::Path;

/// Process raw mouse moves and return smoothed data
///
/// This is used for real-time preview in the editor.
#[tauri::command]
pub async fn smooth_cursor(
    input_file: String,
    config: SpringConfig,
    output_fps: f64,
) -> Result<Vec<SmoothedMouseMove>, String> {
    let path = Path::new(&input_file);
    let content = std::fs::read_to_string(path).map_err(|e| format!("Failed to read input file: {}", e))?;
    let raw_moves: Vec<MouseMove> =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse mouse moves: {}", e))?;

    tracing::info!(
        "Smoothing {} raw moves at {}fps with config: stiffness={}, damping={}, mass={}",
        raw_moves.len(),
        output_fps,
        config.stiffness,
        config.damping,
        config.mass
    );

    let smoothed = smooth_cursor_data(&raw_moves, &config, output_fps);

    tracing::info!("Generated {} smoothed frames", smoothed.len());

    Ok(smoothed)
}

/// Process and write smoothed data to file (for export)
#[tauri::command]
pub async fn process_cursor_smoothing(
    input_file: String,
    output_file: String,
    config: SpringConfig,
    output_fps: f64,
) -> Result<(), String> {
    let path = Path::new(&input_file);
    let content = std::fs::read_to_string(path).map_err(|e| format!("Failed to read input file: {}", e))?;
    let raw_moves: Vec<MouseMove> =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse mouse moves: {}", e))?;

    tracing::info!(
        "Processing cursor smoothing: {} raw moves -> {} at {}fps",
        raw_moves.len(),
        output_file,
        output_fps
    );

    let smoothed = smooth_cursor_data(&raw_moves, &config, output_fps);
    let output = serde_json::to_vec_pretty(&smoothed).map_err(|e| format!("Failed to serialize: {}", e))?;

    std::fs::write(&output_file, output).map_err(|e| format!("Failed to write output file: {}", e))?;

    tracing::info!("Wrote {} smoothed frames to {}", smoothed.len(), output_file);

    Ok(())
}

/// Get default spring configuration
#[tauri::command]
pub async fn get_default_spring_config() -> SpringConfig {
    SpringConfig::default()
}

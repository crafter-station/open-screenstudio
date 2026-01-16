//! Cursor smoothing algorithms for post-recording processing
//!
//! This module processes raw mouse movement data captured at 120Hz
//! and applies spring physics smoothing to produce natural-looking
//! cursor movement for playback and export.

use crate::capture::input::types::MouseMove;
use crate::processing::spring::Spring2D;
use crate::project::schema::SpringConfig;
use serde::{Deserialize, Serialize};

/// Smoothed mouse position with both raw and smoothed coordinates
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmoothedMouseMove {
    /// Smoothed X position
    pub x: f64,
    /// Smoothed Y position
    pub y: f64,
    /// Original raw X position
    pub raw_x: f64,
    /// Original raw Y position
    pub raw_y: f64,
    /// Cursor image ID
    pub cursor_id: String,
    /// Time in milliseconds from recording start
    pub process_time_ms: f64,
}

/// Default teleport detection threshold in pixels
/// If cursor moves more than this distance in one frame, reset spring
pub const DEFAULT_TELEPORT_THRESHOLD: f64 = 500.0;

/// Smooth cursor data from raw input at a given output framerate
///
/// # Arguments
/// * `raw_moves` - Raw mouse movement data from input tracking
/// * `config` - Spring physics configuration
/// * `output_fps` - Target output framerate (e.g., 30.0 or 60.0)
///
/// # Returns
/// Vector of smoothed mouse positions, one per output frame
pub fn smooth_cursor_data(
    raw_moves: &[MouseMove],
    config: &SpringConfig,
    output_fps: f64,
) -> Vec<SmoothedMouseMove> {
    smooth_cursor_data_with_teleport(raw_moves, config, output_fps, DEFAULT_TELEPORT_THRESHOLD)
}

/// Smooth cursor data with custom teleport threshold
pub fn smooth_cursor_data_with_teleport(
    raw_moves: &[MouseMove],
    config: &SpringConfig,
    output_fps: f64,
    teleport_threshold: f64,
) -> Vec<SmoothedMouseMove> {
    if raw_moves.is_empty() {
        return vec![];
    }

    let frame_duration_ms = 1000.0 / output_fps;
    let total_duration_ms = raw_moves.last().map(|m| m.process_time_ms).unwrap_or(0.0);
    
    // Always have at least 1 frame for non-empty input
    let frame_count = ((total_duration_ms / frame_duration_ms).ceil() as usize).max(1);

    let mut result = Vec::with_capacity(frame_count);
    let mut spring = Spring2D::new(raw_moves[0].x, raw_moves[0].y);
    let mut raw_index = 0;
    let mut last_raw_x = raw_moves[0].x;
    let mut last_raw_y = raw_moves[0].y;

    for frame in 0..frame_count {
        let frame_time_ms = frame as f64 * frame_duration_ms;

        // Find the raw move closest to this frame time
        while raw_index + 1 < raw_moves.len()
            && raw_moves[raw_index + 1].process_time_ms <= frame_time_ms
        {
            raw_index += 1;
        }

        let raw = &raw_moves[raw_index];

        // Detect teleport (large jump) and reset spring if needed
        let dx = raw.x - last_raw_x;
        let dy = raw.y - last_raw_y;
        let distance = (dx * dx + dy * dy).sqrt();

        if distance > teleport_threshold {
            // Teleport detected - reset spring to new position instantly
            spring.reset(raw.x, raw.y);
        } else {
            // Normal movement - step spring toward raw position
            let dt = frame_duration_ms / 1000.0; // Convert to seconds
            spring.step(raw.x, raw.y, config, dt);
        }

        last_raw_x = raw.x;
        last_raw_y = raw.y;

        let (smooth_x, smooth_y) = spring.position();

        result.push(SmoothedMouseMove {
            x: smooth_x,
            y: smooth_y,
            raw_x: raw.x,
            raw_y: raw.y,
            cursor_id: raw.cursor_id.clone(),
            process_time_ms: frame_time_ms,
        });
    }

    result
}

/// Detect if cursor teleported between two positions
pub fn detect_teleport(prev: &MouseMove, curr: &MouseMove, threshold_px: f64) -> bool {
    let dx = curr.x - prev.x;
    let dy = curr.y - prev.y;
    let distance = (dx * dx + dy * dy).sqrt();
    distance > threshold_px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_move(x: f64, y: f64, time_ms: f64) -> MouseMove {
        MouseMove {
            x,
            y,
            cursor_id: "test_cursor".to_string(),
            active_modifiers: vec![],
            process_time_ms: time_ms,
            unix_time_ms: 0,
        }
    }

    fn default_config() -> SpringConfig {
        SpringConfig {
            stiffness: 470.0,
            damping: 70.0,
            mass: 3.0,
        }
    }

    #[test]
    fn test_empty_input() {
        let result = smooth_cursor_data(&[], &default_config(), 30.0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_single_point() {
        let moves = vec![make_move(100.0, 200.0, 0.0)];
        let result = smooth_cursor_data(&moves, &default_config(), 30.0);

        // Should have at least one frame
        assert!(!result.is_empty());
        // First frame should be at initial position
        assert!((result[0].x - 100.0).abs() < 0.1);
        assert!((result[0].y - 200.0).abs() < 0.1);
    }

    #[test]
    fn test_smoothing_reduces_jitter() {
        // Create jerky movement with random-ish jitter around a moving target
        let mut moves = Vec::new();
        for i in 0..120 {
            // Base position moves linearly, jitter oscillates
            let base = i as f64 * 2.0;
            let jitter = if i % 2 == 0 { 10.0 } else { -10.0 };
            moves.push(make_move(base + jitter, base + jitter, i as f64 * 8.33));
        }

        let smoothed = smooth_cursor_data(&moves, &default_config(), 30.0);

        // Calculate deviation from the smoothed trajectory
        // The smoothed version should have smaller deviations from its own trend
        let mut raw_deviations = 0.0;
        for i in 1..moves.len() - 1 {
            // Expected position is midpoint of neighbors
            let expected = (moves[i - 1].x + moves[i + 1].x) / 2.0;
            raw_deviations += (moves[i].x - expected).abs();
        }

        let mut smooth_deviations = 0.0;
        for i in 1..smoothed.len() - 1 {
            let expected = (smoothed[i - 1].x + smoothed[i + 1].x) / 2.0;
            smooth_deviations += (smoothed[i].x - expected).abs();
        }

        // Normalize by count
        raw_deviations /= (moves.len() - 2) as f64;
        smooth_deviations /= (smoothed.len() - 2).max(1) as f64;

        assert!(
            smooth_deviations < raw_deviations,
            "Smoothed deviations {} should be less than raw deviations {}",
            smooth_deviations,
            raw_deviations
        );
    }

    #[test]
    fn test_teleport_detection() {
        let prev = make_move(0.0, 0.0, 0.0);
        let curr_near = make_move(10.0, 10.0, 100.0);
        let curr_far = make_move(1000.0, 1000.0, 100.0);

        assert!(!detect_teleport(&prev, &curr_near, 500.0));
        assert!(detect_teleport(&prev, &curr_far, 500.0));
    }

    #[test]
    fn test_teleport_resets_spring() {
        let moves = vec![
            make_move(0.0, 0.0, 0.0),
            make_move(10.0, 10.0, 33.33),
            make_move(20.0, 20.0, 66.66),
            make_move(1000.0, 1000.0, 100.0), // Teleport!
            make_move(1010.0, 1010.0, 133.33),
        ];

        let smoothed = smooth_cursor_data(&moves, &default_config(), 30.0);

        // Find frame after teleport (around 100ms = frame 3)
        let post_teleport = smoothed.iter().find(|m| m.process_time_ms >= 100.0);
        assert!(post_teleport.is_some());

        let frame = post_teleport.unwrap();
        // After teleport, smoothed position should be close to new raw position
        // (spring was reset, not trying to catch up from 0,0)
        assert!(
            (frame.x - frame.raw_x).abs() < 100.0,
            "After teleport, smoothed X {} should be close to raw X {}",
            frame.x,
            frame.raw_x
        );
    }

    #[test]
    fn test_output_fps_affects_frame_count() {
        let moves = vec![
            make_move(0.0, 0.0, 0.0),
            make_move(100.0, 100.0, 1000.0), // 1 second duration
        ];

        let smoothed_30fps = smooth_cursor_data(&moves, &default_config(), 30.0);
        let smoothed_60fps = smooth_cursor_data(&moves, &default_config(), 60.0);

        // 60fps should have roughly twice as many frames as 30fps
        assert!(
            smoothed_60fps.len() > smoothed_30fps.len(),
            "60fps ({} frames) should have more frames than 30fps ({} frames)",
            smoothed_60fps.len(),
            smoothed_30fps.len()
        );
    }
}

//! Recording-related Tauri commands

use crate::capture::audio::get_audio_input_devices;
use crate::capture::traits::{AudioDeviceInfo, DisplayInfo, has_screen_recording_permission, request_screen_recording_permission};
use crate::recorder::state::{RecordingConfig, RecordingResult as RecordingOutput, RecordingState};
use crate::recorder::RecordingCoordinator;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

/// Application state for recording
pub struct RecorderState {
    pub coordinator: Arc<Mutex<RecordingCoordinator>>,
}

impl Default for RecorderState {
    fn default() -> Self {
        Self {
            coordinator: Arc::new(Mutex::new(RecordingCoordinator::new())),
        }
    }
}

/// Get list of available audio input devices (microphones)
#[tauri::command]
pub async fn get_audio_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    Ok(get_audio_input_devices())
}

/// Get list of available displays
#[tauri::command]
pub async fn get_displays() -> Result<Vec<DisplayInfo>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(crate::capture::macos::screen::get_displays())
    }
    
    #[cfg(target_os = "windows")]
    {
        Ok(crate::capture::windows::screen::get_displays())
    }
    
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(vec![])
    }
}

/// Check if screen recording permission is granted
#[tauri::command]
pub async fn check_screen_permission() -> Result<bool, String> {
    Ok(has_screen_recording_permission())
}

/// Request screen recording permission
#[tauri::command]
pub async fn request_screen_permission() -> Result<bool, String> {
    Ok(request_screen_recording_permission())
}

/// Start recording
#[tauri::command]
pub async fn start_recording(
    state: State<'_, RecorderState>,
    config: RecordingConfig,
) -> Result<(), String> {
    // Check permission first
    if !has_screen_recording_permission() {
        request_screen_recording_permission();
        return Err("Screen recording permission not granted. Please allow in System Preferences and try again.".to_string());
    }
    
    let mut coordinator = state.coordinator.lock().await;
    
    // Clear existing channels and add display capture
    coordinator.clear_channels();
    
    #[cfg(target_os = "macos")]
    {
        let display_channel = Box::new(crate::capture::macos::screen::DisplayCaptureChannel::new(config.display_id));
        coordinator.add_channel(display_channel);
    }
    
    #[cfg(target_os = "windows")]
    {
        let display_channel = Box::new(crate::capture::windows::screen::DisplayCaptureChannel::new(config.display_id));
        coordinator.add_channel(display_channel);
    }
    
    // Add microphone channel if enabled
    if config.capture_microphone {
        let mic_channel = Box::new(crate::capture::audio::MicrophoneCaptureChannel::new(
            config.microphone_device_id.clone(),
        ));
        coordinator.add_channel(mic_channel);
    }
    
    coordinator.start(config).await.map_err(|e| e.to_string())
}

/// Stop recording
#[tauri::command]
pub async fn stop_recording(
    state: State<'_, RecorderState>,
) -> Result<RecordingOutput, String> {
    let mut coordinator = state.coordinator.lock().await;
    coordinator.stop().await.map_err(|e| e.to_string())
}

/// Pause recording
#[tauri::command]
pub async fn pause_recording(
    state: State<'_, RecorderState>,
) -> Result<(), String> {
    let mut coordinator = state.coordinator.lock().await;
    coordinator.pause().await.map_err(|e| e.to_string())
}

/// Resume recording
#[tauri::command]
pub async fn resume_recording(
    state: State<'_, RecorderState>,
) -> Result<(), String> {
    let mut coordinator = state.coordinator.lock().await;
    coordinator.resume().await.map_err(|e| e.to_string())
}

/// Get current recording state
#[tauri::command]
pub async fn get_recording_state(
    state: State<'_, RecorderState>,
) -> Result<RecordingState, String> {
    let coordinator = state.coordinator.lock().await;
    Ok(coordinator.state())
}

/// Get current recording duration in milliseconds
#[tauri::command]
pub async fn get_recording_duration(
    state: State<'_, RecorderState>,
) -> Result<f64, String> {
    let coordinator = state.coordinator.lock().await;
    Ok(coordinator.duration_ms())
}

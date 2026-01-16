//! Platform-specific capture implementations
//!
//! This module provides screen, audio, and input capture for each platform.

pub mod traits;
pub mod audio;
pub mod input;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

// Re-export traits
pub use traits::{DisplayInfo, WindowInfo, WindowBounds, AudioDeviceInfo, CameraInfo, Resolution};

// Re-export permission functions from traits (which delegates to platform)
pub use traits::{has_screen_recording_permission, request_screen_recording_permission};

// Re-export audio functions
pub use audio::{get_audio_input_devices, MicrophoneCaptureChannel};

// Re-export input channel
pub use input::InputTrackingChannel;

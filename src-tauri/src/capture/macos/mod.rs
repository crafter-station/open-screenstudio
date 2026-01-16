//! macOS capture implementations
//!
//! Uses ScreenCaptureKit for screen capture and AVFoundation for audio/video.

pub mod permissions;
pub mod screen;
pub mod system_audio;
pub mod input;

pub use permissions::*;
pub use screen::*;
pub use system_audio::*;
pub use input::*;

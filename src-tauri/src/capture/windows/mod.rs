//! Windows capture implementations
//!
//! Uses Windows.Graphics.Capture for screen capture.

pub mod screen;
pub mod system_audio;
pub mod input;

pub use screen::*;
pub use system_audio::*;
pub use input::*;

/// Windows doesn't require explicit permission for screen capture
pub mod permissions {
    pub fn has_screen_recording_permission() -> bool {
        true
    }
    
    pub fn request_screen_recording_permission() -> bool {
        true
    }
}

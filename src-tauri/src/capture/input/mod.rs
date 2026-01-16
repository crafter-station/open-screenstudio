//! Input tracking (mouse, cursor) capture
//!
//! Implements a `RecordingChannel` that records high-frequency mouse movement,
//! mouse clicks, and cursor metadata for later processing (cursor smoothing,
//! auto-zoom, etc.).

pub mod channel;
pub mod types;

pub use channel::InputTrackingChannel;
pub use types::{CursorInfo, MouseClick, MouseMove};

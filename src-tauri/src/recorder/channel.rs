//! Recording channel trait
//!
//! Defines the interface for different recording channels (display, audio, webcam, input).

use async_trait::async_trait;
use std::path::Path;
use thiserror::Error;

/// Errors that can occur during recording
#[derive(Error, Debug)]
pub enum RecordingError {
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Already recording")]
    AlreadyRecording,

    #[error("Not recording")]
    NotRecording,

    #[error("Capture error: {0}")]
    CaptureError(String),

    #[error("Encoding error: {0}")]
    EncodingError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Platform error: {0}")]
    PlatformError(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),
}

/// Result type for recording operations
pub type RecordingResult<T> = Result<T, RecordingError>;

/// Frame data from a capture source
#[derive(Debug)]
pub struct CapturedFrame {
    /// Raw pixel data (BGRA format)
    pub data: Vec<u8>,
    
    /// Frame width in pixels
    pub width: u32,
    
    /// Frame height in pixels
    pub height: u32,
    
    /// Timestamp in milliseconds (process time)
    pub timestamp_ms: f64,
    
    /// Bytes per row (may include padding)
    pub bytes_per_row: u32,
}

/// Trait for recording channels
///
/// Each channel represents a capture source (display, audio, webcam, input).
/// Channels are managed by the RecordingCoordinator.
#[async_trait]
pub trait RecordingChannel: Send + Sync {
    /// Get the channel identifier (e.g., "display", "system-audio", "microphone")
    fn id(&self) -> &str;
    
    /// Get the channel type
    fn channel_type(&self) -> ChannelType;
    
    /// Initialize the channel with the output directory
    async fn initialize(&mut self, output_dir: &Path, session_index: usize) -> RecordingResult<()>;
    
    /// Start recording
    async fn start(&mut self) -> RecordingResult<()>;
    
    /// Stop recording
    async fn stop(&mut self) -> RecordingResult<()>;
    
    /// Pause recording
    async fn pause(&mut self) -> RecordingResult<()>;
    
    /// Resume recording (starts a new session)
    async fn resume(&mut self, session_index: usize) -> RecordingResult<()>;
    
    /// Check if the channel is currently recording
    fn is_recording(&self) -> bool;
    
    /// Get output files created by this channel
    fn output_files(&self) -> Vec<String>;
}

/// Types of recording channels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    /// Screen/display capture
    Display,
    /// System audio capture
    SystemAudio,
    /// Microphone capture
    Microphone,
    /// Webcam capture
    Webcam,
    /// Input tracking (mouse, keyboard)
    Input,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelType::Display => write!(f, "display"),
            ChannelType::SystemAudio => write!(f, "system-audio"),
            ChannelType::Microphone => write!(f, "microphone"),
            ChannelType::Webcam => write!(f, "webcam"),
            ChannelType::Input => write!(f, "input"),
        }
    }
}

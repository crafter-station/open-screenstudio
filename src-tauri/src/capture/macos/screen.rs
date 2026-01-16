//! macOS screen capture using ScreenCaptureKit
//!
//! This module provides screen capture functionality using Apple's ScreenCaptureKit framework.
//! Frames are captured and encoded to H.264 segments using FFmpeg.

use crate::capture::traits::DisplayInfo;
use crate::recorder::channel::{ChannelType, RecordingChannel, RecordingError, RecordingResult};
use async_trait::async_trait;
use core_graphics::display::CGDisplay;
use parking_lot::Mutex as ParkingMutex;
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Get list of available displays
pub fn get_displays() -> Vec<DisplayInfo> {
    let display_ids = CGDisplay::active_displays().unwrap_or_default();

    display_ids
        .iter()
        .enumerate()
        .map(|(index, &id)| {
            let display = CGDisplay::new(id);
            let bounds = display.bounds();
            let is_main = display.is_main();

            // Get refresh rate if available
            let refresh_rate = display
                .display_mode()
                .map(|mode| mode.refresh_rate() as u32)
                .filter(|&r| r > 0);

            DisplayInfo {
                id,
                name: if is_main {
                    "Main Display".to_string()
                } else {
                    format!("Display {}", index + 1)
                },
                width: bounds.size.width as u32,
                height: bounds.size.height as u32,
                scale_factor: display.pixels_high() as f64 / bounds.size.height,
                is_primary: is_main,
                refresh_rate,
            }
        })
        .collect()
}

/// FFmpeg encoder for HLS segment output
struct FFmpegSegmentEncoder {
    process: ParkingMutex<Option<Child>>,
    frame_count: AtomicU64,
    running: AtomicBool,
    output_dir: PathBuf,
    segment_index: usize,
}

impl FFmpegSegmentEncoder {
    fn new(
        width: u32,
        height: u32,
        fps: u32,
        output_dir: &Path,
        segment_index: usize,
    ) -> Result<Self, std::io::Error> {
        // Create output directory if it doesn't exist
        std::fs::create_dir_all(output_dir)?;

        let segment_pattern = output_dir
            .join(format!("segment-{segment_index}-%03d.mp4"))
            .to_string_lossy()
            .to_string();

        // Start FFmpeg process for segmented output
        // Input: raw BGRA frames from stdin
        // Output: HLS-compatible fMP4 segments
        let process = Command::new("ffmpeg")
            .args([
                "-y",           // Overwrite output
                "-f", "rawvideo", // Input format
                "-pixel_format", "bgra", // BGRA pixel format from ScreenCaptureKit
                "-video_size", &format!("{width}x{height}"),
                "-framerate", &fps.to_string(),
                "-i", "-",      // Read from stdin
                "-c:v", "libx264", // H.264 codec
                "-preset", "ultrafast", // Fast encoding for real-time
                "-tune", "zerolatency", // Low latency
                "-pix_fmt", "yuv420p", // Output pixel format
                "-crf", "23",   // Quality (lower = better, 23 is default)
                "-g", &(fps * 2).to_string(), // GOP size = 2 seconds
                "-keyint_min", &fps.to_string(), // Min keyframe interval
                "-sc_threshold", "0", // Disable scene change detection
                "-f", "segment", // Segment muxer
                "-segment_time", "2", // 2-second segments
                "-segment_format", "mp4", // MP4 format
                "-reset_timestamps", "1",
                "-movflags", "+faststart+frag_keyframe+empty_moov",
                &segment_pattern,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()) // Capture stderr for debugging
            .spawn()?;

        tracing::info!(
            "Started FFmpeg encoder: {}x{} @ {}fps, segments to {:?}",
            width,
            height,
            fps,
            output_dir
        );

        Ok(Self {
            process: ParkingMutex::new(Some(process)),
            frame_count: AtomicU64::new(0),
            running: AtomicBool::new(true),
            output_dir: output_dir.to_path_buf(),
            segment_index,
        })
    }

    fn write_frame(&self, data: &[u8]) -> bool {
        if !self.running.load(Ordering::Relaxed) {
            return false;
        }

        let mut guard = self.process.lock();
        if let Some(ref mut process) = *guard {
            if let Some(ref mut stdin) = process.stdin {
                if stdin.write_all(data).is_ok() {
                    self.frame_count.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
            }
        }
        false
    }

    fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    fn finish(&self) -> Result<Vec<String>, std::io::Error> {
        self.running.store(false, Ordering::Relaxed);
        let mut guard = self.process.lock();
        if let Some(mut process) = guard.take() {
            // Close stdin to signal EOF
            drop(process.stdin.take());
            // Wait for FFmpeg to finish
            let output = process.wait_with_output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("FFmpeg exited with status {}: {}", output.status, stderr);
            }
        }

        // Find all generated segment files
        let pattern = format!("segment-{}*.mp4", self.segment_index);
        let mut segments = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.output_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&format!("segment-{}-", self.segment_index))
                    && name.ends_with(".mp4")
                {
                    segments.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
        segments.sort();

        tracing::info!(
            "FFmpeg finished: {} frames, {} segments (pattern: {})",
            self.frame_count(),
            segments.len(),
            pattern
        );

        Ok(segments)
    }
}

/// Frame handler for ScreenCaptureKit
struct FrameHandler {
    encoder: Arc<FFmpegSegmentEncoder>,
    expected_size: usize,
    width: u32,
    height: u32,
}

impl SCStreamOutputTrait for FrameHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, output_type: SCStreamOutputType) {
        if !matches!(output_type, SCStreamOutputType::Screen) {
            return;
        }

        let Some(pixel_buffer) = sample.image_buffer() else {
            return;
        };

        // Lock pixel buffer for CPU access
        let Ok(guard) = pixel_buffer.lock(CVPixelBufferLockFlags::READ_ONLY) else {
            return;
        };

        let data = guard.as_slice();

        // Verify size matches expected (width * height * 4 bytes per pixel for BGRA)
        if data.len() >= self.expected_size
            && self.encoder.write_frame(&data[..self.expected_size])
        {
            let count = self.encoder.frame_count();
            if count.is_multiple_of(60) {
                tracing::debug!(
                    "Captured {} frames ({:.1}s) at {}x{}",
                    count,
                    count as f64 / 30.0,
                    self.width,
                    self.height
                );
            }
        }
    }
}

/// Display capture channel using ScreenCaptureKit
///
/// This implementation captures frames using Apple's ScreenCaptureKit framework
/// and encodes them to H.264 segments using FFmpeg.
pub struct DisplayCaptureChannel {
    /// Channel identifier
    id: String,

    /// Display ID to capture
    display_id: u32,

    /// Whether currently recording
    is_recording: Arc<AtomicBool>,

    /// Output directory
    output_dir: Option<PathBuf>,

    /// Current session index
    session_index: usize,

    /// Output files created
    output_files: Arc<ParkingMutex<Vec<String>>>,

    /// FFmpeg encoder
    encoder: Option<Arc<FFmpegSegmentEncoder>>,

    /// SCStream handle
    stream: Option<SCStream>,

    /// Capture width
    width: u32,

    /// Capture height
    height: u32,

    /// Capture FPS
    fps: u32,
}

impl DisplayCaptureChannel {
    /// Create a new display capture channel
    pub fn new(display_id: u32) -> Self {
        Self {
            id: format!("display-{}", display_id),
            display_id,
            is_recording: Arc::new(AtomicBool::new(false)),
            output_dir: None,
            session_index: 0,
            output_files: Arc::new(ParkingMutex::new(Vec::new())),
            encoder: None,
            stream: None,
            width: 1920,
            height: 1080,
            fps: 30,
        }
    }

    /// Find the SCDisplay matching our display_id
    fn find_display(&self) -> RecordingResult<SCDisplay> {
        let content = SCShareableContent::get().map_err(|e| {
            RecordingError::CaptureError(format!("Failed to get shareable content: {}", e))
        })?;

        let displays: Vec<SCDisplay> = content.displays();
        for display in displays {
            if display.display_id() == self.display_id {
                return Ok(display);
            }
        }

        Err(RecordingError::ConfigurationError(format!(
            "Display {} not found",
            self.display_id
        )))
    }
}

#[async_trait]
impl RecordingChannel for DisplayCaptureChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Display
    }

    async fn initialize(&mut self, output_dir: &Path, session_index: usize) -> RecordingResult<()> {
        // Check permission first
        if !super::permissions::has_screen_recording_permission() {
            super::permissions::request_screen_recording_permission();
            return Err(RecordingError::PermissionDenied(
                "Screen recording permission not granted. Please allow in System Preferences."
                    .to_string(),
            ));
        }

        // Check if FFmpeg is available
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return Err(RecordingError::ConfigurationError(
                "FFmpeg not found. Please install FFmpeg: brew install ffmpeg".to_string(),
            ));
        }

        // Get display info for resolution
        let display = self.find_display()?;
        self.width = display.width();
        self.height = display.height();

        self.output_dir = Some(output_dir.to_path_buf());
        self.session_index = session_index;

        tracing::info!(
            "Display capture channel initialized for display {} ({}x{})",
            self.display_id,
            self.width,
            self.height
        );
        Ok(())
    }

    async fn start(&mut self) -> RecordingResult<()> {
        if self.is_recording.load(Ordering::SeqCst) {
            return Err(RecordingError::AlreadyRecording);
        }

        let output_dir = self.output_dir.clone().ok_or_else(|| {
            RecordingError::ConfigurationError("Output directory not set".to_string())
        })?;

        // Find the display
        let display = self.find_display()?;

        // Create FFmpeg encoder
        let encoder = Arc::new(
            FFmpegSegmentEncoder::new(
                self.width,
                self.height,
                self.fps,
                &output_dir,
                self.session_index,
            )
            .map_err(|e| RecordingError::CaptureError(format!("Failed to start FFmpeg: {}", e)))?,
        );
        self.encoder = Some(encoder.clone());

        // Create content filter for this display
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();

        // Configure stream
        let frame_interval = CMTime::new(1, self.fps as i32);
        let config = SCStreamConfiguration::new()
            .with_width(self.width)
            .with_height(self.height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_minimum_frame_interval(&frame_interval)
            .with_shows_cursor(true);

        // Create frame handler
        let expected_size = (self.width * self.height * 4) as usize;
        let handler = FrameHandler {
            encoder: encoder.clone(),
            expected_size,
            width: self.width,
            height: self.height,
        };

        // Create and start stream
        let mut stream = SCStream::new(&filter, &config);
        stream.add_output_handler(handler, SCStreamOutputType::Screen);
        stream.start_capture().map_err(|e| {
            RecordingError::CaptureError(format!("Failed to start capture: {}", e))
        })?;

        self.stream = Some(stream);
        self.is_recording.store(true, Ordering::SeqCst);

        tracing::info!(
            "Display capture started for display {} ({}x{} @ {}fps)",
            self.display_id,
            self.width,
            self.height,
            self.fps
        );
        Ok(())
    }

    async fn stop(&mut self) -> RecordingResult<()> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Err(RecordingError::NotRecording);
        }

        self.is_recording.store(false, Ordering::SeqCst);

        // Stop the stream
        if let Some(ref mut stream) = self.stream {
            stream.stop_capture().map_err(|e| {
                RecordingError::CaptureError(format!("Failed to stop capture: {}", e))
            })?;
        }
        self.stream = None;

        // Finish encoding and collect output files
        if let Some(ref encoder) = self.encoder {
            let segments = encoder.finish().map_err(|e| {
                RecordingError::CaptureError(format!("Failed to finish encoding: {}", e))
            })?;
            self.output_files.lock().extend(segments);
        }
        self.encoder = None;

        tracing::info!("Display capture stopped");
        Ok(())
    }

    async fn pause(&mut self) -> RecordingResult<()> {
        // For pause, we stop the current stream and encoder
        // Resume will create a new session index
        self.stop().await
    }

    async fn resume(&mut self, session_index: usize) -> RecordingResult<()> {
        self.session_index = session_index;
        self.start().await
    }

    fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }

    fn output_files(&self) -> Vec<String> {
        self.output_files.lock().clone()
    }
}

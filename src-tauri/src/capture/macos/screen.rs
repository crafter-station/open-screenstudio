//! macOS screen capture using CGWindowListCreateImage
//!
//! This module provides screen capture functionality using Core Graphics.
//! Frames are captured and encoded to H.264 segments using FFmpeg.

use crate::capture::traits::DisplayInfo;
use crate::recorder::channel::{ChannelType, RecordingChannel, RecordingError, RecordingResult};
use async_trait::async_trait;
use core_graphics::display::{kCGWindowListOptionOnScreenOnly, CGDisplay};
use parking_lot::Mutex as ParkingMutex;
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

/// Capture a single frame from a display using CGDisplayCreateImage
fn capture_display_frame(display_id: u32) -> Option<(Vec<u8>, u32, u32)> {
    let display = CGDisplay::new(display_id);
    let bounds = display.bounds();

    // Create image of the entire display
    // This captures at native (Retina) resolution automatically
    let image = CGDisplay::screenshot(
        bounds,
        kCGWindowListOptionOnScreenOnly,
        0, // kCGNullWindowID - capture everything
        core_graphics::display::kCGWindowImageDefault,
    )?;

    let width = image.width() as u32;
    let height = image.height() as u32;
    let bytes_per_row = image.bytes_per_row();
    let data = image.data();

    // CGImage data is typically in BGRA format
    // We need to handle potential row padding
    let pixel_data: Vec<u8> = if bytes_per_row == (width as usize * 4) {
        // No padding, direct copy
        data.bytes().to_vec()
    } else {
        // Row padding exists, need to copy row by row
        let mut result = Vec::with_capacity((width * height * 4) as usize);
        let src = data.bytes();
        for y in 0..height as usize {
            let row_start = y * bytes_per_row;
            let row_end = row_start + (width as usize * 4);
            result.extend_from_slice(&src[row_start..row_end]);
        }
        result
    };

    Some((pixel_data, width, height))
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

        let output_file = output_dir
            .join(format!("recording-{segment_index}.mp4"))
            .to_string_lossy()
            .to_string();

        // Start FFmpeg process for MP4 output
        // Input: raw BGRA frames from stdin
        // Output: H.264 encoded MP4
        let process = Command::new("ffmpeg")
            .args([
                "-y",                            // Overwrite output
                "-f", "rawvideo",                // Input format
                "-pixel_format", "bgra",         // BGRA pixel format from CGImage
                "-video_size", &format!("{width}x{height}"),
                "-framerate", &fps.to_string(),
                "-i", "-",                       // Read from stdin
                "-c:v", "libx264",               // H.264 codec
                "-preset", "veryfast",           // Good balance of speed and compression
                "-pix_fmt", "yuv420p",           // Output pixel format (required for compatibility)
                "-crf", "18",                    // High quality (lower = better, 18 is visually lossless)
                "-g", &(fps * 2).to_string(),    // GOP size = 2 seconds
                "-movflags", "+faststart",       // Move moov atom to start for streaming
                &output_file,
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

        // Find the output file
        let output_file = self.output_dir
            .join(format!("recording-{}.mp4", self.segment_index))
            .to_string_lossy()
            .to_string();
        
        let mut files = Vec::new();
        if std::path::Path::new(&output_file).exists() {
            files.push(output_file.clone());
        }

        tracing::info!(
            "FFmpeg finished: {} frames, output: {}",
            self.frame_count(),
            output_file,
        );

        Ok(files)
    }
}

/// Display capture channel using CGWindowListCreateImage
///
/// This implementation captures frames using Core Graphics and
/// encodes them to H.264 segments using FFmpeg.
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

    /// Capture task handle
    capture_handle: Option<tokio::task::JoinHandle<()>>,

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
            capture_handle: None,
            width: 1920,
            height: 1080,
            fps: 30,
        }
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

        // Get display info for resolution - use native (pixel) resolution for Retina displays
        let display = CGDisplay::new(self.display_id);
        self.width = display.pixels_wide() as u32;
        self.height = display.pixels_high() as u32;

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

        self.is_recording.store(true, Ordering::SeqCst);

        // Start capture loop in background task
        let is_recording = self.is_recording.clone();
        let display_id = self.display_id;
        let fps = self.fps;
        let width = self.width;
        let height = self.height;

        let handle = tokio::spawn(async move {
            let frame_interval = std::time::Duration::from_millis(1000 / fps as u64);
            let expected_size = (width * height * 4) as usize; // BGRA = 4 bytes per pixel

            while is_recording.load(Ordering::SeqCst) {
                let start = std::time::Instant::now();

                // Capture frame - returns actual captured dimensions
                if let Some((data, captured_w, captured_h)) = capture_display_frame(display_id) {
                    let captured_size = (captured_w * captured_h * 4) as usize;
                    
                    // Verify dimensions match what FFmpeg expects
                    if captured_w == width && captured_h == height && data.len() >= expected_size {
                        encoder.write_frame(&data[..expected_size]);
                    } else if data.len() >= captured_size {
                        // Dimensions don't match - this shouldn't happen if initialized correctly
                        tracing::warn!(
                            "Frame dimensions mismatch: {}x{} vs expected {}x{}, skipping frame",
                            captured_w, captured_h, width, height
                        );
                    } else {
                        tracing::warn!(
                            "Frame data size mismatch: {} vs expected {}",
                            data.len(),
                            expected_size
                        );
                    }
                }

                // Log progress periodically
                let count = encoder.frame_count();
                if count.is_multiple_of(60) && count > 0 {
                    tracing::debug!(
                        "Captured {} frames ({:.1}s) at {}x{}",
                        count,
                        count as f64 / fps as f64,
                        width,
                        height
                    );
                }

                // Sleep for remaining frame time
                let elapsed = start.elapsed();
                if elapsed < frame_interval {
                    tokio::time::sleep(frame_interval - elapsed).await;
                }
            }
        });

        self.capture_handle = Some(handle);

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

        // Wait for capture task to finish
        if let Some(handle) = self.capture_handle.take() {
            let _ = handle.await;
        }

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

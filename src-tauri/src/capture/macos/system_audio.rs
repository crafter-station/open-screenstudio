//! macOS System Audio Capture using ScreenCaptureKit
//!
//! Uses Apple's ScreenCaptureKit framework (macOS 12.3+) to capture system audio
//! natively without requiring external virtual audio devices like BlackHole.
//!
//! ## Audio Format Handling
//!
//! ScreenCaptureKit can output audio in two formats:
//! - **Interleaved**: Single buffer with stereo samples (LRLRLR...)
//! - **Non-interleaved**: Separate buffers per channel (LLLL... and RRRR...)
//!
//! This module handles both formats and converts to interleaved stereo for FFmpeg.

use crate::capture::audio::AudioEncoder;
use crate::recorder::channel::{ChannelType, RecordingChannel, RecordingError, RecordingResult};
use async_trait::async_trait;
use parking_lot::Mutex as ParkingMutex;
use screencapturekit::cm::{AudioBuffer, AudioBufferList, CMFormatDescription};
use screencapturekit::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Check if system audio capture is available
/// Returns true on macOS 12.3+ (ScreenCaptureKit is available)
pub fn is_system_audio_available() -> bool {
    // ScreenCaptureKit is available on macOS 12.3+
    // The screencapturekit crate handles version checking internally
    true
}

/// Audio output handler that receives audio samples from ScreenCaptureKit
struct AudioOutputHandler {
    encoder: Arc<ParkingMutex<Option<Arc<AudioEncoder>>>>,
    is_recording: Arc<AtomicBool>,
    sample_count: Arc<AtomicU64>,
    format_logged: AtomicBool,
}

impl AudioOutputHandler {
    fn new(
        encoder: Arc<ParkingMutex<Option<Arc<AudioEncoder>>>>,
        is_recording: Arc<AtomicBool>,
        sample_count: Arc<AtomicU64>,
    ) -> Self {
        Self {
            encoder,
            is_recording,
            sample_count,
            format_logged: AtomicBool::new(false),
        }
    }

    /// Interleave non-interleaved stereo audio buffers (f32 samples)
    /// Input: Two buffers [L0,L1,L2,...] and [R0,R1,R2,...]
    /// Output: Interleaved bytes [L0,R0,L1,R1,L2,R2,...]
    fn interleave_stereo_f32(left: &[u8], right: &[u8]) -> Vec<u8> {
        let sample_count = left.len() / 4; // f32 = 4 bytes
        let mut interleaved = Vec::with_capacity(left.len() + right.len());

        for i in 0..sample_count {
            let offset = i * 4;
            // Copy left sample (4 bytes)
            interleaved.extend_from_slice(&left[offset..offset + 4]);
            // Copy right sample (4 bytes)
            interleaved.extend_from_slice(&right[offset..offset + 4]);
        }

        interleaved
    }

    /// Log audio format information (called once on first buffer)
    fn log_audio_format(
        &self,
        audio_buffer_list: &AudioBufferList,
        format_desc: Option<&CMFormatDescription>,
    ) {
        let num_buffers = audio_buffer_list.num_buffers();
        let first_buffer = audio_buffer_list.get(0);

        let channels_per_buffer = first_buffer.map(|b| b.number_channels).unwrap_or(0);
        let bytes_per_buffer = first_buffer.map(|b| b.data_bytes_size).unwrap_or(0);
        let is_interleaved = num_buffers == 1 && channels_per_buffer >= 2;

        if let Some(fd) = format_desc {
            tracing::info!(
                "ScreenCaptureKit audio format: buffers={}, ch/buffer={}, bytes/buffer={}, \
                 sample_rate={:?}Hz, total_channels={:?}, bits={:?}, \
                 float={}, big_endian={}, interleaved={}",
                num_buffers,
                channels_per_buffer,
                bytes_per_buffer,
                fd.audio_sample_rate(),
                fd.audio_channel_count(),
                fd.audio_bits_per_channel(),
                fd.audio_is_float(),
                fd.audio_is_big_endian(),
                is_interleaved
            );

            // Warn if format doesn't match expected 48kHz f32le
            let sample_rate = fd.audio_sample_rate().unwrap_or(0.0) as u32;
            let is_float = fd.audio_is_float();
            let is_big_endian = fd.audio_is_big_endian();

            if sample_rate != 48000 || !is_float || is_big_endian {
                tracing::warn!(
                    "Audio format differs from expected 48kHz/f32le! Actual: {}Hz/{}/{}",
                    sample_rate,
                    if is_float { "float" } else { "int" },
                    if is_big_endian { "big-endian" } else { "little-endian" }
                );
            }
        } else {
            tracing::info!(
                "ScreenCaptureKit audio: buffers={}, ch/buffer={}, bytes/buffer={}, interleaved={} \
                 (no format description available)",
                num_buffers,
                channels_per_buffer,
                bytes_per_buffer,
                is_interleaved
            );
        }
    }

    /// Process audio buffer list, handling both interleaved and non-interleaved formats
    fn process_audio_buffers(
        &self,
        audio_buffer_list: &AudioBufferList,
        format_desc: Option<&CMFormatDescription>,
    ) {
        let num_buffers = audio_buffer_list.num_buffers();

        // Log format info once on first buffer
        if !self.format_logged.swap(true, Ordering::Relaxed) {
            self.log_audio_format(audio_buffer_list, format_desc);
        }

        if let Some(encoder) = self.encoder.lock().as_ref() {
            match num_buffers {
                0 => {
                    // No buffers - nothing to process
                }
                1 => {
                    // Single buffer: already interleaved stereo or mono
                    if let Some(buffer) = audio_buffer_list.get(0) {
                        let data: &[u8] = buffer.data();
                        if !data.is_empty() {
                            encoder.write_samples(data);
                            self.sample_count
                                .fetch_add((data.len() / 4) as u64, Ordering::Relaxed);
                        }
                    }
                }
                2 => {
                    // Two buffers: non-interleaved stereo (one buffer per channel)
                    // MUST interleave before sending to FFmpeg!
                    let left: Option<&[u8]> =
                        audio_buffer_list.get(0).map(|b: &AudioBuffer| b.data());
                    let right: Option<&[u8]> =
                        audio_buffer_list.get(1).map(|b: &AudioBuffer| b.data());

                    if let (Some(left_data), Some(right_data)) = (left, right) {
                        if !left_data.is_empty() && left_data.len() == right_data.len() {
                            let interleaved = Self::interleave_stereo_f32(left_data, right_data);
                            encoder.write_samples(&interleaved);
                            self.sample_count
                                .fetch_add((interleaved.len() / 4) as u64, Ordering::Relaxed);
                        } else if left_data.len() != right_data.len() {
                            tracing::warn!(
                                "Mismatched audio buffer sizes: left={}, right={}",
                                left_data.len(),
                                right_data.len()
                            );
                        }
                    }
                }
                n => {
                    // Multi-channel (5.1, 7.1, etc.): downmix to stereo using first two channels
                    tracing::warn!(
                        "Multi-channel audio ({} buffers) - using first two channels only",
                        n
                    );
                    let left: Option<&[u8]> =
                        audio_buffer_list.get(0).map(|b: &AudioBuffer| b.data());
                    let right: Option<&[u8]> =
                        audio_buffer_list.get(1).map(|b: &AudioBuffer| b.data());

                    if let (Some(left_data), Some(right_data)) = (left, right) {
                        if !left_data.is_empty() && left_data.len() == right_data.len() {
                            let interleaved = Self::interleave_stereo_f32(left_data, right_data);
                            encoder.write_samples(&interleaved);
                            self.sample_count
                                .fetch_add((interleaved.len() / 4) as u64, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
    }
}

impl SCStreamOutputTrait for AudioOutputHandler {
    fn did_output_sample_buffer(
        &self,
        sample_buffer: CMSampleBuffer,
        of_type: SCStreamOutputType,
    ) {
        // Only process audio samples
        if of_type != SCStreamOutputType::Audio {
            return;
        }

        if !self.is_recording.load(Ordering::Relaxed) {
            return;
        }

        // Get audio data from the sample buffer
        if let Some(audio_buffer_list) = sample_buffer.audio_buffer_list() {
            let format_desc = sample_buffer.format_description();
            self.process_audio_buffers(&audio_buffer_list, format_desc.as_ref());
        }
    }
}

/// System audio capture channel for macOS using ScreenCaptureKit
pub struct SystemAudioCaptureChannel {
    id: String,
    display_id: u32,
    is_recording: Arc<AtomicBool>,
    output_dir: Option<PathBuf>,
    session_index: usize,
    output_files: Arc<ParkingMutex<Vec<String>>>,
    encoder: Arc<ParkingMutex<Option<Arc<AudioEncoder>>>>,
    stream: ParkingMutex<Option<SCStream>>,
    sample_count: Arc<AtomicU64>,
}

impl SystemAudioCaptureChannel {
    /// Create a new system audio capture channel
    pub fn new(display_id: u32) -> Self {
        Self {
            id: "system-audio".to_string(),
            display_id,
            is_recording: Arc::new(AtomicBool::new(false)),
            output_dir: None,
            session_index: 0,
            output_files: Arc::new(ParkingMutex::new(Vec::new())),
            encoder: Arc::new(ParkingMutex::new(None)),
            stream: ParkingMutex::new(None),
            sample_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Check if system audio capture is available
    pub fn is_available(&self) -> bool {
        is_system_audio_available()
    }
}

impl Default for SystemAudioCaptureChannel {
    fn default() -> Self {
        Self::new(1) // Default to primary display
    }
}

#[async_trait]
impl RecordingChannel for SystemAudioCaptureChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::SystemAudio
    }

    async fn initialize(&mut self, output_dir: &Path, session_index: usize) -> RecordingResult<()> {
        self.output_dir = Some(output_dir.to_path_buf());
        self.session_index = session_index;

        tracing::info!(
            "System audio channel initialized with ScreenCaptureKit for display {}",
            self.display_id
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

        // Warn about potential Bluetooth audio interference
        tracing::warn!(
            "Starting system audio capture via ScreenCaptureKit. \
             Note: This may interfere with Bluetooth audio devices (AirPods, etc.). \
             If you experience audio issues, try using wired speakers/headphones."
        );

        // Get shareable content to find the display
        let content = SCShareableContent::get().map_err(|e| {
            RecordingError::CaptureError(format!("Failed to get shareable content: {:?}", e))
        })?;

        let displays = content.displays();
        if displays.is_empty() {
            return Err(RecordingError::DeviceNotFound(
                "No displays found".to_string(),
            ));
        }

        // Find the display by ID, or use the first one
        let target_display = displays
            .iter()
            .find(|d| d.display_id() == self.display_id)
            .or_else(|| displays.first())
            .ok_or_else(|| RecordingError::DeviceNotFound("Display not found".to_string()))?;

        tracing::info!(
            "Using display {} for system audio capture",
            target_display.display_id()
        );

        // Create content filter for the display
        let filter = SCContentFilter::create()
            .with_display(target_display)
            .with_excluding_windows(&[])
            .build();

        // Create stream configuration for audio capture
        // We use minimal video settings since we only want audio
        let config = SCStreamConfiguration::new()
            .with_width(2) // Minimal width
            .with_height(2) // Minimal height
            .with_minimum_frame_interval(&CMTime::new(1, 1)) // 1 fps - minimal video
            .with_captures_audio(true)
            .with_sample_rate(48000)
            .with_channel_count(2)
            .with_excludes_current_process_audio(true); // Don't capture our own audio

        // Create the stream
        let mut stream = SCStream::new(&filter, &config);

        // Create encoder (48kHz stereo)
        let encoder = Arc::new(
            AudioEncoder::new(48000, 2, &output_dir, self.session_index, "system").map_err(
                |e| RecordingError::CaptureError(format!("Failed to start audio encoder: {}", e)),
            )?,
        );
        *self.encoder.lock() = Some(encoder.clone());

        self.is_recording.store(true, Ordering::SeqCst);
        self.sample_count.store(0, Ordering::SeqCst);

        // Create output handler with proper interleaving support
        let output_handler = AudioOutputHandler::new(
            self.encoder.clone(),
            self.is_recording.clone(),
            self.sample_count.clone(),
        );

        // Add output handler for audio
        stream.add_output_handler(output_handler, SCStreamOutputType::Audio);

        // Start capture
        stream.start_capture().map_err(|e| {
            RecordingError::CaptureError(format!(
                "Failed to start ScreenCaptureKit stream: {:?}",
                e
            ))
        })?;

        *self.stream.lock() = Some(stream);

        tracing::info!("System audio capture started with ScreenCaptureKit");
        Ok(())
    }

    async fn stop(&mut self) -> RecordingResult<()> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.is_recording.store(false, Ordering::SeqCst);

        // Stop the stream
        if let Some(stream) = self.stream.lock().take() {
            if let Err(e) = stream.stop_capture() {
                tracing::warn!("Error stopping ScreenCaptureKit stream: {:?}", e);
            }
        }

        // Finish encoding
        if let Some(ref encoder) = *self.encoder.lock() {
            if let Ok(Some(output_file)) = encoder.finish() {
                let sample_count = self.sample_count.load(Ordering::SeqCst);
                tracing::info!(
                    "System audio encoding finished: {} samples, output: {}",
                    sample_count,
                    output_file
                );
                self.output_files.lock().push(output_file);
            }
        }
        *self.encoder.lock() = None;

        tracing::info!("System audio capture stopped");
        Ok(())
    }

    async fn pause(&mut self) -> RecordingResult<()> {
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

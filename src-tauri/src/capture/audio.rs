//! Cross-platform audio capture using cpal
//!
//! This module provides microphone capture functionality using the cpal crate.
//! System audio capture is handled separately by platform-specific modules.

use crate::capture::traits::AudioDeviceInfo;
use crate::recorder::channel::{ChannelType, RecordingChannel, RecordingError, RecordingResult};
use async_trait::async_trait;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use parking_lot::Mutex as ParkingMutex;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Get list of available audio input devices
pub fn get_audio_input_devices() -> Vec<AudioDeviceInfo> {
    let host = cpal::default_host();
    let mut devices = Vec::new();

    // Get default input device name for comparison
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok());

    if let Ok(input_devices) = host.input_devices() {
        for device in input_devices {
            if let Ok(name) = device.name() {
                let is_default = default_name.as_ref() == Some(&name);
                devices.push(AudioDeviceInfo {
                    id: name.clone(),
                    name: name.clone(),
                    is_input: true,
                    is_default,
                });
            }
        }
    }

    devices
}

/// Get the default audio input device
pub fn get_default_input_device() -> Option<Device> {
    let host = cpal::default_host();
    host.default_input_device()
}

/// Get an audio input device by name
pub fn get_input_device_by_name(name: &str) -> Option<Device> {
    let host = cpal::default_host();
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(device_name) = device.name() {
                if device_name == name {
                    return Some(device);
                }
            }
        }
    }
    None
}

/// FFmpeg encoder for audio
pub struct AudioEncoder {
    process: ParkingMutex<Option<Child>>,
    sample_count: AtomicU64,
    running: AtomicBool,
    output_path: PathBuf,
}

impl AudioEncoder {
    pub fn new(
        sample_rate: u32,
        channels: u16,
        output_dir: &Path,
        session_index: usize,
        suffix: &str,
    ) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(output_dir)?;

        let output_path = output_dir.join(format!("recording-{}-{}.m4a", session_index, suffix));

        // Start FFmpeg process for audio encoding
        // Input: 32-bit float PCM from cpal
        // Output: AAC in M4A container
        let process = Command::new("ffmpeg")
            .args([
                "-y",                            // Overwrite output
                "-f", "f32le",                   // 32-bit float little-endian PCM
                "-ar", &sample_rate.to_string(), // Sample rate
                "-ac", &channels.to_string(),   // Channel count
                "-i", "-",                       // Read from stdin
                "-c:a", "aac",                   // AAC codec
                "-b:a", "192k",                  // 192kbps bitrate
                "-movflags", "+faststart",       // For streaming
                output_path.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;

        tracing::info!(
            "Started audio encoder: {}Hz {}ch, output: {:?}",
            sample_rate,
            channels,
            output_path
        );

        Ok(Self {
            process: ParkingMutex::new(Some(process)),
            sample_count: AtomicU64::new(0),
            running: AtomicBool::new(true),
            output_path,
        })
    }

    pub fn write_samples(&self, data: &[u8]) -> bool {
        if !self.running.load(Ordering::Relaxed) {
            return false;
        }

        let mut guard = self.process.lock();
        if let Some(ref mut process) = *guard {
            if let Some(ref mut stdin) = process.stdin {
                if stdin.write_all(data).is_ok() {
                    self.sample_count.fetch_add((data.len() / 4) as u64, Ordering::Relaxed);
                    return true;
                }
            }
        }
        false
    }

    pub fn sample_count(&self) -> u64 {
        self.sample_count.load(Ordering::Relaxed)
    }

    pub fn finish(&self) -> Result<Option<String>, std::io::Error> {
        self.running.store(false, Ordering::Relaxed);
        let mut guard = self.process.lock();
        if let Some(mut process) = guard.take() {
            drop(process.stdin.take());
            let output = process.wait_with_output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("FFmpeg audio exited with status {}: {}", output.status, stderr);
            }
        }

        if self.output_path.exists() && self.sample_count() > 0 {
            tracing::info!(
                "Audio encoding finished: {} samples, output: {:?}",
                self.sample_count(),
                self.output_path
            );
            Ok(Some(self.output_path.to_string_lossy().to_string()))
        } else {
            Ok(None)
        }
    }
}

/// Microphone capture channel
/// 
/// Uses a background thread for the audio stream since cpal::Stream is not Send.
pub struct MicrophoneCaptureChannel {
    id: String,
    device_id: Option<String>,
    is_recording: Arc<AtomicBool>,
    output_dir: Option<PathBuf>,
    session_index: usize,
    output_files: Arc<ParkingMutex<Vec<String>>>,
    encoder: Arc<ParkingMutex<Option<Arc<AudioEncoder>>>>,
    stream_handle: Arc<ParkingMutex<Option<std::thread::JoinHandle<()>>>>,
    sample_rate: u32,
    channels: u16,
}

impl MicrophoneCaptureChannel {
    /// Create a new microphone capture channel
    /// If device_id is None, uses the default input device
    pub fn new(device_id: Option<String>) -> Self {
        Self {
            id: "microphone".to_string(),
            device_id,
            is_recording: Arc::new(AtomicBool::new(false)),
            output_dir: None,
            session_index: 0,
            output_files: Arc::new(ParkingMutex::new(Vec::new())),
            encoder: Arc::new(ParkingMutex::new(None)),
            stream_handle: Arc::new(ParkingMutex::new(None)),
            sample_rate: 48000,
            channels: 2,
        }
    }

    fn get_device(&self) -> RecordingResult<Device> {
        match &self.device_id {
            Some(name) => get_input_device_by_name(name).ok_or_else(|| {
                RecordingError::DeviceNotFound(format!("Audio device '{}' not found", name))
            }),
            None => get_default_input_device().ok_or_else(|| {
                RecordingError::DeviceNotFound("No default audio input device".to_string())
            }),
        }
    }
}

#[async_trait]
impl RecordingChannel for MicrophoneCaptureChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Microphone
    }

    async fn initialize(&mut self, output_dir: &Path, session_index: usize) -> RecordingResult<()> {
        // Check if FFmpeg is available
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return Err(RecordingError::ConfigurationError(
                "FFmpeg not found. Please install FFmpeg.".to_string(),
            ));
        }

        // Verify device exists
        let device = self.get_device()?;
        let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());

        // Get supported config
        let config = device.default_input_config().map_err(|e| {
            RecordingError::ConfigurationError(format!("Failed to get audio config: {}", e))
        })?;

        self.sample_rate = config.sample_rate().0;
        self.channels = config.channels();
        self.output_dir = Some(output_dir.to_path_buf());
        self.session_index = session_index;

        tracing::info!(
            "Microphone channel initialized: {} ({}Hz, {}ch)",
            device_name,
            self.sample_rate,
            self.channels
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

        // Create encoder
        let encoder = Arc::new(
            AudioEncoder::new(
                self.sample_rate,
                self.channels,
                &output_dir,
                self.session_index,
                "mic",
            )
            .map_err(|e| RecordingError::CaptureError(format!("Failed to start audio encoder: {}", e)))?,
        );
        *self.encoder.lock() = Some(encoder.clone());

        self.is_recording.store(true, Ordering::SeqCst);

        // Clone values for the thread
        let device_id = self.device_id.clone();
        let is_recording = self.is_recording.clone();

        // Spawn a thread to handle the audio stream (cpal::Stream is not Send)
        let handle = std::thread::spawn(move || {
            let device = match &device_id {
                Some(name) => get_input_device_by_name(name),
                None => get_default_input_device(),
            };

            let device = match device {
                Some(d) => d,
                None => {
                    tracing::error!("Failed to get audio device in thread");
                    return;
                }
            };

            let config = match device.default_input_config() {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to get audio config: {}", e);
                    return;
                }
            };

            let sample_format = config.sample_format();
            let stream_config: StreamConfig = config.into();

            // Log the actual stream configuration for debugging
            tracing::info!(
                "Microphone stream config: format={:?}, sample_rate={}, channels={}",
                sample_format,
                stream_config.sample_rate.0,
                stream_config.channels
            );

            // Callback counter for diagnostic logging
            let callback_count = Arc::new(AtomicU64::new(0));

            let stream = match sample_format {
                SampleFormat::F32 => {
                    let encoder_clone = encoder.clone();
                    let is_rec = is_recording.clone();
                    let cc = callback_count.clone();
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            let count = cc.fetch_add(1, Ordering::Relaxed);
                            // Log first callback and then every 500th to confirm mic is working
                            if count == 0 {
                                tracing::info!("Microphone: first callback received - capture working!");
                            } else if count % 500 == 0 {
                                tracing::debug!("Microphone: {} callbacks, {} samples this batch", count, data.len());
                            }
                            
                            if is_rec.load(Ordering::Relaxed) {
                                let bytes: Vec<u8> = data
                                    .iter()
                                    .flat_map(|&sample| sample.to_le_bytes())
                                    .collect();
                                encoder_clone.write_samples(&bytes);
                            }
                        },
                        |err| tracing::error!("Microphone stream error: {}", err),
                        None,
                    )
                }
                SampleFormat::I16 => {
                    let encoder_clone = encoder.clone();
                    let is_rec = is_recording.clone();
                    let cc = callback_count.clone();
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            let count = cc.fetch_add(1, Ordering::Relaxed);
                            if count == 0 {
                                tracing::info!("Microphone: first callback received - capture working!");
                            } else if count % 500 == 0 {
                                tracing::debug!("Microphone: {} callbacks, {} samples this batch", count, data.len());
                            }
                            
                            if is_rec.load(Ordering::Relaxed) {
                                let bytes: Vec<u8> = data
                                    .iter()
                                    .map(|&sample| sample as f32 / i16::MAX as f32)
                                    .flat_map(|sample| sample.to_le_bytes())
                                    .collect();
                                encoder_clone.write_samples(&bytes);
                            }
                        },
                        |err| tracing::error!("Microphone stream error: {}", err),
                        None,
                    )
                }
                SampleFormat::U16 => {
                    let encoder_clone = encoder.clone();
                    let is_rec = is_recording.clone();
                    let cc = callback_count.clone();
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            let count = cc.fetch_add(1, Ordering::Relaxed);
                            if count == 0 {
                                tracing::info!("Microphone: first callback received - capture working!");
                            } else if count % 500 == 0 {
                                tracing::debug!("Microphone: {} callbacks, {} samples this batch", count, data.len());
                            }
                            
                            if is_rec.load(Ordering::Relaxed) {
                                let bytes: Vec<u8> = data
                                    .iter()
                                    .map(|&sample| (sample as f32 / u16::MAX as f32) * 2.0 - 1.0)
                                    .flat_map(|sample| sample.to_le_bytes())
                                    .collect();
                                encoder_clone.write_samples(&bytes);
                            }
                        },
                        |err| tracing::error!("Microphone stream error: {}", err),
                        None,
                    )
                }
                _ => {
                    tracing::error!("Unsupported microphone sample format: {:?}", sample_format);
                    return;
                }
            };

            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to build audio stream: {}", e);
                    return;
                }
            };

            if let Err(e) = stream.play() {
                tracing::error!("Failed to start microphone stream: {}", e);
                return;
            }

            tracing::info!("Microphone audio stream started successfully");

            // Keep thread alive while recording
            while is_recording.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            // Stream is dropped here, stopping capture
            tracing::info!("Microphone audio stream stopped");
        });

        *self.stream_handle.lock() = Some(handle);

        tracing::info!("Microphone capture started");
        Ok(())
    }

    async fn stop(&mut self) -> RecordingResult<()> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Err(RecordingError::NotRecording);
        }

        self.is_recording.store(false, Ordering::SeqCst);

        // Wait for stream thread to finish
        if let Some(handle) = self.stream_handle.lock().take() {
            let _ = handle.join();
        }

        // Finish encoding
        if let Some(ref encoder) = *self.encoder.lock() {
            if let Ok(Some(output_file)) = encoder.finish() {
                self.output_files.lock().push(output_file);
            }
        }
        *self.encoder.lock() = None;

        tracing::info!("Microphone capture stopped");
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

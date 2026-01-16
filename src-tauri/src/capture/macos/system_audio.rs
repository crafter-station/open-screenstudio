//! macOS System Audio Capture
//!
//! System audio capture on macOS is complex. Options:
//! 1. ScreenCaptureKit (macOS 12.3+) - Best but requires Swift interop
//! 2. Virtual audio devices (BlackHole, Soundflower) - Requires user setup
//! 3. Aggregate devices - Complex to set up programmatically
//!
//! For now, we detect BlackHole and use it if available.
//! TODO: Implement ScreenCaptureKit for native system audio capture.

use crate::capture::audio::AudioEncoder;
use crate::capture::traits::AudioDeviceInfo;
use crate::recorder::channel::{ChannelType, RecordingChannel, RecordingError, RecordingResult};
use async_trait::async_trait;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use parking_lot::Mutex as ParkingMutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Known virtual audio loopback device names
const LOOPBACK_DEVICES: &[&str] = &[
    "BlackHole 2ch",
    "BlackHole 16ch",
    "BlackHole",
    "Soundflower (2ch)",
    "Soundflower (64ch)",
    "Loopback Audio",
];

/// Check if a device is a known loopback device
fn is_loopback_device(name: &str) -> bool {
    LOOPBACK_DEVICES.iter().any(|&d| name.contains(d))
}

/// Check if system audio capture is available (loopback device found)
pub fn is_system_audio_available() -> bool {
    find_loopback_device().is_some()
}

/// Get available system audio loopback devices
pub fn get_system_audio_devices() -> Vec<AudioDeviceInfo> {
    let host = cpal::default_host();
    let mut devices = Vec::new();

    // Look for loopback devices in input devices
    if let Ok(input_devices) = host.input_devices() {
        for device in input_devices {
            if let Ok(name) = device.name() {
                if is_loopback_device(&name) {
                    devices.push(AudioDeviceInfo {
                        id: name.clone(),
                        name: name.clone(),
                        is_input: true,
                        is_default: false,
                    });
                }
            }
        }
    }

    devices
}

/// Find a loopback device for system audio capture
fn find_loopback_device() -> Option<Device> {
    let host = cpal::default_host();

    if let Ok(input_devices) = host.input_devices() {
        for device in input_devices {
            if let Ok(name) = device.name() {
                if is_loopback_device(&name) {
                    tracing::info!("Found loopback device: {}", name);
                    return Some(device);
                }
            }
        }
    }

    None
}

/// System audio capture channel for macOS
///
/// Uses virtual loopback devices (BlackHole, Soundflower) when available.
/// If no loopback device is found, capture is skipped with a warning.
pub struct SystemAudioCaptureChannel {
    id: String,
    device_name: Option<String>,
    is_recording: Arc<AtomicBool>,
    output_dir: Option<PathBuf>,
    session_index: usize,
    output_files: Arc<ParkingMutex<Vec<String>>>,
    encoder: Arc<ParkingMutex<Option<Arc<AudioEncoder>>>>,
    stream_handle: Arc<ParkingMutex<Option<std::thread::JoinHandle<()>>>>,
    sample_rate: u32,
    channels: u16,
    available: bool,
}

impl SystemAudioCaptureChannel {
    /// Create a new system audio capture channel
    pub fn new() -> Self {
        // Check if a loopback device is available
        let loopback = find_loopback_device();
        let (device_name, available) = match loopback {
            Some(device) => (device.name().ok(), true),
            None => {
                tracing::warn!(
                    "No loopback audio device found. System audio capture requires BlackHole or similar. \
                    Install BlackHole from https://existential.audio/blackhole/ and set it as your system output."
                );
                (None, false)
            }
        };

        Self {
            id: "system-audio".to_string(),
            device_name,
            is_recording: Arc::new(AtomicBool::new(false)),
            output_dir: None,
            session_index: 0,
            output_files: Arc::new(ParkingMutex::new(Vec::new())),
            encoder: Arc::new(ParkingMutex::new(None)),
            stream_handle: Arc::new(ParkingMutex::new(None)),
            sample_rate: 48000,
            channels: 2,
            available,
        }
    }

    /// Check if system audio capture is available
    pub fn is_available(&self) -> bool {
        self.available
    }

    fn get_device(&self) -> RecordingResult<Device> {
        match &self.device_name {
            Some(name) => {
                let host = cpal::default_host();
                if let Ok(devices) = host.input_devices() {
                    for device in devices {
                        if let Ok(dev_name) = device.name() {
                            if &dev_name == name {
                                return Ok(device);
                            }
                        }
                    }
                }
                Err(RecordingError::DeviceNotFound(format!(
                    "Loopback device '{}' not found",
                    name
                )))
            }
            None => Err(RecordingError::DeviceNotFound(
                "No loopback device configured".to_string(),
            )),
        }
    }
}

impl Default for SystemAudioCaptureChannel {
    fn default() -> Self {
        Self::new()
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
        if !self.available {
            tracing::warn!("System audio capture not available - no loopback device found");
            return Ok(());
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
            "System audio channel initialized: {} ({}Hz, {}ch)",
            device_name,
            self.sample_rate,
            self.channels
        );
        Ok(())
    }

    async fn start(&mut self) -> RecordingResult<()> {
        if !self.available {
            tracing::warn!("Skipping system audio capture - no loopback device");
            return Ok(());
        }

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
                "system",
            )
            .map_err(|e| {
                RecordingError::CaptureError(format!("Failed to start audio encoder: {}", e))
            })?,
        );
        *self.encoder.lock() = Some(encoder.clone());

        self.is_recording.store(true, Ordering::SeqCst);

        // Clone values for the thread
        let device_name = self.device_name.clone();
        let is_recording = self.is_recording.clone();

        // Spawn a thread to handle the audio stream
        let handle = std::thread::spawn(move || {
            let host = cpal::default_host();
            let device = if let Some(ref name) = device_name {
                host.input_devices()
                    .ok()
                    .and_then(|mut devices| devices.find(|d| d.name().ok().as_ref() == Some(name)))
            } else {
                None
            };

            let device = match device {
                Some(d) => d,
                None => {
                    tracing::error!("Failed to get loopback device in thread");
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

            let stream = match sample_format {
                SampleFormat::F32 => {
                    let encoder_clone = encoder.clone();
                    let is_rec = is_recording.clone();
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if is_rec.load(Ordering::Relaxed) {
                                let bytes: Vec<u8> = data
                                    .iter()
                                    .flat_map(|&sample| sample.to_le_bytes())
                                    .collect();
                                encoder_clone.write_samples(&bytes);
                            }
                        },
                        |err| tracing::error!("System audio stream error: {}", err),
                        None,
                    )
                }
                SampleFormat::I16 => {
                    let encoder_clone = encoder.clone();
                    let is_rec = is_recording.clone();
                    device.build_input_stream(
                        &stream_config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if is_rec.load(Ordering::Relaxed) {
                                let bytes: Vec<u8> = data
                                    .iter()
                                    .map(|&sample| sample as f32 / i16::MAX as f32)
                                    .flat_map(|sample| sample.to_le_bytes())
                                    .collect();
                                encoder_clone.write_samples(&bytes);
                            }
                        },
                        |err| tracing::error!("System audio stream error: {}", err),
                        None,
                    )
                }
                _ => {
                    tracing::error!("Unsupported sample format: {:?}", sample_format);
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
                tracing::error!("Failed to start audio stream: {}", e);
                return;
            }

            tracing::info!("System audio stream started");

            // Keep thread alive while recording
            while is_recording.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            tracing::info!("System audio stream stopped");
        });

        *self.stream_handle.lock() = Some(handle);

        tracing::info!("System audio capture started");
        Ok(())
    }

    async fn stop(&mut self) -> RecordingResult<()> {
        if !self.available {
            return Ok(());
        }

        if !self.is_recording.load(Ordering::SeqCst) {
            return Ok(());
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

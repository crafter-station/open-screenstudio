use crate::capture::input::types::{CursorInfo, MouseClick, MouseMove};
use crate::recorder::channel::{ChannelType, RecordingChannel, RecordingError, RecordingResult};
use async_trait::async_trait;
use parking_lot::Mutex as ParkingMutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
use crate::capture::macos::input as platform;

#[cfg(target_os = "windows")]
use crate::capture::windows::input as platform;

pub struct InputTrackingChannel {
    id: String,
    display_id: u32,
    is_recording: Arc<AtomicBool>,
    output_dir: Option<PathBuf>,
    session_index: usize,
    output_files: Arc<ParkingMutex<Vec<String>>>,

    mouse_moves: Arc<ParkingMutex<Vec<MouseMove>>>,
    mouse_clicks: Arc<ParkingMutex<Vec<MouseClick>>>,
    cursors: Arc<ParkingMutex<HashMap<String, CursorInfo>>>,

    thread_handle: Arc<ParkingMutex<Option<std::thread::JoinHandle<()>>>>,
    start_time: Arc<ParkingMutex<Option<Instant>>>,
}

impl InputTrackingChannel {
    pub fn new(display_id: u32) -> Self {
        Self {
            id: "input".to_string(),
            display_id,
            is_recording: Arc::new(AtomicBool::new(false)),
            output_dir: None,
            session_index: 0,
            output_files: Arc::new(ParkingMutex::new(Vec::new())),
            mouse_moves: Arc::new(ParkingMutex::new(Vec::new())),
            mouse_clicks: Arc::new(ParkingMutex::new(Vec::new())),
            cursors: Arc::new(ParkingMutex::new(HashMap::new())),
            thread_handle: Arc::new(ParkingMutex::new(None)),
            start_time: Arc::new(ParkingMutex::new(None)),
        }
    }

    fn session_basename(&self) -> String {
        format!("recording-{}", self.session_index)
    }

    fn now_unix_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> RecordingResult<()> {
        let data = serde_json::to_vec_pretty(value)
            .map_err(|e| RecordingError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
        std::fs::write(path, data)?;
        Ok(())
    }

    fn flush_to_disk(&mut self) -> RecordingResult<()> {
        let output_dir = self.output_dir.clone().ok_or_else(|| {
            RecordingError::ConfigurationError("Output directory not set".to_string())
        })?;

        std::fs::create_dir_all(&output_dir)?;

        let base = self.session_basename();

        let mouse_moves_path = output_dir.join(format!("{}-mouse-moves.json", base));
        let mouse_clicks_path = output_dir.join(format!("{}-mouse-clicks.json", base));
        let cursors_json_path = output_dir.join(format!("{}-cursors.json", base));
        let cursors_dir = output_dir.join(format!("{}-cursors", base));

        std::fs::create_dir_all(&cursors_dir)?;

        // Write event JSON files
        Self::write_json(&mouse_moves_path, &*self.mouse_moves.lock())?;
        Self::write_json(&mouse_clicks_path, &*self.mouse_clicks.lock())?;
        Self::write_json(&cursors_json_path, &*self.cursors.lock())?;

        // Cursor PNGs are saved during capture (platform impl)

        self.output_files.lock().push(mouse_moves_path.to_string_lossy().to_string());
        self.output_files.lock().push(mouse_clicks_path.to_string_lossy().to_string());
        self.output_files.lock().push(cursors_json_path.to_string_lossy().to_string());

        Ok(())
    }
}

#[async_trait]
impl RecordingChannel for InputTrackingChannel {
    fn id(&self) -> &str {
        &self.id
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Input
    }

    async fn initialize(&mut self, output_dir: &Path, session_index: usize) -> RecordingResult<()> {
        self.output_dir = Some(output_dir.to_path_buf());
        self.session_index = session_index;

        tracing::info!(
            "Input tracking channel initialized (display_id={}, session={})",
            self.display_id,
            self.session_index
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

        // Clear previous buffers
        self.mouse_moves.lock().clear();
        self.mouse_clicks.lock().clear();
        self.cursors.lock().clear();
        self.output_files.lock().clear();

        let base = self.session_basename();
        let cursors_dir = output_dir.join(format!("{}-cursors", base));
        std::fs::create_dir_all(&cursors_dir)?;

        let start_time = Instant::now();
        *self.start_time.lock() = Some(start_time);

        let is_recording = self.is_recording.clone();
        is_recording.store(true, Ordering::SeqCst);

        let mouse_moves = self.mouse_moves.clone();
        let mouse_clicks = self.mouse_clicks.clone();
        let cursors = self.cursors.clone();

        let handle = platform::start_input_tracking(
            is_recording.clone(),
            mouse_moves,
            mouse_clicks,
            cursors,
            cursors_dir,
            start_time,
            Duration::from_micros(8_333),
            Self::now_unix_ms,
        )?;

        *self.thread_handle.lock() = Some(handle);

        tracing::info!("Input tracking started");
        Ok(())
    }

    async fn stop(&mut self) -> RecordingResult<()> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.is_recording.store(false, Ordering::SeqCst);

        if let Some(handle) = self.thread_handle.lock().take() {
            let _ = handle.join();
        }

        self.flush_to_disk()?;

        tracing::info!(
            "Input tracking stopped (moves={}, clicks={}, cursors={})",
            self.mouse_moves.lock().len(),
            self.mouse_clicks.lock().len(),
            self.cursors.lock().len()
        );
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

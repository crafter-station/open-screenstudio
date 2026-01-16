use crate::capture::input::types::{CursorInfo, MouseClick, MouseMove};
use crate::recorder::channel::{RecordingError, RecordingResult};
use parking_lot::Mutex as ParkingMutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn start_input_tracking(
    _is_recording: Arc<AtomicBool>,
    _mouse_moves: Arc<ParkingMutex<Vec<MouseMove>>>,
    _mouse_clicks: Arc<ParkingMutex<Vec<MouseClick>>>,
    _cursors: Arc<ParkingMutex<HashMap<String, CursorInfo>>>,
    _cursors_dir: PathBuf,
    _start_time: Instant,
    _poll_interval: Duration,
    _unix_ms_fn: fn() -> u64,
) -> RecordingResult<std::thread::JoinHandle<()>> {
    Err(RecordingError::PlatformError(
        "Windows input tracking not implemented yet".to_string(),
    ))
}

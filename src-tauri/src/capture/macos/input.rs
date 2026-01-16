use crate::capture::input::types::{CursorInfo, MouseClick, MouseMove};
use crate::recorder::channel::RecordingResult;
use objc2::rc::Retained;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSCursor, NSEvent, NSImage};
use objc2_foundation::{NSDictionary, NSString};
use parking_lot::Mutex as ParkingMutex;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Start input tracking thread (macOS)
///
/// This implementation uses polling for mouse moves at a fixed interval.
/// Click detection is currently best-effort via NSEvent modifier flags and mouse state.
///
/// Note: A full CGEventTap-based implementation may require additional FFI.
pub fn start_input_tracking(
    is_recording: Arc<AtomicBool>,
    mouse_moves: Arc<ParkingMutex<Vec<MouseMove>>>,
    mouse_clicks: Arc<ParkingMutex<Vec<MouseClick>>>,
    cursors: Arc<ParkingMutex<HashMap<String, CursorInfo>>>,
    cursors_dir: PathBuf,
    start_time: Instant,
    poll_interval: Duration,
    unix_ms_fn: fn() -> u64,
) -> RecordingResult<std::thread::JoinHandle<()>> {
    // Ensure cursor directory exists
    std::fs::create_dir_all(&cursors_dir)?;

    let handle = std::thread::spawn(move || {
        tracing::info!(
            "macOS input tracking started (poll_interval={:?})",
            poll_interval
        );

        let mut last_left_down = false;
        let mut last_right_down = false;
        // Track which cursor hashes we've already saved to avoid duplicates
        let mut saved_cursor_hashes: HashSet<u64> = HashSet::new();

        while is_recording.load(Ordering::Relaxed) {
            let loop_start = Instant::now();

            // Mouse position from NSEvent
            // NOTE: location is in global screen coordinates
            let pos = unsafe { NSEvent::mouseLocation() };
            let x = pos.x;
            let y = pos.y;

            // Cursor capture (only on change, using image hash for deduplication)
            // currentSystemCursor returns Option<Retained<NSCursor>>
            let cursor_opt = unsafe { NSCursor::currentSystemCursor() };
            let (cursor_id, cursor_hash) = if let Some(ref cursor) = cursor_opt {
                cursor_id_and_hash(cursor)
            } else {
                ("unknown".to_string(), 0)
            };
            
            // Only save cursor if we haven't seen this exact image before
            if !saved_cursor_hashes.contains(&cursor_hash) && cursor_hash != 0 {
                if let Some(ref cursor) = cursor_opt {
                    if let Some(info) = capture_cursor_png(cursor, &cursor_id, &cursors_dir) {
                        cursors.lock().insert(cursor_id.clone(), info);
                        saved_cursor_hashes.insert(cursor_hash);
                    }
                }
            }

            // Modifier keys (class method in objc2-app-kit v0.2)
            let modifiers = modifiers_from_flags(unsafe { NSEvent::modifierFlags_class() });

            // Record mouse move
            let move_event = MouseMove {
                x,
                y,
                cursor_id: cursor_id.clone(),
                active_modifiers: modifiers.clone(),
                process_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                unix_time_ms: unix_ms_fn(),
            };
            mouse_moves.lock().push(move_event);

            // Best-effort click detection via pressedMouseButtons
            // Bit 0 = left, bit 1 = right, bit 2 = middle
            let buttons = unsafe { NSEvent::pressedMouseButtons() };
            let left_down = (buttons & 1) != 0;
            let right_down = (buttons & 2) != 0;

            if left_down != last_left_down {
                mouse_clicks.lock().push(MouseClick {
                    x,
                    y,
                    button: "left".to_string(),
                    event_type: if left_down { "down".to_string() } else { "up".to_string() },
                    click_count: 1,
                    active_modifiers: modifiers.clone(),
                    process_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                    unix_time_ms: unix_ms_fn(),
                });
                last_left_down = left_down;
            }

            if right_down != last_right_down {
                mouse_clicks.lock().push(MouseClick {
                    x,
                    y,
                    button: "right".to_string(),
                    event_type: if right_down { "down".to_string() } else { "up".to_string() },
                    click_count: 1,
                    active_modifiers: modifiers.clone(),
                    process_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                    unix_time_ms: unix_ms_fn(),
                });
                last_right_down = right_down;
            }

            let elapsed = loop_start.elapsed();
            if elapsed < poll_interval {
                std::thread::sleep(poll_interval - elapsed);
            }
        }

        tracing::info!("macOS input tracking thread stopped");
    });

    Ok(handle)
}

/// Generate a stable cursor ID and hash based on image content.
/// Returns (cursor_id, image_hash) where the hash is used for deduplication.
fn cursor_id_and_hash(cursor: &Retained<NSCursor>) -> (String, u64) {
    unsafe {
        let image: Retained<NSImage> = cursor.image();
        
        // Get TIFF data for hashing
        if let Some(tiff_data) = image.TIFFRepresentation() {
            let len = tiff_data.length() as usize;
            let mut buf = vec![0u8; len.min(4096)]; // Hash first 4KB for speed
            let copy_len = buf.len();
            tiff_data.getBytes_length(
                NonNull::new(buf.as_mut_ptr().cast()).unwrap(),
                copy_len as _,
            );
            
            // Compute hash using std::hash
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            buf.hash(&mut hasher);
            // Also include hotspot in hash for cursors with same image but different hotspots
            let hotspot = cursor.hotSpot();
            ((hotspot.x as i32).hash(&mut hasher));
            ((hotspot.y as i32).hash(&mut hasher));
            let hash = hasher.finish();
            
            let cursor_id = format!("cursor_{:016x}", hash);
            return (cursor_id, hash);
        }
        
        // Fallback to pointer-based ID if we can't get image data
        let ptr = Retained::as_ptr(cursor);
        (format!("cursor_{:p}", ptr), 0)
    }
}

fn capture_cursor_png(cursor: &Retained<NSCursor>, cursor_id: &str, cursors_dir: &PathBuf) -> Option<CursorInfo> {
    // All NSImage/NSBitmapImageRep calls require unsafe in objc2 v0.2
    unsafe {
        let hotspot = cursor.hotSpot();
        let image: Retained<NSImage> = cursor.image();
        let size = image.size();

        let file_name = format!("{}.png", cursor_id);
        let image_path = cursors_dir.join(&file_name);

        // Convert NSImage -> TIFF NSData
        let tiff_data = image.TIFFRepresentation()?;

        // Convert TIFF NSData -> NSBitmapImageRep
        let bitmap = NSBitmapImageRep::imageRepWithData(&tiff_data)?;

        // Encode NSBitmapImageRep -> PNG NSData
        let props: Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> = NSDictionary::dictionary();
        let png_data = bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &props)?;

        // Copy bytes out of NSData
        let len = png_data.length() as usize;
        let mut buf = vec![0u8; len];
        png_data.getBytes_length(
            NonNull::new(buf.as_mut_ptr().cast()).unwrap(),
            len as _,
        );

        if std::fs::write(&image_path, &buf).is_err() {
            return None;
        }

        Some(CursorInfo {
            id: cursor_id.to_string(),
            image_path: image_path.to_string_lossy().to_string(),
            hotspot_x: hotspot.x,
            hotspot_y: hotspot.y,
            width: size.width as u32,
            height: size.height as u32,
        })
    }
}

fn modifiers_from_flags(flags: objc2_app_kit::NSEventModifierFlags) -> Vec<String> {
    use objc2_app_kit::NSEventModifierFlags;
    let mut v = Vec::new();

    // NSEventModifierFlags in v0.2 is a newtype struct - use bitwise AND
    if (flags.0 & NSEventModifierFlags::NSEventModifierFlagShift.0) != 0 {
        v.push("shift".to_string());
    }
    if (flags.0 & NSEventModifierFlags::NSEventModifierFlagControl.0) != 0 {
        v.push("control".to_string());
    }
    if (flags.0 & NSEventModifierFlags::NSEventModifierFlagOption.0) != 0 {
        v.push("alt".to_string());
    }
    if (flags.0 & NSEventModifierFlags::NSEventModifierFlagCommand.0) != 0 {
        v.push("meta".to_string());
    }

    v
}


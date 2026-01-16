# Phase 3: Input Tracking Channel

## Overview

**Goal:** Implement high-frequency mouse input tracking during recording  
**Priority:** Essential for cursor smoothing (Phase 5) and auto-zoom (Phase 6)  
**Platforms:** macOS and Windows  
**Scope:** Mouse only (no keyboard tracking for privacy)  
**Polling Rate:** 120 Hz

Input tracking captures mouse movements, clicks, and cursor images. This data is saved to JSON files and used later for:

- Cursor smoothing with spring physics
- Automatic zoom detection (following clicks)
- Cursor rendering in exported videos

**Note:** Keyboard events are NOT captured to avoid privacy concerns.

---

## Learnings from Previous Phases

### Key Patterns Established

1. **RecordingChannel Trait** - All capture sources implement this trait with:
   - `initialize(output_dir, session_index)` - Set up output paths
   - `start()` / `stop()` / `pause()` / `resume()` - Lifecycle methods
   - `output_files()` - Return list of files created

2. **Platform-specific modules** - Use `#[cfg(target_os = "...")]` with:
   - `src-tauri/src/capture/macos/` for macOS
   - `src-tauri/src/capture/windows/` for Windows
   - Common trait in `src-tauri/src/capture/` root

3. **Threading pattern** - Use `std::thread::spawn` for non-Send types (like cpal streams), with `Arc<AtomicBool>` for stop signals

4. **FFmpeg for encoding** - Shell out to FFmpeg CLI (simpler than Rust bindings)

5. **JSON for metadata** - Use serde_json for structured data files

### Existing Type Definitions

From `src/types/project.ts`:

```typescript
interface InputChannelSession {
  mouseMovesFile: string;
  mouseClicksFile: string;
  // keystrokesFile: string;  // NOT IMPLEMENTED - privacy concerns
  durationMs: number;
}
```

---

## Data Structures

### 3.1 Rust Types (to add to `src-tauri/src/capture/input/types.rs`)

```rust
use serde::{Deserialize, Serialize};

/// Mouse movement event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseMove {
    /// X position in screen coordinates
    pub x: f64,
    /// Y position in screen coordinates
    pub y: f64,
    /// Cursor type identifier (e.g., "arrow", "ibeam", "pointer")
    pub cursor_id: String,
    /// Active modifier keys (e.g., ["shift", "cmd"])
    pub active_modifiers: Vec<String>,
    /// Time since recording started (milliseconds)
    pub process_time_ms: f64,
    /// Unix timestamp (milliseconds)
    pub unix_time_ms: u64,
}

/// Mouse click event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseClick {
    /// X position in screen coordinates
    pub x: f64,
    /// Y position in screen coordinates
    pub y: f64,
    /// Button: "left", "right", "middle"
    pub button: String,
    /// Event type: "down", "up"
    pub event_type: String,
    /// Click count (1 for single, 2 for double, etc.)
    pub click_count: u32,
    /// Active modifier keys
    pub active_modifiers: Vec<String>,
    /// Time since recording started (milliseconds)
    pub process_time_ms: f64,
    /// Unix timestamp (milliseconds)
    pub unix_time_ms: u64,
}

// KeyEvent NOT IMPLEMENTED - privacy concerns
// Keyboard tracking deliberately omitted

/// Cursor image metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorInfo {
    /// Unique cursor identifier
    pub id: String,
    /// Path to cursor image (relative to bundle)
    pub image_path: String,
    /// Cursor hotspot X (where the "click point" is)
    pub hotspot_x: f64,
    /// Cursor hotspot Y
    pub hotspot_y: f64,
    /// Image width
    pub width: u32,
    /// Image height
    pub height: u32,
}

/// Active modifier keys
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,      // Option on macOS
    pub meta: bool,     // Command on macOS, Windows key on Windows
}

impl Modifiers {
    pub fn to_vec(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.shift { v.push("shift".to_string()); }
        if self.control { v.push("control".to_string()); }
        if self.alt { v.push("alt".to_string()); }
        if self.meta { v.push("meta".to_string()); }
        v
    }
}
```

---

## Implementation Plan

### 3.2 Module Structure

```
src-tauri/src/capture/
├── mod.rs                    # Add input module export
├── input/
│   ├── mod.rs                # Platform-conditional exports
│   ├── types.rs              # Data structures above
│   └── channel.rs            # InputTrackingChannel trait impl
├── macos/
│   ├── mod.rs                # Add input export
│   └── input.rs              # macOS CGEvent implementation
└── windows/
    ├── mod.rs                # Add input export
    └── input.rs              # Windows hook implementation
```

### 3.3 InputTrackingChannel

**File:** `src-tauri/src/capture/input/channel.rs`

```rust
pub struct InputTrackingChannel {
    id: String,
    display_id: u32,
    is_recording: Arc<AtomicBool>,
    output_dir: Option<PathBuf>,
    session_index: usize,
    output_files: Arc<ParkingMutex<Vec<String>>>,

    // Event buffers (mouse only - no keyboard for privacy)
    mouse_moves: Arc<ParkingMutex<Vec<MouseMove>>>,
    mouse_clicks: Arc<ParkingMutex<Vec<MouseClick>>>,
    cursors: Arc<ParkingMutex<HashMap<String, CursorInfo>>>,

    // Tracking thread handle
    thread_handle: Arc<ParkingMutex<Option<JoinHandle<()>>>>,

    // Timing
    start_time: Arc<ParkingMutex<Option<Instant>>>,
}
```

Key implementation details:

- Buffer events in memory (avoid file I/O during recording)
- Flush to JSON files on stop
- Poll mouse position at 120Hz (~8.3ms interval)
- Listen for click events via platform event tap
- Capture cursor images when cursor type changes

### 3.4 macOS Implementation

**File:** `src-tauri/src/capture/macos/input.rs`

**Approach:** Use CGEvent tap for global event monitoring

```rust
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapPlacement,
    CGEventTapOptions, CGEventType, EventField,
};

pub fn start_input_tracking(
    is_recording: Arc<AtomicBool>,
    mouse_moves: Arc<ParkingMutex<Vec<MouseMove>>>,
    mouse_clicks: Arc<ParkingMutex<Vec<MouseClick>>>,
    cursors: Arc<ParkingMutex<HashMap<String, CursorInfo>>>,
    start_time: Instant,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        // Create event tap for mouse events only
        let event_mask = CGEventType::MouseMoved as u64
            | CGEventType::LeftMouseDown as u64
            | CGEventType::LeftMouseUp as u64
            | CGEventType::RightMouseDown as u64
            | CGEventType::RightMouseUp as u64
            | CGEventType::OtherMouseDown as u64
            | CGEventType::OtherMouseUp as u64;

        // Poll at 120Hz (8.33ms interval)
        let poll_interval = Duration::from_micros(8333);

        while is_recording.load(Ordering::Relaxed) {
            let start = Instant::now();

            // Get mouse position
            let position = get_mouse_position();
            let cursor_id = get_current_cursor_id(&cursors);
            let modifiers = get_active_modifiers();

            let event = MouseMove {
                x: position.0,
                y: position.1,
                cursor_id,
                active_modifiers: modifiers.to_vec(),
                process_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                unix_time_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            };

            mouse_moves.lock().push(event);

            // Sleep for remaining time in poll interval
            let elapsed = start.elapsed();
            if elapsed < poll_interval {
                std::thread::sleep(poll_interval - elapsed);
            }
        }
    })
}
```

**Mouse Position Polling (backup):**
If CGEventTap proves problematic, use polling:

```rust
use core_graphics::display::CGDisplay;

fn get_mouse_position() -> (f64, f64) {
    let event = CGEvent::new(CGEventSourceRef::null()).unwrap();
    let point = event.location();
    (point.x, point.y)
}
```

**Cursor Image Capture:**

```rust
// Get current cursor
extern "C" {
    fn CGSCurrentCursorSeed() -> u32;
    // or use NSCursor.currentSystemCursor
}
```

For cursor images, use `objc2-app-kit`'s `NSCursor`:

```rust
use objc2_app_kit::NSCursor;

fn capture_current_cursor() -> Option<CursorInfo> {
    let cursor = unsafe { NSCursor::currentSystemCursor() }?;
    let image = cursor.image();
    let hotspot = cursor.hotSpot();
    // ... save image to PNG, return CursorInfo
}
```

### 3.5 Windows Implementation

**File:** `src-tauri/src/capture/windows/input.rs`

**Approach:** Use low-level hooks via `SetWindowsHookEx`

```rust
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowsHookExW, UnhookWindowsHookEx, CallNextHookEx,
    GetCursorPos, WH_MOUSE_LL, WH_KEYBOARD_LL,
    MSLLHOOKSTRUCT, KBDLLHOOKSTRUCT,
};

// Low-level mouse hook callback
unsafe extern "system" fn mouse_hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Process mouse events
    CallNextHookEx(None, code, wparam, lparam)
}

// Low-level keyboard hook callback
unsafe extern "system" fn keyboard_hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Process keyboard events
    CallNextHookEx(None, code, wparam, lparam)
}
```

**Cursor Image Capture:**

```rust
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorInfo, CURSORINFO, GetIconInfo, ICONINFO,
};

fn capture_current_cursor() -> Option<CursorInfo> {
    let mut ci = CURSORINFO::default();
    ci.cbSize = std::mem::size_of::<CURSORINFO>() as u32;

    unsafe {
        GetCursorInfo(&mut ci).ok()?;
        // Get icon info for hotspot
        let mut ii = ICONINFO::default();
        GetIconInfo(ci.hCursor, &mut ii).ok()?;
        // ... render cursor to bitmap, save as PNG
    }
}
```

### 3.6 JSON Output Format

**mouse-moves.json:**

```json
[
  {
    "x": 512.5,
    "y": 384.2,
    "cursorId": "arrow",
    "activeModifiers": [],
    "processTimeMs": 0.0,
    "unixTimeMs": 1705000000000
  },
  {
    "x": 513.1,
    "y": 385.0,
    "cursorId": "arrow",
    "activeModifiers": [],
    "processTimeMs": 8.3,
    "unixTimeMs": 1705000000008
  }
]
```

**mouse-clicks.json:**

```json
[
  {
    "x": 600.0,
    "y": 400.0,
    "button": "left",
    "eventType": "down",
    "clickCount": 1,
    "activeModifiers": [],
    "processTimeMs": 1500.0,
    "unixTimeMs": 1705000001500
  },
  {
    "x": 600.0,
    "y": 400.0,
    "button": "left",
    "eventType": "up",
    "clickCount": 1,
    "activeModifiers": [],
    "processTimeMs": 1550.0,
    "unixTimeMs": 1705000001550
  }
]
```

**cursors.json:**

```json
{
  "arrow": {
    "id": "arrow",
    "imagePath": "cursors/arrow.png",
    "hotspotX": 0,
    "hotspotY": 0,
    "width": 32,
    "height": 32
  },
  "ibeam": {
    "id": "ibeam",
    "imagePath": "cursors/ibeam.png",
    "hotspotX": 16,
    "hotspotY": 16,
    "width": 32,
    "height": 32
  }
}
```

---

## Files to Create/Modify

### New Files

| File                                     | Purpose                      |
| ---------------------------------------- | ---------------------------- |
| `src-tauri/src/capture/input/mod.rs`     | Module exports               |
| `src-tauri/src/capture/input/types.rs`   | Data structures              |
| `src-tauri/src/capture/input/channel.rs` | InputTrackingChannel         |
| `src-tauri/src/capture/macos/input.rs`   | macOS CGEvent implementation |
| `src-tauri/src/capture/windows/input.rs` | Windows hook implementation  |

### Files to Modify

| File                                   | Changes                                 |
| -------------------------------------- | --------------------------------------- |
| `src-tauri/src/capture/mod.rs`         | Add `pub mod input;`                    |
| `src-tauri/src/capture/macos/mod.rs`   | Add `pub mod input;`                    |
| `src-tauri/src/capture/windows/mod.rs` | Add `pub mod input;`                    |
| `src-tauri/src/commands/recording.rs`  | Add InputTrackingChannel to coordinator |
| `src-tauri/Cargo.toml`                 | May need `png` crate for cursor images  |

---

## Dependencies

### Existing (already in Cargo.toml)

- `core-graphics` - macOS CGEvent
- `objc2-app-kit` - macOS NSCursor
- `windows` - Windows hooks (need to add features)
- `serde_json` - JSON serialization
- `parking_lot` - Thread-safe buffers

### To Add

```toml
# For cursor image encoding
png = "0.17"

# Windows features (update existing)
[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_UI_WindowsAndMessaging",
    "Win32_System_LibraryLoader",
    # NEW for input tracking:
    "Win32_UI_Input_KeyboardAndMouse",
] }
```

---

## Implementation Order

1. **Create type definitions** (`input/types.rs`)
2. **Create InputTrackingChannel skeleton** (`input/channel.rs`)
3. **Implement macOS mouse tracking** (`macos/input.rs`)
4. **Implement macOS cursor capture** (NSCursor)
5. **Wire up to recording coordinator** (`commands/recording.rs`)
6. **Test macOS implementation**
7. **Implement Windows mouse tracking** (`windows/input.rs`) - LATER
8. **Implement Windows cursor capture** - LATER
9. **Test Windows implementation** - LATER

**Note:** Focus on macOS first since that's the primary development platform. Windows can be added later.

---

## Testing Plan

### Unit Tests

```rust
#[test]
fn test_modifiers_to_vec() {
    let mods = Modifiers { shift: true, control: false, alt: true, meta: false };
    assert_eq!(mods.to_vec(), vec!["shift", "alt"]);
}

#[test]
fn test_mouse_move_serialization() {
    let event = MouseMove {
        x: 100.0, y: 200.0,
        cursor_id: "arrow".to_string(),
        active_modifiers: vec![],
        process_time_ms: 0.0,
        unix_time_ms: 1705000000000,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"x\":100"));
}
```

### Integration Tests

1. **Start recording with input tracking enabled**
2. **Move mouse around for 5 seconds**
3. **Click a few times**
4. **Stop recording**
5. **Verify JSON files exist and contain expected data:**
   - `recording/recording-0-mouse-moves.json` - Should have ~600 entries (5s × 120Hz)
   - `recording/recording-0-mouse-clicks.json` - Should have click events
   - `recording/cursors/` - Should have cursor PNGs
   - `recording/cursors.json` - Should have cursor metadata

### Manual Testing

```bash
bun run tauri:dev
# Start recording
# Move mouse, click, type
# Stop recording
# Check /tmp/open-screenstudio-*/recording/ for JSON files
cat /tmp/open-screenstudio-*/recording/recording-0-mouse-moves.json | head -20
```

---

## Verification Checklist

- [ ] Types compile and serialize to expected JSON format
- [ ] InputTrackingChannel implements RecordingChannel trait
- [ ] macOS: Mouse position captured at ~120Hz
- [ ] macOS: Click events captured with correct button/type
- [ ] macOS: Modifier keys tracked correctly
- [ ] macOS: Cursor images captured as PNG
- [ ] Windows: Mouse position captured
- [ ] Windows: Click events captured
- [ ] Windows: Keyboard hooks work (if enabled)
- [ ] JSON files created on recording stop
- [ ] No memory leaks (buffers cleared between sessions)
- [ ] Performance: CPU usage < 5% during tracking

---

## Future Enhancements (Not in MVP)

- Scroll wheel events
- Multi-touch trackpad gestures
- Per-application window tracking
- Keystroke timing for typing animations
- Cursor trail effects

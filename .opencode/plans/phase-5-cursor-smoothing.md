# Phase 5: Cursor Smoothing - Implementation Plan

## Overview

**Goal:** Implement spring physics-based cursor smoothing for natural, professional-looking cursor movement in playback and export.

**Timeline:** ~1 week  
**Dependencies:** Phase 3 (Input Tracking) - ✅ Completed  
**Scope:** Full implementation (backend + frontend + UI controls)

---

## Quick Reference: Implementation Order

| Step | File                                           | Description                          |
| ---- | ---------------------------------------------- | ------------------------------------ |
| 1    | `src-tauri/src/processing/mod.rs`              | Create module, export types          |
| 2    | `src-tauri/src/processing/spring.rs`           | Spring physics core (Rust)           |
| 3    | `src-tauri/src/processing/cursor_smoothing.rs` | Batch smoothing logic                |
| 4    | `src-tauri/src/commands/processing.rs`         | Tauri commands                       |
| 5    | `src-tauri/src/lib.rs`                         | Wire up processing module + commands |
| 6    | `src-tauri/src/commands/mod.rs`                | Export processing commands           |
| 7    | `src/processing/spring.ts`                     | Spring physics (TypeScript port)     |
| 8    | `src/processing/cursorSmoothing.ts`            | Real-time smoother class             |
| 9    | `src/components/editor/CursorOverlay.tsx`      | Cursor rendering component           |
| 10   | `src/components/editor/EditorView.tsx`         | Wire up overlay + controls           |

**Verification checkpoints:**

- After step 6: `cargo test` should pass, `smooth_cursor` command available
- After step 10: Full visual preview with smoothing working

---

## Background & Learnings Applied

From previous phases:

1. **Match runtime formats** - Like audio buffers, test actual data before committing to processing approach
2. **Separate capture from processing** - Keep InputTrackingChannel for capture only; processing is a distinct phase
3. **Dual implementation** - Need both Rust (export) and TypeScript (real-time preview) implementations
4. **Use existing schema** - `SpringConfig` and `CursorSmoothingConfig` are already defined in project schema

---

## Input Data Structure

From `src-tauri/src/capture/input/types.rs`:

```rust
pub struct MouseMove {
    pub x: f64,
    pub y: f64,
    pub cursor_id: String,
    pub active_modifiers: Vec<String>,
    pub process_time_ms: f64,    // Relative to recording start
    pub unix_time_ms: u64,       // Absolute timestamp
}
```

**Sample data** (from actual recording):

- ~120Hz capture rate (~8.33ms between samples)
- Coordinates in screen space (can be fractional)
- `process_time_ms` is the key for synchronization with video frames

---

## Spring Physics Algorithm

### Theory

A damped spring system follows:

```
F = -k * x - c * v

where:
  k = stiffness (how quickly spring pulls toward target)
  c = damping (resistance to movement, prevents oscillation)
  x = displacement from target
  v = current velocity
  m = mass (affects inertia)

Acceleration: a = F / m
```

### Implementation

```rust
pub struct SpringState {
    pub position: f64,
    pub velocity: f64,
}

impl SpringState {
    pub fn step(&mut self, target: f64, config: &SpringConfig, dt: f64) {
        let displacement = self.position - target;
        let spring_force = -config.stiffness * displacement;
        let damping_force = -config.damping * self.velocity;
        let acceleration = (spring_force + damping_force) / config.mass;

        self.velocity += acceleration * dt;
        self.position += self.velocity * dt;
    }
}
```

### Default Parameters (from schema)

```rust
SpringConfig {
    stiffness: 470.0,  // Higher = faster tracking
    damping: 70.0,     // Higher = less bouncy
    mass: 3.0,         // Higher = more inertia
}
```

---

## Architecture

### File Structure

```
src-tauri/src/
├── processing/
│   ├── mod.rs                  # Module exports
│   ├── spring.rs               # Spring physics core
│   ├── cursor_smoothing.rs     # Batch cursor smoothing
│   └── types.rs                # SmoothedMouseMove, etc.
└── lib.rs                      # Uncomment `pub mod processing;`

src/
├── processing/
│   ├── spring.ts               # Spring physics (port from Rust)
│   └── cursorSmoothing.ts      # Real-time smoothing
└── components/editor/
    └── CursorOverlay.tsx       # Render smoothed cursor on preview
```

### Data Flow

```
Recording Phase:
  InputTrackingChannel → mouse-moves.json (raw 120Hz data)

Playback Phase:
  mouse-moves.json → useSmoothedCursor hook → CursorOverlay component
                                     ↑
                            SpringConfig from project

Export Phase:
  mouse-moves.json → cursor_smoothing.rs → smoothed-cursor.json
                                     ↑
                            SpringConfig from project
```

---

## Implementation Steps

### Step 1: Rust Spring Physics Core

**File:** `src-tauri/src/processing/spring.rs`

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpringConfig {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
}

impl Default for SpringConfig {
    fn default() -> Self {
        Self {
            stiffness: 470.0,
            damping: 70.0,
            mass: 3.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpringState {
    pub position: f64,
    pub velocity: f64,
}

impl SpringState {
    pub fn new(initial: f64) -> Self {
        Self { position: initial, velocity: 0.0 }
    }

    /// Advance the spring simulation by dt seconds
    pub fn step(&mut self, target: f64, config: &SpringConfig, dt: f64) {
        let displacement = self.position - target;
        let spring_force = -config.stiffness * displacement;
        let damping_force = -config.damping * self.velocity;
        let acceleration = (spring_force + damping_force) / config.mass;

        self.velocity += acceleration * dt;
        self.position += self.velocity * dt;
    }

    /// Check if spring has settled (velocity and displacement below threshold)
    pub fn is_settled(&self, target: f64, threshold: f64) -> bool {
        (self.position - target).abs() < threshold && self.velocity.abs() < threshold
    }
}

/// 2D spring for cursor position
#[derive(Debug, Clone)]
pub struct Spring2D {
    pub x: SpringState,
    pub y: SpringState,
}

impl Spring2D {
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x: SpringState::new(x),
            y: SpringState::new(y),
        }
    }

    pub fn step(&mut self, target_x: f64, target_y: f64, config: &SpringConfig, dt: f64) {
        self.x.step(target_x, config, dt);
        self.y.step(target_y, config, dt);
    }

    pub fn position(&self) -> (f64, f64) {
        (self.x.position, self.y.position)
    }
}
```

### Step 2: Cursor Smoothing Processor

**File:** `src-tauri/src/processing/cursor_smoothing.rs`

```rust
use crate::capture::input::types::MouseMove;
use crate::processing::spring::{Spring2D, SpringConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmoothedMouseMove {
    pub x: f64,
    pub y: f64,
    pub raw_x: f64,
    pub raw_y: f64,
    pub cursor_id: String,
    pub process_time_ms: f64,
}

/// Smooth cursor data at a given output framerate
pub fn smooth_cursor_data(
    raw_moves: &[MouseMove],
    config: &SpringConfig,
    output_fps: f64,
) -> Vec<SmoothedMouseMove> {
    if raw_moves.is_empty() {
        return vec![];
    }

    let frame_duration_ms = 1000.0 / output_fps;
    let total_duration_ms = raw_moves.last().map(|m| m.process_time_ms).unwrap_or(0.0);
    let frame_count = (total_duration_ms / frame_duration_ms).ceil() as usize;

    let mut result = Vec::with_capacity(frame_count);
    let mut spring = Spring2D::new(raw_moves[0].x, raw_moves[0].y);
    let mut raw_index = 0;

    for frame in 0..frame_count {
        let frame_time_ms = frame as f64 * frame_duration_ms;

        // Find the raw move closest to this frame time
        while raw_index + 1 < raw_moves.len()
            && raw_moves[raw_index + 1].process_time_ms <= frame_time_ms
        {
            raw_index += 1;
        }

        let raw = &raw_moves[raw_index];

        // Step spring toward raw position
        let dt = frame_duration_ms / 1000.0; // Convert to seconds
        spring.step(raw.x, raw.y, config, dt);

        let (smooth_x, smooth_y) = spring.position();

        result.push(SmoothedMouseMove {
            x: smooth_x,
            y: smooth_y,
            raw_x: raw.x,
            raw_y: raw.y,
            cursor_id: raw.cursor_id.clone(),
            process_time_ms: frame_time_ms,
        });
    }

    result
}

/// Handle teleporting cursor (large jumps should not be smoothed)
pub fn detect_teleport(prev: &MouseMove, curr: &MouseMove, threshold_px: f64) -> bool {
    let dx = curr.x - prev.x;
    let dy = curr.y - prev.y;
    let distance = (dx * dx + dy * dy).sqrt();
    distance > threshold_px
}
```

### Step 3: TypeScript Spring Implementation

**File:** `src/processing/spring.ts`

```typescript
export interface SpringConfig {
  stiffness: number;
  damping: number;
  mass: number;
}

export const DEFAULT_SPRING_CONFIG: SpringConfig = {
  stiffness: 470,
  damping: 70,
  mass: 3,
};

export interface SpringState {
  position: number;
  velocity: number;
}

export function createSpringState(initial: number): SpringState {
  return { position: initial, velocity: 0 };
}

export function stepSpring(
  state: SpringState,
  target: number,
  config: SpringConfig,
  dt: number,
): SpringState {
  const displacement = state.position - target;
  const springForce = -config.stiffness * displacement;
  const dampingForce = -config.damping * state.velocity;
  const acceleration = (springForce + dampingForce) / config.mass;

  const newVelocity = state.velocity + acceleration * dt;
  const newPosition = state.position + newVelocity * dt;

  return { position: newPosition, velocity: newVelocity };
}

export interface Spring2DState {
  x: SpringState;
  y: SpringState;
}

export function createSpring2D(x: number, y: number): Spring2DState {
  return {
    x: createSpringState(x),
    y: createSpringState(y),
  };
}

export function stepSpring2D(
  state: Spring2DState,
  targetX: number,
  targetY: number,
  config: SpringConfig,
  dt: number,
): Spring2DState {
  return {
    x: stepSpring(state.x, targetX, config, dt),
    y: stepSpring(state.y, targetY, config, dt),
  };
}
```

### Step 4: Real-Time Smoothing Hook

**File:** `src/processing/cursorSmoothing.ts`

```typescript
import type { SpringConfig, Spring2DState } from "./spring";
import { createSpring2D, stepSpring2D, DEFAULT_SPRING_CONFIG } from "./spring";

export interface MouseMoveEvent {
  x: number;
  y: number;
  cursorId: string;
  processTimeMs: number;
}

export interface SmoothedPosition {
  x: number;
  y: number;
  rawX: number;
  rawY: number;
  cursorId: string;
}

export class CursorSmoother {
  private spring: Spring2DState;
  private config: SpringConfig;
  private lastRawPosition: { x: number; y: number } | null = null;
  private teleportThreshold: number;

  constructor(
    config: SpringConfig = DEFAULT_SPRING_CONFIG,
    teleportThreshold = 500,
  ) {
    this.config = config;
    this.teleportThreshold = teleportThreshold;
    this.spring = createSpring2D(0, 0);
  }

  updateConfig(config: SpringConfig) {
    this.config = config;
  }

  /**
   * Get smoothed position for a given raw position
   * @param raw - Raw mouse position
   * @param dt - Time delta in seconds since last update
   */
  update(raw: MouseMoveEvent, dt: number): SmoothedPosition {
    // Detect teleport (large jump)
    if (this.lastRawPosition) {
      const dx = raw.x - this.lastRawPosition.x;
      const dy = raw.y - this.lastRawPosition.y;
      const distance = Math.sqrt(dx * dx + dy * dy);

      if (distance > this.teleportThreshold) {
        // Reset spring to new position instantly
        this.spring = createSpring2D(raw.x, raw.y);
      }
    }

    // Step spring simulation
    this.spring = stepSpring2D(this.spring, raw.x, raw.y, this.config, dt);
    this.lastRawPosition = { x: raw.x, y: raw.y };

    return {
      x: this.spring.x.position,
      y: this.spring.y.position,
      rawX: raw.x,
      rawY: raw.y,
      cursorId: raw.cursorId,
    };
  }

  reset(x: number, y: number) {
    this.spring = createSpring2D(x, y);
    this.lastRawPosition = { x, y };
  }
}
```

### Step 5: Cursor Overlay Component

**File:** `src/components/editor/CursorOverlay.tsx`

```tsx
import { useEffect, useRef, useState } from "react";
import type { SmoothedPosition } from "../../processing/cursorSmoothing";

interface CursorOverlayProps {
  position: SmoothedPosition | null;
  cursorImages: Record<string, string>; // cursorId -> image URL
  cursorSize: number; // Scale factor (default 1.5)
  videoWidth: number;
  videoHeight: number;
  showRawPosition?: boolean; // Debug: show raw vs smoothed
}

export function CursorOverlay({
  position,
  cursorImages,
  cursorSize,
  videoWidth,
  videoHeight,
  showRawPosition = false,
}: CursorOverlayProps) {
  if (!position) return null;

  const cursorImage = cursorImages[position.cursorId];

  // Convert screen coordinates to video preview coordinates
  // (This will need adjustment based on actual video scaling)
  const scaleX = videoWidth > 0 ? 1 : 1;
  const scaleY = videoHeight > 0 ? 1 : 1;

  return (
    <div className="absolute inset-0 pointer-events-none overflow-hidden">
      {/* Smoothed cursor */}
      {cursorImage && (
        <img
          src={cursorImage}
          alt="cursor"
          className="absolute"
          style={{
            left: position.x * scaleX,
            top: position.y * scaleY,
            transform: `scale(${cursorSize})`,
            transformOrigin: "top left",
          }}
        />
      )}

      {/* Debug: raw position indicator */}
      {showRawPosition && (
        <div
          className="absolute w-2 h-2 bg-red-500 rounded-full opacity-50"
          style={{
            left: position.rawX * scaleX,
            top: position.rawY * scaleY,
            transform: "translate(-50%, -50%)",
          }}
        />
      )}
    </div>
  );
}
```

### Step 6: Tauri Commands for Processing

**File:** `src-tauri/src/commands/processing.rs`

```rust
use crate::capture::input::types::MouseMove;
use crate::processing::cursor_smoothing::{smooth_cursor_data, SmoothedMouseMove};
use crate::processing::spring::SpringConfig;
use std::path::Path;

/// Process raw mouse moves and return smoothed data
#[tauri::command]
pub async fn smooth_cursor(
    input_file: String,
    config: SpringConfig,
    output_fps: f64,
) -> Result<Vec<SmoothedMouseMove>, String> {
    let path = Path::new(&input_file);
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let raw_moves: Vec<MouseMove> = serde_json::from_str(&content).map_err(|e| e.to_string())?;

    Ok(smooth_cursor_data(&raw_moves, &config, output_fps))
}

/// Process and write smoothed data to file (for export)
#[tauri::command]
pub async fn process_cursor_smoothing(
    input_file: String,
    output_file: String,
    config: SpringConfig,
    output_fps: f64,
) -> Result<(), String> {
    let path = Path::new(&input_file);
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let raw_moves: Vec<MouseMove> = serde_json::from_str(&content).map_err(|e| e.to_string())?;

    let smoothed = smooth_cursor_data(&raw_moves, &config, output_fps);
    let output = serde_json::to_vec_pretty(&smoothed).map_err(|e| e.to_string())?;

    std::fs::write(&output_file, output).map_err(|e| e.to_string())?;
    Ok(())
}
```

### Step 7: Wire Up Module Exports

**File:** `src-tauri/src/processing/mod.rs`

```rust
pub mod spring;
pub mod cursor_smoothing;

pub use spring::{SpringConfig, SpringState, Spring2D};
pub use cursor_smoothing::{smooth_cursor_data, SmoothedMouseMove};
```

**Update:** `src-tauri/src/lib.rs` - Uncomment the processing module

**Update:** `src-tauri/src/commands/mod.rs` - Add `pub mod processing;`

---

## Edge Cases to Handle

### 1. Teleporting Cursor

When cursor jumps large distance (e.g., switching monitors, clicking menubar), don't smooth - snap instantly.

**Detection:** If distance > 500px between consecutive samples, reset spring.

### 2. Recording Start/End

- **Start:** Initialize spring at first raw position
- **End:** Continue simulation for a few frames to let spring settle

### 3. Pause/Resume Sessions

Each session has its own mouse-moves JSON. Need to handle session transitions:

- Reset spring state at session boundaries
- Or smooth across sessions if they're contiguous

### 4. Screen Edge Behavior

Cursor can move off-screen during recording. Options:

- Clamp to video bounds
- Let it go off-screen (natural behavior)
- Apply padding zone

**Recommendation:** Let it go off-screen naturally, clamp only in final render.

### 5. Per-Slice Smoothing Override

`Slice.disableCursorSmoothing` should bypass smoothing for that slice's time range.

---

## UI Integration

### Editor Settings Panel

Update the existing cursor settings in `EditorView.tsx`:

```tsx
// Cursor smoothing controls
<div className="space-y-2">
  <label className="flex items-center gap-2">
    <input
      type="checkbox"
      checked={cursorConfig.smoothing.enabled}
      onChange={(e) => updateCursorSmoothing({ enabled: e.target.checked })}
    />
    <span>Enable cursor smoothing</span>
  </label>

  {cursorConfig.smoothing.enabled && (
    <>
      <div>
        <label className="text-xs text-gray-400">Smoothness</label>
        <input
          type="range"
          min="100"
          max="800"
          value={cursorConfig.smoothing.spring.stiffness}
          onChange={(e) => updateSpring({ stiffness: Number(e.target.value) })}
        />
      </div>
      <div>
        <label className="text-xs text-gray-400">Responsiveness</label>
        <input
          type="range"
          min="20"
          max="150"
          value={cursorConfig.smoothing.spring.damping}
          onChange={(e) => updateSpring({ damping: Number(e.target.value) })}
        />
      </div>
    </>
  )}
</div>
```

---

## Files to Create/Modify

### New Files

| File                                           | Description                     |
| ---------------------------------------------- | ------------------------------- |
| `src-tauri/src/processing/mod.rs`              | Module exports                  |
| `src-tauri/src/processing/spring.rs`           | Spring physics core             |
| `src-tauri/src/processing/cursor_smoothing.rs` | Batch cursor smoothing          |
| `src-tauri/src/commands/processing.rs`         | Tauri commands for processing   |
| `src/processing/spring.ts`                     | TypeScript spring physics       |
| `src/processing/cursorSmoothing.ts`            | Real-time cursor smoother class |
| `src/components/editor/CursorOverlay.tsx`      | Cursor rendering component      |

### Modified Files

| File                                   | Changes                                            |
| -------------------------------------- | -------------------------------------------------- |
| `src-tauri/src/lib.rs`                 | Uncomment `pub mod processing;`, register commands |
| `src-tauri/src/commands/mod.rs`        | Add `pub mod processing;` export                   |
| `src/components/editor/EditorView.tsx` | Wire up CursorOverlay, add smoothing controls      |

---

## Verification Plan

### Unit Tests (Rust)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spring_approaches_target() {
        let config = SpringConfig::default();
        let mut state = SpringState::new(0.0);

        // Step toward target=100 for 1 second
        for _ in 0..60 {
            state.step(100.0, &config, 1.0 / 60.0);
        }

        // Should be close to target
        assert!((state.position - 100.0).abs() < 5.0);
    }

    #[test]
    fn test_spring_no_overshoot_with_high_damping() {
        let config = SpringConfig { stiffness: 470.0, damping: 150.0, mass: 3.0 };
        let mut state = SpringState::new(0.0);
        let mut max_pos = 0.0f64;

        for _ in 0..120 {
            state.step(100.0, &config, 1.0 / 60.0);
            max_pos = max_pos.max(state.position);
        }

        // Should not overshoot significantly
        assert!(max_pos < 105.0);
    }
}
```

### Manual Testing

1. **Build and run:**

   ```bash
   cd src-tauri && cargo test
   bun run tauri:dev
   ```

2. **Record a session** with varied mouse movement:
   - Slow, deliberate movements
   - Fast swipes
   - Click and hold
   - Teleport (move to different monitor area quickly)

3. **Open in editor** and verify:
   - Cursor appears on preview
   - Smoothing visibly reduces jitter
   - Cursor follows with appropriate lag
   - Teleports are handled (no crazy spring animation)

4. **Adjust parameters:**
   - Toggle smoothing on/off
   - Adjust stiffness slider - cursor should track faster/slower
   - Adjust damping slider - cursor should be more/less bouncy

5. **Check JSON output:**
   - Call `smooth_cursor` command via dev tools
   - Verify smoothed positions are reasonable
   - Verify no NaN or Infinity values

---

## Performance Considerations

1. **TypeScript smoothing runs in UI thread** - Spring math is fast (O(1) per frame), should be fine
2. **Batch processing for export** - Process all frames at once, don't need real-time
3. **Memory:** Smoothed data is same size as raw data - no concern
4. **120Hz input → 30/60fps output** - Downsampling is natural, just sample at output framerate

---

## Future Enhancements (Out of Scope)

- Click emphasis animation (brief zoom/highlight on click)
- Cursor trail effect
- Cursor glow/shadow
- Multiple cursor support (for screen sharing scenarios)

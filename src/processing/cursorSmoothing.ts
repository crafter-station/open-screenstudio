/**
 * Real-time cursor smoothing for editor preview
 *
 * This module provides a CursorSmoother class that applies spring physics
 * to cursor movement during playback, providing smooth, natural-looking
 * cursor animation.
 */

import type { SpringConfig, Spring2DState } from "./spring";
import { createSpring2D, stepSpring2D, DEFAULT_SPRING_CONFIG } from "./spring";

/**
 * Raw mouse movement event from input tracking
 */
export interface MouseMoveEvent {
  x: number;
  y: number;
  cursorId: string;
  processTimeMs: number;
}

/**
 * Smoothed cursor position with both raw and smoothed coordinates
 */
export interface SmoothedPosition {
  /** Smoothed X position */
  x: number;
  /** Smoothed Y position */
  y: number;
  /** Original raw X position */
  rawX: number;
  /** Original raw Y position */
  rawY: number;
  /** Cursor image ID */
  cursorId: string;
}

/**
 * Default teleport detection threshold in pixels
 * If cursor moves more than this distance in one frame, reset spring
 */
export const DEFAULT_TELEPORT_THRESHOLD = 500;

/**
 * Real-time cursor smoother using spring physics
 *
 * Usage:
 * ```typescript
 * const smoother = new CursorSmoother();
 *
 * // In your animation loop:
 * const smoothed = smoother.update(rawPosition, deltaTime);
 * renderCursor(smoothed.x, smoothed.y);
 * ```
 */
export class CursorSmoother {
  private spring: Spring2DState;
  private config: SpringConfig;
  private lastRawPosition: { x: number; y: number } | null = null;
  private teleportThreshold: number;

  constructor(
    config: SpringConfig = DEFAULT_SPRING_CONFIG,
    teleportThreshold: number = DEFAULT_TELEPORT_THRESHOLD,
  ) {
    this.config = config;
    this.teleportThreshold = teleportThreshold;
    this.spring = createSpring2D(0, 0);
  }

  /**
   * Update the spring configuration
   * Changes take effect on the next update call
   */
  updateConfig(config: SpringConfig): void {
    this.config = config;
  }

  /**
   * Update teleport detection threshold
   */
  setTeleportThreshold(threshold: number): void {
    this.teleportThreshold = threshold;
  }

  /**
   * Get smoothed position for a given raw position
   *
   * @param raw - Raw mouse position from input data
   * @param dt - Time delta in seconds since last update
   * @returns Smoothed position with both raw and smoothed coordinates
   */
  update(raw: MouseMoveEvent, dt: number): SmoothedPosition {
    // Detect teleport (large jump)
    if (this.lastRawPosition) {
      const dx = raw.x - this.lastRawPosition.x;
      const dy = raw.y - this.lastRawPosition.y;
      const distance = Math.sqrt(dx * dx + dy * dy);

      if (distance > this.teleportThreshold) {
        // Teleport detected - reset spring to new position instantly
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

  /**
   * Reset the spring to a specific position with zero velocity
   * Use this when jumping to a new time in the timeline
   */
  reset(x: number, y: number): void {
    this.spring = createSpring2D(x, y);
    this.lastRawPosition = { x, y };
  }

  /**
   * Get the current smoothed position without advancing the simulation
   */
  getCurrentPosition(): { x: number; y: number } {
    return {
      x: this.spring.x.position,
      y: this.spring.y.position,
    };
  }

  /**
   * Get the current spring velocity
   * Useful for debugging or UI feedback
   */
  getCurrentVelocity(): { x: number; y: number } {
    return {
      x: this.spring.x.velocity,
      y: this.spring.y.velocity,
    };
  }
}

/**
 * Create a cursor smoother with project settings
 */
export function createCursorSmoother(
  enabled: boolean,
  config?: SpringConfig,
): CursorSmoother | null {
  if (!enabled) {
    return null;
  }
  return new CursorSmoother(config ?? DEFAULT_SPRING_CONFIG);
}

/**
 * Pre-process an array of mouse moves to get smoothed positions
 * This is useful for generating a smoothed track for the timeline
 *
 * @param moves - Array of raw mouse move events
 * @param config - Spring configuration
 * @param outputFps - Target output framerate
 * @returns Array of smoothed positions at the target framerate
 */
export function smoothMouseMoves(
  moves: MouseMoveEvent[],
  config: SpringConfig = DEFAULT_SPRING_CONFIG,
  outputFps: number = 30,
): SmoothedPosition[] {
  if (moves.length === 0) {
    return [];
  }

  const frameDurationMs = 1000 / outputFps;
  const totalDurationMs = moves[moves.length - 1].processTimeMs;
  const frameCount = Math.max(1, Math.ceil(totalDurationMs / frameDurationMs));

  const result: SmoothedPosition[] = [];
  const smoother = new CursorSmoother(config);

  // Initialize at first position
  smoother.reset(moves[0].x, moves[0].y);

  let rawIndex = 0;

  for (let frame = 0; frame < frameCount; frame++) {
    const frameTimeMs = frame * frameDurationMs;

    // Find the raw move closest to this frame time
    while (
      rawIndex + 1 < moves.length &&
      moves[rawIndex + 1].processTimeMs <= frameTimeMs
    ) {
      rawIndex++;
    }

    const raw = moves[rawIndex];
    const dt = frameDurationMs / 1000; // Convert to seconds

    const smoothed = smoother.update(
      {
        x: raw.x,
        y: raw.y,
        cursorId: raw.cursorId,
        processTimeMs: frameTimeMs,
      },
      dt,
    );

    result.push(smoothed);
  }

  return result;
}

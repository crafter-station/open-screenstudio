/**
 * Spring physics simulation for smooth cursor movement
 *
 * Implements a damped spring system that provides natural-feeling
 * cursor smoothing by simulating physical spring dynamics.
 *
 * This is a TypeScript port of the Rust implementation for real-time
 * preview in the editor.
 */

export interface SpringConfig {
  stiffness: number;
  damping: number;
  mass: number;
}

// High stiffness + damping for responsive cursor with minimal lag
// This ensures the cursor reaches its target quickly while still
// having smooth motion (no overshoot)
export const DEFAULT_SPRING_CONFIG: SpringConfig = {
  stiffness: 800,
  damping: 80,
  mass: 1,
};

export interface SpringState {
  position: number;
  velocity: number;
}

/**
 * Create a new spring state at the given position with zero velocity
 */
export function createSpringState(initial: number): SpringState {
  return { position: initial, velocity: 0 };
}

/**
 * Advance the spring simulation by dt seconds toward the target
 *
 * Uses the damped harmonic oscillator equation:
 * F = -k * x - c * v
 * where k = stiffness, c = damping, x = displacement, v = velocity
 *
 * @returns New spring state (immutable update)
 */
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

/**
 * Check if spring has settled (velocity and displacement below threshold)
 */
export function isSpringSettled(
  state: SpringState,
  target: number,
  threshold: number = 0.1,
): boolean {
  return (
    Math.abs(state.position - target) < threshold &&
    Math.abs(state.velocity) < threshold
  );
}

export interface Spring2DState {
  x: SpringState;
  y: SpringState;
}

/**
 * Create a new 2D spring at the given position
 */
export function createSpring2D(x: number, y: number): Spring2DState {
  return {
    x: createSpringState(x),
    y: createSpringState(y),
  };
}

/**
 * Advance both X and Y springs toward the target position
 *
 * @returns New spring state (immutable update)
 */
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

/**
 * Get the current position from a 2D spring
 */
export function getSpring2DPosition(state: Spring2DState): {
  x: number;
  y: number;
} {
  return { x: state.x.position, y: state.y.position };
}

/**
 * Check if both axes have settled
 */
export function isSpring2DSettled(
  state: Spring2DState,
  targetX: number,
  targetY: number,
  threshold: number = 0.1,
): boolean {
  return (
    isSpringSettled(state.x, targetX, threshold) &&
    isSpringSettled(state.y, targetY, threshold)
  );
}

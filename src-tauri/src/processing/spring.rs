//! Spring physics simulation for smooth cursor movement
//!
//! Implements a damped spring system that provides natural-feeling
//! cursor smoothing by simulating physical spring dynamics.

use crate::project::schema::SpringConfig;

/// 1D spring state tracking position and velocity
#[derive(Debug, Clone)]
pub struct SpringState {
    pub position: f64,
    pub velocity: f64,
}

impl SpringState {
    /// Create a new spring state at the given position with zero velocity
    pub fn new(initial: f64) -> Self {
        Self {
            position: initial,
            velocity: 0.0,
        }
    }

    /// Advance the spring simulation by dt seconds toward the target
    ///
    /// Uses the damped harmonic oscillator equation:
    /// F = -k * x - c * v
    /// where k = stiffness, c = damping, x = displacement, v = velocity
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

/// 2D spring for cursor position (X and Y axes)
#[derive(Debug, Clone)]
pub struct Spring2D {
    pub x: SpringState,
    pub y: SpringState,
}

impl Spring2D {
    /// Create a new 2D spring at the given position
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x: SpringState::new(x),
            y: SpringState::new(y),
        }
    }

    /// Advance both X and Y springs toward the target position
    pub fn step(&mut self, target_x: f64, target_y: f64, config: &SpringConfig, dt: f64) {
        self.x.step(target_x, config, dt);
        self.y.step(target_y, config, dt);
    }

    /// Get the current smoothed position
    pub fn position(&self) -> (f64, f64) {
        (self.x.position, self.y.position)
    }

    /// Reset the spring to a new position with zero velocity (for teleports)
    pub fn reset(&mut self, x: f64, y: f64) {
        self.x = SpringState::new(x);
        self.y = SpringState::new(y);
    }

    /// Check if both axes have settled
    pub fn is_settled(&self, target_x: f64, target_y: f64, threshold: f64) -> bool {
        self.x.is_settled(target_x, threshold) && self.y.is_settled(target_y, threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> SpringConfig {
        SpringConfig {
            stiffness: 470.0,
            damping: 70.0,
            mass: 3.0,
        }
    }

    #[test]
    fn test_spring_approaches_target() {
        let config = default_config();
        let mut state = SpringState::new(0.0);

        // Step toward target=100 for 1 second at 60fps
        for _ in 0..60 {
            state.step(100.0, &config, 1.0 / 60.0);
        }

        // Should be close to target
        assert!(
            (state.position - 100.0).abs() < 5.0,
            "Position {} should be close to 100",
            state.position
        );
    }

    #[test]
    fn test_spring_no_overshoot_with_high_damping() {
        let config = SpringConfig {
            stiffness: 470.0,
            damping: 150.0, // Higher damping
            mass: 3.0,
        };
        let mut state = SpringState::new(0.0);
        let mut max_pos = 0.0f64;

        // Step toward target=100 for 2 seconds
        for _ in 0..120 {
            state.step(100.0, &config, 1.0 / 60.0);
            max_pos = max_pos.max(state.position);
        }

        // Should not overshoot significantly with high damping
        assert!(
            max_pos < 105.0,
            "Max position {} should not overshoot much",
            max_pos
        );
    }

    #[test]
    fn test_spring_2d() {
        let config = default_config();
        let mut spring = Spring2D::new(0.0, 0.0);

        // Step toward (100, 200) for 1 second
        for _ in 0..60 {
            spring.step(100.0, 200.0, &config, 1.0 / 60.0);
        }

        let (x, y) = spring.position();
        assert!((x - 100.0).abs() < 5.0, "X {} should be close to 100", x);
        assert!((y - 200.0).abs() < 5.0, "Y {} should be close to 200", y);
    }

    #[test]
    fn test_spring_reset() {
        let config = default_config();
        let mut spring = Spring2D::new(0.0, 0.0);

        // Move toward target
        for _ in 0..30 {
            spring.step(100.0, 100.0, &config, 1.0 / 60.0);
        }

        // Reset to new position
        spring.reset(500.0, 500.0);

        let (x, y) = spring.position();
        assert_eq!(x, 500.0);
        assert_eq!(y, 500.0);
        assert_eq!(spring.x.velocity, 0.0);
        assert_eq!(spring.y.velocity, 0.0);
    }
}

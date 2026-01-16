//! Processing module for post-recording transformations
//!
//! This module contains algorithms for cursor smoothing, zoom detection,
//! and other post-processing operations applied during playback and export.

pub mod cursor_smoothing;
pub mod spring;

pub use cursor_smoothing::{smooth_cursor_data, SmoothedMouseMove};
pub use spring::{Spring2D, SpringState};

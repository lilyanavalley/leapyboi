//! leapyboi library — testable core logic
//!
//! This library contains the animation engine, LED state management, and
//! configuration parsing. It's designed to be testable on the host machine
//! without requiring ESP32 hardware or the full embedded environment.
//!
//! The binary (`main.rs`) uses this library and adds the hardware-specific
//! glue code (WiFi, MQTT, OTA, RMT driver).

pub mod animations;
pub mod config;
pub mod led;

// Re-export commonly used types
pub use animations::{AnimationType, compute_frame};
pub use led::{LightState, LedController};

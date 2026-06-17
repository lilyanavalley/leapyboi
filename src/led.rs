/// WS2812B Neopixel ring controller.
///
/// Wraps a `SmartLedsWrite` driver (backed by the RMT peripheral) and keeps
/// track of the current `LightState` so that callers can update individual
/// fields and then call [`LedController::refresh`] to push the change to
/// the hardware.
///
/// For animated effects, call [`LedController::tick`] from the main loop on
/// every iteration instead of (or in addition to) [`LedController::refresh`].

use anyhow::{Context, Result};
use smart_leds::{SmartLedsWrite, RGB8};
use std::time::Duration;

use crate::animations::{self, AnimationType};
use crate::config::LED_COUNT;

// ── Public state type ─────────────────────────────────────────────────────────

/// Logical state of the LED ring.
///
/// This mirrors the HomeAssistant `light` entity state.  Mutate the fields
/// you need, then call [`LedController::refresh`] to apply the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightState {
    /// Whether the ring is on.
    pub on: bool,
    /// Master brightness (0 = off, 255 = full).
    pub brightness: u8,
    /// Red channel (0-255).
    pub r: u8,
    /// Green channel (0-255).
    pub g: u8,
    /// Blue channel (0-255).
    pub b: u8,
    /// Active animation / effect.
    pub animation: AnimationType,
}

impl LightState {

    /// Set the animation and return the modified state for chaining.
    pub fn with_animation(mut self, animation: AnimationType) -> Self {
        self.animation = animation;
        self
    }

}

impl Default for LightState {
    fn default() -> Self {
        Self {
            on: false,
            brightness: 255,
            r: 255,
            g: 255,
            b: 255,
            animation: AnimationType::default(),
        }
    }
}

// ── Controller ────────────────────────────────────────────────────────────────

/// Controls the physical LED ring via any [`SmartLedsWrite`] driver.
///
/// The generic parameter `D` is intended to be filled by
/// `ws2812_esp32_rmt_driver::LedPixelEsp32Rmt<'_, RGB8, LedPixelColorGrb24>`,
/// but any driver that emits `RGB8` colours will work.
pub struct LedController<D>
where
    D: SmartLedsWrite<Color = RGB8>,
    D::Error: std::error::Error + Send + Sync + 'static,
{
    driver: D,
    /// Current logical state; can be read or written directly.
    pub state: LightState,
    /// Monotonically-increasing tick counter used by animation functions.
    phase: u32,
}

impl<D> LedController<D>
where
    D: SmartLedsWrite<Color = RGB8>,
    D::Error: std::error::Error + Send + Sync + 'static,
{
    /// Wrap an existing driver.
    pub fn new(driver: D) -> Self {
        Self {
            driver,
            state: LightState::default(),
            phase: 0,
        }
    }

    /// Push `self.state` to the physical LEDs using the current animation frame.
    ///
    /// Call this after mutating any field of `self.state` for an immediate
    /// update.  For smooth animated effects call [`tick`](Self::tick) from the
    /// main loop instead.
    pub fn refresh(&mut self) -> Result<()> {
        let pixels = animations::compute_frame(
            self.state.r,
            self.state.g,
            self.state.b,
            self.state.brightness,
            self.state.on,
            self.state.animation,
            self.phase,
        );
        self.driver
            .write(pixels.into_iter())
            .context("failed to write LED pixels")
    }

    /// Apply a complete logical light state and refresh the LEDs.
    pub fn apply_state(&mut self, state: LightState) -> Result<()> {
        self.state = state;
        self.refresh()
    }

    /// Smoothly transition from the current state to `target`.
    pub fn transition_to_state(
        &mut self,
        target: LightState,
        steps: u8,
        step_delay: Duration,
    ) -> Result<()> {
        if steps == 0 || self.state == target {
            return self.apply_state(target);
        }

        let start = if self.state.on {
            self.state
        } else {
            LightState {
                on: false,
                brightness: 0,
                r: 0,
                g: 0,
                b: 0,
                animation: target.animation
            }
        };
        let total = u16::from(steps);

        for step in 1..=steps {
            let ratio = u16::from(step);
            let mut frame = LightState {
                // Keep output lit while interpolating to avoid abrupt off-frames mid-fade.
                // Treat an "off" start state as black so off→on transitions fade up from dark
                // instead of from stale stored brightness/colour values.
                // Off→off transitions are already short-circuited by the equality check above.
                on: start.on || target.on,
                brightness: lerp_u8(start.brightness, target.brightness, ratio, total),
                r: lerp_u8(start.r, target.r, ratio, total),
                g: lerp_u8(start.g, target.g, ratio, total),
                b: lerp_u8(start.b, target.b, ratio, total),
                animation: target.animation,
            };

            if step == steps {
                frame = target;
            }

            self.state = frame;
            self.refresh()?;
            if step != steps {
                std::thread::sleep(step_delay);
            }
        }

        Ok(())

    }

    /// Advance the animation phase by one step and write the new frame.
    ///
    /// Call this on every main-loop iteration (typically every ~50 ms) to
    /// drive smooth animation.  The phase is only incremented when the ring
    /// is on, so static/off states have no overhead.
    pub fn tick(&mut self) -> Result<()> {
        if self.state.on {
            self.phase = self.phase.wrapping_add(1);
        }
        self.refresh()
    }

    /// Reset the animation phase to zero.
    ///
    /// Useful when switching animations or applying a new mmWave presence
    /// event so the new animation always starts from the beginning.
    pub fn reset_phase(&mut self) {
        self.phase = 0;
    }

    /// Turn all LEDs off and update `self.state.on`.
    pub fn all_off(&mut self) -> Result<()> {
        self.state.on = false;
        self.phase = 0;
        self.refresh()
    }

    /// Write a raw colour to all LEDs, bypassing `self.state`.
    ///
    /// Useful for status blinks during boot (e.g. "WiFi connected" green flash)
    /// that should not affect the stored logical state.
    pub fn status_color(&mut self, r: u8, g: u8, b: u8) -> Result<()> {
        let pixels = std::iter::repeat(RGB8::new(r, g, b)).take(LED_COUNT);
        self.driver
            .write(pixels)
            .context("failed to write status colour")
    }
}

/// Linearly interpolate from `start` to `end` at `ratio / total`.
///
/// `ratio` is the current step index and `total` is the number of steps.
fn lerp_u8(start: u8, end: u8, ratio: u16, total: u16) -> u8 {
    let start = i32::from(start);
    let end = i32::from(end);
    let delta = end - start;
    let value = start + ((delta * i32::from(ratio)) / i32::from(total));
    value.clamp(0, 255) as u8
}

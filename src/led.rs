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

use crate::animations::{self, AnimationType};
use crate::config::LED_COUNT;

// ── Public state type ─────────────────────────────────────────────────────────

/// Logical state of the LED ring.
///
/// This mirrors the HomeAssistant `light` entity state.  Mutate the fields
/// you need, then call [`LedController::refresh`] to apply the change, or
/// call [`LedController::tick`] from the main loop to advance animated effects.
#[derive(Debug, Clone, Copy)]
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

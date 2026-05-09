/// WS2812B Neopixel ring controller.
///
/// Wraps a `SmartLedsWrite` driver (backed by the RMT peripheral) and keeps
/// track of the current `LightState` so that callers can update individual
/// fields and then call [`LedController::refresh`] to push the change to
/// the hardware.

use anyhow::{Context, Result};
use smart_leds::{SmartLedsWrite, RGB8};
use std::time::Duration;

use crate::config::LED_COUNT;

// ── Public state type ─────────────────────────────────────────────────────────

/// Logical state of the LED ring.
///
/// This mirrors the HomeAssistant `light` entity state.  Mutate the fields
/// you need, then call [`LedController::refresh`] to apply the change.
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
}

impl Default for LightState {
    fn default() -> Self {
        Self {
            on: false,
            brightness: 255,
            r: 255,
            g: 255,
            b: 255,
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
        }
    }

    /// Push `self.state` to the physical LEDs.
    ///
    /// Call this after mutating any field of `self.state`.
    pub fn refresh(&mut self) -> Result<()> {
        let pixels: Vec<RGB8> = if self.state.on {
            // Scale each channel by brightness / 255.
            let scale = self.state.brightness as f32 / 255.0;
            let r = (self.state.r as f32 * scale) as u8;
            let g = (self.state.g as f32 * scale) as u8;
            let b = (self.state.b as f32 * scale) as u8;
            vec![RGB8::new(r, g, b); LED_COUNT]
        } else {
            vec![RGB8::new(0, 0, 0); LED_COUNT]
        };

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
        if steps == 0 || self.state.on == target.on && self.state.brightness == target.brightness
            && self.state.r == target.r && self.state.g == target.g && self.state.b == target.b
        {
            return self.apply_state(target);
        }

        let start = self.state;
        let total = u16::from(steps);

        for step in 1..=steps {
            let ratio = u16::from(step);
            let mut frame = LightState {
                on: start.on || target.on,
                brightness: lerp_u8(start.brightness, target.brightness, ratio, total),
                r: lerp_u8(start.r, target.r, ratio, total),
                g: lerp_u8(start.g, target.g, ratio, total),
                b: lerp_u8(start.b, target.b, ratio, total),
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

    /// Turn all LEDs off and update `self.state.on`.
    pub fn all_off(&mut self) -> Result<()> {
        self.state.on = false;
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

fn lerp_u8(start: u8, end: u8, ratio: u16, total: u16) -> u8 {
    if total == 0 {
        return end;
    }

    let start = i16::from(start);
    let end = i16::from(end);
    let delta = end - start;
    let value = start + ((delta * ratio as i16) / total as i16);
    value.clamp(0, 255) as u8
}

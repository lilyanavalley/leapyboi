/// leapyboi — ESP32-C6 firmware
///
/// Connects a WS2812B Neopixel ring light to HomeAssistant via MQTT over WiFi.
///
/// Boot sequence:
///   1. LED ring: brief white pulse (power-on indicator)
///   2. WiFi: connect to the configured SSID
///   3. LED ring: brief green pulse (WiFi connected)
///   4. MQTT: connect to the broker, publish HA discovery
///   5. LED ring: brief blue pulse (MQTT connected)
///   6. Main loop: apply incoming HA commands and publish state

use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::gpio::AnyOutputPin,
    hal::peripherals::Peripherals,
    log::EspLogger,
    nvs::EspDefaultNvsPartition,
};
use log::info;
use smart_leds::{SmartLedsWrite, RGB8};
use ws2812_esp32_rmt_driver::{driver::color::LedPixelColorGrb24, LedPixelEsp32Rmt};

mod config;
mod led;
mod mqtt;
mod wifi;

use led::LedController;
use mqtt::LightCommand;

fn main() -> Result<()> {
    // Patch the runtime — must be called before anything else.
    // See: https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Route `log` crate output to the ESP-IDF serial logging facility.
    EspLogger::initialize_default();

    info!("leapyboi starting up…");

    // ── Peripheral handles ────────────────────────────────────────────────────
    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // ── LED ring ──────────────────────────────────────────────────────────────
    // GPIO pin and RMT channel are consumed (moved) here; they live for the
    // remainder of main(), which is effectively 'static on a microcontroller.
    let rmt_channel = peripherals.rmt.channel0;
    // Runtime-selected output pin so wiring can be changed in config.rs.
    let led_pin = unsafe { AnyOutputPin::new(config::led_data_pin_num()) };

    let ws2812_driver =
        LedPixelEsp32Rmt::<RGB8, LedPixelColorGrb24>::new(rmt_channel, led_pin)?;
    let mut led = LedController::new(ws2812_driver);

    // Power-on indicator: brief dim white flash.
    led.status_color(15, 15, 15)?;
    std::thread::sleep(Duration::from_millis(300));
    led.all_off()?;

    // ── WiFi ──────────────────────────────────────────────────────────────────
    info!("Connecting to WiFi…");
    let _wifi = wifi::connect(
        peripherals.modem,
        sysloop,
        nvs,
        config::WIFI_SSID,
        config::WIFI_PASS,
    )?;

    // Brief green flash = WiFi OK.
    led.status_color(0, 20, 0)?;
    std::thread::sleep(Duration::from_millis(500));
    led.all_off()?;

    // ── MQTT ──────────────────────────────────────────────────────────────────
    info!("Starting MQTT…");
    let mut mqtt = mqtt::start()?;

    // Wait for the initial MQTT connection before subscribing.
    info!("Waiting for MQTT broker connection…");
    while !mqtt.reconnected.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    mqtt.on_connected()?;

    // Brief blue flash = MQTT OK.
    led.status_color(0, 0, 20)?;
    std::thread::sleep(Duration::from_millis(500));
    led.all_off()?;

    info!("Ready — waiting for HomeAssistant commands on {}", config::COMMAND_TOPIC);

    // ── Main loop ─────────────────────────────────────────────────────────────
    loop {
        // Re-subscribe after an MQTT reconnect (e.g. broker restart, WiFi drop).
        if mqtt.reconnected.swap(false, Ordering::Relaxed) {
            info!("MQTT reconnected — re-subscribing…");
            if let Err(e) = mqtt.on_connected() {
                log::error!("on_connected error: {e:?}");
            }
        }

        // Drain all queued commands in this iteration.
        while let Some(cmd) = mqtt.try_recv_command() {
            info!("Command received: {:?}", cmd);
            apply_command(&mut led, &cmd)?;
            mqtt.publish_state(&led.state)?;
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}

// ── Command handler ───────────────────────────────────────────────────────────

/// Apply a single HomeAssistant light command to the LED ring.
fn apply_command<D>(led: &mut LedController<D>, cmd: &LightCommand) -> Result<()>
where
    D: SmartLedsWrite<Color = RGB8>,
    D::Error: std::error::Error + Send + Sync + 'static,
{
    if let Some(ref state) = cmd.state {
        led.state.on = state.eq_ignore_ascii_case("on");
    }

    if let Some(brightness) = cmd.brightness {
        led.state.brightness = brightness;
    }

    if let Some(ref color) = cmd.color {
        led.state.r = color.r;
        led.state.g = color.g;
        led.state.b = color.b;
    }

    led.refresh()
}

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

#[cfg(feature = "mmwave")]
mod mmwave;

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
    let led_pin = peripherals.pins.gpio8; // <── adjust to your wiring

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

    // ── mmWave presence sensor (optional feature) ─────────────────────────────
    // Initialise after MQTT so that on_connected() has already published the
    // binary_sensor discovery message before the first presence event fires.
    #[cfg(feature = "mmwave")]
    let mmwave = {
        info!("Initialising mmWave presence sensor…");
        // GPIO 5 → sensor RX (ESP transmits commands, optional)
        // GPIO 4 ← sensor TX (ESP receives the data stream)
        // Adjust these pins in src/main.rs to match your physical wiring.
        mmwave::start(
            peripherals.uart1,
            peripherals.pins.gpio5, // ESP TX → sensor RX
            peripherals.pins.gpio4, // ESP RX ← sensor TX
        )?
    };

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

        // Drain all queued HA light commands.
        while let Some(cmd) = mqtt.try_recv_command() {
            info!("Command received: {:?}", cmd);
            apply_command(&mut led, &cmd)?;
            mqtt.publish_state(&led.state)?;
        }

        // Drain all queued mmWave presence events.
        #[cfg(feature = "mmwave")]
        while let Some(event) = mmwave.try_recv() {
            info!("mmWave event: {:?}", event);
            let detected = event == mmwave::PresenceEvent::Detected;
            apply_presence_event(&mut led, event)?;
            mqtt.publish_state(&led.state)?;
            mqtt.publish_presence(detected)?;
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

// ── mmWave presence handler ───────────────────────────────────────────────────

/// Apply a mmWave presence event to the LED ring.
///
/// - `Detected` → turn the ring on with the configured presence colour.
/// - `Gone`     → switch to the no-presence colour (or turn off if `(0, 0, 0)`).
#[cfg(feature = "mmwave")]
fn apply_presence_event<D>(
    led: &mut LedController<D>,
    event: mmwave::PresenceEvent,
) -> Result<()>
where
    D: SmartLedsWrite<Color = RGB8>,
    D::Error: std::error::Error + Send + Sync + 'static,
{
    match event {
        mmwave::PresenceEvent::Detected => {
            let (r, g, b) = config::PRESENCE_COLOR;
            led.state.on = true;
            led.state.r = r;
            led.state.g = g;
            led.state.b = b;
            led.state.brightness = config::PRESENCE_BRIGHTNESS;
        }
        mmwave::PresenceEvent::Gone => {
            let (r, g, b) = config::NO_PRESENCE_COLOR;
            // Turn the ring off if the no-presence colour is pure black.
            led.state.on = r != 0 || g != 0 || b != 0;
            led.state.r = r;
            led.state.g = g;
            led.state.b = b;
            led.state.brightness = config::NO_PRESENCE_BRIGHTNESS;
        }
    }
    led.refresh()
}

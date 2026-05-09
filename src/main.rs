/// leapyboi — ESP32-C6 firmware
///
/// Connects a WS2812B Neopixel ring light to HomeAssistant via MQTT over WiFi.
///
/// Boot sequence:
///   1. LED ring: brief white pulse (power-on indicator)
///   2. WiFi: connect to the configured SSID
///   3. LED ring: brief green pulse (WiFi connected)
///   4. OTA check: compare running version against GitHub Releases; download
///      and apply any newer firmware then restart (skipped when URLs not set)
///   5. MQTT: connect to the broker, publish HA discovery
///   6. LED ring: brief blue pulse (MQTT connected)
///   7. Main loop: apply incoming HA commands and publish state

use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{anyhow, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::gpio::AnyOutputPin,
    hal::peripherals::Peripherals,
    log::EspLogger,
    nvs::EspDefaultNvsPartition,
};
use log::info;
#[cfg(feature = "mmwave")]
use log::warn;
use smart_leds::{SmartLedsWrite, RGB8};
use ws2812_esp32_rmt_driver::{driver::color::LedPixelColorGrb24, LedPixelEsp32Rmt};

mod config;
mod led;
mod mqtt;
mod ota;
mod wifi;

#[cfg(feature = "mmwave")]
mod mmwave;

use led::LedController;
use mqtt::{IncomingCommand, LightCommand};

#[cfg(feature = "mmwave")]
use esp_idf_svc::sys;
#[cfg(feature = "mmwave")]
use std::ffi::CString;
#[cfg(feature = "mmwave")]
use std::time::Instant;

fn run_startup_led_self_test<D>(led: &mut LedController<D>) -> Result<()>
where
    D: SmartLedsWrite<Color = RGB8>,
    D::Error: std::error::Error + Send + Sync + 'static,
{
    // Fast RGB pattern confirms the data pin and WS2812 signalling path.
    let steps = [(24, 0, 0), (0, 24, 0), (0, 0, 24), (12, 12, 12)];

    for (r, g, b) in steps {
        led.status_color(r, g, b)?;
        std::thread::sleep(Duration::from_millis(180));
    }

    led.all_off()?;
    std::thread::sleep(Duration::from_millis(120));
    Ok(())
}

#[cfg(feature = "mmwave")]
const MMWAVE_SETTINGS_NVS_NAMESPACE: &str = "leapyboi";
#[cfg(feature = "mmwave")]
const MMWAVE_TRANSITION_LOCKOUT_NVS_KEY: &str = "mw_lockout";

#[cfg(feature = "mmwave")]
#[derive(Debug, Clone, Copy)]
struct MmwaveTransitionSettings {
    lockout_ms: u32,
}

#[cfg(feature = "mmwave")]
impl Default for MmwaveTransitionSettings {
    fn default() -> Self {
        Self {
            lockout_ms: config::MMWAVE_TRANSITION_LOCKOUT_DEFAULT_MS,
        }
    }
}

#[cfg(feature = "mmwave")]
fn normalize_transition_lockout_ms(lockout_ms: u32) -> u32 {
    lockout_ms.min(10_000)
}

#[cfg(feature = "mmwave")]
fn read_transition_lockout_from_nvs() -> Result<Option<u32>> {
    let namespace = CString::new(MMWAVE_SETTINGS_NVS_NAMESPACE)?;
    let key = CString::new(MMWAVE_TRANSITION_LOCKOUT_NVS_KEY)?;

    let mut handle: sys::nvs_handle_t = 0;
    let open_err = unsafe {
        sys::nvs_open(
            namespace.as_ptr(),
            sys::nvs_open_mode_t_NVS_READONLY,
            &mut handle,
        )
    };
    if open_err != sys::ESP_OK as _ {
        return Err(anyhow!("nvs_open(readonly) failed: {}", open_err));
    }

    let mut value: u32 = 0;
    let get_err = unsafe { sys::nvs_get_u32(handle, key.as_ptr(), &mut value) };
    unsafe { sys::nvs_close(handle) };

    if get_err == sys::ESP_OK as _ {
        Ok(Some(value))
    } else if get_err == sys::ESP_ERR_NVS_NOT_FOUND as _ {
        Ok(None)
    } else {
        Err(anyhow!("nvs_get_u32 failed: {}", get_err))
    }
}

#[cfg(feature = "mmwave")]
fn write_transition_lockout_to_nvs(lockout_ms: u32) -> Result<()> {
    let namespace = CString::new(MMWAVE_SETTINGS_NVS_NAMESPACE)?;
    let key = CString::new(MMWAVE_TRANSITION_LOCKOUT_NVS_KEY)?;

    let mut handle: sys::nvs_handle_t = 0;
    let open_err = unsafe {
        sys::nvs_open(
            namespace.as_ptr(),
            sys::nvs_open_mode_t_NVS_READWRITE,
            &mut handle,
        )
    };
    if open_err != sys::ESP_OK as _ {
        return Err(anyhow!("nvs_open(readwrite) failed: {}", open_err));
    }

    let set_err = unsafe { sys::nvs_set_u32(handle, key.as_ptr(), lockout_ms) };
    if set_err != sys::ESP_OK as _ {
        unsafe { sys::nvs_close(handle) };
        return Err(anyhow!("nvs_set_u32 failed: {}", set_err));
    }

    let commit_err = unsafe { sys::nvs_commit(handle) };
    unsafe { sys::nvs_close(handle) };
    if commit_err != sys::ESP_OK as _ {
        return Err(anyhow!("nvs_commit failed: {}", commit_err));
    }

    Ok(())
}

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
    let led_pin_num = config::led_data_pin_num();
    info!("LED data output pin configured: GPIO{}", led_pin_num);
    // Runtime-selected output pin so wiring can be changed in config.rs.
    let led_pin = unsafe { AnyOutputPin::new(led_pin_num) };

    let ws2812_driver =
        LedPixelEsp32Rmt::<RGB8, LedPixelColorGrb24>::new(rmt_channel, led_pin)?;
    let mut led = LedController::new(ws2812_driver);

    // Power-on self-test: verify LED transport before networking starts.
    run_startup_led_self_test(&mut led)?;

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

    // ── OTA check ─────────────────────────────────────────────────────────────
    // Performed once on each boot, before starting MQTT.  If a new firmware
    // image is found the device downloads it and restarts; execution only
    // continues here when the firmware is already up to date.
    info!("OTA: checking for firmware update…");
    if let Err(e) = ota::check_and_update(config::OTA_VERSION_URL, config::OTA_FIRMWARE_URL) {
        log::warn!("OTA: check failed (continuing with current firmware): {e:#}");
    }

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
            peripherals.pins.gpio16, // ESP TX → sensor RX
            peripherals.pins.gpio17, // ESP RX ← sensor TX
        )?
    };

    #[cfg(feature = "mmwave")]
    let mut mmwave_settings = {
        let default_settings = MmwaveTransitionSettings::default();
        let lockout_ms = match read_transition_lockout_from_nvs() {
            Ok(Some(value)) => normalize_transition_lockout_ms(value),
            Ok(None) => default_settings.lockout_ms,
            Err(e) => {
                warn!(
                    "mmWave transition lockout load failed; using default {} ms: {e:#}",
                    default_settings.lockout_ms
                );
                default_settings.lockout_ms
            }
        };

        if lockout_ms == 0 {
            warn!(
                "mmWave instant transition mode is active from stored settings. {}",
                config::MMWAVE_INSTANT_MODE_WARNING
            );
        }

        let settings = MmwaveTransitionSettings { lockout_ms };
        if let Err(e) = mqtt.publish_mmwave_transition_lockout(settings.lockout_ms) {
            warn!("failed to publish mmWave transition lockout state: {e:#}");
        }
        settings
    };

    #[cfg(feature = "mmwave")]
    let mut last_presence_apply: Option<Instant> = None;
    #[cfg(feature = "mmwave")]
    let mut pending_presence_event: Option<mmwave::PresenceEvent> = None;

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

        // Handle an on-demand OTA update request received via MQTT.
        if mqtt.ota_requested.swap(false, Ordering::Relaxed) {
            info!("OTA: on-demand update requested via MQTT");
            if let Err(e) = ota::apply_update(config::OTA_FIRMWARE_URL) {
                log::error!("OTA: on-demand update failed: {e:#}");
            }
        }

        // Drain all queued MQTT commands.
        while let Some(cmd) = mqtt.try_recv_command() {
            info!("Command received: {:?}", cmd);
            match cmd {
                IncomingCommand::Light(light_cmd) => {
                    apply_command(&mut led, &light_cmd)?;
                    mqtt.publish_state(&led.state)?;
                }
                #[cfg(feature = "mmwave")]
                IncomingCommand::MmwaveTransitionLockoutMs(lockout_ms) => {
                    mmwave_settings.lockout_ms = normalize_transition_lockout_ms(lockout_ms);
                    if mmwave_settings.lockout_ms == 0 {
                        warn!(
                            "mmWave instant transition mode enabled via MQTT. {}",
                            config::MMWAVE_INSTANT_MODE_WARNING
                        );
                    } else if mmwave_settings.lockout_ms
                        < config::MMWAVE_TRANSITION_LOCKOUT_DEFAULT_MS
                    {
                        warn!(
                            "mmWave transition lockout reduced to {} ms (default {} ms).",
                            mmwave_settings.lockout_ms,
                            config::MMWAVE_TRANSITION_LOCKOUT_DEFAULT_MS
                        );
                    }

                    if let Err(e) = write_transition_lockout_to_nvs(mmwave_settings.lockout_ms) {
                        warn!("failed to persist mmWave transition lockout: {e:#}");
                    }
                    if let Err(e) =
                        mqtt.publish_mmwave_transition_lockout(mmwave_settings.lockout_ms)
                    {
                        warn!("failed to publish mmWave transition lockout state: {e:#}");
                    }
                }
            }
        }

        // Drain all queued mmWave presence events.
        #[cfg(feature = "mmwave")]
        while let Some(event) = mmwave.try_recv() {
            info!("mmWave event: {:?}", event);
            let detected = event == mmwave::PresenceEvent::Detected;
            pending_presence_event = Some(event);
            mqtt.publish_presence(detected)?;
        }

        #[cfg(feature = "mmwave")]
        if let Some(event) = pending_presence_event {
            let can_apply = mmwave_settings.lockout_ms == 0
                || last_presence_apply.is_none_or(|last| {
                    last.elapsed() >= Duration::from_millis(mmwave_settings.lockout_ms as u64)
                });

            if can_apply {
                apply_presence_event(&mut led, event, mmwave_settings.lockout_ms == 0)?;
                mqtt.publish_state(&led.state)?;
                last_presence_apply = Some(Instant::now());
                pending_presence_event = None;
            }
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
    instant: bool,
) -> Result<()>
where
    D: SmartLedsWrite<Color = RGB8>,
    D::Error: std::error::Error + Send + Sync + 'static,
{
    let target = match event {
        mmwave::PresenceEvent::Detected => {
            let (r, g, b) = config::PRESENCE_COLOR;
            led::LightState {
                on: true,
                brightness: config::PRESENCE_BRIGHTNESS,
                r,
                g,
                b,
            }
        }
        mmwave::PresenceEvent::Gone => {
            let (r, g, b) = config::NO_PRESENCE_COLOR;
            led::LightState {
                // Turn the ring off if the no-presence colour is pure black.
                on: r != 0 || g != 0 || b != 0,
                brightness: config::NO_PRESENCE_BRIGHTNESS,
                r,
                g,
                b,
            }
        }
    };

    if instant {
        led.apply_state(target)
    } else {
        led.transition_to_state(
            target,
            config::MMWAVE_TRANSITION_STEPS,
            Duration::from_millis(config::MMWAVE_TRANSITION_STEP_DELAY_MS),
        )
    }
}

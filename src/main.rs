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

#[cfg(feature = "esp32")]
use std::sync::atomic::Ordering;
#[cfg(feature = "esp32")]
use std::time::Duration;

// * mmwave-specific imports are gated behind the "mmwave" feature flag.
#[cfg(feature = "mmwave")]
use esp_idf_svc::sys;
#[cfg(feature = "mmwave")]
use std::ffi::CString;
#[cfg(feature = "mmwave")]
use std::time::Instant;

#[cfg(feature = "esp32")]
use anyhow::{anyhow, Result};
#[cfg(feature = "esp32")]
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::gpio::AnyOutputPin,
    hal::peripherals::Peripherals,
    log::EspLogger,
    nvs::EspDefaultNvsPartition,
};
#[cfg(feature = "mmwave")]
use esp_idf_svc::hal::gpio::AnyIOPin;
#[cfg(feature = "esp32")]
use log:: { debug, info, warn, error };
#[cfg(feature = "esp32")]
use smart_leds::{SmartLedsWrite, RGB8};
#[cfg(feature = "esp32")]
use ws2812_esp32_rmt_driver::{driver::color::LedPixelColorGrb24, LedPixelEsp32Rmt};

// Import from the leapyboi library
#[cfg(feature = "esp32")]
use leapyboi::config;
#[cfg(feature = "esp32")]
use leapyboi::led::LedController;
#[cfg(feature = "esp32")]
use leapyboi::animations::AnimationType;

// Hardware-specific modules (not testable without ESP)
#[cfg(feature = "esp32")]
mod mqtt;
#[cfg(feature = "esp32")]
mod ota;
#[cfg(feature = "esp32")]
mod wifi;
#[cfg(all(feature = "esp32", feature = "mmwave"))]
mod mmwave;

#[cfg(feature = "esp32")]
use mqtt::{IncomingCommand, LightCommand, SwitchCommand};


#[cfg(feature = "esp32")]
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
/// Runtime mmWave transition-safety settings.
struct MmwaveTransitionSettings {
    /// Minimum milliseconds between applied presence-driven light transitions.
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
/// Clamp runtime lockout values to match the exposed HA/MQTT control range.
///
/// HomeAssistant discovery advertises a maximum of `10000` ms, so values above
/// that are reduced to keep runtime behavior consistent with UI limits.
#[cfg(feature = "esp32")]
fn normalize_transition_lockout_ms(lockout_ms: u32) -> u32 {
    lockout_ms.min(10_000)
}

#[cfg(feature = "mmwave")]
#[cfg(feature = "esp32")]
fn log_instant_mode_warning(source: &str) {
    warn!(
        "mmWave instant transition mode enabled via {source}. {}",
        config::MMWAVE_INSTANT_MODE_WARNING
    );
}

#[cfg(feature = "mmwave")]
#[cfg(feature = "esp32")]
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
    if open_err != sys::ESP_OK {
        return Err(anyhow!("nvs_open(readonly) failed: {}", open_err));
    }

    let mut value: u32 = 0;
    let get_err = unsafe { sys::nvs_get_u32(handle, key.as_ptr(), &mut value) };
    unsafe { sys::nvs_close(handle) };

    if get_err == sys::ESP_OK {
        Ok(Some(value))
    } else if get_err == sys::ESP_ERR_NVS_NOT_FOUND {
        Ok(None)
    } else {
        Err(anyhow!("nvs_get_u32 failed: {}", get_err))
    }
}

#[cfg(feature = "mmwave")]
#[cfg(feature = "esp32")]
fn write_transition_lockout_to_nvs(lockout_ms: u32) -> Result<()> {
    let lockout_ms = normalize_transition_lockout_ms(lockout_ms);
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
    if open_err != sys::ESP_OK {
        return Err(anyhow!("nvs_open(readwrite) failed: {}", open_err));
    }

    let set_err = unsafe { sys::nvs_set_u32(handle, key.as_ptr(), lockout_ms) };
    if set_err != sys::ESP_OK {
        unsafe { sys::nvs_close(handle) };
        return Err(anyhow!("nvs_set_u32 failed: {}", set_err));
    }

    let commit_err = unsafe { sys::nvs_commit(handle) };
    unsafe { sys::nvs_close(handle) };
    if commit_err != sys::ESP_OK {
        return Err(anyhow!("nvs_commit failed: {}", commit_err));
    }

    Ok(())
}

#[cfg(feature = "esp32")]
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
        let mmwave_uart_port = config::mmwave_uart_port();
        let mmwave_tx_pin_num = config::mmwave_uart_tx_pin_num();
        let mmwave_rx_pin_num = config::mmwave_uart_rx_pin_num();
        info!(
            "mmWave UART configured: uart{} TX=GPIO{} (ESP->sensor), RX=GPIO{} (sensor->ESP)",
            mmwave_uart_port, mmwave_tx_pin_num, mmwave_rx_pin_num
        );

        // Runtime-selected UART peripheral and pins allow wiring and routing
        // changes via cfg.toml without touching source.
        match mmwave_uart_port {
            0 => {
                let mmwave_tx_pin = unsafe { AnyIOPin::new(mmwave_tx_pin_num) };
                let mmwave_rx_pin = unsafe { AnyIOPin::new(mmwave_rx_pin_num) };
                mmwave::start(
                    peripherals.uart0,
                    mmwave_tx_pin, // ESP TX → sensor RX
                    mmwave_rx_pin, // ESP RX ← sensor TX
                )?
            }
            1 => {
                let mmwave_tx_pin = unsafe { AnyIOPin::new(mmwave_tx_pin_num) };
                let mmwave_rx_pin = unsafe { AnyIOPin::new(mmwave_rx_pin_num) };
                mmwave::start(
                    peripherals.uart1,
                    mmwave_tx_pin, // ESP TX → sensor RX
                    mmwave_rx_pin, // ESP RX ← sensor TX
                )?
            }
            _ => unreachable!("MMWAVE_UART_PORT is validated in config::mmwave_uart_port"),
        }
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
            log_instant_mode_warning("stored settings");
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

    // ── Runtime feature flags ─────────────────────────────────────────────────
    // Both features start enabled; users can toggle them from HA or raw MQTT.
    let mut animations_enabled = true;
    #[cfg(feature = "mmwave")]
    let mut mmwave_enabled = true;

    // Publish initial switch states so HA shows the correct toggle positions
    // immediately after boot (before the user has toggled anything).
    mqtt.publish_animations_state(animations_enabled)?;
    #[cfg(feature = "mmwave")]
    mqtt.publish_mmwave_enable_state(mmwave_enabled)?;

    // ── Main loop ─────────────────────────────────────────────────────────────
    loop {
        // Re-subscribe after an MQTT reconnect (e.g. broker restart, WiFi drop).
        if mqtt.reconnected.swap(false, Ordering::Relaxed) {
            info!("MQTT reconnected — re-subscribing…");
            if let Err(e) = mqtt.on_connected() {
                log::error!("on_connected error: {e:?}");
            }
            // Re-publish switch states so HA is back in sync after reconnect.
            mqtt.publish_animations_state(animations_enabled)?;
            #[cfg(feature = "mmwave")]
            mqtt.publish_mmwave_enable_state(mmwave_enabled)?;
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
                        log_instant_mode_warning("MQTT");
                    } else if mmwave_settings.lockout_ms < 100
                    {
                        warn!(
                            "mmWave transition lockout set to very low value: {} ms.",
                            mmwave_settings.lockout_ms
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

        // Drain all queued runtime switch commands.
        while let Some(sw) = mqtt.try_recv_switch() {
            match sw {
                SwitchCommand::Animations(enabled) => {
                    info!("Animations {}", if enabled { "enabled" } else { "disabled" });
                    animations_enabled = enabled;
                    mqtt.publish_animations_state(animations_enabled)?;
                }
                #[cfg(feature = "mmwave")]
                SwitchCommand::Mmwave(enabled) => {
                    info!("mmWave {}", if enabled { "enabled" } else { "disabled" });
                    mmwave_enabled = enabled;
                    mqtt.publish_mmwave_enable_state(mmwave_enabled)?;
                }
            }
        }

        // Drain all queued mmWave presence events.
        // Events are skipped (but still drained) when mmWave reactions are disabled.
        #[cfg(feature = "mmwave")]
        while let Some(event) = mmwave.try_recv() {
            if mmwave_enabled {
                info!("mmWave event: {:?}", event);
                let detected = event == mmwave::PresenceEvent::Detected;
                // apply_presence_event(&mut led, event, mmwave_settings.lockout_ms == 0)?;
                mqtt.publish_state(&led.state)?;
                mqtt.publish_presence(detected)?;
            } else {
                info!("mmWave event ignored (mmWave disabled): {:?}", event);
            }
        }

        // Advance the animation by one tick and push the frame to the ring.
        //
        // When animations are disabled the ring renders a static solid-colour
        // frame without advancing the phase counter; the stored animation
        // selection is preserved so it resumes correctly when re-enabled.
        if animations_enabled {
            led.tick()?;
        } else {
            let saved = led.state.animation;
            led.state.animation = AnimationType::Solid;
            led.refresh()?;
            led.state.animation = saved;
        }

        // TODO: Document.
        #[cfg(feature = "mmwave")]
        if let Some(event) = pending_presence_event {
            let can_apply = mmwave_settings.lockout_ms == 0
                || last_presence_apply.map_or(true, |last| {
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
#[cfg(feature = "esp32")]
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

    if let Some(ref effect) = cmd.effect {
        let new_anim = AnimationType::from_effect_name(effect);
        if new_anim != led.state.animation {
            led.state.animation = new_anim;
            led.reset_phase(); // start the new animation cleanly from frame 0
        }
    }

    led.refresh()
}

// ── mmWave presence handler ───────────────────────────────────────────────────

/// Apply a mmWave presence event to the LED ring.
///
/// - `Detected` → turn the ring on with the configured presence colour and animation.
/// - `Gone`     → switch to the no-presence colour/animation (or turn off if `(0, 0, 0)`).
#[cfg(feature = "mmwave")]
#[cfg(feature = "esp32")]
fn apply_presence_event<D>(
    led: &mut LedController<D>,
    event: mmwave::PresenceEvent,
    immediate: bool,
) -> Result<()>
where
    D: SmartLedsWrite<Color = RGB8>,
    D::Error: std::error::Error + Send + Sync + 'static,
{
    let target = match event {
        mmwave::PresenceEvent::Detected => {
            let (r, g, b) = config::PRESENCE_COLOR;
            led::LightState {
                // Turn the ring on
                on: true,
                brightness: config::NO_PRESENCE_BRIGHTNESS,
                r,
                g,
                b,
                animation: AnimationType::from_effect_name(config::PRESENCE_ANIMATION)
            }
        }
        mmwave::PresenceEvent::Gone => {
            let (r, g, b) = config::NO_PRESENCE_COLOR;
            // Turn the ring off if the no-presence colour is pure black.
            led::LightState {
                // Turn the ring off if the no-presence colour is pure black.
                on: r != 0 || g != 0 || b != 0,
                brightness: config::NO_PRESENCE_BRIGHTNESS,
                r,
                g,
                b,
                animation: AnimationType::from_effect_name(config::NO_PRESENCE_ANIMATION)
            }
        }
    };

    if immediate {
        led.apply_state(target);
    } else {
        led.transition_to_state(
            target,
            config::MMWAVE_TRANSITION_STEPS,
            Duration::from_millis(config::MMWAVE_TRANSITION_STEP_DELAY_MS),
        );
    }
    
    // restart the animation from the beginning on every presence change
    led.reset_phase();
    led.refresh()

}

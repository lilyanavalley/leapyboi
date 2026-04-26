/// Compile-time configuration for leapyboi.
///
/// WiFi credentials and MQTT settings come from `cfg.toml` (via `build.rs`).
/// Hardware constants are set here — adjust them to match your wiring.

// ── Build-time credentials (populated from cfg.toml by build.rs) ─────────────

/// SSID of the 2.4 GHz WiFi network to join.
pub const WIFI_SSID: &str = env!("WIFI_SSID");

/// WiFi password.
pub const WIFI_PASS: &str = env!("WIFI_PASS");

/// MQTT broker URL, e.g. `"mqtt://192.168.1.100:1883"`.
pub const MQTT_URL: &str = env!("MQTT_URL");

/// MQTT client ID sent to the broker (must be unique per device).
pub const MQTT_CLIENT_ID: &str = env!("MQTT_CLIENT_ID");

// ── Hardware configuration ────────────────────────────────────────────────────

/// GPIO pin number wired to the DIN data line of the WS2812B ring.
/// Adjust to match your physical wiring; any RMT-capable output pin works.
pub const LED_DATA_PIN_NUM: i32 = 8;

/// Number of LEDs in the ring.  Change to match your specific ring module.
pub const LED_COUNT: usize = 12;

// ── HomeAssistant MQTT topics ─────────────────────────────────────────────────

/// MQTT discovery topic — publishes the HA `light` entity configuration.
/// Retained so HA picks it up even after a broker restart.
pub const HA_DISCOVERY_TOPIC: &str = "homeassistant/light/leapyboi/config";

/// Topic that HomeAssistant writes light commands to (JSON schema).
pub const COMMAND_TOPIC: &str = "leapyboi/light/set";

/// Topic the firmware publishes current light state to (JSON schema).
pub const STATE_TOPIC: &str = "leapyboi/light/state";

/// Topic used for LWT / online-offline availability reporting.
pub const AVAILABILITY_TOPIC: &str = "leapyboi/light/availability";

// ── Device metadata shown in HomeAssistant ────────────────────────────────────

pub const DEVICE_NAME: &str = "Leapyboi Ring";
pub const DEVICE_MODEL: &str = "ESP32-C6";
pub const DEVICE_MANUFACTURER: &str = "DIY";
pub const DEVICE_UNIQUE_ID: &str = "leapyboi_ring_light";

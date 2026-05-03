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

/// MQTT username used for broker authentication.
pub const MQTT_USERNAME: &str = env!("MQTT_USERNAME");

/// Optional MQTT username for client authentication.
pub fn mqtt_username() -> Option<&'static str> {
	if MQTT_USERNAME.is_empty() {
		None
	} else {
		Some(MQTT_USERNAME)
	}
}

/// MQTT password used for broker authentication.
pub const MQTT_PASSWORD: &str = env!("MQTT_PASSWORD");

/// Optional MQTT password for client authentication.
pub fn mqtt_password() -> Option<&'static str> {
	if MQTT_PASSWORD.is_empty() {
		None
	} else {
		Some(MQTT_PASSWORD)
	}
}

// ── Hardware configuration ────────────────────────────────────────────────────

/// GPIO pin number wired to the DIN data line of the WS2812B ring.
///
/// This is used at runtime via `AnyOutputPin`, so changing this value does
/// not require touching `main.rs`.
pub const LED_DATA_PIN_NUM: &str = env!("LED_DATA_PIN_NUM");

/// Parsed LED data pin number loaded from cfg.toml at compile time.
pub fn led_data_pin_num() -> i32 {
	LED_DATA_PIN_NUM
		.parse::<i32>()
		.expect("LED_DATA_PIN_NUM must be a valid integer in cfg.toml")
}

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

// ── mmWave presence sensor (optional — enable with `--features mmwave`) ───────

/// MQTT discovery topic for the HA `binary_sensor` presence entity.
#[cfg(feature = "mmwave")]
pub const MMWAVE_DISCOVERY_TOPIC: &str =
    "homeassistant/binary_sensor/leapyboi_presence/config";

/// MQTT topic where presence state (`"ON"` / `"OFF"`) is published.
#[cfg(feature = "mmwave")]
pub const MMWAVE_STATE_TOPIC: &str = "leapyboi/presence/state";

/// Unique ID used in the HA `binary_sensor` discovery message.
#[cfg(feature = "mmwave")]
pub const MMWAVE_UNIQUE_ID: &str = "leapyboi_presence";

/// LED colour (r, g, b) applied when presence **is** detected.
/// Adjust to taste; red is the default.
#[cfg(feature = "mmwave")]
pub const PRESENCE_COLOR: (u8, u8, u8) = (255, 0, 0);

/// LED brightness (0-255) applied when presence is detected.
#[cfg(feature = "mmwave")]
pub const PRESENCE_BRIGHTNESS: u8 = 200;

/// LED colour (r, g, b) applied when **no** presence is detected.
/// Set to `(0, 0, 0)` to turn the ring off when the room is empty.
/// Default is yellow to provide a gentle night light when no one's home. Adjust to taste!
#[cfg(feature = "mmwave")]
pub const NO_PRESENCE_COLOR: (u8, u8, u8) = (255, 255, 0);

/// LED brightness applied when no presence is detected.
#[cfg(feature = "mmwave")]
pub const NO_PRESENCE_BRIGHTNESS: u8 = 50;

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

/// GPIO number used as ESP TX for the mmWave UART (ESP TX -> sensor RX).
#[cfg(feature = "mmwave")]
pub const MMWAVE_UART_TX_PIN_NUM: &str = env!("MMWAVE_UART_TX_PIN_NUM");

/// Parsed mmWave UART TX pin number loaded from cfg.toml at compile time.
#[cfg(feature = "mmwave")]
pub fn mmwave_uart_tx_pin_num() -> i32 {
	MMWAVE_UART_TX_PIN_NUM
		.parse::<i32>()
		.expect("MMWAVE_UART_TX_PIN_NUM must be a valid integer in cfg.toml")
}

/// GPIO number used as ESP RX for the mmWave UART (ESP RX <- sensor TX).
#[cfg(feature = "mmwave")]
pub const MMWAVE_UART_RX_PIN_NUM: &str = env!("MMWAVE_UART_RX_PIN_NUM");

/// Parsed mmWave UART RX pin number loaded from cfg.toml at compile time.
#[cfg(feature = "mmwave")]
pub fn mmwave_uart_rx_pin_num() -> i32 {
	MMWAVE_UART_RX_PIN_NUM
		.parse::<i32>()
		.expect("MMWAVE_UART_RX_PIN_NUM must be a valid integer in cfg.toml")
}

/// UART peripheral index used for mmWave (`0` = uart0, `1` = uart1).
#[cfg(feature = "mmwave")]
pub const MMWAVE_UART_PORT: &str = env!("MMWAVE_UART_PORT");

/// Parsed mmWave UART peripheral index loaded from cfg.toml at compile time.
#[cfg(feature = "mmwave")]
pub fn mmwave_uart_port() -> u8 {
	let port = MMWAVE_UART_PORT
		.parse::<u8>()
		.expect("MMWAVE_UART_PORT must be a valid integer in cfg.toml");
	match port {
		0 | 1 => port,
		_ => panic!("MMWAVE_UART_PORT must be 0 (uart0) or 1 (uart1)"),
	}
}

/// Number of LEDs in the ring.  Change to match your specific ring module.
pub const LED_COUNT: usize = 24;

// ── Animation settings ────────────────────────────────────────────────────────
// All speed constants are in units of "ticks" where 1 tick ≈ 50 ms (20 Hz).

/// Hue steps advanced per tick for the rainbow animation.
/// Higher = faster rotation.  At 1 step/tick the wheel completes in ~12.8 s.
/// At 2 steps/tick the wheel completes in ~6.4 s.
pub const ANIM_RAINBOW_SPEED: u32 = 2;

/// Ticks between each pixel-position advance for the spinning animation.
/// Lower = faster spin.  At 2 ticks/pixel with a 12-LED ring: ~1.2 s/revolution.
pub const ANIM_SPINNING_TICKS_PER_PIXEL: u32 = 2;

/// Phase steps advanced per tick for the breathe animation.
/// Higher = faster breathing.  At 2 steps/tick one full breath takes ~6.4 s.
pub const ANIM_BREATHE_SPEED: u32 = 2;

/// Ticks per frame for the custom PNG animation.
/// At 4 ticks/frame the animation plays at ~5 fps (50 ms × 4 = 200 ms/frame).
pub const ANIM_CUSTOM_TICKS_PER_FRAME: u32 = 4;

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

// ── Runtime switch: animations ────────────────────────────────────────────────

/// MQTT discovery topic for the HA `switch` that enables/disables animations.
pub const ANIMATIONS_DISCOVERY_TOPIC: &str =
    "homeassistant/switch/leapyboi_animations/config";

/// Topic HA writes to toggle animation playback (`"ON"` / `"OFF"`).
pub const ANIMATIONS_COMMAND_TOPIC: &str = "leapyboi/animations/set";

/// Topic the firmware publishes the current animation-enabled state to.
pub const ANIMATIONS_STATE_TOPIC: &str = "leapyboi/animations/state";

// ── Runtime switch: mmWave enable ─────────────────────────────────────────────

/// MQTT discovery topic for the HA `switch` that enables/disables mmWave presence
/// reactions (only published when the `mmwave` feature is compiled in).
#[cfg(feature = "mmwave")]
pub const MMWAVE_ENABLE_DISCOVERY_TOPIC: &str =
    "homeassistant/switch/leapyboi_mmwave_enable/config";

/// Topic HA writes to toggle mmWave reactions at runtime (`"ON"` / `"OFF"`).
#[cfg(feature = "mmwave")]
pub const MMWAVE_ENABLE_COMMAND_TOPIC: &str = "leapyboi/mmwave/enable/set";

/// Topic the firmware publishes the current mmWave-enabled state to.
#[cfg(feature = "mmwave")]
pub const MMWAVE_ENABLE_STATE_TOPIC: &str = "leapyboi/mmwave/enable/state";

// ── Device metadata shown in HomeAssistant ────────────────────────────────────

pub const DEVICE_NAME: &str = "Leapyboi Ring";
pub const DEVICE_MODEL: &str = "ESP32-C6";
pub const DEVICE_MANUFACTURER: &str = "DIY";
pub const DEVICE_UNIQUE_ID: &str = "leapyboi_ring_light";

/// Firmware version currently running on the device.
pub const FIRMWARE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// MQTT discovery topic for the firmware version diagnostic sensor.
pub const FIRMWARE_VERSION_DISCOVERY_TOPIC: &str =
    "homeassistant/sensor/leapyboi_firmware_version/config";

/// MQTT topic where the running firmware version is published.
pub const FIRMWARE_VERSION_STATE_TOPIC: &str = "leapyboi/firmware/version";

/// Unique ID used in the HA firmware version sensor discovery payload.
pub const FIRMWARE_VERSION_UNIQUE_ID: &str = "leapyboi_firmware_version";

/// Entity display name shown in Home Assistant for firmware version reporting.
pub const FIRMWARE_VERSION_ENTITY_NAME: &str = "Leapyboi Firmware Version";

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

/// MQTT discovery topic for the mmWave transition lockout `number` entity.
#[cfg(feature = "mmwave")]
pub const MMWAVE_TRANSITION_LOCKOUT_DISCOVERY_TOPIC: &str =
    "homeassistant/number/leapyboi_mmwave_transition_lockout/config";

/// MQTT topic where the mmWave transition lockout value (milliseconds) is published.
#[cfg(feature = "mmwave")]
pub const MMWAVE_TRANSITION_LOCKOUT_STATE_TOPIC: &str =
    "leapyboi/mmwave/transition_lockout_ms/state";

/// MQTT topic used to set mmWave transition lockout value (milliseconds).
#[cfg(feature = "mmwave")]
pub const MMWAVE_TRANSITION_LOCKOUT_SET_TOPIC: &str =
    "leapyboi/mmwave/transition_lockout_ms/set";

/// Default lockout between mmWave-driven light transitions in milliseconds.
#[cfg(feature = "mmwave")]
pub const MMWAVE_TRANSITION_LOCKOUT_DEFAULT_MS: u32 = 2_000;

/// Warning text logged whenever instant mode is enabled.
#[cfg(feature = "mmwave")]
pub const MMWAVE_INSTANT_MODE_WARNING: &str =
    "WARNING: rapid light transitions within seconds can trigger photosensitive health reactions, including seizure risk.";

/// Number of interpolation steps used for mmWave light transitions.
#[cfg(feature = "mmwave")]
pub const MMWAVE_TRANSITION_STEPS: u8 = 12;

/// Delay between interpolation steps used for mmWave transitions.
#[cfg(feature = "mmwave")]
pub const MMWAVE_TRANSITION_STEP_DELAY_MS: u64 = 60;

/// LED colour (r, g, b) applied when presence **is** detected.
/// Adjust to taste; red is the default.
#[cfg(feature = "mmwave")]
pub const PRESENCE_COLOR: (u8, u8, u8) = (255, 0, 0);

/// LED brightness (0-255) applied when presence is detected.
#[cfg(feature = "mmwave")]
pub const PRESENCE_BRIGHTNESS: u8 = 200;

/// Animation played on the ring when presence **is** detected.
///
/// Must be one of: `"solid"`, `"rainbow"`, `"spinning"`, `"breathe"`, `"custom"`.
/// `"rainbow"` gives a lively indication that someone is in the room.
#[cfg(feature = "mmwave")]
pub const PRESENCE_ANIMATION: &str = "rainbow";

/// LED colour (r, g, b) applied when **no** presence is detected.
/// Set to `(0, 0, 0)` to turn the ring off when the room is empty.
/// Default is yellow to provide a gentle night light when no one's home. Adjust to taste!
#[cfg(feature = "mmwave")]
pub const NO_PRESENCE_COLOR: (u8, u8, u8) = (255, 255, 0);

/// LED brightness applied when no presence is detected.
#[cfg(feature = "mmwave")]
pub const NO_PRESENCE_BRIGHTNESS: u8 = 50;

/// Animation played on the ring when **no** presence is detected.
///
/// Must be one of: `"solid"`, `"rainbow"`, `"spinning"`, `"breathe"`, `"custom"`.
/// `"breathe"` gives a calm, ambient glow when the room is empty.
#[cfg(feature = "mmwave")]
pub const NO_PRESENCE_ANIMATION: &str = "breathe";

// ── OTA (over-the-air) update configuration ───────────────────────────────────

/// URL of the firmware binary to download during an OTA update.
///
/// Populated from `cfg.toml → ota_firmware_url` at compile time.
/// Example: `"https://github.com/you/leapyboi/releases/latest/download/leapyboi.bin"`
pub const OTA_FIRMWARE_URL: &str = env!("OTA_FIRMWARE_URL");

/// URL of the plain-text version file used to check whether a newer firmware is
/// available without downloading the full binary.
///
/// Populated from `cfg.toml → ota_version_url` at compile time.
/// Example: `"https://github.com/you/leapyboi/releases/latest/download/version.txt"`
pub const OTA_VERSION_URL: &str = env!("OTA_VERSION_URL");

/// MQTT topic the device subscribes to for on-demand OTA update requests.
///
/// Publish any payload to this topic to trigger an immediate OTA check.
pub const OTA_UPDATE_TOPIC: &str = "leapyboi/ota/update";

/// MQTT topic where the device reports OTA status strings.
///
/// Published values: `"checking"`, `"up_to_date"`, `"updating"`, `"error: …"`
pub const OTA_STATUS_TOPIC: &str = "leapyboi/ota/status";


/// Build script for leapyboi.
///
/// Two responsibilities:
///  1. Parse `cfg.toml` (user-supplied, gitignored) and expose its values as
///     `cargo:rustc-env` variables so that `env!("WIFI_SSID")` etc. work at
///     compile time.
///  2. Call `embuild::espidf::sysenv::output()` so that the ESP-IDF CMake
///     build system can locate the IDF and emit the correct linker flags.
///
/// Additionally, if `animation.png` exists next to `Cargo.toml`, it is decoded
/// and written as a Rust source file in `$OUT_DIR/custom_anim.rs`.  That file
/// defines three public constants used by `src/animations.rs`:
///
/// | Constant                        | Type          | Description                       |
/// |---------------------------------|---------------|-----------------------------------|
/// | `CUSTOM_ANIMATION_FRAMES`       | `&[u8]`       | Packed RGB bytes, all frames       |
/// | `CUSTOM_ANIMATION_FRAME_COUNT`  | `usize`       | Number of frames (PNG height)      |
/// | `CUSTOM_ANIMATION_FRAME_LEDS`   | `usize`       | LEDs per frame (PNG width)         |
///
/// If `animation.png` is absent the constants are emitted as empty / zero so
/// the firmware builds normally and the `custom` effect is hidden from HA.

use serde::Deserialize;
use std::path::PathBuf;

// ── cfg.toml schema ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CfgFile {
    leapyboi: AppConfig,
}

#[derive(Deserialize)]
struct AppConfig {
    #[serde(default)]
    wifi_ssid: String,

    #[serde(default)]
    wifi_pass: String,

    #[serde(default = "default_mqtt_url")]
    mqtt_url: String,

    #[serde(default = "default_client_id")]
    mqtt_client_id: String,

    #[serde(default)]
    mqtt_username: String,

    #[serde(default)]
    mqtt_password: String,

    #[serde(default = "default_led_data_pin_num")]
    led_data_pin_num: i32,
}

fn default_mqtt_url() -> String {
    "mqtt://localhost:1883".into()
}

fn default_client_id() -> String {
    "leapyboi".into()
}

fn default_led_data_pin_num() -> i32 {
    8
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));

    let cfg_path = manifest_dir.join("cfg.toml");

    if cfg_path.exists() {
        let content =
            std::fs::read_to_string(&cfg_path).expect("failed to read cfg.toml");
        let cfg: CfgFile = toml::from_str(&content)
            .expect("failed to parse cfg.toml — see cfg.toml.example for the expected format");
        let app = &cfg.leapyboi;

        emit_env("WIFI_SSID", &app.wifi_ssid);
        emit_env("WIFI_PASS", &app.wifi_pass);
        emit_env("MQTT_URL", &app.mqtt_url);
        emit_env("MQTT_CLIENT_ID", &app.mqtt_client_id);
        emit_env("MQTT_USERNAME", &app.mqtt_username);
        emit_env("MQTT_PASSWORD", &app.mqtt_password);
        emit_env("LED_DATA_PIN_NUM", &app.led_data_pin_num.to_string());
    } else {
        // No cfg.toml yet — emit empty/default placeholders so that the build
        // succeeds with a clear warning.  The firmware will not connect until
        // cfg.toml is populated.
        emit_env("WIFI_SSID", "");
        emit_env("WIFI_PASS", "");
        emit_env("MQTT_URL", "mqtt://localhost:1883");
        emit_env("MQTT_CLIENT_ID", "leapyboi");
        emit_env("MQTT_USERNAME", "");
        emit_env("MQTT_PASSWORD", "");
        emit_env("LED_DATA_PIN_NUM", "8");

        println!(
            "cargo:warning=cfg.toml not found — \
             copy cfg.toml.example to cfg.toml and fill in your WiFi / MQTT settings."
        );
    }

    // Tell Cargo to re-run this script if cfg.toml changes.
    println!("cargo:rerun-if-changed=cfg.toml");
    println!("cargo:rerun-if-changed=build.rs");

    // Generate the custom animation source file (from animation.png if present).
    generate_custom_anim(&manifest_dir);

    // Emit ESP-IDF build environment variables (linker search paths, etc.).
    embuild::espidf::sysenv::output();
}

fn emit_env(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={value}");
}

// ── Custom animation (animation.png) ──────────────────────────────────────────

/// Decode `animation.png` (if present) and write a Rust source file to
/// `$OUT_DIR/custom_anim.rs` that bakes the pixel data into the firmware.
///
/// The PNG must be 8-bit RGB or RGBA; alpha is ignored.  Width = LEDs per
/// frame, height = number of frames.  A compile-time assertion in
/// `src/animations.rs` verifies that the width matches `LED_COUNT`.
fn generate_custom_anim(manifest_dir: &std::path::Path) {
    let out_dir =
        std::env::var("OUT_DIR").expect("OUT_DIR is not set — is Cargo running this script?");
    let out_path = PathBuf::from(&out_dir).join("custom_anim.rs");

    // Always re-run if the animation file appears, disappears, or changes.
    println!("cargo:rerun-if-changed=animation.png");

    let anim_path = manifest_dir.join("animation.png");

    if !anim_path.exists() {
        // No custom animation — emit empty constants so the code compiles.
        std::fs::write(
            &out_path,
            "pub const CUSTOM_ANIMATION_FRAMES: &[u8] = &[];\n\
             pub const CUSTOM_ANIMATION_FRAME_COUNT: usize = 0;\n\
             pub const CUSTOM_ANIMATION_FRAME_LEDS: usize = 0;\n",
        )
        .expect("failed to write custom_anim.rs");
        return;
    }

    // ── Decode PNG ────────────────────────────────────────────────────────────

    let file = std::fs::File::open(&anim_path).expect("failed to open animation.png");
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info().expect(
        "failed to read animation.png — ensure the file is a valid PNG",
    );

    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .expect("failed to decode animation.png frame data");

    let width = info.width as usize;
    let height = info.height as usize;

    if width == 0 || height == 0 {
        panic!("animation.png must have non-zero width and height");
    }

    // Convert to a flat RGB byte array, dropping any alpha channel.
    let rgb_data: Vec<u8> = match info.color_type {
        png::ColorType::Rgb => {
            buf[..width * height * 3].to_vec()
        }
        png::ColorType::Rgba => {
            let mut rgb = Vec::with_capacity(width * height * 3);
            for chunk in buf[..width * height * 4].chunks_exact(4) {
                rgb.extend_from_slice(&chunk[..3]); // drop alpha byte
            }
            rgb
        }
        other => panic!(
            "animation.png has unsupported colour type {:?}. \
             Please export as 8-bit RGB or RGBA.",
            other
        ),
    };

    // ── Emit Rust source ──────────────────────────────────────────────────────

    // Represent the pixel data as a byte literal array.
    let byte_literals: Vec<String> = rgb_data.iter().map(|b| b.to_string()).collect();
    let data_str = byte_literals.join(", ");

    let source = format!(
        "pub const CUSTOM_ANIMATION_FRAMES: &[u8] = &[{data}];\n\
         pub const CUSTOM_ANIMATION_FRAME_COUNT: usize = {frames};\n\
         pub const CUSTOM_ANIMATION_FRAME_LEDS: usize = {leds};\n",
        data   = data_str,
        frames = height,
        leds   = width,
    );

    std::fs::write(&out_path, source).expect("failed to write custom_anim.rs");

    println!(
        "cargo:warning=animation.png loaded: {} LEDs × {} frames baked into firmware",
        width, height
    );
}

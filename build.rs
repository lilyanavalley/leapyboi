/// Build script for leapyboi.
///
/// Two responsibilities:
///  1. Parse `cfg.toml` (user-supplied, gitignored) and expose its values as
///     `cargo:rustc-env` variables so that `env!("WIFI_SSID")` etc. work at
///     compile time.
///  2. Call `embuild::espidf::sysenv::output()` so that the ESP-IDF CMake
///     build system can locate the IDF and emit the correct linker flags.

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
}

fn default_mqtt_url() -> String {
    "mqtt://localhost:1883".into()
}

fn default_client_id() -> String {
    "leapyboi".into()
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
    } else {
        // No cfg.toml yet — emit empty/default placeholders so that the build
        // succeeds with a clear warning.  The firmware will not connect until
        // cfg.toml is populated.
        emit_env("WIFI_SSID", "");
        emit_env("WIFI_PASS", "");
        emit_env("MQTT_URL", "mqtt://localhost:1883");
        emit_env("MQTT_CLIENT_ID", "leapyboi");

        println!(
            "cargo:warning=cfg.toml not found — \
             copy cfg.toml.example to cfg.toml and fill in your WiFi / MQTT settings."
        );
    }

    // Tell Cargo to re-run this script if cfg.toml changes.
    println!("cargo:rerun-if-changed=cfg.toml");
    println!("cargo:rerun-if-changed=build.rs");

    // Emit ESP-IDF build environment variables (linker search paths, etc.).
    embuild::espidf::sysenv::output();
}

fn emit_env(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={value}");
}

/// MQTT client + HomeAssistant integration.
///
/// # Design overview
///
/// `start()` creates an `EspMqttClient`, spawns a background thread that
/// drives the MQTT connection event loop, and returns a [`MqttHandle`] to
/// the caller.  The background thread forwards incoming light commands to
/// the caller via an `mpsc` channel and signals reconnect events via an
/// `AtomicBool`.
///
/// The caller (main loop) is responsible for:
/// - Calling [`MqttHandle::on_connected`] once after the initial connection to
///   subscribe and publish the HA discovery message.
/// - Polling [`MqttHandle::try_recv_command`] for incoming commands.
/// - Polling [`MqttHandle::try_recv_switch`] for incoming switch commands.
/// - Calling [`publish_state`] after applying a command.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use anyhow::{Context, Result};
use esp_idf_svc::mqtt::client::{
    EspMqttClient, EspMqttConnection, EventPayload, LwtConfiguration,
    MqttClientConfiguration, QoS,
};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};

use crate::config;
use crate::led::LightState;
use crate::animations::AnimationType;

// ── JSON payload types ────────────────────────────────────────────────────────

/// Incoming command from HomeAssistant (MQTT JSON schema).
#[derive(Deserialize, Debug)]
pub struct LightCommand {
    pub state: Option<String>,
    pub brightness: Option<u8>,
    pub color: Option<RgbColor>,
    /// HA effect name — maps to [`AnimationType`].
    pub effect: Option<String>,
}

/// Incoming runtime switch command.
///
/// Delivered by [`MqttHandle::try_recv_switch`]; separate from [`LightCommand`]
/// so the main loop can handle both without routing overhead.
#[derive(Debug)]
pub enum SwitchCommand {
    /// Set the animation-enabled state (`true` = animations running).
    Animations(bool),
    /// Set the mmWave-enabled state (`true` = presence events applied to LEDs).
    #[cfg(feature = "mmwave")]
    Mmwave(bool),
}

/// Outgoing state published back to HomeAssistant.
#[derive(Serialize, Debug)]
struct LightStatePayload<'a> {
    state: &'a str,
    brightness: u8,
    color: RgbColor,
    color_mode: &'a str,
    effect: &'a str,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

// ── Public handle returned by start() ────────────────────────────────────────

/// Owned handle to the running MQTT session.
///
/// Dropping this value closes the MQTT connection.
pub struct MqttHandle {
    client: EspMqttClient<'static>,
    commands: std::sync::mpsc::Receiver<LightCommand>,
    switches: std::sync::mpsc::Receiver<SwitchCommand>,
    /// Set to `true` by the background thread each time MQTT (re)connects.
    pub reconnected: Arc<AtomicBool>,
}

impl MqttHandle {
    /// Subscribe to the command topic and publish the HA discovery message.
    ///
    /// Must be called once after each (re)connection.
    pub fn on_connected(&mut self) -> Result<()> {
        self.client
            .subscribe(config::COMMAND_TOPIC, QoS::AtLeastOnce)
            .context("MQTT subscribe failed")?;

        self.client
            .subscribe(config::ANIMATIONS_COMMAND_TOPIC, QoS::AtLeastOnce)
            .context("MQTT subscribe (animations switch) failed")?;

        #[cfg(feature = "mmwave")]
        self.client
            .subscribe(config::MMWAVE_ENABLE_COMMAND_TOPIC, QoS::AtLeastOnce)
            .context("MQTT subscribe (mmwave enable switch) failed")?;

        publish_discovery(&mut self.client)?;
        publish_animations_switch_discovery(&mut self.client)?;

        #[cfg(feature = "mmwave")]
        publish_presence_discovery(&mut self.client)?;
        #[cfg(feature = "mmwave")]
        publish_mmwave_enable_switch_discovery(&mut self.client)?;

        self.client
            .publish(
                config::AVAILABILITY_TOPIC,
                QoS::AtLeastOnce,
                true, // retain
                b"online",
            )
            .context("failed to publish availability")?;

        info!("MQTT: subscribed and HA discovery published");
        Ok(())
    }

    /// Publish the current presence state (`"ON"` / `"OFF"`) to HomeAssistant.
    #[cfg(feature = "mmwave")]
    pub fn publish_presence(&mut self, detected: bool) -> Result<()> {
        self.client
            .publish(
                config::MMWAVE_STATE_TOPIC,
                QoS::AtLeastOnce,
                true, // retain so HA reflects state after a restart
                if detected { b"ON" } else { b"OFF" },
            )
            .context("failed to publish presence state")
    }

    /// Non-blocking receive of the next light command, if any.
    pub fn try_recv_command(&self) -> Option<LightCommand> {
        self.commands.try_recv().ok()
    }

    /// Non-blocking receive of the next runtime switch command, if any.
    pub fn try_recv_switch(&self) -> Option<SwitchCommand> {
        self.switches.try_recv().ok()
    }

    /// Publish the current light state so HomeAssistant can track it.
    pub fn publish_state(&mut self, state: &LightState) -> Result<()> {
        publish_state(&mut self.client, state)
    }

    /// Publish the current animations-enabled state to HomeAssistant.
    pub fn publish_animations_state(&mut self, enabled: bool) -> Result<()> {
        self.client
            .publish(
                config::ANIMATIONS_STATE_TOPIC,
                QoS::AtLeastOnce,
                true, // retain so HA restores the switch state after a restart
                if enabled { b"ON" } else { b"OFF" },
            )
            .context("failed to publish animations switch state")
            .map(|_| ())
    }

    /// Publish the current mmWave-enabled state to HomeAssistant.
    #[cfg(feature = "mmwave")]
    pub fn publish_mmwave_enable_state(&mut self, enabled: bool) -> Result<()> {
        self.client
            .publish(
                config::MMWAVE_ENABLE_STATE_TOPIC,
                QoS::AtLeastOnce,
                true, // retain
                if enabled { b"ON" } else { b"OFF" },
            )
            .context("failed to publish mmwave-enable switch state")
            .map(|_| ())
    }
}

// ── Constructor ───────────────────────────────────────────────────────────────

/// Initialise the MQTT client and start the background event-loop thread.
///
/// The function returns immediately; the actual TCP connection is established
/// asynchronously by ESP-IDF.  Poll [`MqttHandle::reconnected`] to detect
/// when the connection is ready.
pub fn start() -> Result<MqttHandle> {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<LightCommand>();
    let (sw_tx, sw_rx) = std::sync::mpsc::channel::<SwitchCommand>();
    let reconnected = Arc::new(AtomicBool::new(false));
    let reconnected_thread = Arc::clone(&reconnected);

    let has_username = config::mqtt_username().is_some();
    let has_password = config::mqtt_password().is_some();
    let auth_mode = match (has_username, has_password) {
        (true, true) => "username+password",
        (true, false) => "username-only",
        (false, true) => "password-only",
        (false, false) => "client-id-only",
    };
    info!(
        "MQTT auth mode: {auth_mode} (username={}, password={})",
        if has_username { "set" } else { "unset" },
        if has_password { "set" } else { "unset" },
    );

    // Build a 'static config so that EspMqttClient<'static> can be moved into
    // a thread::spawn closure.  Box::leak is intentional: the MQTT client
    // runs for the entire lifetime of the firmware.
    let conf = Box::new(MqttClientConfiguration {
        client_id: Some(config::MQTT_CLIENT_ID),
        username: config::mqtt_username(),
        password: config::mqtt_password(),
        keep_alive_interval: Some(Duration::from_secs(60)),
        // Last-Will-and-Testament: HA marks the device offline if it drops.
        lwt: Some(LwtConfiguration {
            topic: config::AVAILABILITY_TOPIC,
            payload: b"offline",
            qos: QoS::AtLeastOnce,
            retain: true,
        }),
        ..Default::default()
    });
    let conf: &'static MqttClientConfiguration<'static> = Box::leak(conf);

    let (client, connection) = EspMqttClient::new(config::MQTT_URL, conf)
        .context("failed to create MQTT client")?;

    spawn_event_loop(connection, cmd_tx, sw_tx, reconnected_thread);

    Ok(MqttHandle {
        client,
        commands: cmd_rx,
        switches: sw_rx,
        reconnected,
    })
}

// ── Background event loop ─────────────────────────────────────────────────────

/// Spawn a thread that polls the MQTT connection and forwards events.
///
/// The ESP-IDF MQTT client requires the connection object to be driven
/// continuously; without this the internal state machine stalls.
fn spawn_event_loop(
    mut connection: EspMqttConnection,
    cmd_tx: std::sync::mpsc::Sender<LightCommand>,
    sw_tx: std::sync::mpsc::Sender<SwitchCommand>,
    reconnected: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("mqtt_event".into())
        .stack_size(6 * 1024)
        .spawn(move || {
            info!("MQTT event loop started");
            loop {
                match connection.next() {
                    Err(e) => {
                        error!("MQTT connection error: {:?}", e);
                        break;
                    }
                    Ok(event) => match event.payload() {
                        EventPayload::BeforeConnect => {
                            info!("MQTT: connecting to broker…");
                        }
                        EventPayload::Connected(_session_present) => {
                            info!("MQTT: connected");
                            reconnected.store(true, Ordering::Relaxed);
                        }
                        EventPayload::Disconnected => {
                            warn!("MQTT: disconnected — will retry");
                            reconnected.store(false, Ordering::Relaxed);
                        }
                        EventPayload::Received {
                            topic: Some(topic),
                            data,
                            ..
                        } => {
                            if topic == config::COMMAND_TOPIC {
                                match serde_json::from_slice::<LightCommand>(data) {
                                    Ok(cmd) => { cmd_tx.send(cmd).ok(); }
                                    Err(e) => { warn!("MQTT: invalid light command — {e}"); }
                                }
                            } else if topic == config::ANIMATIONS_COMMAND_TOPIC {
                                if let Some(enabled) = parse_on_off(data, topic) {
                                    sw_tx.send(SwitchCommand::Animations(enabled)).ok();
                                }
                            } else {
                                #[cfg(feature = "mmwave")]
                                if topic == config::MMWAVE_ENABLE_COMMAND_TOPIC {
                                    if let Some(enabled) = parse_on_off(data, topic) {
                                        sw_tx.send(SwitchCommand::Mmwave(enabled)).ok();
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
            info!("MQTT: event loop exited");
        })
        .expect("failed to spawn MQTT event thread");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Publish a HomeAssistant MQTT discovery message for the `light` entity.
///
/// The message is retained so that HA picks it up even after a restart.
fn publish_discovery(client: &mut EspMqttClient<'static>) -> Result<()> {
    // Build the effect_list JSON array from the animation engine.
    let effect_list_json = {
        let names: Vec<_> = AnimationType::effect_list()
            .iter()
            .map(|s| format!(r#""{}""#, s))
            .collect();
        format!("[{}]", names.join(", "))
    };

    // Build the discovery payload as a JSON string.
    // Using format! keeps the dependency count low; switch to serde if the
    // payload grows more complex.
    let payload = format!(
        r#"{{
  "name": "{name}",
  "unique_id": "{uid}",
  "schema": "json",
  "state_topic": "{state}",
  "command_topic": "{cmd}",
  "availability_topic": "{avail}",
  "payload_available": "online",
  "payload_not_available": "offline",
  "brightness": true,
  "brightness_scale": 255,
  "color_mode": true,
  "supported_color_modes": ["rgb"],
  "effect": true,
  "effect_list": {effects},
  "device": {{
    "identifiers": ["{uid}"],
    "name": "{name}",
    "model": "{model}",
    "manufacturer": "{mfr}"
  }}
}}"#,
        name = config::DEVICE_NAME,
        uid = config::DEVICE_UNIQUE_ID,
        state = config::STATE_TOPIC,
        cmd = config::COMMAND_TOPIC,
        avail = config::AVAILABILITY_TOPIC,
        model = config::DEVICE_MODEL,
        mfr = config::DEVICE_MANUFACTURER,
        effects = effect_list_json,
    );

    client
        .publish(
            config::HA_DISCOVERY_TOPIC,
            QoS::AtLeastOnce,
            true, // retain so HA keeps it after a broker restart
            payload.as_bytes(),
        )
        .context("failed to publish HA discovery")
        .map(|_| ())
}

/// Publish the current light state to HomeAssistant.
fn publish_state(client: &mut EspMqttClient<'static>, state: &LightState) -> Result<()> {
    let payload = LightStatePayload {
        state: if state.on { "ON" } else { "OFF" },
        brightness: state.brightness,
        color: RgbColor {
            r: state.r,
            g: state.g,
            b: state.b,
        },
        color_mode: "rgb",
        effect: state.animation.as_effect_name(),
    };

    let json = serde_json::to_string(&payload).context("failed to serialise light state")?;

    client
        .publish(
            config::STATE_TOPIC,
            QoS::AtLeastOnce,
            true, // retain so HA restores state after a restart
            json.as_bytes(),
        )
        .context("failed to publish light state")
        .map(|_| ())
}

/// Publish a HomeAssistant MQTT discovery message for the presence `binary_sensor`.
///
/// Called from [`MqttHandle::on_connected`] whenever the feature is enabled.
#[cfg(feature = "mmwave")]
fn publish_presence_discovery(client: &mut EspMqttClient<'static>) -> Result<()> {
    let payload = format!(
        r#"{{
  "name": "Leapyboi Presence",
  "unique_id": "{uid}",
  "device_class": "presence",
  "state_topic": "{state}",
  "payload_on": "ON",
  "payload_off": "OFF",
  "availability_topic": "{avail}",
  "payload_available": "online",
  "payload_not_available": "offline",
  "device": {{
    "identifiers": ["{dev_uid}"],
    "name": "{dev_name}",
    "model": "{model}",
    "manufacturer": "{mfr}"
  }}
}}"#,
        uid     = config::MMWAVE_UNIQUE_ID,
        state   = config::MMWAVE_STATE_TOPIC,
        avail   = config::AVAILABILITY_TOPIC,
        dev_uid = config::DEVICE_UNIQUE_ID,
        dev_name = config::DEVICE_NAME,
        model   = config::DEVICE_MODEL,
        mfr     = config::DEVICE_MANUFACTURER,
    );

    client
        .publish(
            config::MMWAVE_DISCOVERY_TOPIC,
            QoS::AtLeastOnce,
            true, // retain
            payload.as_bytes(),
        )
        .context("failed to publish presence discovery")
}

/// Parse a switch payload byte string into a boolean.
///
/// Accepts `"ON"` (case-insensitive) as `true` and `"OFF"` as `false`.
/// Logs a warning and returns `None` for any other value so that garbage
/// payloads are surfaced in the serial log without silently toggling state.
fn parse_on_off(data: &[u8], topic: &str) -> Option<bool> {
    if data.eq_ignore_ascii_case(b"on") {
        Some(true)
    } else if data.eq_ignore_ascii_case(b"off") {
        Some(false)
    } else {
        warn!(
            "MQTT: ignored unrecognised payload on {topic}: {:?} (expected ON or OFF)",
            core::str::from_utf8(data).unwrap_or("<invalid UTF-8>")
        );
        None
    }
}

/// Publish a HomeAssistant MQTT discovery message for the animations `switch` entity.
fn publish_animations_switch_discovery(client: &mut EspMqttClient<'static>) -> Result<()> {
    let payload = format!(
        r#"{{
  "name": "Leapyboi Animations",
  "unique_id": "leapyboi_animations_switch",
  "state_topic": "{state}",
  "command_topic": "{cmd}",
  "payload_on": "ON",
  "payload_off": "OFF",
  "availability_topic": "{avail}",
  "payload_available": "online",
  "payload_not_available": "offline",
  "device": {{
    "identifiers": ["{dev_uid}"],
    "name": "{dev_name}",
    "model": "{model}",
    "manufacturer": "{mfr}"
  }}
}}"#,
        state    = config::ANIMATIONS_STATE_TOPIC,
        cmd      = config::ANIMATIONS_COMMAND_TOPIC,
        avail    = config::AVAILABILITY_TOPIC,
        dev_uid  = config::DEVICE_UNIQUE_ID,
        dev_name = config::DEVICE_NAME,
        model    = config::DEVICE_MODEL,
        mfr      = config::DEVICE_MANUFACTURER,
    );

    client
        .publish(
            config::ANIMATIONS_DISCOVERY_TOPIC,
            QoS::AtLeastOnce,
            true, // retain
            payload.as_bytes(),
        )
        .context("failed to publish animations switch discovery")
        .map(|_| ())
}

/// Publish a HomeAssistant MQTT discovery message for the mmWave-enable `switch` entity.
#[cfg(feature = "mmwave")]
fn publish_mmwave_enable_switch_discovery(client: &mut EspMqttClient<'static>) -> Result<()> {
    let payload = format!(
        r#"{{
  "name": "Leapyboi mmWave",
  "unique_id": "leapyboi_mmwave_enable_switch",
  "state_topic": "{state}",
  "command_topic": "{cmd}",
  "payload_on": "ON",
  "payload_off": "OFF",
  "availability_topic": "{avail}",
  "payload_available": "online",
  "payload_not_available": "offline",
  "device": {{
    "identifiers": ["{dev_uid}"],
    "name": "{dev_name}",
    "model": "{model}",
    "manufacturer": "{mfr}"
  }}
}}"#,
        state    = config::MMWAVE_ENABLE_STATE_TOPIC,
        cmd      = config::MMWAVE_ENABLE_COMMAND_TOPIC,
        avail    = config::AVAILABILITY_TOPIC,
        dev_uid  = config::DEVICE_UNIQUE_ID,
        dev_name = config::DEVICE_NAME,
        model    = config::DEVICE_MODEL,
        mfr      = config::DEVICE_MANUFACTURER,
    );

    client
        .publish(
            config::MMWAVE_ENABLE_DISCOVERY_TOPIC,
            QoS::AtLeastOnce,
            true, // retain
            payload.as_bytes(),
        )
        .context("failed to publish mmwave-enable switch discovery")
        .map(|_| ())
}

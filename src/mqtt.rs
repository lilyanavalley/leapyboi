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

// ── JSON payload types ────────────────────────────────────────────────────────

/// Incoming command from HomeAssistant (MQTT JSON schema).
#[derive(Deserialize, Debug)]
pub struct LightCommand {
    pub state: Option<String>,
    pub brightness: Option<u8>,
    pub color: Option<RgbColor>,
}

/// Outgoing state published back to HomeAssistant.
#[derive(Serialize, Debug)]
struct LightStatePayload<'a> {
    state: &'a str,
    brightness: u8,
    color: RgbColor,
    color_mode: &'a str,
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

        publish_discovery(&mut self.client)?;

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

    /// Non-blocking receive of the next light command, if any.
    pub fn try_recv_command(&self) -> Option<LightCommand> {
        self.commands.try_recv().ok()
    }

    /// Publish the current light state so HomeAssistant can track it.
    pub fn publish_state(&mut self, state: &LightState) -> Result<()> {
        publish_state(&mut self.client, state)
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
    let reconnected = Arc::new(AtomicBool::new(false));
    let reconnected_thread = Arc::clone(&reconnected);

    // Build a 'static config so that EspMqttClient<'static> can be moved into
    // a thread::spawn closure.  Box::leak is intentional: the MQTT client
    // runs for the entire lifetime of the firmware.
    let conf = Box::new(MqttClientConfiguration {
        client_id: Some(config::MQTT_CLIENT_ID),
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

    spawn_event_loop(connection, cmd_tx, reconnected_thread);

    Ok(MqttHandle {
        client,
        commands: cmd_rx,
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
                        } if topic == config::COMMAND_TOPIC => {
                            match serde_json::from_slice::<LightCommand>(data) {
                                Ok(cmd) => {
                                    cmd_tx.send(cmd).ok();
                                }
                                Err(e) => {
                                    warn!("MQTT: invalid command payload — {e}");
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

//! Driver for the Seeed XIAO 24 GHz mmWave Human Static Presence Sensor
//! (MR24HPC1 / X004QXKIDH).
//!
//! The sensor streams binary frames over UART at 115 200 baud.  This module
//! parses those frames, extracts human-presence events, and forwards them to
//! the caller via an `mpsc` channel.
//!
//! # Frame format
//!
//! ```text
//! ┌──────────┬──────┬──────┬────────────┬──────────┬──────┬───────────┐
//! │ 0x53 0x59 │ ctrl │ cmd  │ len (2 BE) │ data ... │  mh  │ 0x54 0x43 │
//! └──────────┴──────┴──────┴────────────┴──────────┴──────┴───────────┘
//! ```
//!
//! `mh` = (ctrl + cmd + len_h + len_l + Σdata) & 0xFF
//!
//! # Presence frame
//!
//! | Field   | Value                                        |
//! |---------|----------------------------------------------|
//! | ctrl    | `0x80`                                       |
//! | cmd     | `0x81`                                       |
//! | len     | `0x0001`                                     |
//! | data[0] | `0x01` = someone present, `0x00` = no one   |

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use esp_idf_svc::hal::{
    gpio::{AnyIOPin, InputPin, OutputPin},
    peripheral::Peripheral,
    uart::{config::Config as UartConfig, Uart, UartDriver},
    units::Hertz,
};
use log::{error, info, warn};

// ── Frame constants ───────────────────────────────────────────────────────────

const HEADER_1: u8 = 0x53;
const HEADER_2: u8 = 0x59;
const FOOTER_1: u8 = 0x54;
const FOOTER_2: u8 = 0x43;

const CTRL_PRESENCE: u8 = 0x80;
const CMD_PRESENCE: u8 = 0x81;
const DATA_PRESENT: u8 = 0x01;

/// UART read timeout in FreeRTOS ticks (≈ 1 ms/tick → ~100 ms).
const UART_READ_TIMEOUT_TICKS: u32 = 100;

// ── Public API ────────────────────────────────────────────────────────────────

/// A presence event emitted when the sensor's detection state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceEvent {
    /// A person has been detected.
    Detected,
    /// No person is currently detected.
    Gone,
}

/// Owned handle to the running mmWave sensor session.
pub struct MmwaveHandle {
    /// Atomically updated presence flag — set `true` when someone is present.
    pub presence_detected: Arc<AtomicBool>,
    events: mpsc::Receiver<PresenceEvent>,
}

impl MmwaveHandle {
    /// Non-blocking receive of the next presence-change event, if any.
    pub fn try_recv(&self) -> Option<PresenceEvent> {
        self.events.try_recv().ok()
    }
}

// ── Constructor ───────────────────────────────────────────────────────────────

/// Initialise the UART and start the background reader thread.
///
/// * `uart` — UART peripheral (e.g. `peripherals.uart1`)
/// * `tx`   — ESP GPIO wired to the sensor's **RX** pin (for optional commands)
/// * `rx`   — ESP GPIO wired to the sensor's **TX** pin (data stream)
pub fn start<U, TX, RX>(
    uart: impl Peripheral<P = U> + 'static,
    tx: impl Peripheral<P = TX> + 'static,
    rx: impl Peripheral<P = RX> + 'static,
) -> Result<MmwaveHandle>
where
    U: Uart,
    TX: OutputPin,
    RX: InputPin,
{
    let (event_tx, event_rx) = mpsc::channel::<PresenceEvent>();
    let presence = Arc::new(AtomicBool::new(false));
    let presence_clone = Arc::clone(&presence);

    let cfg = UartConfig::new().baudrate(Hertz(115_200));
    let driver = UartDriver::new(
        uart,
        tx,
        rx,
        None::<AnyIOPin>,
        None::<AnyIOPin>,
        &cfg,
    )
    .context("failed to initialise mmWave UART")?;

    std::thread::Builder::new()
        .name("mmwave".into())
        .stack_size(4 * 1024)
        .spawn(move || run_reader(driver, event_tx, presence_clone))
        .expect("failed to spawn mmWave reader thread");

    info!("mmWave sensor initialised");
    Ok(MmwaveHandle {
        presence_detected: presence,
        events: event_rx,
    })
}

// ── Background reader ─────────────────────────────────────────────────────────

fn run_reader(
    driver: UartDriver<'static>,
    event_tx: mpsc::Sender<PresenceEvent>,
    presence: Arc<AtomicBool>,
) {
    info!("mmWave reader thread started");
    let mut state = ParseState::WaitH1;
    let mut buf = [0u8; 64];

    loop {
        match driver.read(&mut buf, UART_READ_TIMEOUT_TICKS) {
            Ok(0) => {
                // Timeout — no bytes received within the window; loop again.
            }
            Ok(n) => {
                for &byte in &buf[..n] {
                    let (next, frame) = feed(state, byte);
                    state = next;
                    if let Some(f) = frame {
                        handle_frame(f, &event_tx, &presence);
                    }
                }
            }
            Err(e) => {
                error!("mmWave UART read error: {e:?}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_frame(
    frame: ParsedFrame,
    event_tx: &mpsc::Sender<PresenceEvent>,
    presence: &AtomicBool,
) {
    if frame.ctrl == CTRL_PRESENCE && frame.cmd == CMD_PRESENCE && frame.data.len() == 1 {
        let detected = frame.data[0] == DATA_PRESENT;
        // Only emit an event when the state actually changes.
        if detected != presence.load(Ordering::Relaxed) {
            presence.store(detected, Ordering::Relaxed);
            let event = if detected {
                PresenceEvent::Detected
            } else {
                PresenceEvent::Gone
            };
            info!("mmWave: presence {}", if detected { "detected" } else { "gone" });
            if event_tx.send(event).is_err() {
                warn!("mmWave: event channel closed — dropping presence event");
            }
        }
    }
}

// ── Frame parser state machine ────────────────────────────────────────────────

struct ParsedFrame {
    ctrl: u8,
    cmd: u8,
    data: Vec<u8>,
}

/// Byte-at-a-time parser state.
///
/// The `sum` field accumulates the checksum (ctrl + cmd + len bytes + data
/// bytes) so it can be compared against the `mh` byte at the end of the frame.
enum ParseState {
    WaitH1,
    WaitH2,
    ReadCtrl,
    ReadCmd     { ctrl: u8,                                    sum: u16 },
    ReadLenH    { ctrl: u8, cmd: u8,                           sum: u16 },
    ReadLenL    { ctrl: u8, cmd: u8, len_h: u8,                sum: u16 },
    ReadData    { ctrl: u8, cmd: u8, remaining: u16, data: Vec<u8>, sum: u16 },
    ReadMH      { ctrl: u8, cmd: u8,                data: Vec<u8>, sum: u16 },
    ReadFooter1 { ctrl: u8, cmd: u8,                data: Vec<u8> },
    ReadFooter2 { ctrl: u8, cmd: u8,                data: Vec<u8> },
}

/// Advance the parser by one byte.
///
/// Returns `(new_state, Some(frame))` when a complete, valid frame has been
/// received; otherwise returns `(new_state, None)`.
fn feed(state: ParseState, byte: u8) -> (ParseState, Option<ParsedFrame>) {
    use ParseState::*;
    match state {
        WaitH1 => {
            if byte == HEADER_1 {
                (WaitH2, None)
            } else {
                (WaitH1, None)
            }
        }
        WaitH2 => {
            if byte == HEADER_2 {
                (ReadCtrl, None)
            } else if byte == HEADER_1 {
                // Handle 0x53 0x53 … gracefully.
                (WaitH2, None)
            } else {
                (WaitH1, None)
            }
        }
        ReadCtrl => (ReadCmd { ctrl: byte, sum: byte as u16 }, None),
        ReadCmd { ctrl, sum } => (
            ReadLenH {
                ctrl,
                cmd: byte,
                sum: sum + byte as u16,
            },
            None,
        ),
        ReadLenH { ctrl, cmd, sum } => (
            ReadLenL {
                ctrl,
                cmd,
                len_h: byte,
                sum: sum + byte as u16,
            },
            None,
        ),
        ReadLenL { ctrl, cmd, len_h, sum } => {
            let len = ((len_h as u16) << 8) | (byte as u16);
            let sum = sum + byte as u16;
            if len == 0 {
                (ReadMH { ctrl, cmd, data: Vec::new(), sum }, None)
            } else {
                (
                    ReadData {
                        ctrl,
                        cmd,
                        remaining: len,
                        data: Vec::with_capacity(len as usize),
                        sum,
                    },
                    None,
                )
            }
        }
        ReadData { ctrl, cmd, remaining, mut data, sum } => {
            data.push(byte);
            let sum = sum + byte as u16;
            if remaining == 1 {
                (ReadMH { ctrl, cmd, data, sum }, None)
            } else {
                (ReadData { ctrl, cmd, remaining: remaining - 1, data, sum }, None)
            }
        }
        ReadMH { ctrl, cmd, data, sum } => {
            let expected = (sum & 0xFF) as u8;
            if byte == expected {
                (ReadFooter1 { ctrl, cmd, data }, None)
            } else {
                warn!("mmWave checksum mismatch: got {byte:#04x}, expected {expected:#04x}");
                (WaitH1, None)
            }
        }
        ReadFooter1 { ctrl, cmd, data } => {
            if byte == FOOTER_1 {
                (ReadFooter2 { ctrl, cmd, data }, None)
            } else {
                (WaitH1, None)
            }
        }
        ReadFooter2 { ctrl, cmd, data } => {
            if byte == FOOTER_2 {
                (WaitH1, Some(ParsedFrame { ctrl, cmd, data }))
            } else {
                (WaitH1, None)
            }
        }
    }
}

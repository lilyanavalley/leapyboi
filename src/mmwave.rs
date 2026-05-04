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

#[cfg(feature = "mmwave-diagnostics")]
use std::time::Instant;

use anyhow::{Context, Result};
use esp_idf_svc::hal::{
    gpio::{AnyIOPin, InputPin, OutputPin},
    peripheral::Peripheral,
    uart::{config::Config as UartConfig, Uart, UartDriver},
    units::Hertz,
};
use log::{error, info, trace, warn};

// ── Frame constants ───────────────────────────────────────────────────────────

const HEADER_1: u8 = 0x53;
const HEADER_2: u8 = 0x59;
const FOOTER_1: u8 = 0x54;
const FOOTER_2: u8 = 0x43;

// Observed on Seeed 24GHz mmWave for XIAO module:
// fixed 9-byte packets: DF F3 <type> <status> <b0> <b1> <b2> E8 CF
const XIAO_HEADER_1: u8 = 0xDF;
const XIAO_HEADER_2: u8 = 0xF3;
const XIAO_FOOTER_1: u8 = 0xE8;
const XIAO_FOOTER_2: u8 = 0xCF;
const XIAO_FRAME_LEN: usize = 9;

const CTRL_PRESENCE: u8 = 0x80;
const CMD_PRESENCE: u8 = 0x81;
const DATA_PRESENT: u8 = 0x01;

/// UART read timeout in FreeRTOS ticks (≈ 1 ms/tick → ~100 ms).
const UART_READ_TIMEOUT_TICKS: u32 = 100;
#[cfg(feature = "mmwave-diagnostics")]
const UART_DIAG_REPORT_INTERVAL: Duration = Duration::from_secs(5);

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
    Ok(MmwaveHandle { events: event_rx })
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
    let mut xiao_buf = Vec::<u8>::with_capacity(XIAO_FRAME_LEN);

    #[cfg(feature = "mmwave-diagnostics")]
    let mut diag_last_report = Instant::now();
    #[cfg(feature = "mmwave-diagnostics")]
    let mut diag_timeouts: u32 = 0;
    #[cfg(feature = "mmwave-diagnostics")]
    let mut diag_bytes: u32 = 0;
    #[cfg(feature = "mmwave-diagnostics")]
    let mut diag_frames: u32 = 0;
    #[cfg(feature = "mmwave-diagnostics")]
    let mut diag_expected_preambles: u32 = 0;
    #[cfg(feature = "mmwave-diagnostics")]
    let mut diag_xiao_preambles: u32 = 0;
    #[cfg(feature = "mmwave-diagnostics")]
    let mut recent2 = [0u8; 2];

    loop {
        match driver.read(&mut buf, UART_READ_TIMEOUT_TICKS) {
            Ok(0) => {
                // Timeout — no bytes received within the window; loop again.
                #[cfg(feature = "mmwave-diagnostics")]
                {
                    diag_timeouts = diag_timeouts.saturating_add(1);
                }
            }
            Ok(n) => {
                #[cfg(feature = "mmwave-diagnostics")]
                {
                    diag_bytes = diag_bytes.saturating_add(n as u32);
                }

                for &byte in &buf[..n] {
                    #[cfg(feature = "mmwave-diagnostics")]
                    {
                        recent2[0] = recent2[1];
                        recent2[1] = byte;
                        if recent2 == [HEADER_1, HEADER_2] {
                            diag_expected_preambles =
                                diag_expected_preambles.saturating_add(1);
                        }
                        if recent2 == [XIAO_HEADER_1, XIAO_HEADER_2] {
                            diag_xiao_preambles =
                                diag_xiao_preambles.saturating_add(1);
                        }
                    }

                    if let Some(frame) = feed_xiao_frame(&mut xiao_buf, byte) {
                        #[cfg(feature = "mmwave-diagnostics")]
                        {
                            diag_frames = diag_frames.saturating_add(1);
                        }
                        handle_xiao_frame(frame, &event_tx, &presence);
                    }

                    let (next, frame) = feed(state, byte);
                    state = next;
                    if let Some(f) = frame {
                        #[cfg(feature = "mmwave-diagnostics")]
                        {
                            diag_frames = diag_frames.saturating_add(1);
                        }
                        trace!("mmWave frame received: ctrl={:#04x} cmd={:#04x} data={:02x?}", f.ctrl, f.cmd, f.data);
                        handle_frame(f, &event_tx, &presence);
                    }
                }
            }
            Err(e) => {
                error!("mmWave UART read error: {e:?}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        #[cfg(feature = "mmwave-diagnostics")]
        {
            if diag_last_report.elapsed() >= UART_DIAG_REPORT_INTERVAL {
                info!(
                    "mmWave diagnostics: bytes={} frames={} read_timeouts={} preambles(expected_5359={}, xiao_df_f3={}) over {:?}",
                    diag_bytes,
                    diag_frames,
                    diag_timeouts,
                    diag_expected_preambles,
                    diag_xiao_preambles,
                    UART_DIAG_REPORT_INTERVAL
                );
                diag_last_report = Instant::now();
                diag_timeouts = 0;
                diag_bytes = 0;
                diag_frames = 0;
                diag_expected_preambles = 0;
                diag_xiao_preambles = 0;
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct XiaoFrame {
    frame_type: u8,
    status: u8,
    b0: u8,
    b1: u8,
    b2: u8,
}

fn feed_xiao_frame(buf: &mut Vec<u8>, byte: u8) -> Option<XiaoFrame> {
    if buf.is_empty() {
        if byte == XIAO_HEADER_1 {
            buf.push(byte);
        }
        return None;
    }

    if buf.len() == 1 {
        if byte == XIAO_HEADER_2 {
            buf.push(byte);
        } else if byte == XIAO_HEADER_1 {
            buf.clear();
            buf.push(byte);
        } else {
            buf.clear();
        }
        return None;
    }

    buf.push(byte);
    if buf.len() < XIAO_FRAME_LEN {
        return None;
    }

    let parsed = if buf[7] == XIAO_FOOTER_1 && buf[8] == XIAO_FOOTER_2 {
        Some(XiaoFrame {
            frame_type: buf[2],
            status: buf[3],
            b0: buf[4],
            b1: buf[5],
            b2: buf[6],
        })
    } else {
        None
    };

    // Re-sync on next potential header.
    if byte == XIAO_HEADER_1 {
        buf.clear();
        buf.push(byte);
    } else {
        buf.clear();
    }

    parsed
}

fn emit_presence_if_changed(
    detected: bool,
    event_tx: &mpsc::Sender<PresenceEvent>,
    presence: &AtomicBool,
) {
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

fn handle_xiao_frame(
    frame: XiaoFrame,
    event_tx: &mpsc::Sender<PresenceEvent>,
    presence: &AtomicBool,
) {
    let detected = xiao_detected_from_status(frame.status);
    trace!(
        "mmWave XIAO frame: type={:#04x} status={:#04x} b0={:#04x} b1={:#04x} b2={:#04x}",
        frame.frame_type,
        frame.status,
        frame.b0,
        frame.b1,
        frame.b2
    );
    emit_presence_if_changed(detected, event_tx, presence);
}

fn xiao_detected_from_status(status: u8) -> bool {
    // Observed status values (0x08 / 0x18) suggest bit 0x10 carries occupancy.
    (status & 0x10) != 0
}

fn handle_frame(
    frame: ParsedFrame,
    event_tx: &mpsc::Sender<PresenceEvent>,
    presence: &AtomicBool,
) {
    if frame.ctrl == CTRL_PRESENCE && frame.cmd == CMD_PRESENCE && frame.data.len() == 1 {
        let detected = frame.data[0] == DATA_PRESENT;
        emit_presence_if_changed(detected, event_tx, presence);
    } else {
        trace!(
            "mmWave: unhandled frame ctrl={:#04x} cmd={:#04x} len={} data={:02x?}",
            frame.ctrl,
            frame.cmd,
            frame.data.len(),
            frame.data
        );
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

#[cfg(test)]
mod tests {
    #[test]
    fn parses_single_xiao_frame() {
        let mut buf = Vec::new();
        let bytes = [0xDF, 0xF3, 0x20, 0x18, 0x01, 0x0B, 0x08, 0xE8, 0xCF];
        let mut parsed = None;

        for b in bytes {
            parsed = super::feed_xiao_frame(&mut buf, b);
        }

        let frame = parsed.expect("expected XIAO frame");
        assert_eq!(frame.frame_type, 0x20);
        assert_eq!(frame.status, 0x18);
        assert_eq!((frame.b0, frame.b1, frame.b2), (0x01, 0x0B, 0x08));
    }

    #[test]
    fn xiao_parser_resyncs_after_bad_frame() {
        let mut buf = Vec::new();
        // First frame has bad footer; second frame is valid.
        let bytes = [
            0xDF, 0xF3, 0x20, 0x18, 0x01, 0x0B, 0x08, 0x00, 0x00,
            0xDF, 0xF3, 0x20, 0x08, 0x01, 0x0B, 0x01, 0xE8, 0xCF,
        ];

        let mut parsed = None;
        for b in bytes {
            if let Some(frame) = super::feed_xiao_frame(&mut buf, b) {
                parsed = Some(frame);
            }
        }

        let frame = parsed.expect("expected resynced XIAO frame");
        assert_eq!(frame.status, 0x08);
        assert_eq!((frame.b0, frame.b1, frame.b2), (0x01, 0x0B, 0x01));
    }

    #[test]
    fn xiao_status_bitmaps_to_presence() {
        assert!(super::xiao_detected_from_status(0x18));
        assert!(!super::xiao_detected_from_status(0x08));
    }
}

# leapyboi

[![Rust](https://img.shields.io/badge/Rust-%23000000.svg?e&logo=rust&logoColor=white)](#)
[![GitHub Copilot](https://img.shields.io/badge/GitHub%20Copilot-000?logo=githubcopilot&logoColor=fff)](#)
[![GitHub Actions](https://img.shields.io/badge/GitHub_Actions-2088FF?logo=github-actions&logoColor=white)](#)
[![GPL-3 Licensed](https://upload.wikimedia.org/wikipedia/commons/8/86/GPL_v3_Blue_Badge.svg?utm_source=commons.wikimedia.org&utm_campaign=index&utm_content=original)](#)

Working model of an ARC Raiders 'Leaper' with features:
- addressable LED ring of NeoPixels,
- connects to MQTT & HomeAssistant,
- (optional) mmWave Human presence detection

I wanted to build a reactive Leaper model using [this 3D printable by 3DMN](https://makerworld.com/en/models/2314329-arc-raiders-leaper-articulated-rgb-led#profileId-2526937) (credit to the author for their awesome model!).
This firmware uses an ESP32-C6 to connect to WiFi and communicates with HomeAssistant via MQTT to do cool things a Leaper would IRL... Including changing the color of the LED ring in response to a person entering/exiting the mmWave detection area.
As a baseline, this firmware exposes a light entity to HomeAssistant for direct control of the LED ring and consequently any automation(s) you write for it.

---

ESP32-C6 firmware written in Rust that connects a **WS2812B Neopixel LED ring**
to **HomeAssistant** via **MQTT over WiFi**.

The firmware exposes the ring as a standard HA `light` entity with full
on/off, brightness (0-255), and RGB colour control.

Optionally, a **Seeed XIAO 24 GHz mmWave Human Static Presence Sensor**
(MR24HPC1 / X004QXKIDH) can be added to automatically change the ring's colour
based on whether someone is in the room.  When enabled, a `binary_sensor`
presence entity also appears in HomeAssistant.

---

## Hardware

| Component | Notes |
|-----------|-------|
| ESP32-C6 dev board | Any module with ≥ 4 MB flash |
| WS2812B LED ring | 12-LED ring tested; change `LED_COUNT` in `src/config.rs` |
| 5 V power supply | Power the ring directly; **do not** power a full ring from the USB 5 V rail |
| 300–500 Ω resistor | In series on the DIN data line to protect against ringing |
| 1000 µF capacitor | Across the ring's power rails to absorb current spikes |
| *(optional)* Seeed XIAO 24 GHz mmWave sensor | UART, 3.3 V; see [mmWave section](#mmwave-presence-sensor-optional) |

**Default wiring**

```
ESP32-C6 GPIO 2  ──[330Ω]──▶  Ring DIN
ESP32-C6 GND     ────────────  Ring GND
5V supply        ────────────  Ring 5V
```

For Seeed XIAO ESP32-C6, GPIO 2 maps to header pin D0.

Change the pin in `cfg.toml` → `led_data_pin_num` if you use a different GPIO.
Any RMT-capable output pin works.

### Boot LED self-test

Before WiFi starts, firmware now runs a quick LED self-test sequence
(red → green → blue → white). If you do not see this sequence, the issue is
in the LED transport path (pin assignment/wiring/signal level), not MQTT.

The configured LED GPIO is also printed on boot to make pin mismatches obvious.

---

## Prerequisites

### Rust + ESP toolchain

```bash
# Install Rust
curl https://sh.rustup.rs -sSf | sh

# Install espup (manages the ESP Rust fork + tools)
cargo install espup
espup install          # adds the `esp` toolchain and RISC-V targets

# Install espflash (flash + serial monitor)
cargo install espflash

# Install ldproxy (forwards arguments to linker)
cargo install ldproxy
```

### ESP-IDF

`embuild` downloads and caches ESP-IDF automatically on the first build.
Nothing extra is needed; the version is pinned in `.cargo/config.toml`
(`ESP_IDF_VERSION = "v5.2.1"`).

---

## Configuration

```bash
cp cfg.toml.example cfg.toml
$EDITOR cfg.toml          # fill in WiFi, MQTT broker URL, and MQTT auth fields
```

`cfg.toml` is gitignored to keep credentials out of version control.
`build.rs` reads the file at compile time and bakes the values into the
firmware binary.

---

## Build & flash

```bash
# Debug build (larger binary, serial logging enabled)
cargo build

# Release build (size-optimised)
cargo build --release

# Build, flash, and open the serial monitor in one step
cargo run --release
```

> **espflash** auto-detects the serial port.  If it fails, pass the port
> explicitly: `cargo run --release -- --port /dev/ttyUSB0`

---

## mmWave presence sensor *(optional)*

The [Seeed XIAO 24 GHz mmWave Human Static Presence Sensor](https://www.seeedstudio.com/24GHz-mmWave-Sensor-Human-Static-Presence-Module-Lite-p-5524.html)
(MR24HPC1 / X004QXKIDH) can be added to automatically respond to occupancy.

### What it does

| State | LED ring behaviour |
|-------|--------------------|
| Person **detected** | Turns **on** with warm-white (configurable in `src/config.rs`) |
| **No** person detected | Switches to a dim blue tint (or turns off if `NO_PRESENCE_COLOR = (0,0,0)`) |

A `binary_sensor` **Leapyboi Presence** entity also appears in HomeAssistant
via MQTT discovery.  HomeAssistant light commands continue to work normally —
the presence sensor is an additional input, and each presence change is
reflected back to HA.

### Wiring

The sensor operates at **3.3 V** and communicates via **UART at 115 200 baud**.

```
Sensor 3V3  ────────────  ESP32-C6 3V3
Sensor GND  ────────────  ESP32-C6 GND
Sensor TX   ────────────  ESP32-C6 GPIO 17 (ESP RX ← sensor data stream)
Sensor RX   ────────────  ESP32-C6 GPIO 16 (ESP TX → sensor, for config)
```

> **Note:** `Sensor RX` / `ESP GPIO 16` is only needed if you want to send
> configuration commands to the sensor.  For basic presence detection it can
> be left unconnected.

Change the GPIO numbers in `src/main.rs` (`peripherals.pins.gpio16 / gpio17`)
to match your actual wiring.

For Seeed XIAO ESP32-C6, the firmware defaults to:

- `GPIO16` (D6) as ESP TX → sensor RX
- `GPIO17` (D7) as ESP RX ← sensor TX

### Enabling the feature

```bash
# Debug build with mmWave support
cargo build --features mmwave

# Release build
cargo build --release --features mmwave

# Build, flash, and monitor
cargo run --release --features mmwave
```

### Supported UART frame protocols

The mmWave parser currently supports two frame families:

- `0x53 0x59 ... 0x54 0x43` (MR24HPC1-style protocol)
- `0xDF 0xF3 ... 0xE8 0xCF` (Seeed XIAO 24 GHz mmWave stream)

### Optional diagnostics mode

For deeper UART troubleshooting, enable feature-gated diagnostics:

```bash
cargo run --features "mmwave,mmwave-diagnostics"
```

This emits periodic parser diagnostics (bytes, parsed frames, timeout count,
and preamble hit counters) without changing production defaults.

### Customising behaviour

| Constant | Default | Meaning |
|----------|---------|---------|
| `PRESENCE_COLOR` | `(255, 200, 100)` | LED colour when someone is present |
| `PRESENCE_BRIGHTNESS` | `200` | Brightness (0-255) when present |
| `NO_PRESENCE_COLOR` | `(0, 0, 30)` | LED colour when no one is detected |
| `NO_PRESENCE_BRIGHTNESS` | `50` | Brightness when no one is detected |

Set `NO_PRESENCE_COLOR = (0, 0, 0)` to turn the ring **off** when the room is empty.

All constants live in `src/config.rs` under the `mmwave` feature gate.
## Debug logging

On startup, the firmware logs which MQTT authentication mode is active.
This helps diagnose EMQX / broker authz mismatches without exposing secrets.

Example log line:

```text
MQTT auth mode: username+password (username=set, password=set)
```

Possible modes:

- `username+password`
- `username-only`
- `password-only`
- `client-id-only`

`set` / `unset` only indicate whether fields are configured; actual
credential values are never printed.

---

## HomeAssistant setup

The firmware uses the
[MQTT discovery](https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery)
protocol.  Once the device is online, a new **Leapyboi Ring** entity appears
automatically under **Settings → Devices & Services → MQTT**.

No manual YAML configuration is required.

### MQTT broker

The easiest option is the
[Mosquitto add-on](https://github.com/home-assistant/addons/tree/master/mosquitto)
for HA OS / Supervised.  Point `mqtt_url` in `cfg.toml` at the broker's IP
address on port 1883.

---

## Project layout

```
leapyboi/
├── src/
│   ├── main.rs      # Boot sequence and main loop
│   ├── config.rs    # Compile-time constants (pin, topic names, …)
│   ├── led.rs       # WS2812B ring controller (generic over SmartLedsWrite)
│   ├── wifi.rs      # WiFi connection helper
│   ├── mqtt.rs      # MQTT client, HA discovery, command / state handling
│   └── mmwave.rs    # mmWave UART driver + frame parser (feature: mmwave)
├── build.rs         # Reads cfg.toml, calls embuild for ESP-IDF
├── Cargo.toml
├── sdkconfig.defaults  # ESP-IDF Kconfig overrides
├── cfg.toml.example    # Credential template (committed)
└── .cargo/
    └── config.toml  # Build target, linker, runner
```

---

## Customisation

| What | Where |
|------|-------|
| Number of LEDs | `src/config.rs` → `LED_COUNT` |
| Data GPIO pin | `cfg.toml` → `led_data_pin_num` |
| MQTT topics | `src/config.rs` → `HA_DISCOVERY_TOPIC`, `COMMAND_TOPIC`, … |
| Device name shown in HA | `src/config.rs` → `DEVICE_NAME` |
| ESP-IDF version | `.cargo/config.toml` → `ESP_IDF_VERSION` |
| mmWave UART pins | `src/main.rs` → `peripherals.pins.gpio16 / gpio17` |
| mmWave LED colours | `src/config.rs` → `PRESENCE_COLOR`, `NO_PRESENCE_COLOR`, … |


## License

This software is licensed under the GNU General Public License, version 3.

[![GNU GPL-3 License Logo](https://upload.wikimedia.org/wikipedia/commons/9/93/GPLv3_Logo.svg?utm_source=commons.wikimedia.org&utm_campaign=imageinfo&utm_content=original)](#)
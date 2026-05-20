
[![leapyboi Logo](/docs/leapyboi.png)](#)

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
What you see here is firmware for an ESP32-C6 using the **ESP IDF** to connect to WiFi and communicate with **HomeAssistant via MQTT** to do cool things a Leaper would IRL... Including changing the color of the LED ring in response to any automation you write for it.

---

Connects a **WS2812B Neopixel LED ring**
to **HomeAssistant** via **MQTT over WiFi**. This firmware exposes the ring as a standard HA `light` entity with full on/off, brightness (0-255), and RGB colour control.

Optionally, a **Seeed XIAO 24 GHz mmWave Human Static Presence Sensor**
(MR24HPC1 / X004QXKIDH) can be added to automatically change the ring's colour
based on whether someone is in the room.  When enabled, a `binary_sensor`
presence entity also appears in HomeAssistant.

I'm using an ESP32-C6, but with the IDF, you *should* be able to port this code to another ESP32 chip given some tweaking of `cfg.toml` and specifying the correct board when you invoke `cargo`. See instructions below for a sample of this for the C6 chip.

---

## Hardware

| Component | Notes |
|-----------|-------|
| ESP32-C6 dev board | Any module with ≥ 4 MB flash |
| WS2812B LED ring | 24-LED ring tested; change `LED_COUNT` in `src/config.rs` |
| 5 V power supply | Power the ring directly; **do not** power a full ring from the USB 5V rail |
| 300–500 Ω resistor | In series on the DIN data line to WS2812B to protect against ringing |
| 1000 µF capacitor | Across the WS2812B's power rails to absorb current spikes |
| *(optional)* Seeed XIAO 24 GHz mmWave sensor | UART, 3.3 V; see [mmWave section](#mmwave-presence-sensor-optional) |

### Wiring Diagram

```
ESP32-C6 GPIO 2  ──[330Ω]──▶  Ring DIN
ESP32-C6 GND     ────────────  Ring GND
5V supply        ────────────  Ring 5V
```

For a **Seeed XIAO ESP32-C6**, *GPIO 2* maps to header pin *D0*. Change the pin in `cfg.toml` → `led_data_pin_num` if you use a different GPIO pin. ***Make note of the chip's GPIO pin, not the number printed on the board.*** Any RMT-capable output pin works. You can check out [this diagram](https://wiki.seeedstudio.com/xiao_esp32c6_getting_started/#hardware-overview) of the XIAO board for a pinout and the appropriate pin to use.

### Boot LED self-test

Before WiFi starts, firmware runs a quick LED self-test sequence (red → green → blue → white). If you do not see this sequence, you *might* want to check that the pin you've selected in `cfg.toml` is the GPIO pin you've wired to the WS2812B... The configured LED GPIO is also printed on boot to make pin mismatches obvious.

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
$EDITOR cfg.toml # fill in required fields!
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

---

## LED animations

The ring supports several built-in animation effects selectable from
HomeAssistant or via raw MQTT.

### Available effects

| Effect name | Description |
|-------------|-------------|
| `solid`     | All LEDs show a single static colour (default) |
| `rainbow`   | A rotating rainbow hue-wheel fills all LEDs |
| `spinning`  | A single bright pixel (plus short fading tail) circles the ring |
| `breathe`   | All LEDs fade in and out using the current colour |
| `custom`    | User-supplied frame animation — see below |

### Selecting an effect in HomeAssistant

The **Leapyboi Ring** light entity in HA shows an **Effect** dropdown
alongside the usual colour/brightness controls.  Pick any effect from the list
and it takes effect immediately.

You can also send the effect directly over MQTT:

```
Topic:   leapyboi/light/set
Payload: {"state":"ON","effect":"rainbow"}
```

### Tuning animation speed

All timing constants live in `src/config.rs` — no other file needs changing:

| Constant | Default | Effect on… |
|----------|---------|------------|
| `ANIM_RAINBOW_SPEED` | `2` | Hue steps per tick; higher = faster rotation |
| `ANIM_SPINNING_TICKS_PER_PIXEL` | `2` | Ticks between pixel moves; lower = faster spin |
| `ANIM_BREATHE_SPEED` | `2` | Phase steps per tick; higher = faster breathing |
| `ANIM_CUSTOM_TICKS_PER_FRAME` | `4` | Ticks per frame of the custom animation |

One tick ≈ 50 ms (the main-loop period).

### Disabling animations at runtime

A **Leapyboi Animations** toggle switch appears in HomeAssistant alongside the
light card.  Turning it **off** freezes the ring on a static solid colour
(preserving the current brightness and RGB values) without losing the selected
effect — re-enable the switch and the animation resumes from where it left off.

You can also toggle directly over MQTT:

```
Topic:   leapyboi/animations/set
Payload: OFF          # pause animations
Payload: ON           # resume animations
```

---

## Custom animation via `animation.png`

You can bake a fully custom pixel animation into the firmware by placing an
image file called `animation.png` in the project root (next to `Cargo.toml`).

### Image format

| Dimension | Meaning |
|-----------|---------|
| Width     | **Must equal `LED_COUNT`** (default: 12) — one pixel per LED |
| Height    | Number of animation frames — as many as you like |
| Colour    | 8-bit **RGB** or **RGBA** (alpha is ignored) |

Each row in the image is one frame; rows are played top-to-bottom and loop
continuously.

### Creating an animation

1. Open any image editor (GIMP, Aseprite, Photoshop, etc.)
2. Create a new image that is **12 × N** pixels (or `LED_COUNT × N`)
3. Draw each frame as a row of 12 pixels
4. Export as `animation.png` (8-bit RGB or RGBA) and place it in the project root
5. Rebuild and reflash:

```bash
cargo build --release      # build.rs decodes the PNG and bakes it in
cargo run  --release       # flash + open serial monitor
```

If the image dimensions are wrong the build will fail with a helpful error.

### Sharing animations

Because the animation is a standard PNG file you can share it directly with
other Leapyboi users.  Post the file anywhere images are shared; recipients
drop it into their project root and reflash.

### Removing the custom animation

Delete `animation.png` and rebuild.  The `custom` effect will disappear from
the HomeAssistant dropdown automatically.

---

## mmWave presence sensor *(optional)*

The [Seeed XIAO 24 GHz mmWave Human Static Presence Sensor](https://www.seeedstudio.com/24GHz-mmWave-Sensor-Human-Static-Presence-Module-Lite-p-5524.html)
(MR24HPC1 / X004QXKIDH) can be added to automatically respond to occupancy.

### What it does

| State | LED ring behaviour |
|-------|--------------------|
| Person **detected** | Turns **on** with red color (configurable in `src/config.rs`) |
| **No** person detected | Switches to an amber hue (or turns off if `NO_PRESENCE_COLOR = (0,0,0)`) |

To reduce rapid flashing between occupancy changes, mmWave light transitions now use
two safety mechanisms by default:

- **Transition lockout:** `2000 ms` between applied presence-driven light changes
- **Smooth transition:** short interpolated fade instead of instant jump

A `binary_sensor` **Leapyboi Presence** entity also appears in HomeAssistant
via MQTT discovery.  HomeAssistant light commands continue to work normally —
the presence sensor is an additional input, and each presence change is
reflected back to HA.

A separate **Leapyboi mmWave** toggle switch also appears in HA.  Turning it
**off** makes the firmware ignore all presence events (the ring stays at
whatever state HA last commanded) without restarting or reflashing.  Re-enable
it at any time and presence reactions resume immediately.

You can also toggle it directly over MQTT:

```
Topic:   leapyboi/mmwave/enable/set
Payload: OFF    # ignore presence events
Payload: ON     # resume presence reactions
```

### Wiring

The sensor operates at **3.3 V** and communicates via **UART at 115,200 baud**.

```
Sensor 3V3  ────────────  ESP32-C6 3V3
Sensor GND  ────────────  ESP32-C6 GND
Sensor TX   ────────────  ESP32-C6 GPIO 17 (ESP RX ← sensor data stream)
Sensor RX   ────────────  ESP32-C6 GPIO 16 (ESP TX → sensor, for config)
```

> **Note:** `Sensor RX` / `ESP GPIO 16` is only needed if you want to send
> configuration commands to the sensor.  For basic presence detection it can
> be left unconnected.

Set the UART pins in `cfg.toml` to match your actual wiring:

- `mmwave_uart_tx_pin_num` = ESP TX pin wired to sensor RX
- `mmwave_uart_rx_pin_num` = ESP RX pin wired to sensor TX
- `mmwave_uart_port` = UART peripheral (`0` for uart0, `1` for uart1)

Current firmware defaults are:

- `GPIO21` as ESP TX → sensor RX
- `GPIO2` as ESP RX ← sensor TX

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
| `PRESENCE_COLOR` | `(255, 0, 0)` | LED colour when someone is present |
| `PRESENCE_BRIGHTNESS` | `200` | Brightness (0-255) when present |
| `PRESENCE_ANIMATION` | `"rainbow"` | Animation played when present |
| `NO_PRESENCE_COLOR` | `(255, 255, 0)` | LED colour when no one is detected |
| `NO_PRESENCE_BRIGHTNESS` | `50` | Brightness when no one is detected |
| `MMWAVE_TRANSITION_LOCKOUT_DEFAULT_MS` | `2000` | Default minimum time between mmWave-driven light transitions |
| `MMWAVE_TRANSITION_STEPS` | `12` | Number of steps in each mmWave transition |
| `MMWAVE_TRANSITION_STEP_DELAY_MS` | `60` | Delay between mmWave transition steps |

Set `NO_PRESENCE_COLOR = (0, 0, 0)` to turn the ring **off** when the room is empty.

All constants live in `src/config.rs` under the `mmwave` feature gate.

### Runtime transition safety control (MQTT)

You can change the mmWave transition lockout on-the-fly over MQTT:

- Set topic: `leapyboi/mmwave/transition_lockout_ms/set`
- State topic: `leapyboi/mmwave/transition_lockout_ms/state`
- Units: milliseconds

This value is persisted in ESP NVS and restored after reboot.

Setting the lockout to `0` enables **instant mode** (no lockout + no fade).

⚠️ ***Health warning:*** instant mode can cause rapid light transitions within seconds,
which ***may trigger photosensitive responses*** (including ***seizure risk***). Use with care.

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
│   ├── main.rs        # Boot sequence and main loop
│   ├── config.rs      # Compile-time constants (pin, topic names, animation speeds, …)
│   ├── animations.rs  # Animation engine (solid, rainbow, spinning, breathe, custom)
│   ├── led.rs         # WS2812B ring controller (generic over SmartLedsWrite)
│   ├── wifi.rs        # WiFi connection helper
│   ├── mqtt.rs        # MQTT client, HA discovery, command / state handling
│   └── mmwave.rs      # mmWave UART driver + frame parser (feature: mmwave)
├── animation.png       # Optional: custom animation (width=LED_COUNT, height=frames)
├── build.rs            # Reads cfg.toml; decodes animation.png; calls embuild for ESP-IDF
├── Cargo.toml
├── sdkconfig.defaults  # ESP-IDF Kconfig overrides
├── cfg.toml.example    # Credential template (committed)
└── .cargo/
    └── config.toml     # Build target, linker, runner
```

---

## Customisation

| What | Where |
|------|-------|
| Number of LEDs | `src/config.rs` → `LED_COUNT` |
| Data GPIO pin | `cfg.toml` → `led_data_pin_num` |
| MQTT topics | `src/config.rs` → `HA_DISCOVERY_TOPIC`, `COMMAND_TOPIC`, … |
| Device name shown in HA | `src/config.rs` → `DEVICE_NAME` |
| Animation speeds | `src/config.rs` → `ANIM_RAINBOW_SPEED`, `ANIM_BREATHE_SPEED`, … |
| Custom animation frames | Place `animation.png` in project root (width=`LED_COUNT`, height=frames) |
| Toggle animations at runtime | HA **Leapyboi Animations** switch, or `leapyboi/animations/set` (`ON`/`OFF`) |
| ESP-IDF version | `.cargo/config.toml` → `ESP_IDF_VERSION` |
| mmWave UART pins | `src/main.rs` → `peripherals.pins.gpio16 / gpio17` |
| mmWave LED colours | `src/config.rs` → `PRESENCE_COLOR`, `NO_PRESENCE_COLOR`, … |
| mmWave LED animations | `src/config.rs` → `PRESENCE_ANIMATION`, `NO_PRESENCE_ANIMATION` |
| Toggle mmWave at runtime | HA **Leapyboi mmWave** switch, or `leapyboi/mmwave/enable/set` (`ON`/`OFF`) |


## License

This software is licensed under the GNU General Public License, version 3.

[![GNU GPL-3 License Logo](https://upload.wikimedia.org/wikipedia/commons/9/93/GPLv3_Logo.svg?utm_source=commons.wikimedia.org&utm_campaign=imageinfo&utm_content=original)](#)

# leapyboi

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
ESP32-C6 GPIO 8  ──[330Ω]──▶  Ring DIN
ESP32-C6 GND     ────────────  Ring GND
5V supply        ────────────  Ring 5V
```

Change the pin in `cfg.toml` → `led_data_pin_num` if you use a different GPIO.
Any RMT-capable output pin works.

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
| Person **detected** | Ring turns **on** with the configured presence colour and animation |
| **No** person detected | Ring switches to the no-presence colour and animation (or turns off) |

A `binary_sensor` **Leapyboi Presence** entity also appears in HomeAssistant
via MQTT discovery.  HomeAssistant light commands continue to work normally —
the presence sensor is an additional input, and each presence change is
reflected back to HA.

### Wiring

The sensor operates at **3.3 V** and communicates via **UART at 115 200 baud**.

```
Sensor 3V3  ────────────  ESP32-C6 3V3
Sensor GND  ────────────  ESP32-C6 GND
Sensor TX   ────────────  ESP32-C6 GPIO 4  (ESP RX ← sensor data stream)
Sensor RX   ────────────  ESP32-C6 GPIO 5  (ESP TX → sensor, for config)
```

> **Note:** `Sensor RX` / `ESP GPIO 5` is only needed if you want to send
> configuration commands to the sensor.  For basic presence detection it can
> be left unconnected.

Change the GPIO numbers in `src/main.rs` (`peripherals.pins.gpio4 / gpio5`)
to match your actual wiring.

### Enabling the feature

```bash
# Debug build with mmWave support
cargo build --features mmwave

# Release build
cargo build --release --features mmwave

# Build, flash, and monitor
cargo run --release --features mmwave
```

### Customising behaviour

| Constant | Default | Meaning |
|----------|---------|---------|
| `PRESENCE_COLOR` | `(255, 0, 0)` | LED colour when someone is present |
| `PRESENCE_BRIGHTNESS` | `200` | Brightness (0-255) when present |
| `PRESENCE_ANIMATION` | `"rainbow"` | Animation played when present |
| `NO_PRESENCE_COLOR` | `(255, 255, 0)` | LED colour when no one is detected |
| `NO_PRESENCE_BRIGHTNESS` | `50` | Brightness when no one is detected |
| `NO_PRESENCE_ANIMATION` | `"breathe"` | Animation played when no one is detected |

Set `NO_PRESENCE_COLOR = (0, 0, 0)` to turn the ring **off** when the room is empty.
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
| Animation speeds | `src/config.rs` → `ANIM_RAINBOW_SPEED`, `ANIM_BREATHING_SPEED`, … |
| Custom animation frames | Place `animation.png` in project root (width=`LED_COUNT`, height=frames) |
| ESP-IDF version | `.cargo/config.toml` → `ESP_IDF_VERSION` |
| mmWave UART pins | `src/main.rs` → `peripherals.pins.gpio4 / gpio5` |
| mmWave LED colours | `src/config.rs` → `PRESENCE_COLOR`, `NO_PRESENCE_COLOR`, … |
| mmWave LED animations | `src/config.rs` → `PRESENCE_ANIMATION`, `NO_PRESENCE_ANIMATION` |

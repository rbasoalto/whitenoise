# whitenoise

White-noise firmware for the RP2040, written in Rust with Embassy. Audio is
sent over PIO I2S to a MAX98357A amplifier.

## Supported boards

- Raspberry Pi Pico (RP2040, 2 MB flash)
- Waveshare RP2040-Zero (RP2040, 2 MB flash)

Both boards use the same firmware image and GPIO assignments. The Pico's GP25
LED pulses once per second. The RP2040-Zero has no GP25 LED; its GP16 WS2812 is
not used.

## Wiring

### MAX98357A

| RP2040 GPIO | MAX98357A |
| --- | --- |
| GP0 | BCLK |
| GP1 | LRC / WS |
| GP2 | DIN |
| VBUS / 5V | VIN |
| GND | GND |

The MAX98357A does not need MCLK. Connect a 4–8 ohm speaker across the amplifier
outputs. Neither speaker output is ground.

### Buttons

| Control | GPIO | Pico physical pin | Other terminal |
| --- | --- | --- | --- |
| `+` / next | GP4 | 6 | GND |
| `-` / previous | GP5 | 7 | GND |

The inputs are active-low and use internal pull-ups. No external resistors are
required. Pico physical pin 8 is a nearby ground.

- Short press: select the next or previous color; white, pink, and brown wrap.
- Hold for 600 ms: change volume by 2 percentage points every 100 ms.
- Debounce time: 30 ms.

## Toolchain

Install the pinned tools and Rust target:

```sh
mise install
mise exec -- rustup target add thumbv6m-none-eabi
```

Build and test:

```sh
mise exec -- cargo test
mise exec -- cargo firmware
mise run uf2
```

Artifacts:

| Command | Output |
| --- | --- |
| `mise exec -- cargo firmware` | `target/thumbv6m-none-eabi/release/whitenoise` |
| `mise run uf2` | `target/thumbv6m-none-eabi/release/whitenoise.uf2` |

## Flashing

### UF2 mass storage

Build the UF2 with `mise run uf2`, then enter USB boot mode:

- Pico: hold BOOTSEL while connecting USB.
- RP2040-Zero: hold BOOT, press and release RESET, then release BOOT.

The board mounts as `RPI-RP2`. Copy
`target/thumbv6m-none-eabi/release/whitenoise.uf2` to that volume. The board
reboots after the copy completes.

On macOS:

```sh
cp target/thumbv6m-none-eabi/release/whitenoise.uf2 /Volumes/RPI-RP2/
```

### picotool

picotool is optional and installed separately. Enter USB boot mode as described
above, then run:

```sh
picotool load -v -x target/thumbv6m-none-eabi/release/whitenoise.uf2
```

`-v` verifies the write. `-x` starts the image after loading. If loading without
`-x`, run `picotool reboot` afterward. This firmware does not require picotool;
copying the UF2 to `RPI-RP2` is equivalent.

### SWD with probe-rs

The Raspberry Pi Pico exposes SWD. Power the target separately over USB or VSYS;
the debug connector does not power it.

| Debug Probe lead | Pico SWD pad |
| --- | --- |
| SC / orange | SWCLK |
| GND / black | GND |
| SD / yellow | SWDIO |

```sh
mise exec -- cargo flash
mise exec -- cargo flash-debug
```

`cargo flash` builds the optimized image, writes and verifies it, then streams
defmt logs over RTT. `cargo flash-debug` uses the unoptimized debug profile.

The RP2040-Zero does not expose SWD pins. Flash it over USB with UF2 or picotool.

## USB control

The firmware exposes a USB CDC serial port. Commands are UTF-8 text terminated
by a newline.

| Command | Values |
| --- | --- |
| `help` | Show command list |
| `get` | Show current parameters |
| `color VALUE` | `white`, `pink`, `brown`, or `0.0..2.0` |
| `hpf VALUE` | `off` or `0..21600` Hz |
| `lpf VALUE` | `off` or `0..21600` Hz |
| `volume VALUE` | `0..100` percent; `vol` is an alias |

Color is continuous: `0` is white, `1` is pink, and `2` is brown.

Defaults:

```text
color=1.000 hpf=80.0Hz lpf=14000.0Hz volume=20.0%
```

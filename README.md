# whitenoise

A Raspberry Pi Pico white-noise machine in Rust, using Embassy and a MAX98357A
I2S amplifier.

The firmware is being built in layers: a host-tested DSP core, a USB CDC command
interface, and a DMA-fed PIO I2S output.

## Toolchain

The repository pins Rust with `mise`. Install the embedded target once:

```sh
mise exec -- rustup target add thumbv6m-none-eabi
```

Useful commands:

```sh
mise exec -- cargo test
mise exec -- cargo firmware
mise run uf2
mise exec -- cargo flash
mise exec -- cargo flash-debug
```

`cargo firmware` produces `target/thumbv6m-none-eabi/release/whitenoise`.
`mise run uf2` converts it to
`target/thumbv6m-none-eabi/release/whitenoise.uf2`, ready to drag onto the
`RPI-RP2` volume. `cargo flash` uses the connected CMSIS-DAP debug probe via
probe-rs and streams defmt logs over RTT. Neither path requires `picotool`.

The Pico's onboard GP25 LED pulses once per second as an independent firmware
heartbeat; it does not depend on USB or the I2S amplifier being connected.

### Debug probe

The Pico SWD connector carries no power. Power the target Pico separately over
USB or VSYS, then connect the Debug Probe's `D` port as follows:

| Debug Probe lead | Pico SWD pad |
| --- | --- |
| SC / orange | SWCLK |
| GND / black | GND |
| SD / yellow | SWDIO |

`cargo flash` runs the optimized image with RTT logs. `cargo flash-debug` uses
the unoptimized development profile for source-level debugging and useful fault
backtraces.

## Wiring

| Raspberry Pi Pico | MAX98357A |
| --- | --- |
| GP0 | BCLK |
| GP1 | LRC / WS |
| GP2 | DIN |
| VBUS (5 V) | VIN |
| GND | GND |

The MAX98357A does not need MCLK. Connect a 4–8 Ω speaker to the amplifier's
speaker outputs; neither speaker terminal is ground.

## USB controls

The board enumerates as a CDC serial device. Commands are newline terminated:

```text
get
color white
color 1.4
hpf 80
lpf 14000
volume 20
```

Color is continuous: `0` is white, `1` is pink, and `2` is brown. Use `off` for
either filter. Volume is a percentage from 0 to 100.

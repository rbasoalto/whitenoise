# whitenoise

An RP2040-Zero white-noise machine in Rust, using Embassy and a MAX98357A I2S
amplifier.

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
mise exec -- cargo flash
```

`cargo firmware` produces `target/thumbv6m-none-eabi/release/whitenoise`.
`cargo flash` passes that ELF directly to `picotool load -v -x`; put the board
in BOOTSEL mode first.

## Wiring

| RP2040-Zero | MAX98357A |
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

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
```

`cargo firmware` produces
`target/thumbv6m-none-eabi/release/whitenoise`. UF2 packaging and `picotool`
flashing are added alongside the board integration.


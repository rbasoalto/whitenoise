//! Allocation-free DSP blocks used by both firmware and host tests.

use core::f32::consts::TAU;

/// Runtime controls for the audio chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    /// Spectral color: `0.0` is white, `1.0` pink, and `2.0` brown.
    pub color: f32,
    /// First-order high-pass cutoff in hertz. Zero bypasses the block.
    pub high_pass_hz: f32,
    /// First-order low-pass cutoff in hertz. Zero bypasses the block.
    pub low_pass_hz: f32,
    /// Linear output gain from silence (`0.0`) to full scale (`1.0`).
    pub volume: f32,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            color: 1.0,
            high_pass_hz: 80.0,
            low_pass_hz: 14_000.0,
            volume: 0.2,
        }
    }
}

impl Parameters {
    /// Clamp controls to values the processing chain can represent.
    pub fn sanitized(mut self, sample_rate: u32) -> Self {
        let max_cutoff = sample_rate as f32 * 0.45;
        self.color = self.color.clamp(0.0, 2.0);
        self.high_pass_hz = self.high_pass_hz.clamp(0.0, max_cutoff);
        self.low_pass_hz = self.low_pass_hz.clamp(0.0, max_cutoff);
        self.volume = self.volume.clamp(0.0, 1.0);
        self
    }
}

/// Deterministic xorshift32 source, suitable for audio noise (not cryptography).
#[derive(Clone, Copy, Debug)]
struct WhiteNoise {
    state: u32,
}

impl WhiteNoise {
    fn new(seed: u32) -> Self {
        // Xorshift's all-zero state is absorbing.
        Self { state: seed.max(1) }
    }

    fn next(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;

        // The upper 24 bits fit exactly in an f32 mantissa.
        (x >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
    }
}

/// Produces white, pink and brown streams in parallel and interpolates them.
#[derive(Clone, Copy, Debug)]
struct ColoredNoise {
    white: WhiteNoise,
    pink: [f32; 7],
    brown: f32,
}

impl ColoredNoise {
    fn new(seed: u32) -> Self {
        Self {
            white: WhiteNoise::new(seed),
            pink: [0.0; 7],
            brown: 0.0,
        }
    }

    fn next(&mut self, color: f32) -> f32 {
        let white = self.white.next();

        // Paul Kellet's economical pink-noise approximation. The scale brings
        // its RMS level close enough to white for smooth color interpolation.
        self.pink[0] = 0.99886 * self.pink[0] + white * 0.055_517_9;
        self.pink[1] = 0.99332 * self.pink[1] + white * 0.075_075_9;
        self.pink[2] = 0.96900 * self.pink[2] + white * 0.153_852_0;
        self.pink[3] = 0.86650 * self.pink[3] + white * 0.310_485_6;
        self.pink[4] = 0.55000 * self.pink[4] + white * 0.532_952_2;
        self.pink[5] = -0.7616 * self.pink[5] - white * 0.016_898_0;
        let pink = (self.pink[0]
            + self.pink[1]
            + self.pink[2]
            + self.pink[3]
            + self.pink[4]
            + self.pink[5]
            + self.pink[6]
            + white * 0.5362)
            * 0.11;
        self.pink[6] = white * 0.115_926;

        // A leaky random walk avoids unbounded DC drift.
        self.brown = (self.brown + white * 0.02) / 1.02;
        let brown = self.brown * 3.5;

        if color <= 1.0 {
            white + (pink - white) * color
        } else {
            pink + (brown - pink) * (color - 1.0)
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct HighPass {
    alpha: f32,
    previous_input: f32,
    previous_output: f32,
    bypassed: bool,
}

impl HighPass {
    fn new() -> Self {
        Self {
            alpha: 0.0,
            previous_input: 0.0,
            previous_output: 0.0,
            bypassed: true,
        }
    }

    fn configure(&mut self, cutoff_hz: f32, sample_rate: u32) {
        self.bypassed = cutoff_hz <= 0.0;
        self.alpha = 1.0 / (1.0 + TAU * cutoff_hz / sample_rate as f32);
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = if self.bypassed {
            input
        } else {
            self.alpha * (self.previous_output + input - self.previous_input)
        };
        self.previous_input = input;
        self.previous_output = output;
        output
    }
}

#[derive(Clone, Copy, Debug)]
struct LowPass {
    alpha: f32,
    output: f32,
    bypassed: bool,
}

impl LowPass {
    fn new() -> Self {
        Self {
            alpha: 1.0,
            output: 0.0,
            bypassed: true,
        }
    }

    fn configure(&mut self, cutoff_hz: f32, sample_rate: u32) {
        self.bypassed = cutoff_hz <= 0.0;
        self.alpha = if self.bypassed {
            1.0
        } else {
            1.0 / (1.0 + sample_rate as f32 / (TAU * cutoff_hz))
        };
    }

    fn process(&mut self, input: f32) -> f32 {
        self.output = if self.bypassed {
            input
        } else {
            self.output + self.alpha * (input - self.output)
        };
        self.output
    }
}

/// Noise source → HPF → LPF → smoothed gain → saturating PCM output.
#[derive(Clone, Copy, Debug)]
pub struct DspChain {
    sample_rate: u32,
    parameters: Parameters,
    noise: ColoredNoise,
    high_pass: HighPass,
    low_pass: LowPass,
    gain: f32,
    gain_smoothing: f32,
}

impl DspChain {
    pub fn new(sample_rate: u32, seed: u32, parameters: Parameters) -> Self {
        let parameters = parameters.sanitized(sample_rate);
        let mut chain = Self {
            sample_rate,
            parameters,
            noise: ColoredNoise::new(seed),
            high_pass: HighPass::new(),
            low_pass: LowPass::new(),
            // Starting at the requested gain avoids a fade from silence at boot.
            gain: parameters.volume,
            // Reaches 99% of a gain change in roughly 10 ms at 48 kHz.
            gain_smoothing: (460.0 / sample_rate as f32).clamp(0.0, 1.0),
        };
        chain.configure_filters();
        chain
    }

    pub fn parameters(&self) -> Parameters {
        self.parameters
    }

    pub fn set_parameters(&mut self, parameters: Parameters) {
        self.parameters = parameters.sanitized(self.sample_rate);
        self.configure_filters();
    }

    /// Render one mono sample. Send the same value to both I2S slots.
    pub fn next_i16(&mut self) -> i16 {
        let sample = self.noise.next(self.parameters.color);
        let sample = self.high_pass.process(sample);
        let sample = self.low_pass.process(sample);

        self.gain += self.gain_smoothing * (self.parameters.volume - self.gain);
        float_to_i16(sample * self.gain)
    }

    pub fn fill_i16(&mut self, output: &mut [i16]) {
        for sample in output {
            *sample = self.next_i16();
        }
    }

    fn configure_filters(&mut self) {
        self.high_pass
            .configure(self.parameters.high_pass_hz, self.sample_rate);
        self.low_pass
            .configure(self.parameters.low_pass_hz, self.sample_rate);
    }
}

fn float_to_i16(sample: f32) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * i16::MAX as f32;
    scaled as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_are_sanitized() {
        let parameters = Parameters {
            color: 4.0,
            high_pass_hz: -10.0,
            low_pass_hz: 40_000.0,
            volume: 2.0,
        }
        .sanitized(48_000);

        assert_eq!(parameters.color, 2.0);
        assert_eq!(parameters.high_pass_hz, 0.0);
        assert_eq!(parameters.low_pass_hz, 21_600.0);
        assert_eq!(parameters.volume, 1.0);
    }

    #[test]
    fn output_is_deterministic_and_not_silent() {
        let parameters = Parameters {
            color: 0.0,
            high_pass_hz: 0.0,
            low_pass_hz: 0.0,
            volume: 1.0,
        };
        let mut first = DspChain::new(48_000, 0x1234_5678, parameters);
        let mut second = DspChain::new(48_000, 0x1234_5678, parameters);
        let mut energy = 0_i64;

        for _ in 0..256 {
            let a = first.next_i16();
            let b = second.next_i16();
            assert_eq!(a, b);
            energy += i64::from(a).abs();
        }

        assert!(energy > 1_000_000);
    }

    #[test]
    fn colors_become_progressively_smoother() {
        fn roughness(color: f32) -> f64 {
            let mut noise = ColoredNoise::new(0x1234_5678);
            let mut previous = 0.0_f32;
            let mut sum = 0.0_f64;

            // Let the recursive filters settle before measuring sample-to-sample
            // energy, a useful proxy for high-frequency spectral content.
            for _ in 0..2_048 {
                previous = noise.next(color);
            }
            for _ in 0..16_384 {
                let current = noise.next(color);
                let difference = current - previous;
                sum += f64::from(difference * difference);
                previous = current;
            }
            sum
        }

        let white = roughness(0.0);
        let pink = roughness(1.0);
        let brown = roughness(2.0);

        assert!(white > pink, "white={white}, pink={pink}");
        assert!(pink > brown, "pink={pink}, brown={brown}");
    }

    #[test]
    fn zero_volume_reaches_silence_without_clipping() {
        let mut chain = DspChain::new(
            48_000,
            7,
            Parameters {
                color: 2.0,
                high_pass_hz: 0.0,
                low_pass_hz: 0.0,
                volume: 1.0,
            },
        );
        chain.set_parameters(Parameters {
            volume: 0.0,
            ..chain.parameters()
        });

        for _ in 0..2_000 {
            let _ = chain.next_i16();
        }

        assert!(chain.gain < 0.000_001);
    }

    #[test]
    fn high_pass_rejects_dc() {
        let mut filter = HighPass::new();
        filter.configure(100.0, 48_000);

        let mut output = 0.0;
        for _ in 0..10_000 {
            output = filter.process(0.75);
        }

        assert!(output.abs() < 0.000_1);
    }

    #[test]
    fn low_pass_smooths_a_step() {
        let mut filter = LowPass::new();
        filter.configure(1_000.0, 48_000);

        let first = filter.process(1.0);
        let mut settled = first;
        for _ in 0..1_000 {
            settled = filter.process(1.0);
        }

        assert!(first > 0.0 && first < 0.2);
        assert!((settled - 1.0).abs() < 0.000_001);
    }
}

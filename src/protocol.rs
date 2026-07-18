//! Small, allocation-free line protocol for the USB CDC control port.

use core::fmt::{self, Write};
use core::str;

use crate::dsp::Parameters;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Help,
    Get,
    Color(f32),
    HighPass(f32),
    LowPass(f32),
    /// Linear gain, normalized to `0.0..=1.0`.
    Volume(f32),
    Power(PowerCommand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerCommand {
    On,
    Off,
    Toggle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    NotUtf8,
    Empty,
    UnknownCommand,
    MissingValue,
    ExtraArgument,
    InvalidValue,
    OutOfRange,
}

impl ParseError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::NotUtf8 => "command must be UTF-8",
            Self::Empty => "empty command",
            Self::UnknownCommand => "unknown command",
            Self::MissingValue => "missing value",
            Self::ExtraArgument => "too many arguments",
            Self::InvalidValue => "invalid value",
            Self::OutOfRange => "value out of range",
        }
    }
}

/// Parse one newline-delimited command. A trailing CR/LF is optional.
pub fn parse_line(input: &[u8]) -> Result<Command, ParseError> {
    let line = str::from_utf8(input)
        .map_err(|_| ParseError::NotUtf8)?
        .trim();
    let mut words = line.split_ascii_whitespace();
    let name = words.next().ok_or(ParseError::Empty)?;

    if name.eq_ignore_ascii_case("help") {
        require_end(&mut words)?;
        return Ok(Command::Help);
    }
    if name.eq_ignore_ascii_case("get") {
        require_end(&mut words)?;
        return Ok(Command::Get);
    }

    let value = words.next().ok_or(ParseError::MissingValue)?;
    require_end(&mut words)?;

    if name.eq_ignore_ascii_case("color") {
        return parse_color(value).map(Command::Color);
    }
    if name.eq_ignore_ascii_case("hpf") {
        return parse_cutoff(value).map(Command::HighPass);
    }
    if name.eq_ignore_ascii_case("lpf") {
        return parse_cutoff(value).map(Command::LowPass);
    }
    if name.eq_ignore_ascii_case("volume") || name.eq_ignore_ascii_case("vol") {
        let percent = parse_number(value)?;
        if !(0.0..=100.0).contains(&percent) {
            return Err(ParseError::OutOfRange);
        }
        return Ok(Command::Volume(percent / 100.0));
    }
    if name.eq_ignore_ascii_case("power") {
        let power = if value.eq_ignore_ascii_case("on") {
            PowerCommand::On
        } else if value.eq_ignore_ascii_case("off") {
            PowerCommand::Off
        } else if value.eq_ignore_ascii_case("toggle") {
            PowerCommand::Toggle
        } else {
            return Err(ParseError::InvalidValue);
        };
        return Ok(Command::Power(power));
    }

    Err(ParseError::UnknownCommand)
}

impl Command {
    /// Apply a mutating command. Returns `true` when parameters changed.
    pub fn apply(self, parameters: &mut Parameters) -> bool {
        match self {
            Self::Color(value) => parameters.color = value,
            Self::HighPass(value) => parameters.high_pass_hz = value,
            Self::LowPass(value) => parameters.low_pass_hz = value,
            Self::Volume(value) => parameters.volume = value,
            Self::Power(PowerCommand::On) => parameters.enabled = true,
            Self::Power(PowerCommand::Off) => parameters.enabled = false,
            Self::Power(PowerCommand::Toggle) => parameters.enabled = !parameters.enabled,
            Self::Help | Self::Get => return false,
        }
        true
    }
}

/// Format the current controls without allocation.
pub fn write_parameters(output: &mut impl Write, parameters: Parameters) -> fmt::Result {
    writeln!(
        output,
        "power={} color={:.3} hpf={:.1}Hz lpf={:.1}Hz volume={:.1}%",
        if parameters.enabled { "on" } else { "off" },
        parameters.color,
        parameters.high_pass_hz,
        parameters.low_pass_hz,
        parameters.volume * 100.0
    )
}

pub const HELP: &str = "commands:\n\
  get\n\
  color white|pink|brown|0.0..2.0\n\
  hpf off|0..21600\n\
  lpf off|0..21600\n\
  volume 0..100\n\
  power on|off|toggle\n";

fn parse_color(value: &str) -> Result<f32, ParseError> {
    let color = if value.eq_ignore_ascii_case("white") {
        0.0
    } else if value.eq_ignore_ascii_case("pink") {
        1.0
    } else if value.eq_ignore_ascii_case("brown") {
        2.0
    } else {
        parse_number(value)?
    };

    if (0.0..=2.0).contains(&color) {
        Ok(color)
    } else {
        Err(ParseError::OutOfRange)
    }
}

fn parse_cutoff(value: &str) -> Result<f32, ParseError> {
    if value.eq_ignore_ascii_case("off") {
        return Ok(0.0);
    }

    let cutoff = parse_number(value)?;
    if (0.0..=21_600.0).contains(&cutoff) {
        Ok(cutoff)
    } else {
        Err(ParseError::OutOfRange)
    }
}

fn parse_number(value: &str) -> Result<f32, ParseError> {
    let number = value.parse::<f32>().map_err(|_| ParseError::InvalidValue)?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(ParseError::InvalidValue)
    }
}

fn require_end(words: &mut str::SplitAsciiWhitespace<'_>) -> Result<(), ParseError> {
    if words.next().is_some() {
        Err(ParseError::ExtraArgument)
    } else {
        Ok(())
    }
}

/// Fixed-capacity UTF-8 response buffer for USB packets.
pub struct ResponseBuffer<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> ResponseBuffer<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl<const N: usize> Default for ResponseBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Write for ResponseBuffer<N> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.len.checked_add(value.len()).ok_or(fmt::Error)?;
        if end > N {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_and_continuous_colors() {
        assert_eq!(parse_line(b"color white\r\n"), Ok(Command::Color(0.0)));
        assert_eq!(parse_line(b"color pink"), Ok(Command::Color(1.0)));
        assert_eq!(parse_line(b"color brown"), Ok(Command::Color(2.0)));
        assert_eq!(parse_line(b"color 1.25"), Ok(Command::Color(1.25)));
    }

    #[test]
    fn parses_filters_and_volume() {
        assert_eq!(parse_line(b"hpf off"), Ok(Command::HighPass(0.0)));
        assert_eq!(parse_line(b"lpf 12000"), Ok(Command::LowPass(12_000.0)));
        assert_eq!(parse_line(b"vol 37.5"), Ok(Command::Volume(0.375)));
        assert_eq!(
            parse_line(b"power off"),
            Ok(Command::Power(PowerCommand::Off))
        );
        assert_eq!(
            parse_line(b"power toggle"),
            Ok(Command::Power(PowerCommand::Toggle))
        );
    }

    #[test]
    fn rejects_bad_shapes_and_ranges() {
        assert_eq!(parse_line(b""), Err(ParseError::Empty));
        assert_eq!(parse_line(b"volume"), Err(ParseError::MissingValue));
        assert_eq!(parse_line(b"volume 101"), Err(ParseError::OutOfRange));
        assert_eq!(parse_line(b"color blue"), Err(ParseError::InvalidValue));
        assert_eq!(parse_line(b"power maybe"), Err(ParseError::InvalidValue));
        assert_eq!(parse_line(b"get now"), Err(ParseError::ExtraArgument));
    }

    #[test]
    fn applies_changes() {
        let mut parameters = Parameters::default();
        assert!(Command::Color(0.5).apply(&mut parameters));
        assert!(Command::Volume(0.75).apply(&mut parameters));
        assert!(Command::Power(PowerCommand::Off).apply(&mut parameters));
        assert_eq!(parameters.color, 0.5);
        assert_eq!(parameters.volume, 0.75);
        assert!(!parameters.enabled);
        assert!(!Command::Get.apply(&mut parameters));
    }

    #[test]
    fn formats_status_into_fixed_buffer() {
        let mut response = ResponseBuffer::<96>::new();
        write_parameters(&mut response, Parameters::default()).unwrap();
        assert_eq!(
            str::from_utf8(response.as_bytes()).unwrap(),
            "power=on color=1.000 hpf=80.0Hz lpf=14000.0Hz volume=20.0%\n"
        );
    }
}

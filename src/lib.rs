#![cfg_attr(not(test), no_std)]

//! Host-testable signal processing and command handling for the firmware.

pub mod controls;
pub mod dsp;
pub mod protocol;

pub const SAMPLE_RATE: u32 = 48_000;

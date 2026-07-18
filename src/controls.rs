//! Button gesture handling shared by the firmware and host tests.

use crate::dsp::Parameters;

pub const POLL_INTERVAL_MS: u64 = 10;
pub const VOLUME_STEP: f32 = 0.02;

const DEBOUNCE_TICKS: u8 = 3;
const HOLD_TICKS: u16 = 60;
const REPEAT_TICKS: u16 = 10;
const COLORS: [f32; 3] = [0.0, 1.0, 2.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlDelta {
    pub color_steps: i8,
    pub volume_steps: i8,
    pub toggle_power: bool,
}

impl ControlDelta {
    pub const fn is_empty(self) -> bool {
        self.color_steps == 0 && self.volume_steps == 0 && !self.toggle_power
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gesture {
    Short,
    Hold,
}

#[derive(Clone, Copy, Debug)]
struct Button {
    raw_pressed: bool,
    stable_pressed: bool,
    debounce_ticks: u8,
    held_ticks: u16,
    hold_started: bool,
}

impl Button {
    const fn new() -> Self {
        Self {
            raw_pressed: false,
            stable_pressed: false,
            debounce_ticks: 0,
            held_ticks: 0,
            hold_started: false,
        }
    }

    fn update(&mut self, pressed: bool) -> Option<Gesture> {
        if pressed != self.raw_pressed {
            self.raw_pressed = pressed;
            self.debounce_ticks = 0;
        } else if self.stable_pressed != self.raw_pressed {
            self.debounce_ticks = self.debounce_ticks.saturating_add(1);
            if self.debounce_ticks >= DEBOUNCE_TICKS {
                self.stable_pressed = self.raw_pressed;
                self.debounce_ticks = 0;

                if self.stable_pressed {
                    self.held_ticks = 0;
                    self.hold_started = false;
                } else if !self.hold_started {
                    return Some(Gesture::Short);
                }
            }
        }

        // Do not advance the hold timer while a release is being debounced.
        if !self.stable_pressed || !self.raw_pressed {
            return None;
        }

        self.held_ticks = self.held_ticks.saturating_add(1);
        if self.held_ticks == HOLD_TICKS {
            self.hold_started = true;
            Some(Gesture::Hold)
        } else if self.hold_started && (self.held_ticks - HOLD_TICKS).is_multiple_of(REPEAT_TICKS) {
            Some(Gesture::Hold)
        } else {
            None
        }
    }

    fn cancel(&mut self) {
        *self = Self::new();
    }
}

/// Converts two active-low button levels into semantic parameter changes.
pub struct ButtonControls {
    next: Button,
    previous: Button,
    chord: Button,
    chord_triggered: bool,
}

impl ButtonControls {
    pub const fn new() -> Self {
        Self {
            next: Button::new(),
            previous: Button::new(),
            chord: Button::new(),
            chord_triggered: false,
        }
    }

    pub fn update(&mut self, next_pressed: bool, previous_pressed: bool) -> ControlDelta {
        let mut delta = ControlDelta::default();
        let both_pressed = next_pressed && previous_pressed;
        let chord_gesture = self.chord.update(both_pressed);

        if !self.chord.stable_pressed {
            self.chord_triggered = false;
        }

        // A chord owns both buttons from the first raw contact through its
        // debounced release, preventing color or volume side effects.
        if both_pressed || self.chord.stable_pressed {
            self.next.cancel();
            self.previous.cancel();
            if chord_gesture == Some(Gesture::Hold) && !self.chord_triggered {
                self.chord_triggered = true;
                delta.toggle_power = true;
            }
            return delta;
        }

        apply_gesture(self.next.update(next_pressed), 1, &mut delta);
        apply_gesture(self.previous.update(previous_pressed), -1, &mut delta);
        delta
    }
}

impl Default for ButtonControls {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_gesture(gesture: Option<Gesture>, direction: i8, delta: &mut ControlDelta) {
    match gesture {
        Some(Gesture::Short) => delta.color_steps += direction,
        Some(Gesture::Hold) => delta.volume_steps += direction,
        None => {}
    }
}

/// Apply a button-generated change and report whether the parameters changed.
pub fn apply_delta(parameters: &mut Parameters, delta: ControlDelta) -> bool {
    let before = *parameters;

    for _ in 0..delta.color_steps.unsigned_abs() {
        parameters.color = step_color(parameters.color, delta.color_steps.is_positive());
    }
    parameters.volume =
        (parameters.volume + f32::from(delta.volume_steps) * VOLUME_STEP).clamp(0.0, 1.0);
    if delta.toggle_power {
        parameters.enabled = !parameters.enabled;
    }

    *parameters != before
}

fn step_color(color: f32, forwards: bool) -> f32 {
    if forwards {
        COLORS
            .iter()
            .copied()
            .find(|candidate| *candidate > color)
            .unwrap_or(COLORS[0])
    } else {
        COLORS
            .iter()
            .rev()
            .copied()
            .find(|candidate| *candidate < color)
            .unwrap_or(COLORS[COLORS.len() - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(
        controls: &mut ButtonControls,
        next_pressed: bool,
        previous_pressed: bool,
        ticks: usize,
    ) -> ControlDelta {
        let mut total = ControlDelta::default();
        for _ in 0..ticks {
            let delta = controls.update(next_pressed, previous_pressed);
            total.color_steps += delta.color_steps;
            total.volume_steps += delta.volume_steps;
            total.toggle_power ^= delta.toggle_power;
        }
        total
    }

    #[test]
    fn short_presses_step_colors_on_release() {
        let mut controls = ButtonControls::new();

        assert!(poll(&mut controls, true, false, 10).is_empty());
        assert_eq!(poll(&mut controls, false, false, 4).color_steps, 1);
        assert!(poll(&mut controls, false, true, 10).is_empty());
        assert_eq!(poll(&mut controls, false, false, 4).color_steps, -1);
    }

    #[test]
    fn contact_bounce_does_not_create_a_press() {
        let mut controls = ButtonControls::new();

        assert!(controls.update(true, false).is_empty());
        assert!(controls.update(false, false).is_empty());
        assert!(controls.update(true, false).is_empty());
        assert!(controls.update(false, false).is_empty());
        assert!(poll(&mut controls, false, false, 10).is_empty());
    }

    #[test]
    fn hold_repeats_volume_and_suppresses_short_press() {
        let mut controls = ButtonControls::new();
        let held = poll(&mut controls, true, false, 80);

        assert!(held.volume_steps >= 2);
        assert_eq!(held.color_steps, 0);
        let released = poll(&mut controls, false, false, 4);
        assert!(released.is_empty());
    }

    #[test]
    fn holding_both_buttons_toggles_power_once_and_suppresses_other_actions() {
        let mut controls = ButtonControls::new();
        let held = poll(&mut controls, true, true, 100);

        assert!(held.toggle_power);
        assert_eq!(held.color_steps, 0);
        assert_eq!(held.volume_steps, 0);
        assert!(poll(&mut controls, true, true, 100).is_empty());
        assert!(poll(&mut controls, false, false, 4).is_empty());

        assert!(poll(&mut controls, true, true, 100).toggle_power);
    }

    #[test]
    fn colors_wrap_and_continuous_usb_values_join_the_palette() {
        let mut parameters = Parameters::default();

        apply_delta(
            &mut parameters,
            ControlDelta {
                color_steps: 1,
                volume_steps: 0,
                toggle_power: false,
            },
        );
        assert_eq!(parameters.color, 2.0);
        apply_delta(
            &mut parameters,
            ControlDelta {
                color_steps: 1,
                volume_steps: 0,
                toggle_power: false,
            },
        );
        assert_eq!(parameters.color, 0.0);

        parameters.color = 1.4;
        apply_delta(
            &mut parameters,
            ControlDelta {
                color_steps: -1,
                volume_steps: 0,
                toggle_power: false,
            },
        );
        assert_eq!(parameters.color, 1.0);
    }

    #[test]
    fn volume_steps_are_clamped() {
        let mut parameters = Parameters {
            volume: 0.99,
            ..Parameters::default()
        };

        assert!(apply_delta(
            &mut parameters,
            ControlDelta {
                color_steps: 0,
                volume_steps: 1,
                toggle_power: false,
            }
        ));
        assert_eq!(parameters.volume, 1.0);
        assert!(!apply_delta(
            &mut parameters,
            ControlDelta {
                color_steps: 0,
                volume_steps: 1,
                toggle_power: false,
            }
        ));
    }

    #[test]
    fn power_toggle_preserves_volume() {
        let mut parameters = Parameters::default();
        let volume = parameters.volume;

        assert!(apply_delta(
            &mut parameters,
            ControlDelta {
                toggle_power: true,
                ..ControlDelta::default()
            }
        ));
        assert!(!parameters.enabled);
        assert_eq!(parameters.volume, volume);
    }
}

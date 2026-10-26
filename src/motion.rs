//! Animation layer built on [animato](https://github.com/AarambhDevHub/animato).
//!
//! Animato is renderer agnostic: it computes values, Slint draws them. The UI
//! tick pulls a [`Frame`] every ~16 ms and writes it into the Slint properties.
//! Doing it here (instead of relying only on declarative animations) gives us
//! interpolated colour for the ambilight and an overshooting reveal when the
//! track changes.

use animato::{Color, Easing, Loop, Tween, Update};

use crate::artwork::Rgb;

/// Spotify green, used until the first album art provides a palette.
const BRAND: Rgb = Rgb::new(30, 215, 96);

pub struct Motion {
    accent: Tween<Color>,
    accent_target: Rgb,
    reveal: Tween<f32>,
    pulse: Tween<f32>,
    /// The pulse is only useful while the queue is on screen. Pausing the tween
    /// (instead of just not writing it) means it resumes where it stopped
    /// instead of jumping when the queue is opened again.
    pulse_running: bool,
    elapsed: f32,
}

/// Everything the UI needs from the animation layer for one frame.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub accent: Rgb,
    /// 0..1, may overshoot slightly while easing out.
    pub reveal: f32,
    /// 0..1 ping-pong pulse for the current queue row.
    pub pulse: f32,
    /// Degrees, used by the loading spinner.
    pub spin: f32,
}

impl Motion {
    pub fn new() -> Self {
        let accent = Tween::new(to_linear(BRAND), to_linear(BRAND))
            .duration(0.0)
            .build();
        let mut reveal = Tween::new(0.0_f32, 1.0)
            .duration(0.85)
            .easing(Easing::EaseOutBack)
            .build();
        reveal.update(2.0);

        let pulse = Tween::new(0.0_f32, 1.0)
            .duration(2.4)
            .easing(Easing::EaseInOutSine)
            .looping(Loop::PingPong)
            .build();

        Self {
            accent,
            accent_target: BRAND,
            reveal,
            pulse,
            pulse_running: true,
            elapsed: 0.0,
        }
    }

    /// Start or stop the queue's pulse. Stopping freezes the phase, so the row
    /// does not jump when the queue is shown again.
    pub fn set_pulse_running(&mut self, running: bool) {
        self.pulse_running = running;
    }

    /// Retarget the ambilight colour (called when new artwork arrives).
    pub fn set_accent(&mut self, rgb: Rgb) {
        if rgb == self.accent_target {
            return;
        }
        self.accent_target = rgb;
        self.accent = Tween::new(self.accent.value(), to_linear(rgb))
            .duration(1.1)
            .easing(Easing::EaseInOutSine)
            .build();
    }

    /// The accent the UI is showing right now, without advancing the clock.
    pub fn accent(&self) -> Rgb {
        from_linear(self.accent.value())
    }

    /// Restart the artwork entrance animation.
    pub fn reveal(&mut self) {
        self.enter(0.0, 0.9);
    }

    /// Softer entrance for artwork that replaces artwork, so skipping tracks
    /// does not blank the circle while the new cover is on its way.
    pub fn swap(&mut self) {
        self.enter(0.72, 0.35);
    }

    fn enter(&mut self, from: f32, seconds: f32) {
        self.reveal = Tween::new(from, 1.0_f32)
            .duration(seconds)
            .easing(Easing::EaseOutBack)
            .build();
    }

    pub fn update(&mut self, dt: f32) -> Frame {
        let dt = dt.clamp(0.0, 0.2);
        self.elapsed += dt;

        self.accent.update(dt);
        self.reveal.update(dt);
        if self.pulse_running {
            self.pulse.update(dt);
        }

        Frame {
            accent: from_linear(self.accent.value()),
            reveal: self.reveal.value(),
            pulse: self.pulse.value().clamp(0.0, 1.0),
            spin: (self.elapsed * 260.0) % 360.0,
        }
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::new()
    }
}

fn to_linear(rgb: Rgb) -> Color {
    Color::rgba(
        channel_to_linear(rgb.r),
        channel_to_linear(rgb.g),
        channel_to_linear(rgb.b),
        1.0,
    )
}

fn from_linear(color: Color) -> Rgb {
    Rgb::new(
        channel_from_linear(color.r()),
        channel_from_linear(color.g()),
        channel_from_linear(color.b()),
    )
}

fn channel_to_linear(value: u8) -> f32 {
    let c = value as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn channel_from_linear(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_conversion_round_trips() {
        for value in [0u8, 1, 30, 128, 215, 254, 255] {
            assert_eq!(
                channel_from_linear(channel_to_linear(value)),
                value,
                "channel {value}"
            );
        }
    }

    #[test]
    fn accent_tween_reaches_the_target() {
        let mut motion = Motion::new();
        motion.set_accent(Rgb::new(220, 40, 90));
        let mut frame = motion.update(0.016);
        for _ in 0..200 {
            frame = motion.update(0.016);
        }
        let (hue, _, _) = frame.accent.to_hsl();
        let (target_hue, _, _) = Rgb::new(220, 40, 90).vivid().to_hsl();
        assert!((hue - target_hue).abs() < 2.0, "{hue} vs {target_hue}");
    }

    #[test]
    fn reveal_overshoots_then_settles() {
        let mut motion = Motion::new();
        motion.reveal();
        let mut frame = motion.update(0.016);
        for _ in 0..120 {
            frame = motion.update(0.016);
        }
        assert!((frame.reveal - 1.0).abs() < 0.01, "{}", frame.reveal);
    }

    #[test]
    fn pulse_stays_in_range() {
        let mut motion = Motion::new();
        let mut frame = motion.update(0.016);
        for _ in 0..300 {
            frame = motion.update(0.04);
        }
        assert!((0.0..=1.0).contains(&frame.pulse));
    }

    #[test]
    fn pulse_freezes_while_the_queue_is_hidden() {
        let mut motion = Motion::new();
        motion.set_pulse_running(false);

        let frozen = motion.update(0.016).pulse;
        let later = motion.update(0.5).pulse;
        assert_eq!(frozen, later, "a hidden pulse must not advance");

        motion.set_pulse_running(true);
        let resumed = motion.update(0.016).pulse;
        assert!(
            resumed > frozen,
            "the pulse must continue from where it stopped"
        );
    }
}

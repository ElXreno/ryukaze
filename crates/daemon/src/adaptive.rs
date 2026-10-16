//! The adaptive side of the advanced fan mode: ryukazed, not the EC,
//! picks the speed. It smooths each temperature, reads the speed off the
//! curve straight between its dots, and writes that into the EC's table
//! as a flat line with a step to full on top, so fans left on their own
//! (the daemon stopped) still speed up when it gets hot.

use ryukaze_core::fans::{Curve, Fan};
use serde::Deserialize;

/// Where a curve's first dot sits; dot `i` sits at `temps[i - 1]`. The
/// window's editor draws them there too.
pub const FIRST_DOT: f32 = 20.0;

/// The speed `curve` asks for at `temp`, straight between its dots.
pub fn speed_at(curve: &Curve, temp: f32) -> f32 {
    let dots = std::iter::once(FIRST_DOT)
        .chain(curve.temps.iter().map(|&temp| f32::from(temp)))
        .zip(curve.speeds.iter().map(|&speed| f32::from(speed)));
    let mut below = None;
    for (at, speed) in dots {
        if temp <= at {
            return match below {
                None => speed,
                Some((from, low)) => low + (speed - low) * (temp - from) / (at - from),
            };
        }
        below = Some((at, speed));
    }
    f32::from(curve.speeds[6])
}

/// How the temperatures are smoothed before they meet the curves.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Smoothing {
    /// Time constant while it heats up, in seconds.
    pub rise_secs: f32,
    /// Time constant while it cools down; longer, so the fans don't drop
    /// the moment a burst of load ends.
    pub fall_secs: f32,
    /// A reading at or past this (°C) goes through unsmoothed.
    pub panic_c: u8,
}

impl Default for Smoothing {
    fn default() -> Self {
        Self {
            rise_secs: 10.0,
            fall_secs: 60.0,
            panic_c: 90,
        }
    }
}

/// Exponential smoothing, quicker up than down: a spike barely moves it,
/// a climb that lasts comes through.
#[derive(Debug, Default)]
pub struct Smoothed {
    value: Option<f32>,
}

impl Smoothed {
    /// Feeds a reading taken `dt` seconds after the last one; returns what
    /// the curve should see.
    pub fn update(&mut self, reading: f32, dt: f32, smoothing: &Smoothing) -> f32 {
        let value = match self.value {
            None => reading,
            Some(value) => {
                let tau = if reading > value {
                    smoothing.rise_secs
                } else {
                    smoothing.fall_secs
                };
                let alpha = if tau <= 0.0 {
                    1.0
                } else {
                    1.0 - (-dt / tau).exp()
                };
                value + (reading - value) * alpha
            }
        };
        self.value = Some(value);
        if reading >= f32::from(smoothing.panic_c) {
            reading
        } else {
            value
        }
    }
}

/// Where the EC goes to full by itself if the daemon isn't there to.
fn full_from(fan: Fan) -> u8 {
    match fan {
        Fan::Cpu => 90,
        Fan::Gpu => 85,
    }
}

/// The table that holds `fan` at `speed` percent, with the step to full
/// on top.
pub fn hold(fan: Fan, speed: u8) -> Curve {
    let speed = speed.min(100);
    let top = full_from(fan);
    Curve {
        temps: [top - 5, top - 4, top - 3, top - 2, top - 1, top],
        speeds: [speed, speed, speed, speed, speed, speed, 100],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn factory() -> Curve {
        Curve::new(&[50, 57, 64, 71, 78, 85], &[0, 40, 45, 50, 57, 65, 80]).unwrap()
    }

    #[test]
    fn reads_between_the_dots() {
        let curve = factory();
        assert_eq!(speed_at(&curve, 10.0), 0.0);
        assert_eq!(speed_at(&curve, 35.0), 20.0);
        assert_eq!(speed_at(&curve, 50.0), 40.0);
        assert!((speed_at(&curve, 60.5) - 47.5).abs() < 0.01);
        assert_eq!(speed_at(&curve, 85.0), 80.0);
        assert_eq!(speed_at(&curve, 99.0), 80.0);
    }

    #[test]
    fn a_spike_barely_moves_it() {
        let smoothing = Smoothing::default();
        let mut temp = Smoothed::default();
        temp.update(50.0, 0.0, &smoothing);
        assert!(temp.update(80.0, 2.0, &smoothing) < 56.0);
        // Back down: it falls slower than it rose.
        let after = temp.update(50.0, 2.0, &smoothing);
        assert!(after > 50.0 && after < 56.0, "{after}");
    }

    #[test]
    fn a_lasting_climb_comes_through() {
        let smoothing = Smoothing::default();
        let mut temp = Smoothed::default();
        temp.update(50.0, 0.0, &smoothing);
        let mut seen = 0.0;
        for _ in 0..30 {
            seen = temp.update(75.0, 2.0, &smoothing);
        }
        assert!(seen > 74.0, "{seen}");
    }

    #[test]
    fn panic_goes_straight_through() {
        let smoothing = Smoothing::default();
        let mut temp = Smoothed::default();
        temp.update(50.0, 0.0, &smoothing);
        assert_eq!(temp.update(95.0, 2.0, &smoothing), 95.0);
    }

    #[test]
    fn holds_are_valid_curves() {
        for fan in Fan::ALL {
            for speed in [0, 35, 100, 150] {
                let curve = hold(fan, speed);
                assert!(Curve::new(&curve.temps, &curve.speeds).is_ok(), "{curve:?}");
                assert_eq!(curve.speeds[6], 100);
            }
        }
    }
}

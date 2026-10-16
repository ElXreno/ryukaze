//! Cooler boost by itself when it gets hot: on once a temperature has
//! stayed at its mark for a few seconds, off once everything has been well
//! below for a while. It only turns off what it turned on, and a boost
//! switched off by hand stays off until things have cooled down.

use serde::Deserialize;

/// Ticks (two seconds each) a temperature has to stay at its mark.
const HOT_TICKS: u8 = 3;

/// Ticks everything has to stay `DROP` below its mark before the boost ends.
const COOL_TICKS: u8 = 5;

/// How far below the mark counts as cooled down, °C.
const DROP: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Off unless asked for: cooler boost is loud.
    pub auto: bool,
    pub cpu_c: u8,
    pub gpu_c: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto: false,
            cpu_c: 95,
            gpu_c: 85,
        }
    }
}

#[derive(Debug, Default)]
pub struct AutoBoost {
    pub config: Config,
    hot_ticks: u8,
    cool_ticks: u8,
    /// It turned the boost on and hasn't turned it off yet.
    boosting: bool,
    /// The boost it turned on was turned off by hand while still hot.
    overridden: bool,
}

impl AutoBoost {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// One tick with the CPU and GPU temperatures and whether the boost is
    /// on now; `Some` says what to switch it to.
    pub fn tick(&mut self, temps: [Option<u8>; 2], boost_on: bool) -> Option<bool> {
        if !self.config.auto {
            return self.stop(boost_on);
        }
        let marks = [self.config.cpu_c, self.config.gpu_c];
        let hot = temps
            .iter()
            .zip(marks)
            .any(|(temp, mark)| temp.is_some_and(|temp| temp >= mark));
        let cool = temps
            .iter()
            .zip(marks)
            .all(|(temp, mark)| temp.is_none_or(|temp| temp < mark.saturating_sub(DROP)));

        if self.boosting {
            if !boost_on {
                // Switched off by hand: leave it until it's cool again.
                self.boosting = false;
                self.overridden = true;
                return None;
            }
            self.cool_ticks = if cool { self.cool_ticks + 1 } else { 0 };
            if self.cool_ticks >= COOL_TICKS {
                self.boosting = false;
                self.cool_ticks = 0;
                return Some(false);
            }
            return None;
        }

        if self.overridden {
            self.overridden = !cool;
            return None;
        }
        self.hot_ticks = if hot { self.hot_ticks + 1 } else { 0 };
        // A boost already on is someone else's to switch off.
        if self.hot_ticks >= HOT_TICKS && !boost_on {
            self.hot_ticks = 0;
            self.boosting = true;
            return Some(true);
        }
        None
    }

    /// Ends a boost it started, as when switched off or the daemon stops.
    pub fn stop(&mut self, boost_on: bool) -> Option<bool> {
        let was = std::mem::take(&mut self.boosting);
        self.hot_ticks = 0;
        self.cool_ticks = 0;
        self.overridden = false;
        (was && boost_on).then_some(false)
    }

    #[cfg(test)]
    pub fn boosting(&self) -> bool {
        self.boosting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto() -> AutoBoost {
        AutoBoost::new(Config {
            auto: true,
            ..Config::default()
        })
    }

    #[test]
    fn a_spike_doesnt_start_it() {
        let mut boost = auto();
        assert_eq!(boost.tick([Some(97), Some(0)], false), None);
        assert_eq!(boost.tick([Some(97), Some(0)], false), None);
        assert_eq!(boost.tick([Some(80), Some(0)], false), None);
        assert_eq!(boost.tick([Some(97), Some(0)], false), None);
    }

    #[test]
    fn lasting_heat_starts_it_and_cooling_ends_it() {
        let mut boost = auto();
        boost.tick([Some(96), None], false);
        boost.tick([Some(96), None], false);
        assert_eq!(boost.tick([Some(96), None], false), Some(true));
        // Below the mark but not 10 under: stays on.
        for _ in 0..10 {
            assert_eq!(boost.tick([Some(90), None], true), None);
        }
        for _ in 0..4 {
            assert_eq!(boost.tick([Some(80), None], true), None);
        }
        assert_eq!(boost.tick([Some(80), None], true), Some(false));
    }

    #[test]
    fn the_gpu_has_its_own_mark() {
        let mut boost = auto();
        for _ in 0..2 {
            boost.tick([Some(70), Some(86)], false);
        }
        assert_eq!(boost.tick([Some(70), Some(86)], false), Some(true));
    }

    #[test]
    fn switched_off_by_hand_it_waits_for_the_cool() {
        let mut boost = auto();
        for _ in 0..3 {
            boost.tick([Some(96), None], false);
        }
        assert!(boost.boosting());
        // Turned off by hand, still hot: no fight.
        for _ in 0..5 {
            assert_eq!(boost.tick([Some(96), None], false), None);
        }
        // Cool once, then hot again: it's its own call again.
        boost.tick([Some(70), None], false);
        boost.tick([Some(96), None], false);
        boost.tick([Some(96), None], false);
        assert_eq!(boost.tick([Some(96), None], false), Some(true));
    }

    #[test]
    fn a_boost_on_by_hand_is_left_alone() {
        let mut boost = auto();
        for _ in 0..5 {
            assert_eq!(boost.tick([Some(96), None], true), None);
        }
        for _ in 0..10 {
            assert_eq!(boost.tick([Some(60), None], true), None);
        }
    }

    #[test]
    fn switching_the_feature_off_ends_its_boost() {
        let mut boost = auto();
        for _ in 0..3 {
            boost.tick([Some(96), None], false);
        }
        boost.config.auto = false;
        assert_eq!(boost.tick([Some(96), None], true), Some(false));
        assert_eq!(boost.tick([Some(96), None], false), None);
    }
}

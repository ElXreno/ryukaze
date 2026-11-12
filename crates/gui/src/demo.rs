//! `ryukaze --demo`: the window over a made-up laptop, for trying the
//! window out without ryukazed, or without changing what it runs. Picks
//! change only what's shown.

use std::collections::BTreeMap;

use ryukaze_core::Status;

use crate::app::Message;

/// Its config, as the daemon would give it for the copy.
pub const CONFIG: &str = "apex_on_ac = true\ncharge_limit = 80\n\n[profiles.ac]\nscenario = \"performance\"\n\n[[scenarios]]\nname = \"silent\"\nshift_mode = \"eco\"\n\n[[scenarios]]\nname = \"balanced\"\nshift_mode = \"comfort\"\n\n[[scenarios]]\nname = \"performance\"\nshift_mode = \"turbo\"\nepp = 132\n";

/// A Titan 18 HX, more or less.
pub fn status() -> Status {
    let names = |names: &[&str]| Some(names.iter().map(|&name| name.to_owned()).collect());
    Status {
        firmware: Some("1824EMS1.509".into()),
        shift_mode: Some("apex".into()),
        shift_modes: names(&["eco", "comfort", "turbo", "apex"]),
        fan_mode: Some("advanced".into()),
        fan_modes: names(&["auto", "silent", "advanced"]),
        super_battery: Some(false),
        epp_names: names(&[
            "default",
            "performance",
            "balance_performance",
            "balance_power",
            "power",
        ]),
        epp_numbers: Some(true),
        cooler_boost: Some(false),
        scenario: Some("performance".into()),
        scenarios: names(&["silent", "balanced", "performance"]),
        cpu_temp: Some(48),
        gpu_temp: Some(39),
        cpu_fan: Some(20),
        gpu_fan: Some(20),
        cpu_rpm: Some(1297),
        gpu_rpm: Some(1325),
        cpu_curve: Some((
            vec![58, 64, 70, 76, 82, 88],
            vec![20, 20, 35, 44, 58, 75, 100],
        )),
        gpu_curve: Some((
            vec![52, 58, 64, 70, 76, 82],
            vec![20, 20, 35, 44, 58, 75, 100],
        )),
        adaptive_fans: Some(true),
        gpu_fan_follows_cpu: Some(true),
        auto_cooler_boost: Some(true),
        cooler_boost_cpu_c: Some(92),
        cooler_boost_gpu_c: Some(82),
        webcam: Some(true),
        fn_key: Some("right".into()),
        local_dimming: Some(false),
        keyboard_profile: Some("breathe".into()),
        keyboard_profiles: names(&["static", "breathe", "cycle", "wave", "reactive", "Static 2"]),
        keyboard_saved_profiles: names(&["Static 2"]),
        keyboard_scenarios: Some(BTreeMap::new()),
        power_light: Some("profile".into()),
        power_light_dgpu: Some(false),
        power_colors: Some(BTreeMap::new()),
        numpad_follows_numlock: Some(true),
        lighting_parts: names(&["bar", "logo"]),
        ac_scenario: Some("performance".into()),
        battery_scenario: Some("*".into()),
        config_keys: names(&["charge_limit", "fans.cpu", "fans.gpu", "profiles.ac"]),
        power_profiles_sync: Some(true),
        gpu_mode: Some("discrete".into()),
        gpu_mode_next: Some("discrete".into()),
        gpu_modes: names(&["hybrid", "discrete", "integrated"]),
        dgpu_awake: Some(true),
        dgpu_for_secs: Some(3600),
        dgpu_users: names(&["niri"]),
        on_ac: Some(true),
        battery_level: Some(67),
        battery_state: Some("Not charging".into()),
        battery_watts: Some(0.0),
        battery_health: Some(99),
        charge_limit: Some(70),
        ..Status::default()
    }
}

/// What a pick would change, changed here; false for what it leaves to
/// the daemon, which isn't there.
pub fn apply(status: &mut Status, message: &Message) -> bool {
    let set = |slot: &mut Option<String>, value: &str| *slot = Some(value.to_owned());
    match message {
        Message::ShiftMode(mode) => set(&mut status.shift_mode, mode),
        Message::FanMode(mode) => set(&mut status.fan_mode, mode),
        Message::Scenario(name) => set(&mut status.scenario, name),
        Message::KeyboardProfile(name) => set(&mut status.keyboard_profile, name),
        Message::GpuMode(mode) => set(&mut status.gpu_mode_next, mode),
        Message::FnKey(side) => set(&mut status.fn_key, side),
        Message::PowerLight(follows) => set(&mut status.power_light, follows),
        Message::CoolerBoost(on) => status.cooler_boost = Some(*on),
        Message::AutoCoolerBoost(on) => status.auto_cooler_boost = Some(*on),
        Message::SuperBattery(on) => status.super_battery = Some(*on),
        Message::Webcam(on) => status.webcam = Some(*on),
        Message::LocalDimming(on) => status.local_dimming = Some(*on),
        Message::UsbShare(on) => status.usb_share = Some(*on),
        Message::NumpadFollowsNumlock(on) => status.numpad_follows_numlock = Some(*on),
        Message::PowerProfilesSync(on) => status.power_profiles_sync = Some(*on),
        Message::AdaptiveFans(on) => status.adaptive_fans = Some(*on),
        Message::GpuFollowsCpu(on) => status.gpu_fan_follows_cpu = Some(*on),
        Message::PowerLightDgpu(on) => status.power_light_dgpu = Some(*on),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_change_what_is_shown() {
        let mut status = status();
        assert!(apply(&mut status, &Message::ShiftMode("eco".into())));
        assert!(apply(&mut status, &Message::CoolerBoost(true)));
        assert_eq!(status.shift_mode.as_deref(), Some("eco"));
        assert_eq!(status.cooler_boost, Some(true));
        assert!(!apply(&mut status, &Message::Tick));
    }
}

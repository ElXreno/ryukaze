//! What ryukazed reports, in words: `ryukaze --status` prints it, and the
//! window says the same things the same way.

use std::collections::HashMap;

use ryukaze_core::Status;
use zbus::zvariant::{LE, OwnedValue, Value, serialized::Context, to_bytes};

use crate::app::{FAN_ORDER, SHIFT_ORDER, ordered};

/// The status as lines of a label and a value, grouped as the window's
/// sections are. Of the choices, the current one is in brackets, as sysfs
/// has it: `eco [comfort] sport turbo`.
pub fn text(status: &Status) -> String {
    let advanced = status.fan_mode.as_deref() == Some("advanced");
    let adaptive = status.adaptive_fans == Some(true);
    let groups = [
        vec![
            (
                "Scenario",
                choices(status.scenario.as_deref(), &status.scenarios, &[]),
            ),
            (
                "Performance",
                choices(
                    status.shift_mode.as_deref(),
                    &status.shift_modes,
                    SHIFT_ORDER,
                ),
            ),
        ],
        vec![
            (
                "CPU",
                sensor(status.cpu_temp, status.cpu_fan, status.cpu_rpm),
            ),
            (
                "GPU",
                sensor(status.gpu_temp, status.gpu_fan, status.gpu_rpm),
            ),
            (
                "Fans",
                choices(status.fan_mode.as_deref(), &status.fan_modes, FAN_ORDER),
            ),
            ("Cooler boost", status.cooler_boost.map(on)),
            (
                "Cooler boost when hot",
                status.auto_cooler_boost.map(|auto| {
                    match status.cooler_boost_cpu_c.zip(status.cooler_boost_gpu_c) {
                        Some((cpu, gpu)) if auto => format!("on, at {cpu}°C CPU or {gpu}°C GPU"),
                        _ => on(auto),
                    }
                }),
            ),
            (
                "Adaptive",
                status.adaptive_fans.filter(|_| advanced).map(on),
            ),
            (
                "GPU fan helps the CPU",
                status
                    .gpu_fan_follows_cpu
                    .filter(|_| advanced && adaptive)
                    .map(on),
            ),
            (
                "CPU curve",
                status.cpu_curve.as_ref().filter(|_| advanced).map(curve),
            ),
            (
                "GPU curve",
                status.gpu_curve.as_ref().filter(|_| advanced).map(curve),
            ),
        ],
        vec![
            (
                "Battery",
                status
                    .battery_level
                    .map(|level| match battery_doing(status) {
                        Some(doing) => format!("{level}%, {doing}"),
                        None => format!("{level}%"),
                    }),
            ),
            (
                "Battery health",
                status.battery_health.map(|health| format!("{health}%")),
            ),
            (
                "Charge limit",
                status
                    .charge_limit
                    .map(|limit| format!("{limit}%, {}", resumes(limit))),
            ),
            ("Super battery", status.super_battery.map(on)),
        ],
        vec![
            ("Camera", status.webcam.map(on)),
            (
                "Fn key",
                status.fn_key.as_ref().map(|side| format!("{side} of Win")),
            ),
            ("Charge USB when off", status.usb_share.map(on)),
        ],
        vec![
            (
                "Graphics",
                status
                    .gpu_mode
                    .as_ref()
                    .map(|now| match &status.gpu_mode_next {
                        Some(next) if next != now => format!("{now}, {next} after a restart"),
                        _ => now.clone(),
                    }),
            ),
            ("dGPU", dgpu_state(status)),
        ],
        vec![(
            "EC firmware",
            status
                .firmware
                .as_ref()
                .map(|firmware| match &status.firmware_date {
                    Some(date) => format!("{firmware}, {date}"),
                    None => firmware.clone(),
                }),
        )],
    ];

    let groups: Vec<Vec<(&str, String)>> = groups
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .filter_map(|(label, value)| Some((label, value?)))
                .collect::<Vec<_>>()
        })
        .filter(|group| !group.is_empty())
        .collect();
    let width = groups
        .iter()
        .flatten()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or_default();
    if groups.is_empty() {
        return "ryukazed has nothing to report here".to_owned();
    }
    groups
        .iter()
        .map(|group| {
            group
                .iter()
                .map(|(label, value)| format!("{label:width$}  {value}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The status as a JSON object on one line, under the keys of the D-Bus
/// dictionary.
pub fn json(status: &Status) -> Result<String, String> {
    let bytes = to_bytes(Context::new_dbus(LE, 0), status).map_err(|err| err.to_string())?;
    let (fields, _): (HashMap<String, OwnedValue>, _) =
        bytes.deserialize().map_err(|err| err.to_string())?;
    let object: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .map(|(key, value)| (key.clone(), plain(value)))
        .collect();
    serde_json::to_string(&object).map_err(|err| err.to_string())
}

/// A D-Bus value as JSON: tuples as arrays.
fn plain(value: &Value) -> serde_json::Value {
    use serde_json::Value as Json;
    match value {
        Value::Bool(on) => Json::from(*on),
        Value::U8(number) => Json::from(*number),
        Value::I16(number) => Json::from(*number),
        Value::U16(number) => Json::from(*number),
        Value::I32(number) => Json::from(*number),
        Value::U32(number) => Json::from(*number),
        Value::I64(number) => Json::from(*number),
        Value::U64(number) => Json::from(*number),
        Value::F64(number) => Json::from(*number),
        Value::Str(text) => Json::from(text.as_str()),
        Value::Value(inner) => plain(inner),
        Value::Array(items) => items.iter().map(plain).collect(),
        Value::Structure(fields) => fields.fields().iter().map(plain).collect(),
        _ => Json::Null,
    }
}

/// What the battery does: `on battery · 12 W`, `held at the charge limit`.
pub fn battery_doing(status: &Status) -> Option<String> {
    let level = status.battery_level?;
    let watts = status
        .battery_watts
        .filter(|&watts| watts >= 0.5)
        .map(|watts| format!(" · {watts:.0} W"))
        .unwrap_or_default();
    // Not charging on AC at or over the limit's lower edge: the EC holds it.
    let held = status.on_ac == Some(true)
        && status
            .charge_limit
            .is_some_and(|limit| limit < 100 && level + 10 >= limit);
    Some(match status.battery_state.as_deref()? {
        "Charging" => format!("charging{watts}"),
        "Discharging" => format!("on battery{watts}"),
        "Not charging" if held => "held at the charge limit".to_owned(),
        other => other.to_lowercase(),
    })
}

/// When the battery charges again under a limit.
pub fn resumes(limit: u8) -> String {
    if limit >= 100 {
        "charges to full".to_owned()
    } else {
        format!("charging resumes below {}%", limit.saturating_sub(10))
    }
}

/// Whether the dGPU sleeps, for how long, and who keeps it up:
/// `awake for 3 min, held by firefox, steam`.
pub fn dgpu_state(status: &Status) -> Option<String> {
    let awake = status.dgpu_awake?;
    let since = status
        .dgpu_for_secs
        .map(|secs| format!(" for {}", duration(secs)))
        .unwrap_or_default();
    let users = status.dgpu_users.as_deref().unwrap_or_default();
    Some(match (awake, users) {
        (false, _) => format!("asleep{since}"),
        (true, []) => format!("awake{since}"),
        (true, users) => format!("awake{since}, held by {}", users.join(", ")),
    })
}

/// 40 s, 12 min, 2 h 5 min.
fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        _ => format!("{} h {} min", secs / 3600, secs % 3600 / 60),
    }
}

fn on(on: bool) -> String {
    if on { "on" } else { "off" }.to_owned()
}

/// The choices in order with the current one in brackets; one they don't
/// include goes after them, bracketed too.
fn choices(current: Option<&str>, all: &Option<Vec<String>>, order: &[&str]) -> Option<String> {
    let all = all.as_deref().unwrap_or_default();
    if all.is_empty() {
        return current.map(str::to_owned);
    }
    let mut words: Vec<String> = ordered(all, order)
        .into_iter()
        .map(|choice| {
            if current == Some(choice.as_str()) {
                format!("[{choice}]")
            } else {
                choice.clone()
            }
        })
        .collect();
    if let Some(current) = current
        && !all.iter().any(|choice| choice == current)
    {
        words.push(format!("[{current}]"));
    }
    Some(words.join(" "))
}

/// A temperature and the fan under it. The EC reads 0 °C off a sleeping
/// GPU.
fn sensor(temp: Option<u8>, fan: Option<u8>, rpm: Option<u16>) -> Option<String> {
    let temp = temp.map(|temp| match temp {
        0 => "asleep".to_owned(),
        temp => format!("{temp}°C"),
    });
    let fan = fan.map(|fan| match rpm {
        Some(rpm) => format!("fan {fan}% · {rpm} rpm"),
        None => format!("fan {fan}%"),
    });
    match (temp, fan) {
        (Some(temp), Some(fan)) => Some(format!("{temp}, {fan}")),
        (temp, fan) => temp.or(fan),
    }
}

/// A fan curve as its steps: the speed below the first temperature, then
/// each speed from where it starts.
fn curve((temps, speeds): &(Vec<u8>, Vec<u8>)) -> String {
    let mut steps = speeds.iter().map(|speed| format!("{speed}%"));
    let first = steps.next().unwrap_or_default();
    std::iter::once(first)
        .chain(
            steps
                .zip(temps)
                .map(|(speed, temp)| format!("{speed} at {temp}°C")),
        )
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha17() -> Status {
        Status {
            firmware: Some("17KKIMS1.115".into()),
            firmware_date: Some("2023/04/21".into()),
            shift_mode: Some("comfort".into()),
            shift_modes: Some(vec!["eco".into(), "comfort".into(), "turbo".into()]),
            fan_mode: Some("auto".into()),
            fan_modes: Some(vec!["auto".into(), "silent".into(), "advanced".into()]),
            super_battery: Some(false),
            cooler_boost: Some(false),
            scenario: Some("balanced".into()),
            scenarios: Some(vec![
                "silent".into(),
                "balanced".into(),
                "performance".into(),
            ]),
            cpu_temp: Some(62),
            gpu_temp: Some(0),
            cpu_fan: Some(45),
            gpu_fan: Some(45),
            cpu_rpm: Some(2900),
            gpu_rpm: Some(3100),
            cpu_curve: Some((
                vec![55, 62, 69, 76, 83, 90],
                vec![0, 30, 35, 40, 50, 60, 100],
            )),
            gpu_curve: Some((
                vec![55, 62, 69, 76, 83, 90],
                vec![0, 30, 35, 40, 50, 60, 100],
            )),
            adaptive_fans: Some(true),
            gpu_fan_follows_cpu: Some(true),
            auto_cooler_boost: Some(true),
            cooler_boost_cpu_c: Some(90),
            cooler_boost_gpu_c: Some(85),
            webcam: Some(true),
            fn_key: Some("left".into()),
            usb_share: Some(false),
            gpu_mode: Some("hybrid".into()),
            gpu_mode_next: Some("discrete".into()),
            dgpu_awake: Some(false),
            dgpu_for_secs: Some(7500),
            on_ac: Some(false),
            battery_level: Some(78),
            battery_state: Some("Discharging".into()),
            battery_watts: Some(12.4),
            battery_health: Some(94),
            charge_limit: Some(80),
            ..Status::default()
        }
    }

    #[test]
    fn says_it_in_lines() {
        assert_eq!(
            text(&alpha17()),
            "\
Scenario               silent [balanced] performance
Performance            eco [comfort] turbo

CPU                    62°C, fan 45% · 2900 rpm
GPU                    asleep, fan 45% · 3100 rpm
Fans                   [auto] silent advanced
Cooler boost           off
Cooler boost when hot  on, at 90°C CPU or 85°C GPU

Battery                78%, on battery · 12 W
Battery health         94%
Charge limit           80%, charging resumes below 70%
Super battery          off

Camera                 on
Fn key                 left of Win
Charge USB when off    off

Graphics               hybrid, discrete after a restart
dGPU                   asleep for 2 h 5 min

EC firmware            17KKIMS1.115, 2023/04/21"
        );
    }

    #[test]
    fn the_curves_in_advanced() {
        let advanced = text(&Status {
            fan_mode: Some("advanced".into()),
            ..alpha17()
        });
        for line in [
            "Fans                   auto silent [advanced]",
            "Adaptive               on",
            "GPU fan helps the CPU  on",
            "CPU curve              0%, 30% at 55°C, 35% at 62°C, 40% at 69°C, 50% at 76°C, \
             60% at 83°C, 100% at 90°C",
        ] {
            assert!(
                advanced.lines().any(|shown| shown == line),
                "{line}\n{advanced}"
            );
        }
    }

    #[test]
    fn brackets_what_is_current() {
        let none = text(&Status {
            scenario: None,
            shift_mode: Some("unknown (5)".into()),
            ..alpha17()
        });
        assert!(none.contains("Scenario               silent balanced performance\n"));
        assert!(none.contains("Performance            eco comfort turbo [unknown (5)]\n"));
        // Without a list, the mode alone.
        assert_eq!(
            text(&Status {
                shift_mode: Some("eco".into()),
                ..Status::default()
            }),
            "Performance  eco"
        );
    }

    #[test]
    fn nothing_to_report() {
        assert_eq!(
            text(&Status {
                scenarios: Some(vec![]),
                ..Status::default()
            }),
            "ryukazed has nothing to report here"
        );
    }

    #[test]
    fn json_under_the_dictionary_keys() {
        let parsed: serde_json::Value = serde_json::from_str(&json(&alpha17()).unwrap()).unwrap();
        assert_eq!(parsed["shift_mode"], "comfort");
        assert_eq!(parsed["scenarios"][2], "performance");
        assert_eq!(parsed["cpu_temp"], 62);
        assert_eq!(parsed["battery_watts"], 12.4);
        assert_eq!(parsed["webcam"], true);
        assert_eq!(parsed["cpu_curve"][0][5], 90);
        assert_eq!(parsed["cpu_curve"][1][6], 100);
        // What the EC lacks isn't there at all.
        assert!(parsed.get("dgpu_users").is_none());
        assert_eq!(json(&Status::default()).unwrap(), "{}");
    }
}

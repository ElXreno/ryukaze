//! What the window can change, in the shape of `services.ryukaze.settings`:
//! for a config that's to have what the window changed.

use std::fmt::Write;

use ryukaze_core::{
    Status,
    fans::{Curve, Fan},
};

pub enum Value {
    Bool(bool),
    Int(u32),
    Str(String),
    List(Vec<Value>),
    Attrs(Vec<(String, Value)>),
}

/// The settings `status` reports, with the scenarios and the keyboard's
/// saved profiles read beside it, as attributes to go inside
/// `settings = { … };`.
pub fn settings(
    status: &Status,
    curves: &[(Fan, Curve)],
    scenarios: Vec<Value>,
    profiles: Vec<Value>,
) -> String {
    let int = |n: u8| Value::Int(n.into());
    let mut top = Vec::new();
    put(&mut top, "charge_limit", status.charge_limit.map(int));
    put(&mut top, "webcam", status.webcam.map(Value::Bool));
    put(&mut top, "fn_key", status.fn_key.clone().map(Value::Str));
    put(&mut top, "usb_share", status.usb_share.map(Value::Bool));
    put(
        &mut top,
        "local_dimming",
        status.local_dimming.map(Value::Bool),
    );

    let mut boost = Vec::new();
    put(
        &mut boost,
        "auto",
        status.auto_cooler_boost.map(Value::Bool),
    );
    put(&mut boost, "cpu_c", status.cooler_boost_cpu_c.map(int));
    put(&mut boost, "gpu_c", status.cooler_boost_gpu_c.map(int));
    if !boost.is_empty() {
        top.push(("cooler_boost".into(), Value::Attrs(boost)));
    }

    let mut fans = Vec::new();
    put(&mut fans, "adaptive", status.adaptive_fans.map(Value::Bool));
    put(
        &mut fans,
        "gpu_follows_cpu",
        status.gpu_fan_follows_cpu.map(Value::Bool),
    );
    for (fan, curve) in curves {
        let list = |values: &[u8]| Value::List(values.iter().map(|&value| int(value)).collect());
        let curve = Value::Attrs(vec![
            ("temps".into(), list(&curve.temps)),
            ("speeds".into(), list(&curve.speeds)),
        ]);
        fans.push((fan.to_string(), curve));
    }
    if !fans.is_empty() {
        top.push(("fans".into(), Value::Attrs(fans)));
    }
    if !scenarios.is_empty() {
        top.push(("scenarios".into(), Value::List(scenarios)));
    }
    let keyboard = keyboard(status, profiles);
    if !keyboard.is_empty() {
        top.push(("keyboard".into(), Value::Attrs(keyboard)));
    }

    let mut nix = String::new();
    attrs(&mut nix, &top, 0);
    nix
}

/// The keyboard light's settings: the profile shown outside scenarios with
/// one of their own, those scenarios', the profiles saved here, the numpad
/// and the power button's light.
fn keyboard(status: &Status, profiles: Vec<Value>) -> Vec<(String, Value)> {
    let mut keyboard = Vec::new();
    let scenarios = status.keyboard_scenarios.clone().unwrap_or_default();
    // The profile shown is the scenario's where it has one: then the
    // usual one isn't known here.
    let own = status
        .scenario
        .as_ref()
        .is_some_and(|scenario| scenarios.contains_key(scenario));
    if !own {
        put(
            &mut keyboard,
            "profile",
            status.keyboard_profile.clone().map(Value::Str),
        );
    }
    if !profiles.is_empty() {
        keyboard.push(("profiles".into(), Value::List(profiles)));
    }
    if !scenarios.is_empty() {
        keyboard.push((
            "scenarios".into(),
            Value::Attrs(
                scenarios
                    .into_iter()
                    .map(|(scenario, profile)| (scenario, Value::Str(profile)))
                    .collect(),
            ),
        ));
    }
    put(
        &mut keyboard,
        "numpad_follows_numlock",
        status.numpad_follows_numlock.map(Value::Bool),
    );
    let mut power = Vec::new();
    put(
        &mut power,
        "follows",
        status.power_light.clone().map(Value::Str),
    );
    put(
        &mut power,
        "dgpu_waking",
        status.power_light_dgpu.map(Value::Bool),
    );
    let colors = status.power_colors.clone().unwrap_or_default();
    let modes: Vec<(String, Value)> = colors
        .iter()
        .filter(|(name, _)| *name != "dgpu")
        .map(|(name, color)| (name.clone(), Value::Str(color.clone())))
        .collect();
    if !modes.is_empty() {
        power.push(("shift_modes".into(), Value::Attrs(modes)));
    }
    put(
        &mut power,
        "dgpu_awake",
        colors.get("dgpu").cloned().map(Value::Str),
    );
    if !power.is_empty() {
        keyboard.push(("power".into(), Value::Attrs(power)));
    }
    keyboard
}

/// An attribute set of its own, `{ … }`, for an element of a list.
#[cfg(test)]
pub fn attrset(entries: &[(String, Value)]) -> String {
    let mut nix = String::from("{\n");
    attrs(&mut nix, entries, 1);
    nix.push('}');
    nix
}

pub fn put(attrs: &mut Vec<(String, Value)>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        attrs.push((key.to_owned(), value));
    }
}

/// A name as Nix takes it: bare if it's an identifier, quoted if not, as a
/// key called `102nd` or `0` has to be.
fn name(key: &str) -> String {
    let mut chars = key.chars();
    let bare = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '\''));
    if bare { key.to_owned() } else { string(key) }
}

/// `text` as a Nix string: quoted, with `\`, `"` and `${` escaped and the
/// control characters Nix has escapes for written as them; anything else
/// as it is.
fn string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '$' if chars.peek() == Some(&'{') => quoted.push_str("\\$"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// A value that goes on one line.
fn scalar(value: &Value) -> String {
    match value {
        Value::Bool(on) => on.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Str(text) => string(text),
        Value::List(_) | Value::Attrs(_) => unreachable!("not a scalar"),
    }
}

/// As nixfmt lays them out: an attribute a line, a list of more than one
/// value a value a line.
fn attrs(nix: &mut String, attrs: &[(String, Value)], depth: usize) {
    for (key, value) in attrs {
        let _ = write!(nix, "{:indent$}{} = ", "", name(key), indent = depth * 2);
        match value {
            Value::List(values) if values.len() == 1 && !matches!(values[0], Value::Attrs(_)) => {
                let _ = writeln!(nix, "[ {} ];", scalar(&values[0]));
            }
            Value::List(values) => {
                nix.push_str("[\n");
                for value in values {
                    if let Value::Attrs(inner) = value {
                        let _ = writeln!(nix, "{:indent$}{{", "", indent = depth * 2 + 2);
                        self::attrs(nix, inner, depth + 2);
                        let _ = writeln!(nix, "{:indent$}}}", "", indent = depth * 2 + 2);
                        continue;
                    }
                    let _ = writeln!(
                        nix,
                        "{:indent$}{}",
                        "",
                        scalar(value),
                        indent = depth * 2 + 2
                    );
                }
                let _ = writeln!(nix, "{:indent$}];", "", indent = depth * 2);
            }
            Value::Attrs(inner) => {
                nix.push_str("{\n");
                self::attrs(nix, inner, depth + 1);
                let _ = writeln!(nix, "{:indent$}}};", "", indent = depth * 2);
            }
            scalar_value => {
                let _ = writeln!(nix, "{};", scalar(scalar_value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text Nix reads back as it was, `${` and the joiners included.
    #[test]
    fn writes_strings_nix_reads_back() {
        assert_eq!(string("a${b}"), r#""a\${b}""#);
        assert_eq!(string("$x {y}"), r#""$x {y}""#);
        assert_eq!(string("q\"\\"), r#""q\"\\""#);
        assert_eq!(string("\u{2764}\u{fe0f}"), "\"\u{2764}\u{fe0f}\"");
        assert_eq!(string("a\nb"), r#""a\nb""#);
        assert_eq!(name("my key"), r#""my key""#);
    }

    /// The keyboard light as the window set it: the usual profile but where
    /// a scenario has its own, the scenarios', the numpad, the power button.
    #[test]
    fn writes_the_keyboard_light() {
        let status = Status {
            scenario: Some("silent".into()),
            keyboard_profile: Some("wave".into()),
            keyboard_scenarios: Some([("turbo".to_owned(), "static 2".to_owned())].into()),
            numpad_follows_numlock: Some(true),
            power_light: Some("shift_mode".into()),
            power_light_dgpu: Some(false),
            power_colors: Some(
                [
                    ("eco".to_owned(), "#00ff00".to_owned()),
                    ("dgpu".to_owned(), "#ff0000".to_owned()),
                ]
                .into(),
            ),
            ..Status::default()
        };
        let nix = settings(&status, &[], Vec::new(), Vec::new());
        for line in [
            "profile = \"wave\";",
            "turbo = \"static 2\";",
            "numpad_follows_numlock = true;",
            "follows = \"shift_mode\";",
            "eco = \"#00ff00\";",
            "dgpu_awake = \"#ff0000\";",
        ] {
            assert!(nix.contains(line), "{line} in\n{nix}");
        }
        // In a scenario of its own, the usual profile isn't known.
        let own = Status {
            scenario: Some("turbo".into()),
            ..status
        };
        assert!(!settings(&own, &[], Vec::new(), Vec::new()).contains("profile = "));
    }

    #[test]
    fn writes_what_is_there() {
        let status = Status {
            charge_limit: Some(80),
            fn_key: Some("left".into()),
            usb_share: Some(false),
            auto_cooler_boost: Some(true),
            cooler_boost_cpu_c: Some(90),
            cooler_boost_gpu_c: Some(85),
            adaptive_fans: Some(true),
            gpu_fan_follows_cpu: Some(true),
            ..Status::default()
        };
        let cpu = Curve::new(&[50, 57, 64, 71, 78, 85], &[0, 40, 45, 50, 57, 65, 80]).unwrap();
        let nix = settings(&status, &[(Fan::Cpu, cpu)], Vec::new(), Vec::new());
        assert_eq!(
            nix,
            r#"charge_limit = 80;
fn_key = "left";
usb_share = false;
cooler_boost = {
  auto = true;
  cpu_c = 90;
  gpu_c = 85;
};
fans = {
  adaptive = true;
  gpu_follows_cpu = true;
  cpu = {
    temps = [
      50
      57
      64
      71
      78
      85
    ];
    speeds = [
      0
      40
      45
      50
      57
      65
      80
    ];
  };
};
"#
        );
    }

    #[test]
    fn quotes_names_nix_wont_take_bare() {
        let entries = vec![
            ("name".to_owned(), Value::Str("custom".into())),
            ("period_ms".to_owned(), Value::Int(4000)),
            (
                "colors".to_owned(),
                Value::List(vec![
                    Value::Str("#ff0000".into()),
                    Value::Str("#0000ff".into()),
                ]),
            ),
            (
                "keys".to_owned(),
                Value::Attrs(vec![
                    ("w".to_owned(), Value::Str("#ff0000".into())),
                    ("102nd".to_owned(), Value::Str("off".into())),
                    ("0".to_owned(), Value::Str("#ffffff".into())),
                ]),
            ),
        ];
        assert_eq!(
            attrset(&entries),
            r##"{
  name = "custom";
  period_ms = 4000;
  colors = [
    "#ff0000"
    "#0000ff"
  ];
  keys = {
    w = "#ff0000";
    "102nd" = "off";
    "0" = "#ffffff";
  };
}"##
        );
    }

    #[test]
    fn nothing_for_what_the_ec_lacks() {
        let status = Status {
            charge_limit: Some(100),
            ..Status::default()
        };
        assert_eq!(
            settings(&status, &[], Vec::new(), Vec::new()),
            "charge_limit = 100;\n"
        );
        let profile = Value::Attrs(vec![
            ("name".into(), Value::Str("mine".into())),
            ("color".into(), Value::Str("#ff6000".into())),
        ]);
        assert_eq!(
            settings(&status, &[], Vec::new(), vec![profile]),
            "charge_limit = 100;\nkeyboard = {\n  profiles = [\n    {\n      name = \"mine\";\n      color = \"#ff6000\";\n    }\n  ];\n};\n"
        );
    }
}

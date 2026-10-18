//! What the window can change, in the shape of `services.ryukaze.settings`:
//! a change made here lasts until the daemon restarts, and this keeps it.

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

/// The settings `status` reports, as attributes to go inside
/// `settings = { … };`.
pub fn settings(status: &Status, curves: &[(Fan, Curve)]) -> String {
    let int = |n: u8| Value::Int(n.into());
    let mut top = Vec::new();
    put(&mut top, "charge_limit", status.charge_limit.map(int));
    put(&mut top, "webcam", status.webcam.map(Value::Bool));
    put(&mut top, "fn_key", status.fn_key.clone().map(Value::Str));
    put(&mut top, "usb_share", status.usb_share.map(Value::Bool));

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

    let mut nix = String::new();
    attrs(&mut nix, &top, 0);
    nix
}

/// An attribute set of its own, `{ … }`, for an element of a list.
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
    if bare {
        key.to_owned()
    } else {
        format!("{key:?}")
    }
}

/// A value that goes on one line.
fn scalar(value: &Value) -> String {
    match value {
        Value::Bool(on) => on.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Str(text) => format!("{text:?}"),
        Value::List(_) | Value::Attrs(_) => unreachable!("not a scalar"),
    }
}

/// As nixfmt lays them out: an attribute a line, a list of more than one
/// value a value a line.
fn attrs(nix: &mut String, attrs: &[(String, Value)], depth: usize) {
    for (key, value) in attrs {
        let _ = write!(nix, "{:indent$}{} = ", "", name(key), indent = depth * 2);
        match value {
            Value::List(values) if values.len() == 1 => {
                let _ = writeln!(nix, "[ {} ];", scalar(&values[0]));
            }
            Value::List(values) => {
                nix.push_str("[\n");
                for value in values {
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
        let nix = settings(&status, &[(Fan::Cpu, cpu)]);
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
        assert_eq!(settings(&status, &[]), "charge_limit = 100;\n");
    }
}

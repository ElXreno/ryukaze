//! What the window can change, in the shape of `services.ryukaze.settings`:
//! a change made here lasts until the daemon restarts, and this keeps it.

use std::fmt::Write;

use ryukaze_core::{
    Status,
    fans::{Curve, Fan},
};

enum Value {
    Bool(bool),
    Int(u8),
    Str(String),
    List(Vec<u8>),
    Attrs(Vec<(&'static str, Value)>),
}

/// The settings `status` reports, as attributes to go inside
/// `settings = { … };`.
pub fn settings(status: &Status, curves: &[(Fan, Curve)]) -> String {
    let mut top = Vec::new();
    put(
        &mut top,
        "charge_limit",
        status.charge_limit.map(Value::Int),
    );
    put(&mut top, "webcam", status.webcam.map(Value::Bool));
    put(&mut top, "fn_key", status.fn_key.clone().map(Value::Str));
    put(&mut top, "usb_share", status.usb_share.map(Value::Bool));

    let mut boost = Vec::new();
    put(
        &mut boost,
        "auto",
        status.auto_cooler_boost.map(Value::Bool),
    );
    put(
        &mut boost,
        "cpu_c",
        status.cooler_boost_cpu_c.map(Value::Int),
    );
    put(
        &mut boost,
        "gpu_c",
        status.cooler_boost_gpu_c.map(Value::Int),
    );
    if !boost.is_empty() {
        top.push(("cooler_boost", Value::Attrs(boost)));
    }

    let mut fans = Vec::new();
    put(&mut fans, "adaptive", status.adaptive_fans.map(Value::Bool));
    put(
        &mut fans,
        "gpu_follows_cpu",
        status.gpu_fan_follows_cpu.map(Value::Bool),
    );
    for (fan, curve) in curves {
        let curve = Value::Attrs(vec![
            ("temps", Value::List(curve.temps.to_vec())),
            ("speeds", Value::List(curve.speeds.to_vec())),
        ]);
        fans.push((
            match fan {
                Fan::Cpu => "cpu",
                Fan::Gpu => "gpu",
            },
            curve,
        ));
    }
    if !fans.is_empty() {
        top.push(("fans", Value::Attrs(fans)));
    }

    let mut nix = String::new();
    attrs(&mut nix, &top, 0);
    nix
}

fn put(attrs: &mut Vec<(&'static str, Value)>, key: &'static str, value: Option<Value>) {
    if let Some(value) = value {
        attrs.push((key, value));
    }
}

/// As nixfmt lays them out: an attribute a line, a list of more than one
/// value a value a line.
fn attrs(nix: &mut String, attrs: &[(&str, Value)], depth: usize) {
    for (key, value) in attrs {
        let _ = write!(nix, "{:indent$}{key} = ", "", indent = depth * 2);
        match value {
            Value::Bool(on) => {
                let _ = writeln!(nix, "{on};");
            }
            Value::Int(n) => {
                let _ = writeln!(nix, "{n};");
            }
            Value::Str(text) => {
                let _ = writeln!(nix, "{text:?};");
            }
            Value::List(values) if values.len() == 1 => {
                let _ = writeln!(nix, "[ {} ];", values[0]);
            }
            Value::List(values) => {
                nix.push_str("[\n");
                for value in values {
                    let _ = writeln!(nix, "{:indent$}{value}", "", indent = depth * 2 + 2);
                }
                let _ = writeln!(nix, "{:indent$}];", "", indent = depth * 2);
            }
            Value::Attrs(inner) => {
                nix.push_str("{\n");
                self::attrs(nix, inner, depth + 1);
                let _ = writeln!(nix, "{:indent$}}};", "", indent = depth * 2);
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
    fn nothing_for_what_the_ec_lacks() {
        let status = Status {
            charge_limit: Some(100),
            ..Status::default()
        };
        assert_eq!(settings(&status, &[]), "charge_limit = 100;\n");
    }
}

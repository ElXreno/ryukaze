//! The daemon's config, TOML as `ReadConfig` gives it, in the shape of
//! `services.ryukaze.settings`: every key of it, whatever it is, for a
//! config that's to have what the window changed.

use std::fmt::Write;

enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Attrs(Vec<(String, Value)>),
}

/// `config`, TOML, as attributes to go inside `settings = { … };`.
pub fn settings(config: &str) -> Result<String, String> {
    let table: toml::Table = toml::from_str(config).map_err(|err| err.to_string())?;
    let Value::Attrs(top) = value(toml::Value::Table(table)) else {
        unreachable!("a table is an attribute set");
    };
    let mut nix = String::new();
    attrs(&mut nix, &top, 0);
    Ok(nix)
}

fn value(value: toml::Value) -> Value {
    match value {
        toml::Value::Boolean(on) => Value::Bool(on),
        toml::Value::Integer(n) => Value::Int(n),
        toml::Value::Float(n) => Value::Float(n),
        toml::Value::String(text) => Value::Str(text),
        toml::Value::Datetime(when) => Value::Str(when.to_string()),
        toml::Value::Array(values) => Value::List(values.into_iter().map(self::value).collect()),
        toml::Value::Table(table) => Value::Attrs(
            table
                .into_iter()
                .map(|(key, value)| (key, self::value(value)))
                .collect(),
        ),
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
        Value::Float(n) => format!("{n:?}"),
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
            Value::List(values) if values.is_empty() => nix.push_str("[ ];\n"),
            Value::List(values)
                if values.len() == 1 && !matches!(values[0], Value::Attrs(_) | Value::List(_)) =>
            {
                let _ = writeln!(nix, "[ {} ];", scalar(&values[0]));
            }
            Value::List(values) => {
                nix.push_str("[\n");
                list(nix, values, depth + 1);
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

/// A list's values, a line each, sets and lists in it over lines of
/// their own.
fn list(nix: &mut String, values: &[Value], depth: usize) {
    for value in values {
        let _ = write!(nix, "{:indent$}", "", indent = depth * 2);
        match value {
            Value::Attrs(inner) => {
                nix.push_str("{\n");
                attrs(nix, inner, depth + 1);
                let _ = writeln!(nix, "{:indent$}}}", "", indent = depth * 2);
            }
            Value::List(inner) => {
                nix.push_str("[\n");
                list(nix, inner, depth + 1);
                let _ = writeln!(nix, "{:indent$}]", "", indent = depth * 2);
            }
            scalar_value => {
                let _ = writeln!(nix, "{}", scalar(scalar_value));
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

    /// Every key of a config, whatever it is and however deep.
    #[test]
    fn writes_a_config_whole() {
        let config = "apex_on_ac = true\ncharge_limit = 70\nratio = 0.5\nempty = []\nlone = [3]\n\n[fans.cpu]\ntemps = [61, 67]\n\n[profiles.ac]\nscenario = \"performance\"\n\n[[scenarios]]\nname = \"performance\"\nshift_mode = \"turbo\"\nepp = 132\n\n[[scenarios]]\nname = \"saving\"\nepp = \"power\"\n\n[keyboard.power.shift_modes]\neco = \"#00ff00\"\n\n[some.\"new key\"]\nnested = [[1, 2], [3]]\n";
        assert_eq!(
            settings(config).unwrap(),
            "apex_on_ac = true;\ncharge_limit = 70;\nempty = [ ];\nfans = {\n  cpu = {\n    temps = [\n      61\n      67\n    ];\n  };\n};\nkeyboard = {\n  power = {\n    shift_modes = {\n      eco = \"#00ff00\";\n    };\n  };\n};\nlone = [ 3 ];\nprofiles = {\n  ac = {\n    scenario = \"performance\";\n  };\n};\nratio = 0.5;\nscenarios = [\n  {\n    epp = 132;\n    name = \"performance\";\n    shift_mode = \"turbo\";\n  }\n  {\n    epp = \"power\";\n    name = \"saving\";\n  }\n];\nsome = {\n  \"new key\" = {\n    nested = [\n      [\n        1\n        2\n      ]\n      [\n        3\n      ]\n    ];\n  };\n};\n"
        );
        assert!(settings("a = ").is_err());
    }
}

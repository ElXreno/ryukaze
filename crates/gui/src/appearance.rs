//! The desktop's appearance, from the XDG settings portal: whether it
//! prefers dark, its accent, whether it wants less motion, and any other
//! change to how it looks, after which its palette is read again (see
//! [`crate::theme::read_desktop`]).

use iced::{
    Color,
    futures::{SinkExt, Stream, StreamExt},
    stream,
};
use zbus::zvariant::Value;

use crate::system;

const NAMESPACE: &str = "org.freedesktop.appearance";
const SCHEME: &str = "color-scheme";
const ACCENT: &str = "accent-color";
const MOTION: &str = "reduced-motion";

#[zbus::proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop",
    gen_blocking = false
)]
trait Settings {
    fn read_one(&self, namespace: &str, key: &str) -> zbus::Result<zbus::zvariant::OwnedValue>;

    #[zbus(signal)]
    fn setting_changed(&self, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Dark(bool),
    /// None where the desktop has no accent.
    Accent(Option<Color>),
    /// Whether it asks for less motion.
    Reduced(bool),
    /// Something else about how it looks, the contrast say.
    Other,
}

/// Whether the desktop asks for dark and its accent, now and on every
/// change. Without a portal it says nothing, and the window stays light.
pub fn changes() -> impl Stream<Item = Change> {
    stream::channel(4, async |mut output| {
        let Ok(connection) = zbus::Connection::session().await else {
            return;
        };
        let Ok(settings) = SettingsProxy::new(&connection).await else {
            return;
        };
        // Listen before reading, so a change in between isn't lost.
        let Ok(mut changes) = settings
            .receive_setting_changed_with_args(&[(0, NAMESPACE)])
            .await
        else {
            return;
        };
        for key in [SCHEME, ACCENT, MOTION] {
            if let Ok(value) = settings.read_one(NAMESPACE, key).await
                && let Some(change) = change(key, &value)
            {
                let _ = output.send(change).await;
            }
        }
        while let Some(signal) = changes.next().await {
            let Ok(args) = signal.args() else {
                continue;
            };
            let change = change(args.key(), args.value()).unwrap_or(Change::Other);
            if output.send(change).await.is_err() {
                return;
            }
        }
    })
}

fn change(key: &str, value: &Value<'_>) -> Option<Change> {
    match key {
        SCHEME => is_dark(value).map(Change::Dark),
        ACCENT => Some(Change::Accent(accent(value))),
        MOTION => Some(Change::Reduced(is_reduced(value))),
        _ => None,
    }
}

/// 1 is dark, 2 light, 0 no preference. Some portals wrap it once more.
fn is_dark(value: &Value<'_>) -> Option<bool> {
    match value {
        Value::Value(inner) => is_dark(inner),
        Value::U32(scheme) => Some(*scheme == 1),
        _ => None,
    }
}

/// 1 is less motion; anything else, no preference.
fn is_reduced(value: &Value<'_>) -> bool {
    match value {
        Value::Value(inner) => is_reduced(inner),
        Value::U32(motion) => *motion == 1,
        _ => false,
    }
}

/// `(ddd)`, each 0 to 1; out of range, none.
fn accent(value: &Value<'_>) -> Option<Color> {
    match value {
        Value::Value(inner) => accent(inner),
        Value::Structure(fields) => match fields.fields() {
            [Value::F64(r), Value::F64(g), Value::F64(b)] => system::accent((*r, *g, *b)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::StructureBuilder;

    #[test]
    fn reads_the_scheme() {
        assert_eq!(is_dark(&Value::U32(1)), Some(true));
        assert_eq!(is_dark(&Value::U32(2)), Some(false));
        assert_eq!(is_dark(&Value::U32(0)), Some(false));
        assert_eq!(is_dark(&Value::Value(Box::new(Value::U32(1)))), Some(true));
        assert_eq!(is_dark(&Value::from("dark")), None);
    }

    #[test]
    fn reads_the_accent() {
        let rgb = |r: f64, g: f64, b: f64| {
            Value::Structure(
                StructureBuilder::new()
                    .add_field(r)
                    .add_field(g)
                    .add_field(b)
                    .build()
                    .unwrap(),
            )
        };
        let blue = accent(&rgb(0.207_843, 0.517_647, 0.894_118)).unwrap();
        assert!((blue.g - 0.517_647).abs() < 1e-5);
        assert!(accent(&Value::Value(Box::new(rgb(0.0, 0.0, 1.0)))).is_some());
        // Unset, the portal gives a colour out of range.
        assert_eq!(accent(&rgb(-1.0, -1.0, -1.0)), None);
        assert_eq!(change(ACCENT, &Value::U32(1)), Some(Change::Accent(None)));
        assert_eq!(change(SCHEME, &Value::U32(1)), Some(Change::Dark(true)));
        assert_eq!(change(MOTION, &Value::U32(1)), Some(Change::Reduced(true)));
        assert_eq!(change(MOTION, &Value::U32(7)), Some(Change::Reduced(false)));
        assert_eq!(change("contrast", &Value::U32(1)), None);
    }
}

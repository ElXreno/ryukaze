//! The desktop's light or dark preference, from the XDG settings portal.

use iced::{
    futures::{SinkExt, Stream, StreamExt},
    stream,
};
use zbus::zvariant::Value;

const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

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

/// Whether the desktop asks for dark, now and on every change. Without a
/// portal it says nothing, and the window stays light.
pub fn dark() -> impl Stream<Item = bool> {
    stream::channel(4, async |mut output| {
        let Ok(connection) = zbus::Connection::session().await else {
            return;
        };
        let Ok(settings) = SettingsProxy::new(&connection).await else {
            return;
        };
        // Listen before reading, so a change in between isn't lost.
        let Ok(mut changes) = settings
            .receive_setting_changed_with_args(&[(0, NAMESPACE), (1, KEY)])
            .await
        else {
            return;
        };
        if let Ok(value) = settings.read_one(NAMESPACE, KEY).await
            && let Some(dark) = is_dark(&value)
        {
            let _ = output.send(dark).await;
        }
        while let Some(change) = changes.next().await {
            if let Ok(args) = change.args()
                && let Some(dark) = is_dark(args.value())
                && output.send(dark).await.is_err()
            {
                return;
            }
        }
    })
}

/// 1 is dark, 2 light, 0 no preference. Some portals wrap it once more.
fn is_dark(value: &Value<'_>) -> Option<bool> {
    match value {
        Value::Value(inner) => is_dark(inner),
        Value::U32(scheme) => Some(*scheme == 1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_scheme() {
        assert_eq!(is_dark(&Value::U32(1)), Some(true));
        assert_eq!(is_dark(&Value::U32(2)), Some(false));
        assert_eq!(is_dark(&Value::U32(0)), Some(false));
        assert_eq!(is_dark(&Value::Value(Box::new(Value::U32(1)))), Some(true));
        assert_eq!(is_dark(&Value::from("dark")), None);
    }
}

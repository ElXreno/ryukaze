//! ryukazed as seen from here.

use ryukaze_core::Status;

/// Mirrors crates/daemon/src/service.rs; zbus takes only literals here.
#[zbus::proxy(
    interface = "io.github.kontonkara.ryukaze1",
    default_service = "io.github.kontonkara.ryukaze",
    default_path = "/io/github/kontonkara/ryukaze",
    gen_blocking = false
)]
pub trait Ryukaze {
    fn status(&self) -> zbus::Result<Status>;
    fn set_shift_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_fan_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_super_battery(&self, on: bool) -> zbus::Result<()>;
    fn set_cooler_boost(&self, on: bool) -> zbus::Result<()>;
    fn set_charge_limit(&self, percent: u8) -> zbus::Result<()>;
    fn set_fan_curve(&self, fan: &str, temps: &[u8], speeds: &[u8]) -> zbus::Result<()>;
    fn reset_fan_curve(&self, fan: &str) -> zbus::Result<()>;
    fn set_adaptive_fans(&self, on: bool) -> zbus::Result<()>;
    fn set_gpu_fan_follows_cpu(&self, on: bool) -> zbus::Result<()>;
}

pub type Client = RyukazeProxy<'static>;

/// The system bus, or the session bus for `ryukazed --session`.
pub async fn connect(session: bool) -> Result<Client, String> {
    let connection = if session {
        zbus::Connection::session().await
    } else {
        zbus::Connection::system().await
    };
    let connection = connection.map_err(describe)?;
    RyukazeProxy::new(&connection).await.map_err(describe)
}

/// What went wrong, in words for the window rather than D-Bus error names.
pub fn describe(err: zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(name, text, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner" => "ryukazed isn't running".into(),
            "org.freedesktop.DBus.Error.AccessDenied" => {
                "Not allowed to change the EC: see services.ryukaze.users".into()
            }
            _ => text.unwrap_or_else(|| name.to_string()),
        },
        other => other.to_string(),
    }
}

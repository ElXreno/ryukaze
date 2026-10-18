//! ryukazed as seen from here.

use iced::{
    futures::{SinkExt, Stream, StreamExt},
    stream,
};
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
    fn set_auto_cooler_boost(&self, on: bool) -> zbus::Result<()>;
    fn set_webcam(&self, on: bool) -> zbus::Result<()>;
    fn set_fn_key(&self, side: &str) -> zbus::Result<()>;
    fn set_usb_share(&self, on: bool) -> zbus::Result<()>;
    fn set_gpu_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_cooler_boost_marks(&self, cpu_c: u8, gpu_c: u8) -> zbus::Result<()>;
    fn next_scenario(&self) -> zbus::Result<String>;
    fn set_scenario(&self, name: &str) -> zbus::Result<()>;
    fn set_keyboard_profile(&self, name: &str) -> zbus::Result<()>;
    fn next_keyboard_profile(&self) -> zbus::Result<String>;
    fn preview_keyboard_profile(&self, profile: &str) -> zbus::Result<()>;
    fn read_keyboard_profile(&self, name: &str) -> zbus::Result<String>;
    fn save_keyboard_profile(&self, profile: &str) -> zbus::Result<()>;
    fn delete_keyboard_profile(&self, name: &str) -> zbus::Result<()>;
    fn set_numpad_follows_numlock(&self, on: bool) -> zbus::Result<()>;
    fn set_power_profiles_sync(&self, on: bool) -> zbus::Result<()>;
    fn set_source_scenario(&self, source: &str, scenario: &str) -> zbus::Result<()>;
    fn set_scenario_keyboard_profile(&self, scenario: &str, profile: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn modes_changed(&self, modes: Status) -> zbus::Result<()>;
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

/// One call without the app, for the command line.
pub fn once<T, F, Fut>(session: bool, call: F) -> Result<T, String>
where
    F: FnOnce(Client) -> Fut,
    Fut: Future<Output = zbus::Result<T>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    runtime.block_on(async {
        let client = connect(session).await?;
        call(client).await.map_err(describe)
    })
}

/// The modes each time ryukazed says they changed. Silent while it can't
/// be reached: the polls cover that.
pub fn modes(session: &bool) -> impl Stream<Item = Status> + use<> {
    let session = *session;
    stream::channel(4, async move |mut output| {
        let Ok(client) = connect(session).await else {
            return;
        };
        let Ok(mut changes) = client.receive_modes_changed().await else {
            return;
        };
        while let Some(change) = changes.next().await {
            if let Ok(args) = change.args()
                && output.send(args.modes).await.is_err()
            {
                return;
            }
        }
    })
}

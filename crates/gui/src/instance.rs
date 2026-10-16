//! One ryukaze per session: a second start shows the first one's window.

use iced::{
    futures::{SinkExt, Stream},
    stream,
};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::app::Message;

const NAME: &str = "io.github.kontonkara.ryukaze.gui";
const PATH: &str = "/io/github/kontonkara/ryukaze/gui";

/// Before the app starts: when another ryukaze holds the name, asks it to
/// show its window (unless this start is a hidden one) and says so.
pub fn hand_over(hidden: bool) -> bool {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    runtime.block_on(async {
        let Ok(connection) = zbus::Connection::session().await else {
            return false;
        };
        let Ok(bus) = zbus::fdo::DBusProxy::new(&connection).await else {
            return false;
        };
        let Ok(name) = zbus::names::BusName::from_static_str(NAME) else {
            return false;
        };
        if !bus.name_has_owner(name).await.unwrap_or(false) {
            return false;
        }
        if !hidden {
            let _ = connection
                .call_method(
                    Some(NAME),
                    PATH,
                    Some("io.github.kontonkara.ryukaze.gui1"),
                    "Show",
                    &(),
                )
                .await;
        }
        true
    })
}

struct Remote {
    messages: UnboundedSender<Message>,
}

#[zbus::interface(name = "io.github.kontonkara.ryukaze.gui1")]
impl Remote {
    fn show(&self) {
        let _ = self.messages.send(Message::Show);
    }
}

/// Holds the name on the session bus and turns `Show` calls into messages.
pub fn serve() -> impl Stream<Item = Message> {
    stream::channel(4, async |mut output| {
        let (messages, mut shown) = mpsc::unbounded_channel();
        let connection = zbus::connection::Builder::session()
            .and_then(|builder| builder.name(NAME))
            .and_then(|builder| builder.serve_at(PATH, Remote { messages }));
        let _connection = match connection {
            Ok(builder) => match builder.build().await {
                Ok(connection) => connection,
                Err(err) => {
                    eprintln!("ryukaze: not reachable for a second start: {err}");
                    return;
                }
            },
            Err(err) => {
                eprintln!("ryukaze: not reachable for a second start: {err}");
                return;
            }
        };
        while let Some(message) = shown.recv().await {
            if output.send(message).await.is_err() {
                return;
            }
        }
    })
}

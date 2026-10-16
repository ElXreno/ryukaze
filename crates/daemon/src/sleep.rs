//! Suspend and resume, from logind. A delay lock holds the suspend back
//! until the EC's modes are noted, so they can go back after resume.

use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;
use zbus::zvariant::OwnedFd;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    gen_blocking = false
)]
trait Manager {
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[derive(Debug)]
pub enum Event {
    /// About to sleep; dropping the lock lets it.
    Sleeping(Option<OwnedFd>),
    Woke,
}

/// Watches logind on the system bus, also under `--session`.
pub async fn watch(events: UnboundedSender<Event>) -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let manager = ManagerProxy::new(&connection).await?;
    let mut signals = manager.receive_prepare_for_sleep().await?;
    let mut lock = take_lock(&manager).await;

    tokio::spawn(async move {
        while let Some(signal) = signals.next().await {
            let Ok(args) = signal.args() else {
                continue;
            };
            let event = if *args.start() {
                Event::Sleeping(lock.take())
            } else {
                lock = take_lock(&manager).await;
                Event::Woke
            };
            if events.send(event).is_err() {
                return;
            }
        }
    });
    Ok(())
}

async fn take_lock(manager: &ManagerProxy<'_>) -> Option<OwnedFd> {
    manager
        .inhibit(
            "sleep",
            "ryukaze",
            "Noting the EC's modes to put back after resume",
            "delay",
        )
        .await
        .inspect_err(|err| warn!("no sleep delay lock: {err}"))
        .ok()
}

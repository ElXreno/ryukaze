//! Applies the config: the charge limit, and a profile per power source.

use std::path::PathBuf;

use ryukaze_core::{
    OBJECT_PATH, Status,
    power_supply::{self, Source},
};
use tracing::{info, warn};
use zbus::{Connection, fdo};

use crate::{
    config::Config,
    service::{Service, Snapshot},
};

pub struct Profiles {
    config: Config,
    sysfs: PathBuf,
    connection: Connection,
    /// The source whose profile was applied last.
    applied: Option<Source>,
    /// The EC's modes as they were when the laptop went to sleep.
    before_sleep: Option<Snapshot>,
    /// The modes as last seen; none before the first look.
    announced: Option<Status>,
}

impl Profiles {
    pub fn new(config: Config, sysfs: PathBuf, connection: Connection) -> Self {
        Self {
            config,
            sysfs,
            connection,
            applied: None,
            before_sleep: None,
            announced: None,
        }
    }

    /// Announces the modes when they differ from the last look: after a
    /// setter, and every tick for what the EC changes by itself.
    pub async fn look(&mut self) {
        let service = self.service().await;
        let modes = service.get().await.modes();
        if self
            .announced
            .as_ref()
            .is_some_and(|before| *before != modes)
            && let Err(err) = Service::modes_changed(service.signal_emitter(), modes.clone()).await
        {
            warn!("announcing the modes: {err}");
        }
        self.announced = Some(modes);
    }

    /// At start and when msi-ec binds: everything, as if just booted.
    pub async fn apply_all(&mut self) {
        let service = self.service().await;
        let mut service = service.get_mut().await;
        if let Some(percent) = self.config.charge_limit {
            report(service.set_charge_limit(percent));
        }
        if let Some(on) = self.config.webcam {
            report(service.set_webcam(on));
        }
        if let Some(on) = self.config.usb_share {
            report(service.set_usb_share(on));
        }
        if let Some(side) = &self.config.fn_key {
            report(service.set_fn_key(side));
        }
        report(service.sync_curves());
        drop(service);
        self.applied = None;
        self.power_changed().await;
    }

    /// The adaptive fans and the automatic boost, every couple of seconds.
    pub async fn tick(&self) {
        report(self.service().await.get_mut().await.tick());
    }

    /// Hands the fans back to the EC's own stepping before the daemon exits.
    pub async fn stop(&self) {
        self.service().await.get_mut().await.stop();
    }

    pub async fn sleeping(&mut self) {
        self.before_sleep = Some(self.service().await.get().await.snapshot());
    }

    /// Through S3 the EC resets: back to how it was, then the profile if
    /// the laptop was plugged or unplugged while it slept.
    pub async fn woke(&mut self) {
        if let Some(snapshot) = self.before_sleep.take() {
            self.service().await.get_mut().await.restore(&snapshot);
        }
        self.power_changed().await;
    }

    /// Applies the profile only when the source differs from the last one,
    /// so a mode picked over D-Bus lasts until the next plug or unplug.
    pub async fn power_changed(&mut self) {
        let source = match power_supply::source(&self.sysfs) {
            Ok(Some(source)) => source,
            Ok(None) => {
                info!("no AC adapter registered yet, waiting for it");
                return;
            }
            Err(err) => {
                warn!("power supplies: {err}");
                return;
            }
        };
        if self.applied == Some(source) {
            return;
        }
        self.applied = Some(source);

        let Some(profile) = self.config.profiles.get(source) else {
            info!("on {source}, no profile for it");
            return;
        };
        match &profile.scenario {
            Some(scenario) => info!("on {source}, applying its profile ({scenario})"),
            None => info!("on {source}, applying its profile"),
        }
        report(self.service().await.get_mut().await.apply_profile(profile));
    }

    /// Goes through the object server, so it waits for D-Bus calls in
    /// flight and the other way round.
    async fn service(&self) -> zbus::object_server::InterfaceRef<Service> {
        self.connection
            .object_server()
            .interface(OBJECT_PATH)
            .await
            .expect("the service is served from the start")
    }
}

/// Setters already log what they change; a failure gets its own line.
fn report(result: fdo::Result<()>) {
    if let Err(err) = result {
        warn!("{err}");
    }
}

//! Applies the config: the charge limit, and a profile per power source.

use std::path::PathBuf;

use ryukaze_core::{
    OBJECT_PATH,
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
}

impl Profiles {
    pub fn new(config: Config, sysfs: PathBuf, connection: Connection) -> Self {
        Self {
            config,
            sysfs,
            connection,
            applied: None,
            before_sleep: None,
        }
    }

    /// At start and when msi-ec binds: everything, as if just booted.
    pub async fn apply_all(&mut self) {
        if let Some(percent) = self.config.charge_limit {
            let result = self
                .service()
                .await
                .get_mut()
                .await
                .set_charge_limit(percent);
            report(result);
        }
        report(self.service().await.get().await.sync_curves());
        self.applied = None;
        self.power_changed().await;
    }

    /// The adaptive fans' tick.
    pub async fn adapt(&self) {
        report(self.service().await.get_mut().await.adapt());
    }

    /// Hands the fans back to the EC's own stepping before the daemon exits.
    pub async fn stop(&self) {
        self.service().await.get_mut().await.stop_adapting();
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
        info!("on {source}, applying its profile");
        let service = self.service().await;
        let mut service = service.get_mut().await;
        let shift_mode = |service: &mut Service| {
            if let Some(mode) = &profile.shift_mode {
                report(service.set_shift_mode(mode));
            }
        };
        let fan_mode = |service: &mut Service| {
            if let Some(mode) = &profile.fan_mode {
                report(service.set_fan_mode(mode));
            }
        };
        let super_battery = |service: &mut Service| {
            if let Some(on) = profile.super_battery {
                report(service.set_super_battery(on));
            }
        };
        // Leave super battery before raising the shift mode, and lower the
        // shift mode before entering super battery.
        match source {
            Source::Ac => {
                super_battery(&mut service);
                fan_mode(&mut service);
                shift_mode(&mut service);
            }
            Source::Battery => {
                shift_mode(&mut service);
                fan_mode(&mut service);
                super_battery(&mut service);
            }
        }
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

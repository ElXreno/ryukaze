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
    power_profiles::{self, Link},
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
    /// power-profiles-daemon, kept in step with the shift mode.
    link: Option<Link>,
}

impl Profiles {
    pub fn new(config: Config, sysfs: PathBuf, connection: Connection, link: Option<Link>) -> Self {
        Self {
            config,
            sysfs,
            connection,
            applied: None,
            before_sleep: None,
            announced: None,
            link,
        }
    }

    /// Announces the modes when they differ from the last look: after a
    /// setter, and every tick for what the EC changes by itself. A new
    /// scenario takes its colour to the keyboard, and a new shift mode goes
    /// to power-profiles-daemon, the first one too.
    pub async fn look(&mut self) {
        let service = self.service().await;
        let modes = service.get().await.modes();
        let before = self.announced.replace(modes.clone());
        if before.as_ref().is_some_and(|before| *before != modes)
            && let Err(err) = Service::modes_changed(service.signal_emitter(), modes.clone()).await
        {
            warn!("announcing the modes: {err}");
        }
        // The first look too: the profile applied at start may have
        // changed the scenario since the colours went on.
        if before
            .as_ref()
            .is_none_or(|before| before.scenario != modes.scenario)
        {
            report(
                service
                    .get_mut()
                    .await
                    .keyboard_for(modes.scenario.as_deref()),
            );
        }
        report(service.get().await.refresh_power());
        let (sync, resync) = {
            let mut service = service.get_mut().await;
            (
                service.power_profiles_sync(),
                service.take_power_profiles_resync(),
            )
        };
        if let (Some(link), Some(shift_mode)) = (&mut self.link, &modes.shift_mode)
            && sync
            && (resync || before.is_none_or(|before| before.shift_mode != modes.shift_mode))
        {
            link.push(shift_mode).await;
        }
    }

    /// power-profiles-daemon changed its profile, or started again.
    pub async fn power_profile(&mut self, event: power_profiles::Event) {
        let service = self.service().await;
        let Some(link) = &mut self.link else {
            return;
        };
        let sync = service.get().await.power_profiles_sync();
        match event {
            power_profiles::Event::Profile(profile) => {
                if !link.heard(&profile) || !sync {
                    return;
                }
                let followed = service.get_mut().await.follow_power_profile(&profile);
                match followed {
                    Ok(Some(applied)) => info!("power-profiles-daemon: {profile}, so {applied}"),
                    Ok(None) => {}
                    Err(err) => warn!("power-profiles-daemon: {profile}: {err}"),
                }
            }
            // It starts from the profile it saved: tell it this one.
            power_profiles::Event::Started => {
                link.restarted().await;
                if let Some(shift_mode) = self
                    .announced
                    .as_ref()
                    .and_then(|modes| modes.shift_mode.as_deref())
                    && sync
                {
                    link.push(shift_mode).await;
                }
            }
        }
    }

    /// At start and when msi-ec binds: everything, as if just booted, the
    /// config's settings and those changed in the window and kept.
    pub async fn apply_all(&mut self) {
        let service = self.service().await;
        let mut service = service.get_mut().await;
        let (config, kept) = (&self.config, service.changes().clone());
        service.power_profiles_found(self.link.is_some());
        if let Some(percent) = config.charge_limit.or(kept.charge_limit) {
            report(service.put_charge_limit(percent));
        }
        if let Some(on) = config.webcam.or(kept.webcam) {
            report(service.put_webcam(on));
        }
        if let Some(on) = config.usb_share.or(kept.usb_share) {
            report(service.put_usb_share(on));
        }
        if let Some(side) = config.fn_key.as_ref().or(kept.fn_key.as_ref()) {
            report(service.put_fn_key(side));
        }
        let keyboard = &config.keyboard;
        let scenario = service.modes().scenario;
        report(
            service.configure_keyboard(
                keyboard.all_profiles(),
                keyboard.profile.clone(),
                keyboard
                    .profiles
                    .first()
                    .map(|profile| profile.name.clone()),
                keyboard.scenario_profiles(),
                scenario.as_deref(),
            ),
        );
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
        self.keyboard_back().await;
        self.power_changed().await;
    }

    /// NumLock went on or off.
    pub async fn numlock(&self, on: bool) {
        report(self.service().await.get_mut().await.numlock(on));
    }

    /// The keyboard's colour again, where the controller may have lost it.
    pub async fn keyboard_back(&self) {
        report(self.service().await.get().await.keyboard_back());
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

        let service = self.service().await;
        let mut service = service.get_mut().await;
        let Some(profile) = service.source_profile(source) else {
            info!("on {source}, no profile for it");
            return;
        };
        match &profile.scenario {
            Some(scenario) => info!("on {source}, applying its profile ({scenario})"),
            None => info!("on {source}, applying its profile"),
        }
        report(service.apply_profile(&profile));
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

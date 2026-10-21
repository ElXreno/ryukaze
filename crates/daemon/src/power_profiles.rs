//! power-profiles-daemon and the shift mode in step, both ways: eco is
//! power-saver, comfort balanced, sport and turbo performance. A shift mode
//! changed here, by a scenario, a power profile or the EC itself, sets
//! ppd's profile; a profile picked there, from a desktop's power menu,
//! applies the first scenario with a shift mode for it, or the mode alone.

use std::collections::VecDeque;

use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};
use zbus::{Connection, fdo, names::BusName, proxy::CacheProperties, zvariant::Value};

const SERVICE: &str = "org.freedesktop.UPower.PowerProfiles";
const PATH: &str = "/org/freedesktop/UPower/PowerProfiles";

#[zbus::proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles",
    gen_blocking = false
)]
trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
}

/// ppd's profile for a shift mode.
pub fn profile_for(shift_mode: &str) -> Option<&'static str> {
    match shift_mode {
        "eco" => Some("power-saver"),
        "comfort" => Some("balanced"),
        "sport" | "turbo" | "apex" => Some("performance"),
        _ => None,
    }
}

/// The shift modes for ppd's profile, the likelier first.
pub fn shift_modes_for(profile: &str) -> &'static [&'static str] {
    match profile {
        "power-saver" => &["eco"],
        "balanced" => &["comfort"],
        "performance" => &["turbo", "sport"],
        _ => &[],
    }
}

#[derive(Debug)]
pub enum Event {
    /// ppd's profile is this now.
    Profile(String),
    /// ppd started, or started again.
    Started,
}

/// ppd as the daemon sees it.
pub struct Link {
    proxy: PowerProfilesProxy<'static>,
    echoes: Echoes,
}

/// ppd runs, or the bus starts it when called.
pub async fn installed(connection: &Connection) -> bool {
    let Ok(bus) = fdo::DBusProxy::new(connection).await else {
        return false;
    };
    let Ok(name) = BusName::try_from(SERVICE) else {
        return false;
    };
    bus.name_has_owner(name.clone()).await.unwrap_or(false)
        || bus
            .list_activatable_names()
            .await
            .is_ok_and(|names| names.iter().any(|known| known.as_str() == SERVICE))
}

impl Link {
    /// Watches ppd on the daemon's bus, so that `--session` leaves the
    /// system's alone, and reads its profile. One that isn't there yet is
    /// waited for.
    pub async fn new(
        connection: &Connection,
        events: UnboundedSender<Event>,
    ) -> zbus::Result<Self> {
        let properties = fdo::PropertiesProxy::builder(connection)
            .destination(SERVICE)?
            .path(PATH)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let mut changes = properties.receive_properties_changed().await?;
        let bus = fdo::DBusProxy::new(connection).await?;
        let mut owners = bus
            .receive_name_owner_changed_with_args(&[(0, SERVICE)])
            .await?;
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    Some(change) = changes.next() => {
                        let Ok(args) = change.args() else { continue };
                        if args.interface_name().as_str() != SERVICE {
                            continue;
                        }
                        match args.changed_properties().get("ActiveProfile") {
                            Some(Value::Str(profile)) => Event::Profile(profile.to_string()),
                            _ => continue,
                        }
                    }
                    Some(owner) = owners.next() => {
                        let Ok(args) = owner.args() else { continue };
                        if args.new_owner().is_none() {
                            continue;
                        }
                        Event::Started
                    }
                    else => return,
                };
                if events.send(event).is_err() {
                    return;
                }
            }
        });

        let proxy = PowerProfilesProxy::builder(connection)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let mut echoes = Echoes::default();
        match proxy.active_profile().await {
            Ok(profile) => {
                info!("in step with power-profiles-daemon, at {profile}");
                echoes.known = Some(profile);
            }
            Err(err) if absent(&err) => info!("no power-profiles-daemon yet"),
            Err(err) => warn!("power-profiles-daemon: {err}"),
        }
        Ok(Self { proxy, echoes })
    }

    /// The shift mode is `shift_mode` now: ppd's profile follows.
    pub async fn push(&mut self, shift_mode: &str) {
        let Some(profile) = profile_for(shift_mode) else {
            return;
        };
        if !self.echoes.send(profile) {
            return;
        }
        match self.proxy.set_active_profile(profile).await {
            Ok(()) => info!("power-profiles-daemon: {profile}, for {shift_mode}"),
            Err(err) => {
                self.echoes.lost(profile);
                if !absent(&err) {
                    warn!("power-profiles-daemon: setting {profile}: {err}");
                }
            }
        }
    }

    /// ppd says it's at `profile`: whether that came from elsewhere, to be
    /// followed, rather than from here.
    pub fn heard(&mut self, profile: &str) -> bool {
        self.echoes.heard(profile)
    }

    /// ppd started, or started again from the profile it saved: what it's
    /// at now is no news.
    pub async fn restarted(&mut self) {
        self.echoes = Echoes {
            known: self.proxy.active_profile().await.ok(),
            ..Echoes::default()
        };
    }
}

/// ppd's profile as last heard, and those sent to it whose change hasn't
/// come back yet, in order. ppd takes them one by one and says so for each,
/// so a change other than the next one sent came from elsewhere. Fn+F7
/// pressed twice quickly sends two before the first comes back.
#[derive(Debug, Default)]
struct Echoes {
    known: Option<String>,
    sent: VecDeque<String>,
}

impl Echoes {
    /// Notes `profile` as sent, unless ppd is at it or about to be.
    fn send(&mut self, profile: &str) -> bool {
        if self
            .sent
            .back()
            .or(self.known.as_ref())
            .is_some_and(|last| last == profile)
        {
            return false;
        }
        self.sent.push_back(profile.to_owned());
        true
    }

    /// ppd turned `profile` down: no change is coming for it.
    fn lost(&mut self, profile: &str) {
        if self.sent.back().is_some_and(|last| last == profile) {
            self.sent.pop_back();
        }
    }

    fn heard(&mut self, profile: &str) -> bool {
        // Said again: ppd announces the profile it starts at, even to the
        // one that woke it by reading it.
        if self.known.as_deref() == Some(profile) {
            return false;
        }
        self.known = Some(profile.to_owned());
        if self.sent.front().is_some_and(|next| next == profile) {
            self.sent.pop_front();
            return false;
        }
        self.sent.clear();
        true
    }
}

/// ppd isn't on the bus and can't be started.
fn absent(err: &zbus::Error) -> bool {
    let name = match err {
        zbus::Error::MethodError(name, ..) => name.as_str(),
        zbus::Error::FDO(err) => match **err {
            fdo::Error::ServiceUnknown(_) | fdo::Error::NameHasNoOwner(_) => return true,
            _ => return false,
        },
        _ => return false,
    };
    matches!(
        name,
        "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_ways_agree() {
        for profile in ["power-saver", "balanced", "performance"] {
            for mode in shift_modes_for(profile) {
                assert_eq!(profile_for(mode), Some(profile));
            }
        }
        assert_eq!(profile_for("unknown (5)"), None);
        assert!(shift_modes_for("eco").is_empty());
    }

    #[test]
    fn its_own_changes_come_back_as_echoes() {
        let mut echoes = Echoes {
            known: Some("performance".into()),
            ..Echoes::default()
        };
        // Already there.
        assert!(!echoes.send("performance"));
        // Fn+F7 twice quickly: both sent before either comes back.
        assert!(echoes.send("power-saver"));
        assert!(echoes.send("balanced"));
        assert!(!echoes.heard("power-saver"));
        assert!(!echoes.heard("balanced"));
        // Then a pick in the desktop's menu.
        assert!(echoes.heard("performance"));
        assert!(!echoes.send("performance"));
        // Said again, as ppd does when it starts.
        assert!(!echoes.heard("performance"));
    }

    #[test]
    fn a_pick_elsewhere_in_between_wins() {
        let mut echoes = Echoes {
            known: Some("balanced".into()),
            ..Echoes::default()
        };
        assert!(echoes.send("power-saver"));
        // ppd took a pick from elsewhere before this one.
        assert!(echoes.heard("performance"));
        // Then this one, now news too: ppd ends up there, and so the EC.
        assert!(echoes.heard("power-saver"));
        // One turned down isn't waited for.
        assert!(echoes.send("balanced"));
        echoes.lost("balanced");
        assert!(echoes.heard("balanced"));
    }
}

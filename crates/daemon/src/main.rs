//! ryukazed: the only process that touches the EC. Serves
//! io.github.kontonkara.ryukaze on the system bus, or on the session bus
//! with --session for development, and applies /etc/ryukaze/config.toml.

mod adaptive;
mod boost;
mod config;
mod numlock;
mod power_profiles;
mod profiles;
mod service;
mod sleep;
mod uevent;

use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use ryukaze_core::{BUS_NAME, OBJECT_PATH, msi_ec::MsiEc};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::mpsc,
    time::MissedTickBehavior,
};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::{
    boost::AutoBoost,
    config::Config,
    profiles::Profiles,
    service::{Fans, Service},
};

/// How often the adaptive fans look at the temperatures, and the daemon at
/// the modes, which the EC may change by itself (Fn+F8).
const ADAPT_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

const USAGE: &str = "usage: ryukazed [--session] [--sysfs <dir>] [--config <file>]";

struct Args {
    session: bool,
    sysfs: PathBuf,
    config: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        session: false,
        sysfs: PathBuf::from("/sys"),
        config: PathBuf::from("/etc/ryukaze/config.toml"),
    };
    let mut argv = std::env::args_os().skip(1);
    while let Some(arg) = argv.next() {
        match arg.to_str() {
            Some("--session") => args.session = true,
            Some("--sysfs") => args.sysfs = argv.next().context(USAGE)?.into(),
            Some("--config") => args.config = argv.next().context(USAGE)?.into(),
            _ => bail!("unknown argument {arg:?}\n{USAGE}"),
        }
    }
    Ok(args)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // The journal adds its own timestamps.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .without_time()
        .init();

    let args = parse_args()?;
    let mut config = Config::load(&args.config)?;
    let ec = MsiEc::new(&args.sysfs);
    let firmware = ec.firmware().ok();
    match &firmware {
        Some(firmware) => info!("msi-ec: EC firmware {firmware}"),
        // Keep serving: the status then just has nothing in it.
        None => warn!("msi-ec is not bound (not loaded, or it rejected this EC firmware)"),
    }
    let fans = Fans::open(
        &args.sysfs,
        firmware.as_deref(),
        std::mem::take(&mut config.fans),
    );
    let boost = AutoBoost::new(config.cooler_boost);
    let scenarios = std::mem::take(&mut config.scenarios);

    let builder = if args.session {
        zbus::connection::Builder::session()?
    } else {
        zbus::connection::Builder::system()?
    };
    // Before the first colours go on, so the numpad starts as NumLock has it.
    let (numlock_events, mut numlock) = mpsc::unbounded_channel();
    let numlock_now = if config.keyboard.numpad_follows_numlock {
        numlock::watch(&args.sysfs, numlock_events)
            .inspect_err(|err| warn!("the numpad doesn't follow NumLock: {err}"))
            .ok()
    } else {
        None
    };
    let mut service = Service::new(ec, fans, boost, scenarios);
    if let Some(on) = numlock_now
        && let Err(err) = service.numlock(on)
    {
        warn!("{err}");
    }
    let poke = service.poke();
    let connection = builder
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()
        .await
        .with_context(|| format!("cannot take {BUS_NAME} on the bus"))?;
    info!("serving {BUS_NAME}");

    // Listen before the first apply, so nothing in between is missed.
    let (kernel_events, mut kernel) = mpsc::unbounded_channel();
    if let Err(err) = uevent::listen(kernel_events) {
        warn!("no kernel uevents ({err}): profiles apply at start only");
    }
    let (sleep_events, mut sleep) = mpsc::unbounded_channel();
    if let Err(err) = sleep::watch(sleep_events).await {
        warn!("no word from logind ({err}): nothing is put back after sleep");
    }
    let (ppd_events, mut ppd) = mpsc::unbounded_channel();
    let link = if config.power_profiles_daemon.sync {
        power_profiles::Link::new(&connection, ppd_events)
            .await
            .inspect_err(|err| warn!("not in step with power-profiles-daemon: {err}"))
            .ok()
    } else {
        None
    };
    let mut profiles = Profiles::new(config, args.sysfs, connection, link);
    profiles.apply_all().await;

    // zbus answers calls on its own tasks.
    let mut tick = tokio::time::interval(ADAPT_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    loop {
        tokio::select! {
            Some(event) = kernel.recv() => match event {
                uevent::Event::Mains => profiles.power_changed().await,
                uevent::Event::MsiEcBound => {
                    info!("msi-ec bound again");
                    profiles.apply_all().await;
                }
                uevent::Event::Hidraw => profiles.keyboard_back().await,
            },
            Some(event) = sleep.recv() => match event {
                sleep::Event::Sleeping(lock) => {
                    profiles.sleeping().await;
                    drop(lock);
                }
                sleep::Event::Woke => profiles.woke().await,
            },
            Some(event) = ppd.recv() => profiles.power_profile(event).await,
            Some(on) = numlock.recv() => profiles.numlock(on).await,
            _ = poke.notified() => profiles.look().await,
            _ = tick.tick() => {
                profiles.tick().await;
                profiles.look().await;
            }
            _ = terminate.recv() => break,
            _ = interrupt.recv() => break,
        }
    }
    profiles.stop().await;
    info!("stopped");
    Ok(())
}

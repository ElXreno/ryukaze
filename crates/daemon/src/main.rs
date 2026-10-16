//! ryukazed: the only process that touches the EC. Serves
//! io.github.kontonkara.ryukaze on the system bus, or on the session bus
//! with --session for development, and applies /etc/ryukaze/config.toml.

mod config;
mod profiles;
mod service;
mod uevent;

use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use ryukaze_core::{BUS_NAME, OBJECT_PATH, msi_ec::MsiEc};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::{config::Config, profiles::Profiles, uevent::Event};

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
    let config = Config::load(&args.config)?;
    let ec = MsiEc::new(&args.sysfs);
    match ec.firmware() {
        Ok(firmware) => info!("msi-ec: EC firmware {firmware}"),
        // Keep serving: the status then just has nothing in it.
        Err(_) => warn!("msi-ec is not bound (not loaded, or it rejected this EC firmware)"),
    }

    let builder = if args.session {
        zbus::connection::Builder::session()?
    } else {
        zbus::connection::Builder::system()?
    };
    let connection = builder
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service::Service::new(ec))?
        .build()
        .await
        .with_context(|| format!("cannot take {BUS_NAME} on the bus"))?;
    info!("serving {BUS_NAME}");

    // Listen before the first apply, so a plug in between isn't missed.
    let (events, mut received) = mpsc::unbounded_channel();
    if let Err(err) = uevent::listen(events) {
        warn!("no kernel uevents ({err}): profiles apply at start only");
    }
    let mut profiles = Profiles::new(config, args.sysfs, connection);
    profiles.apply_all().await;

    // zbus answers calls on its own tasks; SIGTERM ends the process.
    while let Some(event) = received.recv().await {
        match event {
            Event::Mains => profiles.power_changed().await,
            Event::MsiEcBound => {
                info!("msi-ec bound again");
                profiles.apply_all().await;
            }
        }
    }
    std::future::pending().await
}

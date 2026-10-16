//! Shared by ryukazed and its clients: the daemon's D-Bus names, the status
//! it reports, the sysfs it reads (msi-ec, power supplies) and the EC RAM
//! behind ec_sys for the fan curves.

pub mod fans;
pub mod msi_ec;
pub mod power_supply;
pub mod raw_ec;
mod status;

pub use status::Status;

/// ryukazed's well-known name on the system bus.
pub const BUS_NAME: &str = "io.github.kontonkara.ryukaze";

/// Where ryukazed serves its interface.
pub const OBJECT_PATH: &str = "/io/github/kontonkara/ryukaze";

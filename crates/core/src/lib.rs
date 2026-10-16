//! Shared by ryukazed and its clients: the daemon's D-Bus names, the status
//! it reports, the sysfs it reads (msi-ec, power supplies) and the EC RAM
//! behind ec_sys for what msi-ec lacks (the fan curves, USB Power Share, the
//! GPU MUX).

pub mod fans;
pub mod gpu_mux;
pub mod msi_ec;
pub mod power_supply;
pub mod raw_ec;
mod status;
pub mod usb_share;

pub use status::Status;

/// ryukazed's well-known name on the system bus.
pub const BUS_NAME: &str = "io.github.kontonkara.ryukaze";

/// Where ryukazed serves its interface.
pub const OBJECT_PATH: &str = "/io/github/kontonkara/ryukaze";

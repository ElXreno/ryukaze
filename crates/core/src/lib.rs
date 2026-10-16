//! Shared by ryukazed and its clients: the daemon's D-Bus names, the status
//! it reports, and the msi-ec driver's sysfs.

pub mod msi_ec;
mod status;

pub use status::Status;

/// ryukazed's well-known name on the system bus.
pub const BUS_NAME: &str = "io.github.kontonkara.ryukaze";

/// Where ryukazed serves its interface.
pub const OBJECT_PATH: &str = "/io/github/kontonkara/ryukaze";

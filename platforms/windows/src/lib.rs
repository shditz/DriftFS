#[cfg(windows)]
pub mod context;
#[cfg(windows)]
pub mod error;
#[cfg(windows)]
pub mod fs;
#[cfg(windows)]
pub mod mount;

#[cfg(windows)]
pub use error::PlatformError;
#[cfg(windows)]
pub use fs::WindowsVfs;
#[cfg(windows)]
pub use mount::{find_available_drive_letter, DriftFsMount, MountConfig};

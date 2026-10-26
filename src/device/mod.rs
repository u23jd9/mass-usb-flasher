#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
pub mod linux;
#[cfg(not(windows))]
pub use linux::*;

pub mod watcher;

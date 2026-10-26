#[cfg(windows)]
pub mod windows;

pub mod watcher;

#[cfg(windows)]
pub use windows::*;
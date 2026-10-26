#[cfg(windows)]
pub mod windows;

pub mod writer;

#[cfg(windows)]
pub use windows::*;
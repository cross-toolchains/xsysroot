//! `xios` — Utility for extracting and splatting the iOS SDK sysroot for cross-compilation.

pub mod splat;
pub mod xip;

/// Returns the current `xios` crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

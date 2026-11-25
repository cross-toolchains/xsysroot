//! `xohos` — Utility for downloading and splatting the OpenHarmony NDK sysroot for cross-compilation.

pub mod sdk;
pub mod splat;

/// Returns the current `xohos` crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

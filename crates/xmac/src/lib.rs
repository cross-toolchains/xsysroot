//! `xmac` — Utility for downloading and splatting the macOS SDK for cross-compilation.

pub mod catalog;
pub mod pkg;
pub mod splat;

/// Returns the current `xmac` crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

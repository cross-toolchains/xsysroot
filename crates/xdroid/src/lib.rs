//! `xdroid` — Utility for downloading and splatting the Android NDK sysroot for cross-compilation.

pub mod ndk;
pub mod remote_zip;
pub mod splat;

/// Returns the current `xdroid` crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

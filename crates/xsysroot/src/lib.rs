//! `xsysroot` — Unified cross-platform sysroot manager and facade library.
//!
//! Re-exports all platform sysroot crates (`xdroid`, `xmac`, `xios`, `xlinux`, `xohos`, `xwindows`)
//! alongside shared utilities in `xsysroot-core`.

pub use xdroid;
pub use xios;
pub use xlinux;
pub use xmac;
pub use xohos;
pub use xsysroot_core as core;
pub use xwindows;

/// Returns the current `xsysroot` crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

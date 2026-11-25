//! `xlinux` — Utility for downloading and splatting minimal Linux glibc/musl sysroots for cross-compilation.

pub mod archive;
pub mod repo;
pub mod splat;
pub mod target;

/// Returns the current `xlinux` crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

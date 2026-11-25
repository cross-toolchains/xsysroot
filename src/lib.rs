//! `xsysroot` — Unified cross-platform sysroot manager.

/// Returns the current `xsysroot` crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_package_version() {
        assert_eq!(version(), "0.1.0");
    }
}

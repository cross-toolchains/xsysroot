//! Linux target triple and glibc/musl baseline definitions.

pub const DEFAULT_TARGET: &str = "x86_64-unknown-linux-gnu";
pub const DEFAULT_GLIBC: &str = "2.31";
pub const DEFAULT_ALPINE_BRANCH: &str = "v3.21";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Libc {
    Gnu,
    Musl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxTarget {
    pub triple: String,
    pub arch: Arch,
    pub libc: Libc,
}

#[derive(Debug, Clone, Copy)]
pub struct DebianSuite {
    pub glibc: &'static str,
    pub codename: &'static str,
    pub mirror: &'static str,
    pub gcc_major: &'static str,
}

pub const DEBIAN_SUITES: &[DebianSuite] = &[
    DebianSuite {
        glibc: "2.36",
        codename: "bookworm",
        mirror: "https://deb.debian.org/debian",
        gcc_major: "12",
    },
    DebianSuite {
        glibc: "2.31",
        codename: "bullseye",
        mirror: "https://deb.debian.org/debian",
        gcc_major: "10",
    },
    DebianSuite {
        glibc: "2.28",
        codename: "buster",
        mirror: "https://archive.debian.org/debian",
        gcc_major: "8",
    },
];

pub const KNOWN_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
];

impl LinuxTarget {
    pub fn parse(raw: &str) -> Result<(Self, Option<String>), String> {
        let (triple_part, glibc_suffix) = match raw.split_once(".2.") {
            Some((base, minor)) => (base, Some(format!("2.{minor}"))),
            None => (raw, None),
        };
        let arch = if triple_part.starts_with("x86_64-") {
            Arch::X86_64
        } else if triple_part.starts_with("aarch64-") {
            Arch::Aarch64
        } else {
            return Err(format!(
                "unsupported Linux architecture in `{raw}` (supported: x86_64, aarch64)"
            ));
        };
        let libc = if triple_part.ends_with("-musl") {
            Libc::Musl
        } else if triple_part.ends_with("-gnu") {
            Libc::Gnu
        } else {
            return Err(format!(
                "unsupported Linux libc in `{raw}` (supported: -gnu, -musl)"
            ));
        };
        Ok((
            Self {
                triple: triple_part.to_string(),
                arch,
                libc,
            },
            glibc_suffix,
        ))
    }

    pub fn debian_arch(&self) -> &'static str {
        match self.arch {
            Arch::X86_64 => "amd64",
            Arch::Aarch64 => "arm64",
        }
    }

    pub fn alpine_arch(&self) -> &'static str {
        match self.arch {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        }
    }

    pub fn multiarch_dir(&self) -> &'static str {
        match self.arch {
            Arch::X86_64 => "x86_64-linux-gnu",
            Arch::Aarch64 => "aarch64-linux-gnu",
        }
    }
}

pub fn resolve_debian_suite(glibc: &str) -> Result<DebianSuite, String> {
    DEBIAN_SUITES
        .iter()
        .copied()
        .find(|s| s.glibc == glibc)
        .ok_or_else(|| {
            format!(
                "unsupported glibc version `{glibc}` (supported: {})",
                DEBIAN_SUITES
                    .iter()
                    .map(|s| s.glibc)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_targets_with_optional_glibc_suffix() {
        let (t, ver) = LinuxTarget::parse("x86_64-unknown-linux-gnu.2.31").unwrap();
        assert_eq!(t.triple, "x86_64-unknown-linux-gnu");
        assert_eq!(t.arch, Arch::X86_64);
        assert_eq!(t.libc, Libc::Gnu);
        assert_eq!(ver.as_deref(), Some("2.31"));

        let (m, ver2) = LinuxTarget::parse("aarch64-unknown-linux-musl").unwrap();
        assert_eq!(m.arch, Arch::Aarch64);
        assert_eq!(m.libc, Libc::Musl);
        assert_eq!(ver2, None);
    }
}

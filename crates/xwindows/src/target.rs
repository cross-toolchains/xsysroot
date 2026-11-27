//! Windows target triple definitions covering both MSVC (`*-pc-windows-msvc`)
//! and MinGW / UCRT (`*-pc-windows-gnullvm` / `*-pc-windows-gnu`).

use std::fmt;

use crate::msvc::Arch as MsvcArch;

/// Default Windows target triple.
pub const DEFAULT_TARGET: &str = "x86_64-pc-windows-msvc";

/// Default `mstorsjo/llvm-mingw` release tag used for MinGW/UCRT sysroots.
pub const DEFAULT_MINGW_RELEASE: &str = "20261117";

/// Supported Windows target triples shown by `xwindows list`.
pub const KNOWN_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "i686-pc-windows-msvc",
    "x86_64-pc-windows-gnullvm",
    "aarch64-pc-windows-gnullvm",
    "i686-pc-windows-gnullvm",
    "armv7-pc-windows-gnullvm",
    "x86_64-pc-windows-gnu",
    "aarch64-pc-windows-gnu",
    "i686-pc-windows-gnu",
];

/// Windows target architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowsArch {
    X86_64,
    Aarch64,
    X86,
    Armv7,
}

impl WindowsArch {
    /// Canonical Rust/LLVM architecture name used in target triples.
    pub const fn rust_arch(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::X86 => "i686",
            Self::Armv7 => "armv7",
        }
    }

    /// MinGW-w64 sysroot prefix (`<arch>-w64-mingw32`).
    pub const fn mingw_prefix(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-w64-mingw32",
            Self::Aarch64 => "aarch64-w64-mingw32",
            Self::X86 => "i686-w64-mingw32",
            Self::Armv7 => "armv7-w64-mingw32",
        }
    }

    /// `xwin`-compatible architecture enum for MSVC payloads.
    pub const fn to_msvc_arch(self) -> MsvcArch {
        match self {
            Self::X86_64 => MsvcArch::X86_64,
            Self::Aarch64 => MsvcArch::Aarch64,
            Self::X86 => MsvcArch::X86,
            Self::Armv7 => MsvcArch::Aarch,
        }
    }

    /// Directory name used by `xwin` under `crt/lib/` and `sdk/lib/*/`.
    pub const fn msvc_lib_dir(self, preserve_ms_arch: bool) -> &'static str {
        if preserve_ms_arch {
            match self {
                Self::X86_64 => "x64",
                Self::Aarch64 => "arm64",
                Self::X86 => "x86",
                Self::Armv7 => "arm",
            }
        } else {
            match self {
                Self::X86_64 => "x86_64",
                Self::Aarch64 => "aarch64",
                Self::X86 => "x86",
                Self::Armv7 => "aarch",
            }
        }
    }

    /// LLVM `clang` compiler-rt architecture suffix (`libclang_rt.builtins-<arch>.a`).
    pub const fn compiler_rt_arch(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::X86 => "i386",
            Self::Armv7 => "arm",
        }
    }
}

/// Windows ABI / toolchain family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowsAbi {
    /// Microsoft Visual C++ CRT + Windows SDK (`*-pc-windows-msvc`).
    Msvc,
    /// LLVM-MinGW UCRT/MSVCRT (`*-pc-windows-gnullvm`).
    GnuLlvm,
    /// GNU-compatible MinGW (`*-pc-windows-gnu`, backed by LLVM-MinGW sysroot).
    Gnu,
}

impl WindowsAbi {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Msvc => "msvc",
            Self::GnuLlvm => "gnullvm",
            Self::Gnu => "gnu",
        }
    }

    pub const fn is_mingw(self) -> bool {
        matches!(self, Self::GnuLlvm | Self::Gnu)
    }
}

/// MinGW C runtime variant (`ucrt` or `msvcrt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MingwCrt {
    #[default]
    Ucrt,
    Msvcrt,
}

impl MingwCrt {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ucrt => "ucrt",
            Self::Msvcrt => "msvcrt",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ucrt" => Ok(Self::Ucrt),
            "msvcrt" => Ok(Self::Msvcrt),
            other => Err(format!(
                "unsupported MinGW CRT `{other}` (expected `ucrt` or `msvcrt`)"
            )),
        }
    }
}

/// Parsed Windows cross-compilation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowsTarget {
    pub arch: WindowsArch,
    pub abi: WindowsAbi,
}

impl WindowsTarget {
    /// Parses a Windows target triple such as `x86_64-pc-windows-msvc`,
    /// `aarch64-pc-windows-gnullvm`, or shorthand aliases like `msvc` / `mingw`.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "msvc" | "x86_64-pc-windows-msvc" | "x86_64-windows-msvc" => Ok(Self {
                arch: WindowsArch::X86_64,
                abi: WindowsAbi::Msvc,
            }),
            "aarch64-pc-windows-msvc" | "arm64-pc-windows-msvc" | "aarch64-windows-msvc" => {
                Ok(Self {
                    arch: WindowsArch::Aarch64,
                    abi: WindowsAbi::Msvc,
                })
            }
            "i686-pc-windows-msvc" | "x86-pc-windows-msvc" | "i686-windows-msvc" => Ok(Self {
                arch: WindowsArch::X86,
                abi: WindowsAbi::Msvc,
            }),
            "armv7-pc-windows-msvc" | "thumbv7a-pc-windows-msvc" => Ok(Self {
                arch: WindowsArch::Armv7,
                abi: WindowsAbi::Msvc,
            }),
            "mingw" | "gnullvm" | "x86_64-pc-windows-gnullvm" | "x86_64-w64-mingw32" => Ok(Self {
                arch: WindowsArch::X86_64,
                abi: WindowsAbi::GnuLlvm,
            }),
            "aarch64-pc-windows-gnullvm" | "arm64-pc-windows-gnullvm" | "aarch64-w64-mingw32" => {
                Ok(Self {
                    arch: WindowsArch::Aarch64,
                    abi: WindowsAbi::GnuLlvm,
                })
            }
            "i686-pc-windows-gnullvm" | "i686-w64-mingw32" => Ok(Self {
                arch: WindowsArch::X86,
                abi: WindowsAbi::GnuLlvm,
            }),
            "armv7-pc-windows-gnullvm" | "armv7-w64-mingw32" => Ok(Self {
                arch: WindowsArch::Armv7,
                abi: WindowsAbi::GnuLlvm,
            }),
            "x86_64-pc-windows-gnu" => Ok(Self {
                arch: WindowsArch::X86_64,
                abi: WindowsAbi::Gnu,
            }),
            "aarch64-pc-windows-gnu" => Ok(Self {
                arch: WindowsArch::Aarch64,
                abi: WindowsAbi::Gnu,
            }),
            "i686-pc-windows-gnu" => Ok(Self {
                arch: WindowsArch::X86,
                abi: WindowsAbi::Gnu,
            }),
            "armv7-pc-windows-gnu" => Ok(Self {
                arch: WindowsArch::Armv7,
                abi: WindowsAbi::Gnu,
            }),
            _ => Err(format!(
                "unsupported Windows target `{raw}` (supported: {})",
                KNOWN_TARGETS.join(", ")
            )),
        }
    }

    /// Canonical Rust target triple string (e.g. `x86_64-pc-windows-msvc`).
    pub const fn triple(self) -> &'static str {
        match (self.arch, self.abi) {
            (WindowsArch::X86_64, WindowsAbi::Msvc) => "x86_64-pc-windows-msvc",
            (WindowsArch::Aarch64, WindowsAbi::Msvc) => "aarch64-pc-windows-msvc",
            (WindowsArch::X86, WindowsAbi::Msvc) => "i686-pc-windows-msvc",
            (WindowsArch::Armv7, WindowsAbi::Msvc) => "armv7-pc-windows-msvc",
            (WindowsArch::X86_64, WindowsAbi::GnuLlvm) => "x86_64-pc-windows-gnullvm",
            (WindowsArch::Aarch64, WindowsAbi::GnuLlvm) => "aarch64-pc-windows-gnullvm",
            (WindowsArch::X86, WindowsAbi::GnuLlvm) => "i686-pc-windows-gnullvm",
            (WindowsArch::Armv7, WindowsAbi::GnuLlvm) => "armv7-pc-windows-gnullvm",
            (WindowsArch::X86_64, WindowsAbi::Gnu) => "x86_64-pc-windows-gnu",
            (WindowsArch::Aarch64, WindowsAbi::Gnu) => "aarch64-pc-windows-gnu",
            (WindowsArch::X86, WindowsAbi::Gnu) => "i686-pc-windows-gnu",
            (WindowsArch::Armv7, WindowsAbi::Gnu) => "armv7-pc-windows-gnu",
        }
    }

    /// Clang `--target=` triple string.
    ///
    /// For MinGW targets, Clang expects `<arch>-w64-windows-gnu` (matching the
    /// `llvm-mingw` driver convention and UCRT/MSVCRT headers).
    pub const fn clang_target(self) -> &'static str {
        match (self.arch, self.abi) {
            (WindowsArch::X86_64, WindowsAbi::Msvc) => "x86_64-pc-windows-msvc",
            (WindowsArch::Aarch64, WindowsAbi::Msvc) => "aarch64-pc-windows-msvc",
            (WindowsArch::X86, WindowsAbi::Msvc) => "i686-pc-windows-msvc",
            (WindowsArch::Armv7, WindowsAbi::Msvc) => "armv7-pc-windows-msvc",
            (WindowsArch::X86_64, WindowsAbi::GnuLlvm | WindowsAbi::Gnu) => {
                "x86_64-w64-windows-gnu"
            }
            (WindowsArch::Aarch64, WindowsAbi::GnuLlvm | WindowsAbi::Gnu) => {
                "aarch64-w64-windows-gnu"
            }
            (WindowsArch::X86, WindowsAbi::GnuLlvm | WindowsAbi::Gnu) => "i686-w64-windows-gnu",
            (WindowsArch::Armv7, WindowsAbi::GnuLlvm | WindowsAbi::Gnu) => "armv7-w64-windows-gnu",
        }
    }
}

impl fmt::Display for WindowsTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.triple())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_msvc_and_mingw_targets() {
        let t = WindowsTarget::parse("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(t.arch, WindowsArch::X86_64);
        assert_eq!(t.abi, WindowsAbi::Msvc);
        assert_eq!(t.triple(), "x86_64-pc-windows-msvc");

        let g = WindowsTarget::parse("aarch64-pc-windows-gnullvm").unwrap();
        assert_eq!(g.arch, WindowsArch::Aarch64);
        assert_eq!(g.abi, WindowsAbi::GnuLlvm);
        assert_eq!(g.clang_target(), "aarch64-w64-windows-gnu");
        assert_eq!(g.arch.mingw_prefix(), "aarch64-w64-mingw32");

        let alias = WindowsTarget::parse("mingw").unwrap();
        assert_eq!(alias.triple(), "x86_64-pc-windows-gnullvm");

        assert!(WindowsTarget::parse("unknown-pc-windows-msvc").is_err());
    }
}

<div align="center">

# xsysroot

**Minimal, hermetic target sysroots for cross-compilation — from anywhere.**

Pure-Rust monorepo of single-binary sysroot fetchers for Android, macOS, iOS, Linux (`glibc` & `musl`), and OpenHarmony —
no Docker, no heavy IDEs, and nothing redistributed.

[![crates.io](https://img.shields.io/crates/v/xsysroot.svg)](https://crates.io/crates/xsysroot)
[![license: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license--legal)

</div>

---

## 🧰 Crates

| crate | target | upstream source | typical download |
| :--- | :--- | :--- | :---: |
| [**`xdroid`**](crates/xdroid/README.md) | Android (`aarch64`/`armv7a`/`i686`/`x86_64`) | Google NDK `.zip` (HTTP Range `sysroot/` only) | **~ 47 MiB** |
| [**`xmac`**](crates/xmac/README.md) | macOS (`arm64` / `x86_64`) | Apple public CLT CDN (`xar -> pbzx -> cpio`) | **~ 56 MiB** |
| [**`xios`**](crates/xios/README.md) | iOS / iOS Simulator (`arm64` / `x86_64`) | User-supplied `Xcode.xip` / `Xcode.app` | local |
| [**`xlinux`**](crates/xlinux/README.md) | Linux (`glibc 2.28/2.31/2.36` & `musl`) | Debian `.deb` / Alpine `.apk` | **~ 9–11 MiB** |
| [**`xohos`**](crates/xohos/README.md) | OpenHarmony (`aarch64` / `armv7` / `x86_64`) | OpenHarmony public SDK mirror | streaming |
| [**`xsysroot`**](crates/xsysroot) | Unified CLI & facade library | Dispatches all platforms above | — |

> For Windows MSVC (`*-pc-windows-msvc`), pair with [Jake-Shadle/xwin](https://github.com/Jake-Shadle/xwin).

## 📦 Install

Install the unified `xsysroot` CLI (or any individual platform crate such as `xdroid`, `xmac`, `xios`, `xlinux`, `xohos`):

```sh
# Prebuilt binary via cargo-binstall
cargo binstall xsysroot

# Or build from source (with lockfile)
cargo install xsysroot --locked
cargo install xsysroot
```

Or download a prebuilt binary from [Releases](https://github.com/cross-toolchains/xsysroot/releases):

```sh
curl -fsSL https://github.com/cross-toolchains/xsysroot/releases/latest/download/xsysroot-x86_64-unknown-linux-musl.tar.gz | tar -xz
```

## 🚀 Quick Start

```sh
# List all supported platforms
xsysroot list

# Splat a target sysroot
xsysroot splat android --accept-license
xsysroot splat macos   --accept-license
xsysroot splat linux   --target x86_64-unknown-linux-gnu.2.31
xsysroot splat ohos
xsysroot splat ios     --from /path/to/Xcode.xip
```

## ⚖️ License & Legal

All tools in this repository are dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.

`xsysroot` does **not** host, mirror, or redistribute any third-party SDK or NDK files. Every machine downloads directly from upstream vendors under their respective license terms.

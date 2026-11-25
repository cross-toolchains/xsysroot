<div align="center">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.iconify.design/fa6-brands/apple.svg?color=%23ffffff" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.iconify.design/fa6-brands/apple.svg?color=%23000000" />
    <img src="https://api.iconify.design/fa6-brands/apple.svg?color=%23000000" height="28" align="absmiddle" alt="Apple" />
  </picture>
  xios
</h1>

**Extract and splat minimal iOS SDK sysroots for cross-compilation.**

Pure-Rust CLI that extracts `iPhoneOS.sdk` and `iPhoneSimulator.sdk` from a user-supplied `Xcode.xip` archive or local `Xcode.app` —
no external `xar`/`pbzx`/`cpio` tools and nothing redistributed.

[![crates.io](https://img.shields.io/crates/v/xios.svg)](https://crates.io/crates/xios)
[![license: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license--legal)

<br>

```console
$ xios splat
  iPhoneOS  [ 1s] █████████████████████████████████ 209.09 MiB/209.09 MiB  📦    splatted

$ ~/.cache/xios/splat/bin/aarch64-apple-ios-clang -c hello.c -o hello.o
$ file hello.o
hello.o: Mach-O 64-bit object arm64
```

</div>

---

Unlike `MacOSX.sdk` (which Apple serves unauthenticated via Command Line Tools for [`xmac`](https://crates.io/crates/xmac)), `iPhoneOS.sdk` and `iPhoneSimulator.sdk` are bundled exclusively inside `Xcode.xip` / `Xcode.app`. **xios** parses `Xcode.xip` (`xar -> pbzx -> cpio`) in pure Rust (or reads a local `Xcode.app` on macOS), extracts only the portable iOS SDKs into `~/.cache/xios/splat`, and generates ready-to-use `clang` / `clang++` wrappers for device (`aarch64-apple-ios`) and simulator (`aarch64-apple-ios-sim`, `x86_64-apple-ios`).

## 📦 Install

### From binary

```sh
cargo binstall xios
```

### From tarball

Download a prebuilt archive from [Releases](https://github.com/cross-toolchains/xsysroot/releases):

```sh
curl -fsSL https://github.com/cross-toolchains/xsysroot/releases/latest/download/xios-x86_64-unknown-linux-musl.tar.gz | tar -xz
```

### From source

```sh
cargo install xios --locked
# or
cargo install xios
```

`xios` is a self-contained single binary (pure Rust with `flate2` and `lzma-rs` — no `xar`, `pbzx`, `cpio`, or `xz` binaries required). To cross-compile with the generated wrappers, you only need LLVM `clang` and `lld` (`ld64.lld`) on your `PATH`.

## 🚀 Usage

```sh
# On Linux / Windows / macOS: extract directly from a downloaded Xcode.xip
xios splat --from /path/to/Xcode.xip

# Also extract iPhoneSimulator.sdk + simulator wrappers
xios splat --from /path/to/Xcode.xip --simulator

# On macOS: auto-detect local /Applications/Xcode.app
xios splat --simulator

# Customize minimum iOS deployment target and output directory
xios splat --from /path/to/Xcode.xip --min-ios 15.0 --output /opt/ios-sdk

# Print detected local Xcode Developer path (macOS)
xios detect
```

### Options (`xios splat`)

| flag | description | default |
| :--- | :--- | :--- |
| `--from <PATH>` | Path to `Xcode.xip` archive or `Xcode.app` directory | auto-detect local Xcode (macOS) |
| `--output <DIR>` | Destination directory for extracted SDK(s) and `bin/` wrappers | `~/.cache/xios/splat` |
| `--min-ios <VER>` | Minimum iOS deployment target for generated wrappers | `13.0` |
| `--simulator` | Also extract `iPhoneSimulator.sdk` and simulator wrappers | `false` |
| `--clang <PATH>` | Explicit LLVM `clang` binary to back the generated wrappers | auto-discovered |
| `-v, --verbose` | Print resolved SDK and wrapper paths after splatting | `false` |

### Output layout

```text
~/.cache/xios/splat/
├── iPhoneOS.sdk/                                # Device SDK (headers + .tbd stubs)
├── iPhoneSimulator.sdk/                         # Simulator SDK (when --simulator is set)
└── bin/
    ├── aarch64-apple-ios-clang                  # Device (arm64) wrappers
    ├── aarch64-apple-ios-clang++
    ├── aarch64-apple-ios-sim-clang              # Simulator (arm64) wrappers
    ├── aarch64-apple-ios-sim-clang++
    ├── x86_64-apple-ios-clang                   # Simulator (x86_64) wrappers
    └── x86_64-apple-ios-clang++
```

## 🛠 Cross-compiling

### Cargo / Rust

```sh
rustup target add aarch64-apple-ios

export SDKROOT=~/.cache/xios/splat/iPhoneOS.sdk
export IPHONEOS_DEPLOYMENT_TARGET=13.0
export CARGO_TARGET_AARCH64_APPLE_IOS_LINKER=~/.cache/xios/splat/bin/aarch64-apple-ios-clang
export CC_aarch64_apple_ios=~/.cache/xios/splat/bin/aarch64-apple-ios-clang
export CXX_aarch64_apple_ios=~/.cache/xios/splat/bin/aarch64-apple-ios-clang++

cargo build --target aarch64-apple-ios --release
```

Or via [`xforge`](https://crates.io/crates/xforge):

```sh
xforge build --target aarch64-apple-ios --apple-sdk ~/.cache/xios/splat/iPhoneOS.sdk --release
```

## ⚙️ How it works

```text
Xcode.xip ──► xar (zlib TOC: Content) ──► pbzx (chunked XZ) ──► cpio (ODC)
                                                                   │
                                 filter: .../iPhoneOS*.sdk/**      ▼
                                         .../iPhoneSimulator*.sdk/**
                                                                   ▼
                                                      splat/iPhoneOS.sdk + bin/
```

## 📚 What's in the SDK (and what isn't)

`iPhoneOS.sdk` (and `iPhoneSimulator.sdk` when `--simulator` is passed) contains C/Objective-C/C++ headers, module maps, and `.tbd` (Text-Based Stub) symbol tables for all iOS system libraries and frameworks (`UIKit`, `Foundation`, `CoreGraphics`, `Metal`, `Security`, etc.) — everything LLVM `clang` + `ld64.lld` needs to compile and link native iOS static libraries (`.a`), dynamic frameworks, and executables.

Not included: the iOS Simulator Runtime image (for booting simulators), `xcodebuild`, `codesign`, `actool`, `ibtool`, the Swift compiler, or `.ipa` packaging/signing tools. For cross-platform code signing, see [`xsign`](https://crates.io/crates/xsign) / [`apple-codesign`](https://crates.io/crates/apple-codesign).

## ⚖️ License & Legal

The `xios` _tool_ is dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.

`xios` does **not** host, mirror, or redistribute any Apple files. Users supply their own Xcode archive subject to Apple's Xcode and Apple SDKs License Agreement.

This project is not affiliated with or endorsed by Apple Inc.

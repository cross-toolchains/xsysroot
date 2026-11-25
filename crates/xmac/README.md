<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.iconify.design/fa6-brands/apple.svg?color=%23ffffff" />
  <source media="(prefers-color-scheme: light)" srcset="https://api.iconify.design/fa6-brands/apple.svg?color=%23000000" />
  <img src="https://api.iconify.design/fa6-brands/apple.svg?color=%23000000" height="56" alt="Apple" />
</picture>

<h1>xmac</h1>

**Cross-compile to macOS from anywhere.**

Pure-Rust CLI that downloads and splats minimal `MacOSX.sdk` sysroots straight from Apple's public software update CDN —
no Apple ID, no Mac, no Xcode, no external `xar`/`pbzx`/`cpio` tools, and nothing redistributed.

[![crates.io](https://img.shields.io/crates/v/xmac.svg)](https://crates.io/crates/xmac)
[![license: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license--legal)

<br>

```console
$ xmac splat
Do you accept the license at https://www.apple.com/legal/sla/ (yes | no)?
yes
license accepted!
  Catalog [ 3s] █████████████████████████████████   6.72 MiB/  6.72 MiB  📥  downloaded
  SDK     [ 6s] █████████████████████████████████ 626.58 MiB/626.58 MiB  📦    splatted

$ ~/.cache/xmac/splat/bin/aarch64-apple-darwin-clang -o hello hello.c
$ file hello
hello: Mach-O 64-bit executable arm64
```

_A complete, linkable `MacOSX.sdk` in seconds — downloading only **~56 MiB** instead of a multi-GiB Xcode archive._

</div>

---

[xwin](https://github.com/Jake-Shadle/xwin) made it possible to target Windows from Linux/macOS CI, and [Jarred-Sumner/xmac](https://github.com/Jarred-Sumner/xmac) pioneered pulling `MacOSX.sdk` from Apple's public CDN. **This `xmac` is a self-contained, single-binary Rust implementation**: it fetches Apple's public software update catalog (`swscan.apple.com`), downloads only the standalone **~56 MiB** `CLTools_macOS*_SDK.pkg` sub-package with **no authentication**, unpacks Apple's `xar -> pbzx -> cpio` layers completely in-process, and generates ready-to-use `clang` / `clang++` wrappers for `arm64` and `x86_64`.

| | download | needs Apple ID | needs a Mac |
| :--- | :--- | :---: | :---: |
| Xcode `.xip` | 3 – 8 GiB | ✅ yes | ✅ yes |
| Command Line Tools `.dmg` | ~ 1 GiB | ✅ yes | ✅ yes |
| **xmac** (CLT SDK sub-package) | **~ 56 MiB** | **❌ no** | **❌ no** |

Because every machine downloads the SDK package directly from Apple's public CDN, `xmac` never hosts or redistributes any Apple files. Users acknowledge Apple's Software License Agreement (`--accept-license`) before downloading.

## 📦 Install

### From binary

```sh
cargo binstall xmac
```

### From tarball

Download a prebuilt archive from [Releases](https://github.com/cross-toolchains/xsysroot/releases):

```sh
curl -fsSL https://github.com/cross-toolchains/xsysroot/releases/latest/download/xmac-x86_64-unknown-linux-musl.tar.gz | tar -xz
```

### From source

```sh
cargo install xmac --locked
# or
cargo install xmac
```

`xmac` is a self-contained single binary (pure Rust with `ureq`, `flate2`, and `lzma-rs` — no Bun, `xz`, `xar`, `pbzx`, or `cpio` binaries required). To actually cross-compile with the generated wrappers, you only need LLVM `clang` and `lld` (`ld64.lld`) on your `PATH`.

## 🚀 Usage

```sh
# Interactive: prompts to accept Apple's license and splats into ~/.cache/xmac/splat
xmac splat

# CI one-liner (or set XMAC_ACCEPT_LICENSE=1)
xmac splat --accept-license

# Pin a specific Apple catalog product ID and custom output directory
xmac splat --accept-license --product 052-59890 --output /opt/mac-sdk

# Pre-download only the ~56 MiB SDK .pkg into ~/.cache/xmac/dl
xmac download --accept-license

# List available CLTools macOS SDK packages from Apple's public catalog
xmac list
```

### Options (`xmac splat`)

| flag | description | default |
| :--- | :--- | :--- |
| `--accept-license` | Acknowledge Apple's Software License Agreement (also `XMAC_ACCEPT_LICENSE=1`) | interactive prompt in TTY |
| `--product <ID>` | Specific Apple software update catalog product ID | latest available |
| `--output <DIR>` | Destination directory for `MacOSX.sdk` and `bin/` wrappers | `<cache-dir>/splat` |
| `--cache-dir <DIR>` | Cache root directory (`.pkg` archives go to `<cache-dir>/dl`) | `~/.cache/xmac` |
| `--clang <PATH>` | Explicit LLVM `clang` binary to back the generated wrappers | auto-discovered |
| `-v, --verbose` | Print resolved `MacOSX.sdk` and `bin/` paths after splatting | `false` |

### Output layout

```text
~/.cache/xmac/
├── dl/
│   └── CLTools_<product>_macOSNMOS_SDK.pkg      # ~56 MiB cached Apple SDK package
└── splat/                                       # default --output directory
    ├── MacOSX.sdk/
    │   ├── SDKSettings.json
    │   ├── usr/include/                         # C, POSIX, Darwin & libc++ headers
    │   ├── usr/lib/                             # libSystem.tbd & system stub libraries
    │   └── System/Library/Frameworks/           # Cocoa, CoreFoundation, Metal, etc. (.tbd)
    └── bin/
        ├── aarch64-apple-darwin-clang           # clang/clang++ wrappers for arm64 & x86_64
        ├── aarch64-apple-darwin-clang++
        ├── x86_64-apple-darwin-clang
        └── x86_64-apple-darwin-clang++
```

## 🛠 Cross-compiling

### C / C++ / Objective-C (shell, Make, Ninja)

```sh
~/.cache/xmac/splat/bin/aarch64-apple-darwin-clang -framework CoreFoundation -o hello hello.c
~/.cache/xmac/splat/bin/x86_64-apple-darwin-clang++ -o app main.cpp
```

### Cargo / Rust

```sh
rustup target add aarch64-apple-darwin

export SDKROOT=~/.cache/xmac/splat/MacOSX.sdk
export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=~/.cache/xmac/splat/bin/aarch64-apple-darwin-clang
export CC_aarch64_apple_darwin=~/.cache/xmac/splat/bin/aarch64-apple-darwin-clang
export CXX_aarch64_apple_darwin=~/.cache/xmac/splat/bin/aarch64-apple-darwin-clang++

cargo build --target aarch64-apple-darwin --release
```

Or zero-config via [`xforge`](https://crates.io/crates/xforge):

```sh
xforge build --target aarch64-apple-darwin --accept-apple-license --release
```

### CMake

```sh
cmake -B build -G Ninja \
  -DCMAKE_SYSTEM_NAME=Darwin \
  -DCMAKE_OSX_SYSROOT=~/.cache/xmac/splat/MacOSX.sdk \
  -DCMAKE_C_COMPILER=~/.cache/xmac/splat/bin/aarch64-apple-darwin-clang \
  -DCMAKE_CXX_COMPILER=~/.cache/xmac/splat/bin/aarch64-apple-darwin-clang++
cmake --build build
```

### Go (CGO)

```sh
CGO_ENABLED=1 GOOS=darwin GOARCH=arm64 \
  CC=~/.cache/xmac/splat/bin/aarch64-apple-darwin-clang \
  go build -o hello .
```

## ⚙️ How it works

```text
swscan.apple.com ──► public software-update catalog (.sucatalog XML plist)
                        │  locate latest CLTools_macOS*_SDK.pkg
                        ▼
swcdn.apple.com  ──► CLTools_macOS*_SDK.pkg           (~56 MiB, no auth)
                        │
                        ▼
                     xar (zlib TOC) ──► pbzx (chunked XZ) ──► cpio (ODC)
                                                                 │
                                                                 ▼
                                                      splat/MacOSX.sdk/ + bin/
```

`xmac` parses all three Apple container layers (`xar`, `pbzx`, and `cpio`) in pure Rust, streaming the decompressed `MacOSX*.sdk/` tree straight to disk with symlinks and permissions intact.

## 📚 What's in the SDK (and what isn't)

`MacOSX.sdk` contains headers, module maps, and `.tbd` (Text-Based Stub) symbol tables for all macOS system libraries and frameworks — everything LLVM `clang` + `ld64.lld` needs to compile and link complete Mach-O executables and `.dylib` libraries for `aarch64-apple-darwin` and `x86_64-apple-darwin`.

Not included (macOS-only tools not shipped in the SDK sub-package): `codesign`, `xcodebuild`, `actool`, `ibtool`, and the Swift compiler. For code signing and notarization from Linux/Windows, see [`xsign`](https://crates.io/crates/xsign) / [`apple-codesign`](https://crates.io/crates/apple-codesign).

## ⚖️ License & Legal

The `xmac` _tool_ is dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.

The SDK it downloads is Apple's, subject to Apple's [Software License Agreement](https://www.apple.com/legal/sla/). `xmac` does **not** host, mirror, or redistribute any Apple files — run `xmac` directly in your local environment or CI job rather than committing or redistributing the extracted SDK. `xmac` only communicates with `swscan.apple.com` and `swcdn.apple.com`.

This project is not affiliated with or endorsed by Apple Inc.

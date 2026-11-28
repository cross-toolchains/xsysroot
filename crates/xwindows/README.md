<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.iconify.design/fa6-brands/windows.svg?color=%2300a4ef" />
  <source media="(prefers-color-scheme: light)" srcset="https://api.iconify.design/fa6-brands/windows.svg?color=%230078d4" />
  <img src="https://api.iconify.design/fa6-brands/windows.svg?color=%230078d4" height="56" alt="Windows" />
</picture>

<h1>xwindows</h1>

**Cross-compile to Windows (MSVC & MinGW/UCRT) from anywhere.**

Pure-Rust CLI and library that downloads and splats minimal Windows MSVC CRT + Windows SDK sysroots (adapted from [`Jake-Shadle/xwin`](https://github.com/Jake-Shadle/xwin)) **and** minimal MinGW / UCRT sysroots from upstream [`mstorsjo/llvm-mingw`](https://github.com/mstorsjo/llvm-mingw) over HTTP `Range` —
no Windows machine, no Visual Studio installer, no Docker, and nothing redistributed.

[![crates.io](https://img.shields.io/crates/v/xwindows.svg)](https://crates.io/crates/xwindows)
[![license: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license--legal)

<br>

```console
$ xwindows splat --target x86_64-pc-windows-msvc --accept-license
$ ~/.cache/xwindows/splat/x86_64-pc-windows-msvc/bin/x86_64-pc-windows-msvc-clang -o hello.exe hello.c
$ file hello.exe
hello.exe: PE32+ executable (console) x86-64, for MS Windows

$ xwindows splat --target aarch64-pc-windows-gnullvm
$ ~/.cache/xwindows/splat/aarch64-pc-windows-gnullvm/bin/aarch64-pc-windows-gnullvm-clang -o hello_arm64.exe hello.c
$ file hello_arm64.exe
hello_arm64.exe: PE32+ executable (console) Aarch64, for MS Windows
```

_Both **MSVC** (`*-pc-windows-msvc`) and **MinGW / UCRT** (`*-pc-windows-gnullvm` / `*-pc-windows-gnu`) in one self-contained Rust tool._

</div>

---

`xwindows` unifies both Windows cross-compilation ABIs under the same `xsysroot` workflow:

| ABI | targets | upstream source | license prompt |
| :--- | :--- | :--- | :---: |
| **MSVC** | `x86_64`, `aarch64`, `i686` (`*-pc-windows-msvc`) | Microsoft Visual Studio Build Tools manifest (adapted from [`xwin`](https://github.com/Jake-Shadle/xwin)) | `--accept-license` |
| **MinGW / UCRT** | `x86_64`, `aarch64`, `i686`, `armv7` (`*-pc-windows-gnullvm` / `-gnu`) | [`mstorsjo/llvm-mingw`](https://github.com/mstorsjo/llvm-mingw) (HTTP `Range` sysroot only) | open source |

Because every machine downloads directly from Microsoft's CDN or GitHub Releases, `xwindows` never hosts or redistributes any third-party SDK files.

## 📦 Install

### From binary

```sh
cargo binstall xwindows
```

### From tarball

Download a prebuilt archive from [Releases](https://github.com/cross-toolchains/xsysroot/releases):

```sh
curl -fsSL https://github.com/cross-toolchains/xsysroot/releases/latest/download/xwindows-x86_64-unknown-linux-musl.tar.gz | tar -xz
```

### From source

```sh
cargo install xwindows --locked
# or
cargo install xwindows
```

To compile and link with the generated wrappers, you only need LLVM `clang` and `lld` (`lld-link` / `ld.lld`) on your `PATH` (or managed via [`xllvm`](https://crates.io/crates/xllvm)).

## 🚀 Usage

```sh
# List all supported Windows targets (MSVC + MinGW/UCRT)
xwindows list

# Splat Windows x86_64 MSVC CRT + Windows SDK (interactive license prompt)
xwindows splat --target x86_64-pc-windows-msvc

# CI one-liner for MSVC (or set XWINDOWS_ACCEPT_LICENSE=1)
xwindows splat --target aarch64-pc-windows-msvc --accept-license

# Splat minimal MinGW / UCRT sysroot over HTTP Range (no license prompt required)
xwindows splat --target x86_64-pc-windows-gnullvm
xwindows splat --target aarch64-pc-windows-gnullvm

# Pair with Layer 2 managed LLVM (`xllvm`)
xwindows splat --target x86_64-pc-windows-msvc --accept-license --clang "$(xllvm path --managed)"

# Pre-download packages into ~/.cache/xwindows without splatting
xwindows download --target x86_64-pc-windows-gnullvm
```

### Options (`xwindows splat`)

| flag | description | default |
| :--- | :--- | :--- |
| `-t, --target <TRIPLE>` | Target triple (`x86_64-pc-windows-msvc`, `aarch64-pc-windows-gnullvm`, …) | `x86_64-pc-windows-msvc` |
| `--output <DIR>` | Destination directory for sysroot and `bin/` wrappers | `<cache-dir>/splat/<target>` |
| `--cache-dir <DIR>` | Cache root directory | `~/.cache/xwindows` |
| `--clang <PATH>` | Explicit LLVM `clang` binary to back the generated wrappers | auto-discovered |
| `--accept-license` | Acknowledge Microsoft's license for MSVC (`XWINDOWS_ACCEPT_LICENSE=1`) | interactive prompt in TTY |
| `--sdk-version <VER>` | Specific Windows SDK version (MSVC) | latest available |
| `--crt-version <VER>` | Specific MSVC CRT version (MSVC) | latest available |
| `--include-atl` | Include Active Template Library headers and libs (MSVC) | `false` |
| `--use-winsysroot-style` | Use `clang-cl /winsysroot` layout (MSVC) | `false` |
| `--mingw-release <TAG>` | Upstream `mstorsjo/llvm-mingw` release tag (MinGW) | `20261117` |
| `--crt <ucrt\|msvcrt>` | MinGW C runtime variant | `ucrt` |
| `-v, --verbose` | Print resolved sysroot and wrapper paths after splatting | `false` |

### Output layout

```text
~/.cache/xwindows/
├── msvc/                                               # cached Microsoft VS manifests & payloads
├── dl/
│   └── llvm-mingw-<release>-ucrt-<arch>-sysroot.zip    # HTTP Range mini-zip cache
└── splat/
    ├── x86_64-pc-windows-msvc/
    │   ├── crt/                                        # MSVC CRT headers & libs
    │   ├── sdk/                                        # Windows SDK (ucrt / um / shared)
    │   └── bin/
    │       ├── x86_64-pc-windows-msvc-clang
    │       ├── x86_64-pc-windows-msvc-clang++
    │       ├── x86_64-pc-windows-msvc-clang-cl
    │       └── x86_64-pc-windows-msvc-lld-link
    └── x86_64-pc-windows-gnullvm/
        ├── sysroot/                                    # MinGW-w64 UCRT headers, libs, libc++, builtins
        └── bin/
            ├── x86_64-pc-windows-gnullvm-clang
            └── x86_64-pc-windows-gnullvm-clang++
```

## 🛠 Cross-compiling

### C / C++ (MSVC & MinGW)

```sh
# MSVC
~/.cache/xwindows/splat/x86_64-pc-windows-msvc/bin/x86_64-pc-windows-msvc-clang -o hello.exe hello.c
~/.cache/xwindows/splat/x86_64-pc-windows-msvc/bin/x86_64-pc-windows-msvc-clang++ -o app.exe main.cpp

# MinGW / UCRT
~/.cache/xwindows/splat/x86_64-pc-windows-gnullvm/bin/x86_64-pc-windows-gnullvm-clang -o hello.exe hello.c
~/.cache/xwindows/splat/x86_64-pc-windows-gnullvm/bin/x86_64-pc-windows-gnullvm-clang++ -o app.exe main.cpp
```

### Cargo / Rust

```sh
# Windows MSVC
rustup target add x86_64-pc-windows-msvc
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=~/.cache/xwindows/splat/x86_64-pc-windows-msvc/bin/x86_64-pc-windows-msvc-lld-link
export CC_x86_64_pc_windows_msvc=~/.cache/xwindows/splat/x86_64-pc-windows-msvc/bin/x86_64-pc-windows-msvc-clang-cl
cargo build --target x86_64-pc-windows-msvc --release

# Windows MinGW / UCRT (gnullvm)
rustup target add x86_64-pc-windows-gnullvm
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=~/.cache/xwindows/splat/x86_64-pc-windows-gnullvm/bin/x86_64-pc-windows-gnullvm-clang
export CC_x86_64_pc_windows_gnullvm=~/.cache/xwindows/splat/x86_64-pc-windows-gnullvm/bin/x86_64-pc-windows-gnullvm-clang
cargo build --target x86_64-pc-windows-gnullvm --release
```

## 🦀 Library

```sh
cargo add xwindows --no-default-features
```

```toml
[dependencies]
xwindows = { version = "0.1", default-features = false }
```

```rust
use xwindows::splat::{self, SplatOptions};
use xwindows::target::WindowsTarget;

let target = WindowsTarget::parse("x86_64-pc-windows-gnullvm")?;
let opts = SplatOptions::for_target(target);
splat::run_splat(&opts)?;
```

## ⚖️ License & Legal

The `xwindows` _tool_ is dual-licensed under either of [Apache-2.0](../../LICENSE-APACHE) or [MIT](../../LICENSE-MIT) at your option.

The MSVC CRT and Windows SDK fetching, unpacking, and splatting engine in `src/msvc/` is adapted from [**Jake-Shadle/xwin**](https://github.com/Jake-Shadle/xwin) (dual-licensed under Apache-2.0 OR MIT).

Microsoft CRT and Windows SDK files downloaded for `*-pc-windows-msvc` targets are subject to Microsoft's [Visual Studio / Windows SDK license](https://go.microsoft.com/fwlink/?LinkId=2086102). MinGW/UCRT sysroots are fetched directly from upstream [`mstorsjo/llvm-mingw`](https://github.com/mstorsjo/llvm-mingw) under its open-source licenses. `xwindows` does **not** host, mirror, or redistribute any third-party SDK files.

This project is not affiliated with or endorsed by Microsoft Corporation.

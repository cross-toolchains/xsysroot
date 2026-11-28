<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.iconify.design/simple-icons/linux.svg?color=%23ffffff" />
  <source media="(prefers-color-scheme: light)" srcset="https://api.iconify.design/simple-icons/linux.svg?color=%23000000" />
  <img src="https://api.iconify.design/simple-icons/linux.svg?color=%23000000" height="56" alt="Linux" />
</picture>

<h1>xlinux</h1>

**Cross-compile to Linux (`glibc` & `musl`) from anywhere.**

Pure-Rust CLI that downloads and splats minimal, version-pinned Linux `glibc` (Debian) and `musl` (Alpine) sysroots for LLVM `clang` + `lld` —
no Docker, no Zig, no heavy GCC cross-toolchains, and nothing redistributed.

[![crates.io](https://img.shields.io/crates/v/xlinux.svg)](https://crates.io/crates/xlinux)
[![license: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license--legal)

<br>

```console
$ xlinux splat --target x86_64-unknown-linux-gnu.2.31
  Index   [ 4s] █████████████████████████████████  10.58 MiB/ 10.58 MiB  📥  downloaded
  Sysroot [ 1s] █████████████████████████████████  61.50 MiB/ 61.50 MiB  📦    splatted

$ ~/.cache/xlinux/splat/x86_64-unknown-linux-gnu.2.31/bin/x86_64-unknown-linux-gnu-clang -o hello hello.c
$ file hello
hello: ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), dynamically linked, interpreter /lib64/ld-linux-x86-64.so.2, for GNU/Linux 3.2.0
```

_A complete, linkable Linux `glibc 2.31` sysroot in seconds — downloading only **~11 MiB** of `.deb` packages (~9 MiB for Alpine `musl`)._

</div>

---

Targeting older `glibc` baselines (such as `2.28` or `2.31`) or `musl` from macOS, Windows, or a newer Linux host traditionally required Docker containers or multi-hundred-MiB GCC cross-toolchains. **xlinux** downloads only the minimal C headers, kernel UAPI headers, CRT objects, `libgcc`/`libstdc++`, and versioned `.so` libraries directly from official Debian and Alpine mirrors, unpacks `.deb` (`ar` -> `data.tar.{xz,gz}`) and `.apk` (`tar.gz`) completely in-process, automatically rewrites absolute GNU linker scripts and symlinks into portable relative paths, and generates ready-to-use `clang` / `clang++` wrappers.

| target baseline | upstream source | package download | unpacked sysroot |
| :--- | :--- | :---: | :---: |
| `x86_64` / `aarch64` `musl` | Alpine `v3.21` (`.apk`) | **~ 9.1 MiB** | **~ 41.5 MiB** |
| `x86_64` / `aarch64` `glibc 2.31` | Debian 11 Bullseye (`.deb`) | **~ 11.0 MiB** | **~ 61.5 MiB** |
| `x86_64` / `aarch64` `glibc 2.36` | Debian 12 Bookworm (`.deb`) | ~ 12 MiB | ~ 65 MiB |
| `x86_64` / `aarch64` `glibc 2.28` | Debian 10 Buster (`.deb`) | ~ 11 MiB | ~ 60 MiB |

## 📦 Install

### From binary

```sh
cargo binstall xlinux
```

### From tarball

Download a prebuilt archive from [Releases](https://github.com/cross-toolchains/xsysroot/releases):

```sh
curl -fsSL https://github.com/cross-toolchains/xsysroot/releases/latest/download/xlinux-x86_64-unknown-linux-musl.tar.gz | tar -xz
```

### From source

```sh
cargo install xlinux --locked
# or
cargo install xlinux
```

`xlinux` is a self-contained single binary (pure Rust with `ureq`, `flate2`, and `lzma-rs` — no `dpkg`, `ar`, `tar`, `xz`, or `curl` binaries required). To cross-compile with the generated wrappers, you only need LLVM `clang` and `lld` (`ld.lld`) on your `PATH`.

## 🚀 Usage

```sh
# Splat x86_64 Linux glibc 2.31 sysroot (default)
xlinux splat --target x86_64-unknown-linux-gnu.2.31

# Splat aarch64 Linux glibc 2.28 sysroot
xlinux splat --target aarch64-unknown-linux-gnu --glibc 2.28

# Splat aarch64 Alpine musl sysroot
xlinux splat --target aarch64-unknown-linux-musl

# Specify an explicit LLVM clang binary (recommended on macOS to avoid Apple Clang without lld)
xlinux splat --target x86_64-unknown-linux-gnu.2.31 --clang /opt/homebrew/opt/llvm/bin/clang

# List supported targets and glibc/musl baselines
xlinux list
```

### Options (`xlinux splat`)

| flag | description | default |
| :--- | :--- | :--- |
| `-t, --target <TRIPLE>` | Linux target triple (supports `.2.XX` glibc suffix) | `x86_64-unknown-linux-gnu` |
| `--glibc <VERSION>` | glibc version baseline (`2.28`, `2.31`, `2.36`) for `*-linux-gnu` | `2.31` |
| `--output <DIR>` | Destination directory for `sysroot/` and `bin/` wrappers | `<cache-dir>/splat/<target>` |
| `--cache-dir <DIR>` | Cache root directory (downloaded packages go to `<cache-dir>/dl`) | `~/.cache/xlinux` |
| `--clang <PATH>` | Explicit LLVM `clang` binary to back the generated wrappers | auto-discovered |
| `-v, --verbose` | Print resolved `sysroot/` and `bin/` paths after splatting | `false` |

### Output layout

```text
~/.cache/xlinux/
├── dl/
│   ├── debian-2.31-amd64/*.deb                  # cached Debian packages
│   └── alpine-aarch64/*.apk                     # cached Alpine packages
└── splat/
    └── x86_64-unknown-linux-gnu.2.31/
        ├── sysroot/                             # portable sysroot (relative symlinks & linker scripts)
        │   ├── lib/x86_64-linux-gnu/
        │   └── usr/{include,lib}/
        └── bin/
            ├── x86_64-unknown-linux-gnu-clang
            └── x86_64-unknown-linux-gnu-clang++
```

## 🛠 Cross-compiling

### C / C++ (shell, Make, Ninja)

```sh
~/.cache/xlinux/splat/x86_64-unknown-linux-gnu.2.31/bin/x86_64-unknown-linux-gnu-clang -o hello hello.c
```

### Cargo / Rust

```sh
rustup target add x86_64-unknown-linux-gnu

export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=~/.cache/xlinux/splat/x86_64-unknown-linux-gnu.2.31/bin/x86_64-unknown-linux-gnu-clang
export CC_x86_64_unknown_linux_gnu=~/.cache/xlinux/splat/x86_64-unknown-linux-gnu.2.31/bin/x86_64-unknown-linux-gnu-clang
export CXX_x86_64_unknown_linux_gnu=~/.cache/xlinux/splat/x86_64-unknown-linux-gnu.2.31/bin/x86_64-unknown-linux-gnu-clang++

cargo build --target x86_64-unknown-linux-gnu --release
```

Or via [`xforge`](https://crates.io/crates/xforge):

```sh
xforge build --target x86_64-unknown-linux-gnu --sysroot ~/.cache/xlinux/splat/x86_64-unknown-linux-gnu.2.31/sysroot --release
```

## ⚙️ How it works

```text
Debian mirror ──► Packages.gz ──► libc6 / libc6-dev / linux-libc-dev / libgcc / libstdc++ (.deb)
                                       │  ar -> data.tar.{xz,gz} (pure Rust)
                                       ▼  rewrite /lib/* linker scripts & absolute symlinks
                                  splat/<target>/sysroot/ + bin/*-clang

Alpine mirror ──► APKINDEX.tar.gz ──► musl / musl-dev / linux-headers / libgcc / libstdc++ (.apk)
                                       │  multi-member gzip -> tar (pure Rust)
                                       ▼
                                  splat/<target>/sysroot/ + bin/*-clang
```

## 📚 What's in the sysroot (and what isn't)

The extracted `sysroot/` contains C/POSIX/C++ headers, Linux kernel UAPI headers, CRT startup objects (`crt1.o`, `Scrt1.o`, `crti.o`, `crtn.o`, `crtbegin*.o`), and version-pinned `glibc` or `musl` + `libstdc++` libraries — everything LLVM `clang` + `ld.lld` needs to cross-compile and link Linux ELF binaries and `.so` shared libraries.

Not included: host compilers (`gcc`), system daemons, package managers, or third-party desktop libraries (GTK, X11, Wayland, OpenSSL).

## 🦀 Library

```sh
cargo add xlinux --no-default-features
```

```toml
[dependencies]
xlinux = { version = "0.1", default-features = false }
```

```rust
use xlinux::splat::{self, SplatOptions};
use xlinux::target::LinuxTarget;

let (target, glibc) = LinuxTarget::parse("x86_64-unknown-linux-gnu.2.31")?;
let glibc = glibc.unwrap_or_else(|| "2.31".to_string());
let cache_dir = splat::default_cache_dir();
let opts = SplatOptions {
    output_dir: splat::default_splat_dir(&cache_dir, &target, &glibc),
    target,
    glibc,
    cache_dir,
    clang_bin: None,
    verbose: false,
};
splat::run_splat(&opts)?;
```

## ⚖️ License & Legal

The `xlinux` _tool_ is dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.

`xlinux` does **not** host, mirror, or redistribute third-party distribution archives. Every machine downloads official packages directly from upstream Debian and Alpine mirrors.

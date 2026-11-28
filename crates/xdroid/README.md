<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.iconify.design/fa6-brands/android.svg?color=%23ffffff" />
  <source media="(prefers-color-scheme: light)" srcset="https://api.iconify.design/fa6-brands/android.svg?color=%23000000" />
  <img src="https://api.iconify.design/fa6-brands/android.svg?color=%23000000" height="52" alt="Android" />
</picture>

<h1>xdroid</h1>

**Cross-compile to Android from anywhere.**

Downloads and splats minimal Android NDK sysroots straight from Google's public repository —
no Android Studio, no `sdkmanager`, no Java, and nothing redistributed.

[![crates.io](https://img.shields.io/crates/v/xdroid.svg)](https://crates.io/crates/xdroid)
[![license: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license--legal)

<br>

```console
$ xdroid splat
Do you accept the license at https://developer.android.com/studio/terms (yes | no)?
yes
license accepted!
  Index   [ 1s] █████████████████████████████████   1.41 MiB/  1.41 MiB  📥  downloaded
  Sysroot [ 9s] █████████████████████████████████ 133.32 MiB/133.32 MiB  📦    splatted

$ ~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang -o hello hello.c
$ file hello
hello: ELF 64-bit LSB pie executable, ARM aarch64, dynamically linked, interpreter /system/bin/linker64
```

_A complete, linkable Android NDK sysroot in about ten seconds — downloading only **~47 MiB** instead of ~800 MiB._

</div>

---

[xwin](https://github.com/Jake-Shadle/xwin) made it possible to target Windows from Linux/macOS CI, and [xmac](https://github.com/cross-toolchains/xmac) brought the same model to macOS. **xdroid is the same idea for Android**: Google serves standalone Android NDK `.zip` archives from `dl.google.com` with **no authentication** and full HTTP `Range` support. `xdroid` reads only the remote ZIP's Central Directory (~1.4 MiB), downloads **only the ~45 MiB `sysroot/` byte range** (skipping `lldb`, `clang`, Python, and host tools — **94% bandwidth saved**), extracts the portable `sysroot/` in-process, and generates ready-to-use `clang` / `clang++` wrappers for all four Android ABIs.

| | download | needs Android Studio | needs `sdkmanager` / JDK |
| :--- | :--- | :---: | :---: |
| Android Studio + SDK Manager | ~2 GiB + JDK | ✅ yes | ✅ yes |
| Full Android NDK `.zip` | ~800 MiB | ❌ no | ❌ no |
| **xdroid** (HTTP Range `sysroot/` only) | **~ 47 MiB** | **❌ no** | **❌ no** |

Because every machine downloads the official NDK archive directly from `dl.google.com`, `xdroid` never hosts or redistributes any Google files. Users acknowledge Google's Android SDK License Agreement (`--accept-license`) before downloading.

## 📦 Install

### From binary

```sh
cargo binstall xdroid
```

### From tarball

Download a prebuilt archive from [Releases](https://github.com/cross-toolchains/xsysroot/releases):

```sh
curl -fsSL https://github.com/cross-toolchains/xsysroot/releases/latest/download/xdroid-x86_64-unknown-linux-musl.tar.gz | tar -xz
```

### From source

```sh
cargo install xdroid --locked
# or
cargo install xdroid
```

`xdroid` is a self-contained single binary (pure Rust with `ureq` + `zip` — no `curl` or `unzip` required). To actually cross-compile with the generated wrappers, you only need LLVM `clang` and `lld` on your `PATH`.

## 🚀 Usage

```sh
# Interactive: prompts to accept the Android SDK license and splats into ~/.cache/xdroid/splat
xdroid splat

# CI one-liner (or set XDROID_ACCEPT_LICENSE=1)
xdroid splat --accept-license

# Pin NDK version, Android API level, and custom output directory
xdroid splat --accept-license --ndk r27c --api 24 --output /opt/android-sysroot

# Pre-download only the ~47 MiB sysroot archive into ~/.cache/xdroid/dl (e.g. Docker layer caching)
xdroid download --accept-license

# List known NDK releases and official Google download URLs
xdroid list
```

### Options (`xdroid splat`)

| flag | description | default |
| :--- | :--- | :--- |
| `--accept-license` | Acknowledge the Android SDK License Agreement (also `XDROID_ACCEPT_LICENSE=1`) | interactive prompt in TTY |
| `--ndk <VERSION>` | Official Android NDK release tag | `r27c` |
| `--api <LEVEL>` | Minimum Android API level for generated compiler shims | `24` |
| `--output <DIR>` | Destination directory for `sysroot/` and `bin/` wrappers | `<cache-dir>/splat` |
| `--cache-dir <DIR>` | Cache root directory (downloaded mini-archives go to `<cache-dir>/dl`) | `~/.cache/xdroid` |
| `--clang <PATH>` | Explicit LLVM `clang` binary to back the generated wrappers | auto-discovered |
| `-v, --verbose` | Print resolved `sysroot/` and `bin/` paths after splatting | `false` |

### Output layout

```text
~/.cache/xdroid/
├── dl/
│   └── android-ndk-r27c-<host>-sysroot.zip      # ~47 MiB cached sysroot slice
└── splat/                                       # default --output directory
    ├── sysroot/
    │   └── usr/
    │       ├── include/                         # Bionic C, POSIX, Android & C++ headers
    │       └── lib/
    │           ├── aarch64-linux-android/       # API 21–35 .so stubs, crt*.o, libc++
    │           ├── arm-linux-androideabi/
    │           ├── i686-linux-android/
    │           └── x86_64-linux-android/
    └── bin/
        ├── aarch64-linux-android24-clang        # clang/clang++ wrappers for all 4 ABIs
        ├── aarch64-linux-android24-clang++
        ├── armv7a-linux-androideabi24-clang
        ├── armv7a-linux-androideabi24-clang++
        ├── i686-linux-android24-clang
        ├── i686-linux-android24-clang++
        ├── x86_64-linux-android24-clang
        └── x86_64-linux-android24-clang++
```

## 🛠 Cross-compiling

### C / C++ (shell, Make, Ninja)

```sh
~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang -o hello hello.c
~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang++ -shared -fPIC -o libfoo.so foo.cpp
```

### Cargo / Rust

```sh
rustup target add aarch64-linux-android

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang
export CC_aarch64_linux_android=~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang
export CXX_aarch64_linux_android=~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang++

cargo build --target aarch64-linux-android --release
```

Or zero-config via [`xforge`](https://crates.io/crates/xforge):

```sh
xforge build --target aarch64-linux-android --accept-android-license --release
```

### CMake

```sh
cmake -B build -G Ninja \
  -DCMAKE_SYSTEM_NAME=Android \
  -DCMAKE_SYSTEM_VERSION=24 \
  -DCMAKE_ANDROID_ARCH_ABI=arm64-v8a \
  -DCMAKE_SYSROOT=~/.cache/xdroid/splat/sysroot \
  -DCMAKE_C_COMPILER=~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang \
  -DCMAKE_CXX_COMPILER=~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang++
cmake --build build
```

### Go (CGO)

```sh
CGO_ENABLED=1 GOOS=android GOARCH=arm64 \
  CC=~/.cache/xdroid/splat/bin/aarch64-linux-android24-clang \
  go build -o hello .
```

## ⚙️ How it works

```text
dl.google.com ──► HEAD android-ndk-r27c-<host>.zip        (content-length: ~797 MiB)
                        │
                        ▼
                  Range: last 64 KiB + Central Directory  (~1.4 MiB)
                        │  filter: */toolchains/llvm/prebuilt/*/sysroot/*
                        ▼
                  Range: coalesced sysroot byte span      (~45.2 MiB, 1 request)
                        │
                        ▼
                  reassemble valid mini .zip in dl/  ──►  in-process zip extract ──► splat/sysroot/
```

Instead of downloading the entire ~800 MiB NDK archive (most of which is a bundled host LLVM/LLDB/Python toolchain), `xdroid` parses the remote ZIP's End of Central Directory and Central Directory via HTTP `Range`, coalesces all matching `sysroot/` entry spans into a single HTTP `Range` request, rewrites a standalone mini `.zip` into `<cache-dir>/dl/`, and extracts it directly in-process using the `zip` crate.

## 📚 What's in the sysroot (and what isn't)

The extracted `sysroot/` contains everything needed to compile and link native Android ELF executables and JNI `.so` libraries for `aarch64`, `armv7a`, `i686`, and `x86_64`:
- Bionic C/POSIX headers, Linux kernel UAPI headers, and Android NDK headers (`<android/log.h>`, `<vulkan/vulkan.h>`, OpenGL ES, etc.)
- Per-API-level `.so` link stubs (`libc.so`, `libm.so`, `libdl.so`, `liblog.so`, `libz.so`, `libandroid.so`, etc.)
- `crtbegin_*.o` / `crtend_*.o` startup objects and `libc++` (`libc++_shared.so`, `libc++_static.a`)

Not included (not part of native cross-compilation): Android Studio, `sdkmanager`, `adb`, `d8`, Gradle, Java/JDK, or `.apk`/`.aab` packaging tools.

## 🦀 Library

```sh
cargo add xdroid --no-default-features
```

```toml
[dependencies]
xdroid = { version = "0.1", default-features = false }
```

```rust
use xdroid::splat::{self, SplatOptions};

let cache_dir = splat::default_cache_dir();
let opts = SplatOptions {
    ndk_version: "r27c".to_string(),
    api_level: 24,
    output_dir: splat::default_splat_dir(&cache_dir),
    cache_dir,
    clang_bin: None,
    accept_license: true,
    verbose: false,
};
splat::run_splat(&opts)?;
```

## ⚖️ License & Legal

The `xdroid` _tool_ is dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.

The NDK sysroot it downloads is Google's, subject to Google's [Android Software Development Kit License Agreement](https://developer.android.com/studio/terms). `xdroid` does **not** host, mirror, or redistribute any Google or Android files — run `xdroid` directly in your local environment or CI job rather than committing or redistributing the extracted sysroot. `xdroid` only communicates with `dl.google.com`.

This project is not affiliated with or endorsed by Google LLC.

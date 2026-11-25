<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.iconify.design/simple-icons/harmonyos.svg?color=%23ffffff" />
  <source media="(prefers-color-scheme: light)" srcset="https://api.iconify.design/simple-icons/harmonyos.svg?color=%23000000" />
  <img src="https://api.iconify.design/simple-icons/harmonyos.svg?color=%23000000" height="52" alt="OpenHarmony" />
</picture>

<h1>xohos</h1>

**Cross-compile to OpenHarmony / HarmonyOS from anywhere.**

Pure-Rust CLI that streams only `native-*.zip` from official OpenHarmony public SDK releases, extracts `sysroot/` in-process, and generates LLVM `clang` wrappers —
no DevEco Studio, no `tar`/`unzip` required, and nothing redistributed.

[![crates.io](https://img.shields.io/crates/v/xohos.svg)](https://crates.io/crates/xohos)
[![license: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license--legal)

<br>

```console
$ xohos splat
  Sysroot [ 0s] █████████████████████████████████  50.33 MiB/ 50.33 MiB  📦    splatted

$ ~/.cache/xohos/splat/bin/aarch64-unknown-linux-ohos-clang -o hello hello.c
$ file hello
hello: ELF 64-bit LSB pie executable, ARM aarch64, dynamically linked, interpreter /lib/ld-musl-aarch64.so.1
```

</div>

---

**xohos** brings the [xwin](https://github.com/Jake-Shadle/xwin) / [xmac](https://crates.io/crates/xmac) / [xdroid](https://crates.io/crates/xdroid) model to OpenHarmony (`*-unknown-linux-ohos`). Instead of downloading and unpacking multi-GiB full SDK bundles (DevEco Studio, ETS, JS, Previewer), `xohos` streams the official OpenHarmony `.tar.gz` release, extracts only `native-*.zip` (stopping the HTTP stream early as soon as it is read), unpacks `native/sysroot/` in-process, and generates ready-to-use `clang` / `clang++` wrappers for `aarch64`, `armv7`, and `x86_64`.

## 📦 Install

### From binary

```sh
cargo binstall xohos
```

### From tarball

Download a prebuilt archive from [Releases](https://github.com/cross-toolchains/xsysroot/releases):

```sh
curl -fsSL https://github.com/cross-toolchains/xsysroot/releases/latest/download/xohos-x86_64-unknown-linux-musl.tar.gz | tar -xz
```

### From source

```sh
cargo install xohos --locked
# or
cargo install xohos
```

`xohos` is a self-contained single binary (pure Rust with `ureq`, `flate2`, and `zip` — no `curl`, `tar`, or `unzip` required). To cross-compile with the generated wrappers, you only need LLVM `clang` and `lld` on your `PATH`.

## 🚀 Usage

```sh
# Download and splat OpenHarmony 6.1-Release sysroot into ~/.cache/xohos/splat
xohos splat

# Pin a specific OpenHarmony release and custom output directory
xohos splat --version 6.0-Release --output /opt/ohos-sysroot

# Offline: extract from a local native-*.zip or ohos-sdk-*.tar.gz
xohos splat --from /path/to/native-linux-x64-6.1-Release.zip

# Pre-download only native-*.zip into ~/.cache/xohos/dl
xohos download

# List known OpenHarmony public SDK release URLs
xohos list
```

### Options (`xohos splat`)

| flag | description | default |
| :--- | :--- | :--- |
| `--version <TAG>` | OpenHarmony release tag | `6.1-Release` |
| `--from <PATH>` | Local `native-*.zip` or `ohos-sdk-*.tar.gz` (skips download) | download from official mirror |
| `--output <DIR>` | Destination directory for `sysroot/` and `bin/` wrappers | `<cache-dir>/splat` |
| `--cache-dir <DIR>` | Cache root directory (cached `native-*.zip` goes to `<cache-dir>/dl`) | `~/.cache/xohos` |
| `--clang <PATH>` | Explicit LLVM `clang` binary to back the generated wrappers | auto-discovered |
| `-v, --verbose` | Print resolved `sysroot/` and `bin/` paths after splatting | `false` |

### Output layout

```text
~/.cache/xohos/
├── dl/
│   └── ohos-native-6.1-Release.zip              # cached native SDK archive
└── splat/                                       # default --output directory
    ├── sysroot/
    │   └── usr/
    │       ├── include/                         # musl C, POSIX, OpenHarmony & C++ headers
    │       └── lib/
    │           ├── aarch64-linux-ohos/          # .so stubs, crt*.o, libc++
    │           ├── arm-linux-ohos/
    │           └── x86_64-linux-ohos/
    └── bin/
        ├── aarch64-unknown-linux-ohos-clang     # clang/clang++ wrappers for all 3 ABIs
        ├── aarch64-unknown-linux-ohos-clang++
        ├── armv7-unknown-linux-ohos-clang
        ├── armv7-unknown-linux-ohos-clang++
        ├── x86_64-unknown-linux-ohos-clang
        └── x86_64-unknown-linux-ohos-clang++
```

## 🛠 Cross-compiling

### C / C++ (shell, Make, Ninja)

```sh
~/.cache/xohos/splat/bin/aarch64-unknown-linux-ohos-clang -o hello hello.c
~/.cache/xohos/splat/bin/aarch64-unknown-linux-ohos-clang++ -shared -fPIC -o libfoo.so foo.cpp
```

### Cargo / Rust

```sh
rustup target add aarch64-unknown-linux-ohos

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER=~/.cache/xohos/splat/bin/aarch64-unknown-linux-ohos-clang
export CC_aarch64_unknown_linux_ohos=~/.cache/xohos/splat/bin/aarch64-unknown-linux-ohos-clang
export CXX_aarch64_unknown_linux_ohos=~/.cache/xohos/splat/bin/aarch64-unknown-linux-ohos-clang++

cargo build --target aarch64-unknown-linux-ohos --release
```

### CMake

```sh
cmake -B build -G Ninja \
  -DCMAKE_SYSTEM_NAME=OHOS \
  -DCMAKE_SYSROOT=~/.cache/xohos/splat/sysroot \
  -DCMAKE_C_COMPILER=~/.cache/xohos/splat/bin/aarch64-unknown-linux-ohos-clang \
  -DCMAKE_CXX_COMPILER=~/.cache/xohos/splat/bin/aarch64-unknown-linux-ohos-clang++
cmake --build build
```

## ⚙️ How it works

```text
repo.huaweicloud.com ──► stream ohos-sdk-mac-public.tar.gz
                            │  inline gzip + POSIX tar scan
                            ▼  stop stream immediately after native-*.zip
                         dl/ohos-native-<ver>.zip
                            │  in-process zip extract (filter: */sysroot/*)
                            ▼
                         splat/sysroot/ + bin/*-clang
```

Because `native/sysroot/` contains only target-architecture headers and `.so` stubs, it is 100% host-independent — streaming the smaller macOS public SDK archive and aborting the download as soon as `native-*.zip` is extracted avoids downloading gigabytes of unrelated JS/ETS/Previewer tools.

## 📚 What's in the sysroot (and what isn't)

The extracted `sysroot/` contains everything needed to compile and link native OpenHarmony ELF executables and N-API `.so` libraries for `aarch64-unknown-linux-ohos`, `armv7-unknown-linux-ohos`, and `x86_64-unknown-linux-ohos`:
- musl C/POSIX headers, Linux UAPI headers, and OpenHarmony native API headers (`<hilog/log.h>`, `<napi/native_api.h>`, `<rawfile/...>`, etc.)
- System `.so` link stubs, `crt*.o` startup objects, and `libc++`

Not included: DevEco Studio, ArkTS/ETS compiler, Previewer, `hdc`, or `.hap`/`.har`/`.app` packaging and signing tools.

## ⚖️ License & Legal

The `xohos` _tool_ is dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.

`xohos` does **not** host, mirror, or redistribute any OpenHarmony SDK/NDK files. Every machine downloads official public releases directly from upstream.

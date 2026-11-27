//! `xwindows` — Pure-Rust Windows MSVC & MinGW/UCRT sysroot fetcher and platter.
//!
//! Supports both:
//! - **MSVC** (`*-pc-windows-msvc`): fetches official Microsoft Visual Studio
//!   Build Tools CRT + Windows SDK packages (adapted from `Jake-Shadle/xwin`).
//! - **MinGW / UCRT** (`*-pc-windows-gnullvm` / `*-pc-windows-gnu`): fetches
//!   minimal UCRT headers, import libraries, `libc++`, `libunwind`, and
//!   `compiler-rt` builtins from upstream `mstorsjo/llvm-mingw` releases over
//!   HTTP `Range` partial ZIP requests.

pub mod mingw;
pub mod msvc;
pub mod splat;
pub mod target;

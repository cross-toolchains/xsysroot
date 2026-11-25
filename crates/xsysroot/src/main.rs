//! `xsysroot` unified CLI entry point.
//!
//! Dispatches `xsysroot <platform> ...` across `android` (`xdroid`), `macos` (`xmac`),
//! `ios` (`xios`), `linux` (`xlinux`), and `ohos` (`xohos`).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Unified cross-platform sysroot manager.
#[derive(Debug, Parser)]
#[command(name = "xsysroot", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Splat a target sysroot by platform (`android`, `macos`, `ios`, `linux`, `ohos`).
    Splat {
        /// Target platform (`android`, `macos`, `ios`, `linux`, `ohos`).
        platform: String,
        /// Accept upstream vendor license agreement (Android / Apple).
        #[arg(long)]
        accept_license: bool,
        /// Optional target triple (used for `linux`, e.g. `x86_64-unknown-linux-gnu.2.31`).
        #[arg(short, long)]
        target: Option<String>,
        /// Optional output directory override.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Optional local archive path (`Xcode.xip` for `ios`, `native-*.zip` for `ohos`).
        #[arg(long)]
        from: Option<PathBuf>,
        /// Verbose output.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show available platforms and their backing crates.
    List,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::List => {
            xsysroot_core::print_table(
                &["PLATFORM", "CRATE", "UPSTREAM SOURCE"],
                vec![
                    vec![
                        "android".into(),
                        "xdroid".into(),
                        "Google Android NDK (HTTP Range ~47 MiB)".into(),
                    ],
                    vec![
                        "macos".into(),
                        "xmac".into(),
                        "Apple public CLT CDN (~56 MiB)".into(),
                    ],
                    vec![
                        "ios".into(),
                        "xios".into(),
                        "User-supplied Xcode.xip / Xcode.app".into(),
                    ],
                    vec![
                        "linux".into(),
                        "xlinux".into(),
                        "Debian (glibc) / Alpine (musl) (~9–11 MiB)".into(),
                    ],
                    vec![
                        "ohos".into(),
                        "xohos".into(),
                        "OpenHarmony public SDK mirror".into(),
                    ],
                ],
            );
            ExitCode::SUCCESS
        }
        Commands::Splat {
            platform,
            accept_license,
            target,
            output,
            from,
            verbose,
        } => {
            let res = match platform.as_str() {
                "android" | "droid" => {
                    let cache_dir = xdroid::splat::default_cache_dir();
                    let output_dir =
                        output.unwrap_or_else(|| xdroid::splat::default_splat_dir(&cache_dir));
                    xdroid::splat::run_splat(&xdroid::splat::SplatOptions {
                        ndk_version: xdroid::ndk::DEFAULT_NDK_VERSION.to_string(),
                        api_level: xdroid::ndk::DEFAULT_API_LEVEL,
                        output_dir,
                        cache_dir,
                        clang_bin: None,
                        accept_license,
                        verbose,
                    })
                }
                "macos" | "mac" | "darwin" => {
                    let cache_dir = xmac::splat::default_cache_dir();
                    let output_dir =
                        output.unwrap_or_else(|| xmac::splat::default_splat_dir(&cache_dir));
                    xmac::splat::run_splat(&xmac::splat::SplatOptions {
                        product_id: None,
                        output_dir,
                        cache_dir,
                        clang_bin: None,
                        accept_license,
                        verbose,
                    })
                }
                "ios" => {
                    let output_dir = output.unwrap_or_else(|| {
                        xios::splat::default_splat_dir(&xios::splat::default_cache_dir())
                    });
                    xios::splat::run_splat(&xios::splat::SplatOptions {
                        from,
                        output_dir,
                        min_ios: xios::splat::DEFAULT_MIN_IOS.to_string(),
                        include_simulator: false,
                        clang_bin: None,
                        verbose,
                    })
                }
                "linux" => {
                    let raw = target.as_deref().unwrap_or(xlinux::target::DEFAULT_TARGET);
                    match xlinux::target::LinuxTarget::parse(raw) {
                        Ok((parsed, suffix)) => {
                            let glibc =
                                suffix.unwrap_or_else(|| xlinux::target::DEFAULT_GLIBC.to_string());
                            let cache_dir = xlinux::splat::default_cache_dir();
                            let output_dir = output.unwrap_or_else(|| {
                                xlinux::splat::default_splat_dir(&cache_dir, &parsed, &glibc)
                            });
                            xlinux::splat::run_splat(&xlinux::splat::SplatOptions {
                                target: parsed,
                                glibc,
                                output_dir,
                                cache_dir,
                                clang_bin: None,
                                verbose,
                            })
                        }
                        Err(e) => Err(e),
                    }
                }
                "ohos" | "harmony" | "openharmony" => {
                    let cache_dir = xohos::splat::default_cache_dir();
                    let output_dir =
                        output.unwrap_or_else(|| xohos::splat::default_splat_dir(&cache_dir));
                    xohos::splat::run_splat(&xohos::splat::SplatOptions {
                        version: xohos::sdk::DEFAULT_OHOS_VERSION.to_string(),
                        from,
                        output_dir,
                        cache_dir,
                        clang_bin: None,
                        verbose,
                    })
                }
                other => Err(format!(
                    "unknown platform `{other}` (supported: android, macos, ios, linux, ohos)"
                )),
            };
            match res {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xsysroot] error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

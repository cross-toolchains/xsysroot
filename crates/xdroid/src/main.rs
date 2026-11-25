//! `xdroid` CLI entry point.
//!
//! Downloads and splats minimal Android NDK sysroots directly from Google's
//! public repository for zero-setup cross-compilation.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use xdroid::ndk::{
    DEFAULT_API_LEVEL, DEFAULT_NDK_VERSION, KNOWN_RELEASES, host_ndk_os, ndk_download_url,
};
use xdroid::splat::{
    DownloadOptions, SplatOptions, default_cache_dir, default_splat_dir, run_download, run_splat,
};

/// Utility for downloading and splatting the Android NDK sysroot for cross-compilation.
#[derive(Debug, Parser)]
#[command(name = "xdroid", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Download only the `sysroot/` slice of the official Android NDK into `<cache-dir>/dl`.
    Download(DownloadArgs),
    /// Download and splat `sysroot/` plus `clang` wrappers into `<output>` (default: `<cache-dir>/splat`).
    Splat(SplatArgs),
    /// List known Android NDK release versions and official download URLs.
    List,
}

#[derive(Debug, Args)]
struct DownloadArgs {
    /// Acknowledge Google's Android SDK License Agreement.
    #[arg(long, env = "XDROID_ACCEPT_LICENSE")]
    accept_license: bool,

    /// Android NDK release version.
    #[arg(long, default_value = DEFAULT_NDK_VERSION)]
    ndk: String,

    /// Cache root directory (defaults to `~/.cache/xdroid`; archives go to `<cache-dir>/dl`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct SplatArgs {
    /// Acknowledge Google's Android SDK License Agreement.
    #[arg(long, env = "XDROID_ACCEPT_LICENSE")]
    accept_license: bool,

    /// Android NDK release version.
    #[arg(long, default_value = DEFAULT_NDK_VERSION)]
    ndk: String,

    /// Minimum Android API level for generated compiler shims.
    #[arg(long, default_value_t = DEFAULT_API_LEVEL)]
    api: u32,

    /// Destination directory for the splatted `sysroot/` and `bin/` wrappers (defaults to `<cache-dir>/splat`).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Cache root directory (defaults to `~/.cache/xdroid`; archives go to `<cache-dir>/dl`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Explicit LLVM `clang` binary path (auto-discovered if omitted).
    #[arg(long)]
    clang: Option<PathBuf>,

    /// Print verbose output (e.g. resolved sysroot and wrapper paths).
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::List => {
            let host = host_ndk_os().unwrap_or("linux");
            let rows = KNOWN_RELEASES
                .iter()
                .map(|ver| vec![ver.to_string(), ndk_download_url(ver, host)])
                .collect();
            xsysroot_core::print_table(&["NDK", "URL"], rows);
            ExitCode::SUCCESS
        }
        Commands::Download(args) => {
            let opts = DownloadOptions {
                ndk_version: args.ndk,
                cache_dir: args.cache_dir.unwrap_or_else(default_cache_dir),
                accept_license: args.accept_license,
            };
            match run_download(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xdroid] error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::Splat(args) => {
            let cache_dir = args.cache_dir.unwrap_or_else(default_cache_dir);
            let output_dir = args.output.unwrap_or_else(|| default_splat_dir(&cache_dir));
            let opts = SplatOptions {
                ndk_version: args.ndk,
                api_level: args.api,
                output_dir,
                cache_dir,
                clang_bin: args.clang,
                accept_license: args.accept_license,
                verbose: args.verbose,
            };
            match run_splat(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xdroid] error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_splat_options() {
        let cli = Cli::try_parse_from([
            "xdroid",
            "splat",
            "--accept-license",
            "--ndk",
            "r27c",
            "--api",
            "26",
            "--output",
            "/tmp/out",
        ])
        .unwrap();
        match cli.command {
            Commands::Splat(args) => {
                assert!(args.accept_license);
                assert_eq!(args.ndk, "r27c");
                assert_eq!(args.api, 26);
                assert_eq!(args.output, Some(PathBuf::from("/tmp/out")));
            }
            _ => panic!("expected splat subcommand"),
        }
    }

    #[test]
    fn cli_parses_download_options() {
        let cli = Cli::try_parse_from(["xdroid", "download", "--accept-license", "--ndk", "r27c"])
            .unwrap();
        match cli.command {
            Commands::Download(args) => {
                assert!(args.accept_license);
                assert_eq!(args.ndk, "r27c");
            }
            _ => panic!("expected download subcommand"),
        }
    }
}

//! `xmac` CLI entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use xmac::catalog::fetch_sdk_packages;
use xmac::splat::{
    DownloadOptions, SplatOptions, default_cache_dir, default_splat_dir, run_download, run_splat,
};

/// Utility for downloading and splatting the macOS SDK for cross-compilation.
#[derive(Debug, Parser)]
#[command(name = "xmac", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Download the standalone macOS SDK `.pkg` into `<cache-dir>/dl`.
    Download(DownloadArgs),
    /// Download and splat `MacOSX.sdk` plus `clang` wrappers into `<output>` (default: `<cache-dir>/splat`).
    Splat(SplatArgs),
    /// List available macOS SDK packages from Apple's public software update catalog.
    List,
}

#[derive(Debug, Args)]
struct DownloadArgs {
    /// Acknowledge Apple's Software License Agreement.
    #[arg(long, env = "XMAC_ACCEPT_LICENSE")]
    accept_license: bool,

    /// Specific Apple catalog product ID (defaults to the latest available SDK).
    #[arg(long)]
    product: Option<String>,

    /// Cache root directory (defaults to `~/.cache/xmac`; archives go to `<cache-dir>/dl`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct SplatArgs {
    /// Acknowledge Apple's Software License Agreement.
    #[arg(long, env = "XMAC_ACCEPT_LICENSE")]
    accept_license: bool,

    /// Specific Apple catalog product ID (defaults to the latest available SDK).
    #[arg(long)]
    product: Option<String>,

    /// Destination directory for `MacOSX.sdk` and `bin/` wrappers (defaults to `<cache-dir>/splat`).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Cache root directory (defaults to `~/.cache/xmac`; archives go to `<cache-dir>/dl`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Explicit LLVM `clang` binary path (auto-discovered if omitted).
    #[arg(long)]
    clang: Option<PathBuf>,

    /// Print verbose output (e.g. resolved SDK and wrapper paths).
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::List => match fetch_sdk_packages(false) {
            Ok(pkgs) => {
                let rows = pkgs
                    .iter()
                    .take(15)
                    .map(|p| {
                        let mib = p.size as f64 / (1024.0 * 1024.0);
                        vec![
                            p.product_id.clone(),
                            p.post_date.clone(),
                            p.kind.clone(),
                            format!("{mib:.1} MiB"),
                            p.url.clone(),
                        ]
                    })
                    .collect();
                xsysroot_core::print_table(&["PRODUCT", "DATE", "KIND", "SIZE", "URL"], rows);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("[xmac] error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::Download(args) => {
            let opts = DownloadOptions {
                product_id: args.product,
                cache_dir: args.cache_dir.unwrap_or_else(default_cache_dir),
                accept_license: args.accept_license,
            };
            match run_download(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xmac] error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::Splat(args) => {
            let cache_dir = args.cache_dir.unwrap_or_else(default_cache_dir);
            let output_dir = args.output.unwrap_or_else(|| default_splat_dir(&cache_dir));
            let opts = SplatOptions {
                product_id: args.product,
                output_dir,
                cache_dir,
                clang_bin: args.clang,
                accept_license: args.accept_license,
                verbose: args.verbose,
            };
            match run_splat(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xmac] error: {err}");
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
    fn cli_parses_splat_and_download() {
        let cli = Cli::try_parse_from([
            "xmac",
            "splat",
            "--accept-license",
            "--output",
            "/tmp/mac-sdk",
        ])
        .unwrap();
        match cli.command {
            Commands::Splat(args) => {
                assert!(args.accept_license);
                assert_eq!(args.output, Some(PathBuf::from("/tmp/mac-sdk")));
            }
            _ => panic!("expected splat"),
        }
    }
}

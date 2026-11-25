//! `xohos` CLI entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use xohos::sdk::{DEFAULT_OHOS_VERSION, KNOWN_RELEASES, ohos_sdk_url};
use xohos::splat::{
    DownloadOptions, SplatOptions, default_cache_dir, default_splat_dir, run_download, run_splat,
};

/// Utility for downloading and splatting the OpenHarmony NDK sysroot for cross-compilation.
#[derive(Debug, Parser)]
#[command(name = "xohos", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Stream only `native-*.zip` from the official OpenHarmony SDK release into `<cache-dir>/dl`.
    Download(DownloadArgs),
    /// Download (or read `--from`) and splat `sysroot/` + `clang` wrappers into `<output>`.
    Splat(SplatArgs),
    /// List known OpenHarmony SDK release versions and official download URLs.
    List,
}

#[derive(Debug, Args)]
struct DownloadArgs {
    /// OpenHarmony release tag.
    #[arg(long, default_value = DEFAULT_OHOS_VERSION)]
    version: String,

    /// Cache root directory (defaults to `~/.cache/xohos`; archives go to `<cache-dir>/dl`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct SplatArgs {
    /// OpenHarmony release tag.
    #[arg(long, default_value = DEFAULT_OHOS_VERSION)]
    version: String,

    /// Optional local `native-*.zip` or `ohos-sdk-*.tar.gz` archive (skips network download).
    #[arg(long)]
    from: Option<PathBuf>,

    /// Destination directory for `sysroot/` and `bin/` wrappers (default: `<cache-dir>/splat`).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Cache root directory (defaults to `~/.cache/xohos`; archives go to `<cache-dir>/dl`).
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
            let rows = KNOWN_RELEASES
                .iter()
                .map(|ver| vec![ver.to_string(), ohos_sdk_url(ver)])
                .collect();
            xsysroot_core::print_table(&["VERSION", "URL"], rows);
            ExitCode::SUCCESS
        }
        Commands::Download(args) => {
            let opts = DownloadOptions {
                version: args.version,
                cache_dir: args.cache_dir.unwrap_or_else(default_cache_dir),
            };
            match run_download(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xohos] error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::Splat(args) => {
            let cache_dir = args.cache_dir.unwrap_or_else(default_cache_dir);
            let output_dir = args
                .output
                .unwrap_or_else(|| default_splat_dir(&cache_dir));
            let opts = SplatOptions {
                version: args.version,
                from: args.from,
                output_dir,
                cache_dir,
                clang_bin: args.clang,
                verbose: args.verbose,
            };
            match run_splat(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xohos] error: {err}");
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
            "xohos",
            "splat",
            "--version",
            "5.0.0-Release",
            "--output",
            "/tmp/ohos",
        ])
        .unwrap();
        match cli.command {
            Commands::Splat(args) => {
                assert_eq!(args.version, "5.0.0-Release");
                assert_eq!(args.output, Some(PathBuf::from("/tmp/ohos")));
            }
            _ => panic!("expected splat"),
        }
    }
}

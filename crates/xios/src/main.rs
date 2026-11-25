//! `xios` CLI entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use xios::splat::{
    DEFAULT_MIN_IOS, SplatOptions, default_cache_dir, default_splat_dir, detect_local_xcode,
    run_splat,
};

/// Utility for extracting and splatting the iOS SDK sysroot for cross-compilation.
#[derive(Debug, Parser)]
#[command(name = "xios", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Extract `iPhoneOS.sdk` (and optionally `iPhoneSimulator.sdk`) from `Xcode.xip` or `Xcode.app`.
    Splat(SplatArgs),
    /// List locally detected iOS / iOS Simulator SDKs (macOS).
    #[command(alias = "detect")]
    List,
}

#[derive(Debug, Args)]
struct SplatArgs {
    /// Path to `Xcode.xip` archive or `Xcode.app` (auto-detected on macOS if omitted).
    #[arg(long)]
    from: Option<PathBuf>,

    /// Destination directory for extracted SDK(s) and `bin/` wrappers (default: `~/.cache/xios/splat`).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Minimum iOS deployment target for generated clang wrappers.
    #[arg(long, default_value = DEFAULT_MIN_IOS)]
    min_ios: String,

    /// Also extract `iPhoneSimulator.sdk` and simulator wrappers.
    #[arg(long)]
    simulator: bool,

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
        Commands::List => match detect_local_xcode() {
            Some(dev_dir) => {
                let mut rows = Vec::new();
                for (platform, prefix) in [
                    ("iPhoneOS.platform", "iPhoneOS"),
                    ("iPhoneSimulator.platform", "iPhoneSimulator"),
                ] {
                    let sdks_dir = dev_dir.join("Platforms").join(platform).join("Developer/SDKs");
                    if let Ok(entries) = std::fs::read_dir(&sdks_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            let name = entry.file_name().to_string_lossy().into_owned();
                            if name.starts_with(prefix)
                                && name.ends_with(".sdk")
                                && std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir())
                            {
                                rows.push(vec![name, path.display().to_string()]);
                            }
                        }
                    }
                }
                xsysroot_core::print_table(&["SDK", "PATH"], rows);
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("[xios] no local Xcode installation found (use `xios splat --from /path/to/Xcode.xip`)");
                ExitCode::SUCCESS
            }
        },
        Commands::Splat(args) => {
            let output_dir = args
                .output
                .unwrap_or_else(|| default_splat_dir(&default_cache_dir()));
            let opts = SplatOptions {
                from: args.from,
                output_dir,
                min_ios: args.min_ios,
                include_simulator: args.simulator,
                clang_bin: args.clang,
                verbose: args.verbose,
            };
            match run_splat(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xios] error: {err}");
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
    fn cli_parses_splat_args() {
        let cli = Cli::try_parse_from([
            "xios",
            "splat",
            "--from",
            "/tmp/Xcode.xip",
            "--simulator",
            "--min-ios",
            "15.0",
        ])
        .unwrap();
        match cli.command {
            Commands::Splat(args) => {
                assert_eq!(args.from, Some(PathBuf::from("/tmp/Xcode.xip")));
                assert!(args.simulator);
                assert_eq!(args.min_ios, "15.0");
            }
            _ => panic!("expected splat"),
        }
    }
}

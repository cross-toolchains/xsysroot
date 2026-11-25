//! `xlinux` CLI entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use xlinux::splat::{SplatOptions, default_cache_dir, default_splat_dir, run_splat};
use xlinux::target::{
    DEBIAN_SUITES, DEFAULT_ALPINE_BRANCH, DEFAULT_GLIBC, DEFAULT_TARGET, KNOWN_TARGETS, LinuxTarget,
};

/// Utility for downloading and splatting minimal Linux glibc and musl sysroots for cross-compilation.
#[derive(Debug, Parser)]
#[command(name = "xlinux", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Download and splat a minimal Linux sysroot + `clang` wrappers into `<output>`.
    Splat(SplatArgs),
    /// List supported Linux targets and glibc/musl baselines.
    List,
}

#[derive(Debug, Args)]
struct SplatArgs {
    /// Target triple (supports optional glibc suffix e.g. `x86_64-unknown-linux-gnu.2.31`).
    #[arg(short, long, default_value = DEFAULT_TARGET)]
    target: String,

    /// glibc version baseline for `*-linux-gnu` targets (`2.28`, `2.31`, `2.36`).
    #[arg(long, default_value = DEFAULT_GLIBC)]
    glibc: String,

    /// Destination directory for `sysroot/` and `bin/` wrappers (default: `<cache-dir>/splat/<target>`).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Cache root directory (defaults to `~/.cache/xlinux`; downloaded packages go to `<cache-dir>/dl`).
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
            let mut rows = Vec::new();
            for t in KNOWN_TARGETS {
                if t.ends_with("-gnu") {
                    for s in DEBIAN_SUITES {
                        rows.push(vec![
                            t.to_string(),
                            "glibc".to_string(),
                            s.glibc.to_string(),
                            format!("{} ({})", s.mirror, s.codename),
                        ]);
                    }
                } else {
                    rows.push(vec![
                        t.to_string(),
                        "musl".to_string(),
                        DEFAULT_ALPINE_BRANCH.to_string(),
                        format!("https://dl-cdn.alpinelinux.org/alpine ({DEFAULT_ALPINE_BRANCH})"),
                    ]);
                }
            }
            xsysroot_core::print_table(&["TARGET", "LIBC", "BASELINE", "UPSTREAM"], rows);
            ExitCode::SUCCESS
        }
        Commands::Splat(args) => {
            let (target, suffix_glibc) = match LinuxTarget::parse(&args.target) {
                Ok(parsed) => parsed,
                Err(err) => {
                    eprintln!("[xlinux] error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let glibc = suffix_glibc.unwrap_or(args.glibc);
            let cache_dir = args.cache_dir.unwrap_or_else(default_cache_dir);
            let output_dir = args
                .output
                .unwrap_or_else(|| default_splat_dir(&cache_dir, &target, &glibc));
            let opts = SplatOptions {
                target,
                glibc,
                output_dir,
                cache_dir,
                clang_bin: args.clang,
                verbose: args.verbose,
            };
            match run_splat(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xlinux] error: {err}");
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
            "xlinux",
            "splat",
            "--target",
            "aarch64-unknown-linux-musl",
            "--output",
            "/tmp/linux",
        ])
        .unwrap();
        match cli.command {
            Commands::Splat(args) => {
                assert_eq!(args.target, "aarch64-unknown-linux-musl");
                assert_eq!(args.output, Some(PathBuf::from("/tmp/linux")));
            }
            _ => panic!("expected splat"),
        }
    }
}

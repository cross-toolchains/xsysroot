//! `xwindows` CLI entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use xwindows::splat::{
    DownloadOptions, SplatOptions, default_cache_dir, default_splat_dir, run_download, run_splat,
};
use xwindows::target::{
    DEFAULT_MINGW_RELEASE, DEFAULT_TARGET, KNOWN_TARGETS, MingwCrt, WindowsAbi, WindowsTarget,
};

/// Utility for downloading and splatting Windows MSVC and MinGW/UCRT sysroots for cross-compilation.
#[derive(Debug, Parser)]
#[command(name = "xwindows", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Download and splat a Windows MSVC or MinGW/UCRT sysroot + `clang` wrappers into `<output>`.
    Splat(SplatArgs),
    /// Download Windows sysroot packages into `<cache-dir>` without splatting.
    Download(DownloadArgs),
    /// List supported Windows target triples and upstream sources.
    List,
}

#[derive(Debug, Args)]
struct DownloadArgs {
    /// Target triple (e.g. `x86_64-pc-windows-msvc`, `aarch64-pc-windows-gnullvm`).
    #[arg(short, long, default_value = DEFAULT_TARGET)]
    target: String,

    /// Cache root directory (defaults to `~/.cache/xwindows`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Accept Microsoft's Windows SDK / MSVC license without prompting (also `XWINDOWS_ACCEPT_LICENSE=1`).
    #[arg(long, env = "XWINDOWS_ACCEPT_LICENSE")]
    accept_license: bool,

    /// Specific Windows SDK version for MSVC targets (defaults to latest).
    #[arg(long)]
    sdk_version: Option<String>,

    /// Specific MSVC CRT version for MSVC targets (defaults to latest).
    #[arg(long)]
    crt_version: Option<String>,

    /// Include Active Template Library (ATL) headers and libs for MSVC targets.
    #[arg(long)]
    include_atl: bool,

    /// Upstream `mstorsjo/llvm-mingw` release tag for MinGW targets.
    #[arg(long, default_value = DEFAULT_MINGW_RELEASE)]
    mingw_release: String,

    /// MinGW C runtime variant (`ucrt` or `msvcrt`).
    #[arg(long, default_value = "ucrt")]
    crt: String,
}

#[derive(Debug, Args)]
struct SplatArgs {
    /// Target triple (e.g. `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`, `x86_64-pc-windows-gnullvm`).
    #[arg(short, long, default_value = DEFAULT_TARGET)]
    target: String,

    /// Destination directory for splatted sysroot and `bin/` wrappers (default: `<cache-dir>/splat/<target>`).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Cache root directory (defaults to `~/.cache/xwindows`).
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Explicit LLVM `clang` binary path (auto-discovered if omitted).
    #[arg(long)]
    clang: Option<PathBuf>,

    /// Accept Microsoft's Windows SDK / MSVC license without prompting (also `XWINDOWS_ACCEPT_LICENSE=1`).
    #[arg(long, env = "XWINDOWS_ACCEPT_LICENSE")]
    accept_license: bool,

    /// Specific Windows SDK version for MSVC targets (defaults to latest).
    #[arg(long)]
    sdk_version: Option<String>,

    /// Specific MSVC CRT version for MSVC targets (defaults to latest).
    #[arg(long)]
    crt_version: Option<String>,

    /// Include Active Template Library (ATL) headers and libs for MSVC targets.
    #[arg(long)]
    include_atl: bool,

    /// Include debug CRT libraries for MSVC targets.
    #[arg(long)]
    include_debug_libs: bool,

    /// Include PDB debug symbols for MSVC targets.
    #[arg(long)]
    include_debug_symbols: bool,

    /// Disable case-sensitivity symlinks in MSVC sysroots.
    #[arg(long)]
    disable_symlinks: bool,

    /// Preserve Microsoft architecture directory names (`x64`/`arm64`) instead of LLVM names (`x86_64`/`aarch64`).
    #[arg(long)]
    preserve_ms_arch_notation: bool,

    /// Use `clang-cl /winsysroot` directory layout for MSVC targets.
    #[arg(long)]
    use_winsysroot_style: bool,

    /// Upstream `mstorsjo/llvm-mingw` release tag for MinGW targets.
    #[arg(long, default_value = DEFAULT_MINGW_RELEASE)]
    mingw_release: String,

    /// MinGW C runtime variant (`ucrt` or `msvcrt`).
    #[arg(long, default_value = "ucrt")]
    crt: String,

    /// Print verbose output (e.g. resolved sysroot and wrapper paths).
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::List => {
            let mut rows = Vec::new();
            for triple in KNOWN_TARGETS {
                if let Ok(t) = WindowsTarget::parse(triple) {
                    let (abi, crt, upstream) = match t.abi {
                        WindowsAbi::Msvc => (
                            "msvc",
                            "MSVC CRT + WinSDK",
                            "Microsoft VS Build Tools manifest (xwin)".to_string(),
                        ),
                        WindowsAbi::GnuLlvm => (
                            "gnullvm",
                            "UCRT / MSVCRT",
                            format!("mstorsjo/llvm-mingw ({DEFAULT_MINGW_RELEASE}, HTTP Range)"),
                        ),
                        WindowsAbi::Gnu => (
                            "gnu",
                            "UCRT / MSVCRT",
                            format!("mstorsjo/llvm-mingw ({DEFAULT_MINGW_RELEASE}, HTTP Range)"),
                        ),
                    };
                    rows.push(vec![
                        triple.to_string(),
                        abi.to_string(),
                        crt.to_string(),
                        upstream,
                    ]);
                }
            }
            xsysroot_core::print_table(&["TARGET", "ABI", "RUNTIME", "UPSTREAM"], rows);
            ExitCode::SUCCESS
        }
        Commands::Download(args) => {
            let target = match WindowsTarget::parse(&args.target) {
                Ok(t) => t,
                Err(err) => {
                    eprintln!("[xwindows] error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let mingw_crt = match MingwCrt::parse(&args.crt) {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("[xwindows] error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let opts = DownloadOptions {
                target,
                cache_dir: args.cache_dir.unwrap_or_else(default_cache_dir),
                accept_license: args.accept_license,
                sdk_version: args.sdk_version,
                crt_version: args.crt_version,
                include_atl: args.include_atl,
                mingw_release: args.mingw_release,
                mingw_crt,
            };
            match run_download(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xwindows] error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::Splat(args) => {
            let target = match WindowsTarget::parse(&args.target) {
                Ok(t) => t,
                Err(err) => {
                    eprintln!("[xwindows] error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let mingw_crt = match MingwCrt::parse(&args.crt) {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("[xwindows] error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let cache_dir = args.cache_dir.unwrap_or_else(default_cache_dir);
            let output_dir = args
                .output
                .unwrap_or_else(|| default_splat_dir(&cache_dir, target));
            let opts = SplatOptions {
                target,
                output_dir,
                cache_dir,
                clang_bin: args.clang,
                accept_license: args.accept_license,
                sdk_version: args.sdk_version,
                crt_version: args.crt_version,
                include_atl: args.include_atl,
                include_debug_libs: args.include_debug_libs,
                include_debug_symbols: args.include_debug_symbols,
                disable_symlinks: args.disable_symlinks,
                preserve_ms_arch_notation: args.preserve_ms_arch_notation,
                use_winsysroot_style: args.use_winsysroot_style,
                mingw_release: args.mingw_release,
                mingw_crt,
                verbose: args.verbose,
            };
            match run_splat(&opts) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("[xwindows] error: {err}");
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
            "xwindows",
            "splat",
            "--target",
            "aarch64-pc-windows-gnullvm",
            "--output",
            "/tmp/win-mingw",
        ])
        .unwrap();
        match cli.command {
            Commands::Splat(args) => {
                assert_eq!(args.target, "aarch64-pc-windows-gnullvm");
                assert_eq!(args.output, Some(PathBuf::from("/tmp/win-mingw")));
            }
            _ => panic!("expected splat"),
        }
    }
}

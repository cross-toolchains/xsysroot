//! Downloads official Apple `CLTools_macOS*_SDK.pkg` archives, extracts
//! `MacOSX.sdk/` in pure Rust, and generates cross-compilation `clang` wrappers.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::catalog::{
    MACOS_TRIPLES, SdkPackage, download_file, fetch_sdk_packages, format_status, make_bytes_bar,
};
use crate::pkg::extract_macos_sdk_pkg;

const APPLE_LICENSE_URL: &str = "https://www.apple.com/legal/sla/";

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub product_id: Option<String>,
    pub cache_dir: PathBuf,
    pub accept_license: bool,
}

#[derive(Debug, Clone)]
pub struct SplatOptions {
    pub product_id: Option<String>,
    pub output_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub clang_bin: Option<PathBuf>,
    pub accept_license: bool,
    pub verbose: bool,
}

pub fn default_cache_dir() -> PathBuf {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cache").join("xmac")
}

pub fn default_splat_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("splat")
}

pub fn run_download(opts: &DownloadOptions) -> Result<(), String> {
    ensure_license_accepted(opts.accept_license)?;
    let (pkg, pkg_path, already_cached) = resolve_and_cache_pkg(opts.product_id.as_deref(), &opts.cache_dir)?;

    if already_cached {
        let cached_len = fs::metadata(&pkg_path).map(|m| m.len()).unwrap_or(pkg.size);
        let pb = make_bytes_bar("SDK", cached_len, "📥", "cached");
        pb.set_position(cached_len);
        pb.finish_with_message(format_status("📥", "cached"));
    } else {
        let pb = make_bytes_bar("SDK", pkg.size, "📥", "downloading");
        download_file(&pkg.url, &pkg_path, &pb)?;
        pb.finish_with_message(format_status("📥", "downloaded"));
    }

    Ok(())
}

pub fn run_splat(opts: &SplatOptions) -> Result<(), String> {
    ensure_license_accepted(opts.accept_license)?;
    let (pkg, pkg_path, already_cached) = resolve_and_cache_pkg(opts.product_id.as_deref(), &opts.cache_dir)?;

    let sdk_pb = if already_cached {
        make_bytes_bar("SDK", 0, "📦", "unpacking")
    } else {
        let pb = make_bytes_bar("SDK", pkg.size, "📥", "downloading");
        download_file(&pkg.url, &pkg_path, &pb)?;
        pb
    };

    let dest_sdk = opts.output_dir.join("MacOSX.sdk");
    if dest_sdk.exists() {
        fs::remove_dir_all(&dest_sdk)
            .map_err(|e| format!("failed to clean {}: {e}", dest_sdk.display()))?;
    }
    fs::create_dir_all(&dest_sdk)
        .map_err(|e| format!("failed to create {}: {e}", dest_sdk.display()))?;

    sdk_pb.set_message(format_status("📦", "unpacking"));
    extract_macos_sdk_pkg(&pkg_path, &dest_sdk, &sdk_pb)?;
    sdk_pb.finish_with_message(format_status("📦", "splatted"));

    if let Ok(clang) = resolve_clang(opts.clang_bin.as_deref()) {
        let bin_dir = opts.output_dir.join("bin");
        generate_wrappers(&bin_dir, &dest_sdk, &clang)?;
        if opts.verbose {
            eprintln!(
                "[xmac] ready: sdk at {}, wrappers at {}",
                dest_sdk.display(),
                bin_dir.display()
            );
        }
    } else if opts.verbose {
        eprintln!("[xmac] ready: sdk at {}", dest_sdk.display());
    }

    Ok(())
}

fn resolve_and_cache_pkg(
    requested_product: Option<&str>,
    cache_dir: &Path,
) -> Result<(SdkPackage, PathBuf, bool), String> {
    let dl_dir = cache_dir.join("dl");
    fs::create_dir_all(&dl_dir)
        .map_err(|e| format!("failed to create download cache dir {}: {e}", dl_dir.display()))?;

    // Fast path: if a cached `.pkg` already matches (or latest cached when no product_id requested),
    // skip downloading the multi-MB Apple catalog.
    if let Some(cached_path) = find_cached_pkg(&dl_dir, requested_product) {
        let cached_len = fs::metadata(&cached_path).map(|m| m.len()).unwrap_or(0);
        let cat_pb = make_bytes_bar("Catalog", 0, "📥", "cached");
        cat_pb.finish_with_message(format_status("📥", "cached"));
        let dummy = SdkPackage {
            product_id: requested_product.unwrap_or("cached").to_string(),
            post_date: String::new(),
            kind: "macOS_SDK".to_string(),
            size: cached_len,
            url: String::new(),
        };
        return Ok((dummy, cached_path, true));
    }

    let pkgs = fetch_sdk_packages(true)?;
    let selected = if let Some(pid) = requested_product {
        pkgs.into_iter()
            .find(|p| p.product_id == pid)
            .ok_or_else(|| format!("product `{pid}` not found in Apple software update catalog"))?
    } else {
        pkgs.into_iter()
            .next()
            .ok_or_else(|| "no macOS SDK packages found in Apple catalog".to_string())?
    };

    let filename = format!("CLTools_{}_{}.pkg", selected.product_id, selected.kind);
    let pkg_path = dl_dir.join(&filename);
    let exists = pkg_path.is_file();
    Ok((selected, pkg_path, exists))
}

fn find_cached_pkg(dl_dir: &Path, requested_product: Option<&str>) -> Option<PathBuf> {
    let mut matches: Vec<PathBuf> = fs::read_dir(dl_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("pkg")
                && match requested_product {
                    Some(pid) => p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.contains(pid)),
                    None => true,
                }
        })
        .collect();
    matches.sort();
    matches.pop()
}

fn ensure_license_accepted(flag_accepted: bool) -> Result<(), String> {
    let accepted = flag_accepted
        || matches!(
            env::var("XMAC_ACCEPT_LICENSE").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
    if accepted {
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        return Err(format!(
            "macOS SDK download requires accepting Apple's Software License Agreement ({APPLE_LICENSE_URL}).\n  \
             Re-run with `--accept-license` or set `XMAC_ACCEPT_LICENSE=1`."
        ));
    }

    eprintln!("Do you accept the license at {APPLE_LICENSE_URL} (yes | no)?");
    let _ = io::stderr().flush();

    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| format!("failed to read license confirmation: {e}"))?;
    if is_affirmative(&answer) {
        eprintln!("license accepted!");
        Ok(())
    } else {
        Err("Apple Software License Agreement was not accepted.".to_string())
    }
}

fn is_affirmative(input: &str) -> bool {
    matches!(input.trim().to_ascii_lowercase().as_str(), "yes" | "y")
}

fn resolve_clang(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Ok(p.to_path_buf());
        }
        return Err(format!("specified --clang does not exist: {}", p.display()));
    }
    let exe = if cfg!(windows) { "clang.exe" } else { "clang" };
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/opt/llvm/bin"),
        PathBuf::from("/usr/local/opt/llvm/bin"),
    ];
    if let Some(path_var) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path_var));
    }
    for dir in dirs {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("clang not found in PATH or Homebrew LLVM (pass --clang <PATH>)".to_string())
}

fn generate_wrappers(bin_dir: &Path, sdk_dir: &Path, clang: &Path) -> Result<(), String> {
    fs::create_dir_all(bin_dir)
        .map_err(|e| format!("failed to create {}: {e}", bin_dir.display()))?;

    let clang_dir = clang.parent().unwrap_or_else(|| Path::new(""));
    let clangxx = clang_dir.join(if cfg!(windows) { "clang++.exe" } else { "clang++" });
    let clangxx_str = if clangxx.is_file() {
        clangxx.display().to_string()
    } else {
        clang.display().to_string()
    };
    let clang_str = clang.display().to_string();
    let sdk_str = sdk_dir.display().to_string();

    for triple in MACOS_TRIPLES {
        write_shim(
            &bin_dir.join(format!("{triple}-clang")),
            &clang_str,
            triple,
            &sdk_str,
        )?;
        write_shim(
            &bin_dir.join(format!("{triple}-clang++")),
            &clangxx_str,
            triple,
            &sdk_str,
        )?;
    }
    Ok(())
}

fn write_shim(path: &Path, compiler: &str, target: &str, sdk: &str) -> Result<(), String> {
    let contents = format!(
        "#!/bin/sh\nexec \"{compiler}\" --target={target} -isysroot \"{sdk}\" -fuse-ld=lld \"$@\"\n"
    );
    fs::write(path, contents)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|e| format!("failed to stat {}: {e}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .map_err(|e| format!("failed to chmod {}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_license_confirmation_answers() {
        assert!(is_affirmative("yes\n"));
        assert!(is_affirmative("Y"));
        assert!(!is_affirmative("no"));
    }

    #[test]
    fn generates_macos_shims_for_arm64_and_x86_64() {
        let tmp = std::env::temp_dir().join("xmac_shim_test");
        let _ = fs::remove_dir_all(&tmp);
        let bin_dir = tmp.join("bin");
        let sdk_dir = tmp.join("MacOSX.sdk");
        fs::create_dir_all(&sdk_dir).unwrap();

        generate_wrappers(&bin_dir, &sdk_dir, Path::new("/usr/bin/clang")).unwrap();
        let arm_cc = bin_dir.join("aarch64-apple-darwin-clang");
        assert!(arm_cc.is_file());
        let text = fs::read_to_string(&arm_cc).unwrap();
        assert!(text.contains("--target=aarch64-apple-darwin"));
        assert!(text.contains("-isysroot"));

        let _ = fs::remove_dir_all(&tmp);
    }
}

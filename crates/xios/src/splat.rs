//! Extracts `iPhoneOS.sdk` (and optionally `iPhoneSimulator.sdk`) from a local
//! `Xcode.xip` archive or `Xcode.app` bundle and generates LLVM `clang` wrappers.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

use crate::xip::{create_symlink, extract_ios_sdks_from_xip};

pub const DEFAULT_MIN_IOS: &str = "13.0";

#[derive(Debug, Clone)]
pub struct SplatOptions {
    pub from: Option<PathBuf>,
    pub output_dir: PathBuf,
    pub min_ios: String,
    pub include_simulator: bool,
    pub clang_bin: Option<PathBuf>,
    pub verbose: bool,
}

pub fn default_cache_dir() -> PathBuf {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cache").join("xios")
}

pub fn default_splat_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("splat")
}

pub fn format_status(icon: &str, label: &str) -> String {
    format!("{icon}  {label:>11}")
}

fn make_bytes_bar(prefix: &'static str, len: u64, icon: &str, label: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    if let Ok(style) = ProgressStyle::with_template(
        "{spinner:.green} {prefix:<9.bold} [{elapsed:>3}] {wide_bar:.green} {binary_bytes:>10}/{binary_total_bytes:>10}  {msg}",
    ) {
        pb.set_style(style.progress_chars("█▄ "));
    }
    pb.set_prefix(prefix);
    pb.set_message(format_status(icon, label));
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Auto-detects a local `Xcode.app` on macOS if installed.
pub fn detect_local_xcode() -> Option<PathBuf> {
    if let Ok(dev_dir) = env::var("DEVELOPER_DIR") {
        let p = PathBuf::from(dev_dir);
        if p.join("Platforms/iPhoneOS.platform").is_dir() {
            return Some(p);
        }
    }
    let candidates = [
        PathBuf::from("/Applications/Xcode.app/Contents/Developer"),
        PathBuf::from("/Applications/Xcode-beta.app/Contents/Developer"),
    ];
    candidates
        .into_iter()
        .find(|p| p.join("Platforms/iPhoneOS.platform").is_dir())
}

pub fn run_splat(opts: &SplatOptions) -> Result<(), String> {
    let source = match &opts.from {
        Some(p) => p.clone(),
        None => detect_local_xcode().ok_or_else(|| {
            "no Xcode source specified and no local Xcode.app detected.\n  \
             Pass `--from /path/to/Xcode.xip` (or `--from /Applications/Xcode.app`)."
                .to_string()
        })?,
    };

    fs::create_dir_all(&opts.output_dir)
        .map_err(|e| format!("failed to create {}: {e}", opts.output_dir.display()))?;

    let device_dest = opts.output_dir.join("iPhoneOS.sdk");
    let sim_dest = opts.output_dir.join("iPhoneSimulator.sdk");
    if device_dest.exists() {
        fs::remove_dir_all(&device_dest)
            .map_err(|e| format!("failed to clean {}: {e}", device_dest.display()))?;
    }
    if opts.include_simulator && sim_dest.exists() {
        fs::remove_dir_all(&sim_dest)
            .map_err(|e| format!("failed to clean {}: {e}", sim_dest.display()))?;
    }

    if source.is_file() {
        let pb = make_bytes_bar("iPhoneOS", 0, "📦", "unpacking");
        extract_ios_sdks_from_xip(&source, &opts.output_dir, opts.include_simulator, &pb)?;
        pb.finish_with_message(format_status("📦", "splatted"));
    } else if source.is_dir() {
        let dev_dir = resolve_developer_dir(&source)?;
        let device_src = find_sdk_dir(&dev_dir.join("Platforms/iPhoneOS.platform/Developer/SDKs"), "iPhoneOS")?;
        let device_bytes = dir_byte_size(&device_src);
        let pb = make_bytes_bar("iPhoneOS", device_bytes, "📦", "unpacking");
        copy_tree(&device_src, &device_dest, &pb)?;
        pb.finish_with_message(format_status("📦", "splatted"));

        if opts.include_simulator {
            let sim_src = find_sdk_dir(
                &dev_dir.join("Platforms/iPhoneSimulator.platform/Developer/SDKs"),
                "iPhoneSimulator",
            )?;
            let sim_bytes = dir_byte_size(&sim_src);
            let sim_pb = make_bytes_bar("Simulator", sim_bytes, "📦", "unpacking");
            copy_tree(&sim_src, &sim_dest, &sim_pb)?;
            sim_pb.finish_with_message(format_status("📦", "splatted"));
        }
    } else {
        return Err(format!("source path does not exist: {}", source.display()));
    }

    if let Ok(clang) = resolve_clang(opts.clang_bin.as_deref()) {
        let bin_dir = opts.output_dir.join("bin");
        generate_wrappers(
            &bin_dir,
            &device_dest,
            opts.include_simulator.then_some(sim_dest.as_path()),
            &clang,
            &opts.min_ios,
        )?;
        if opts.verbose {
            eprintln!(
                "[xios] ready: sdk at {}, wrappers at {}",
                device_dest.display(),
                bin_dir.display()
            );
        }
    } else if opts.verbose {
        eprintln!("[xios] ready: sdk at {}", device_dest.display());
    }

    Ok(())
}

fn resolve_developer_dir(dir: &Path) -> Result<PathBuf, String> {
    if dir.join("Platforms/iPhoneOS.platform").is_dir() {
        return Ok(dir.to_path_buf());
    }
    let nested = dir.join("Contents/Developer");
    if nested.join("Platforms/iPhoneOS.platform").is_dir() {
        return Ok(nested);
    }
    Err(format!(
        "directory `{}` does not look like an Xcode.app or Contents/Developer directory",
        dir.display()
    ))
}

fn find_sdk_dir(sdks_root: &Path, prefix: &str) -> Result<PathBuf, String> {
    let entries = fs::read_dir(sdks_root)
        .map_err(|e| format!("failed reading {}: {e}", sdks_root.display()))?;
    let mut best: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with(prefix) || !name_str.ends_with(".sdk") {
            continue;
        }
        // Prefer the real versioned directory over the unversioned symlink.
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.is_dir()
        {
            best = Some(path);
        }
    }
    best.ok_or_else(|| format!("no {prefix}*.sdk directory found in {}", sdks_root.display()))
}

fn dir_byte_size(root: &Path) -> u64 {
    let mut total: u64 = 0;
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        } else if meta.is_dir() {
            total += dir_byte_size(&path);
        } else {
            total += meta.len();
        }
    }
    total
}

fn copy_tree(src: &Path, dst: &Path, pb: &ProgressBar) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("failed creating {}: {e}", dst.display()))?;
    let entries =
        fs::read_dir(src).map_err(|e| format!("failed reading {}: {e}", src.display()))?;
    for entry in entries.flatten() {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let meta = fs::symlink_metadata(&src_path)
            .map_err(|e| format!("failed stating {}: {e}", src_path.display()))?;
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&src_path)
                .map_err(|e| format!("failed reading symlink {}: {e}", src_path.display()))?;
            create_symlink(&target.to_string_lossy(), &dst_path)?;
        } else if meta.is_dir() {
            copy_tree(&src_path, &dst_path, pb)?;
        } else {
            let copied = fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "failed copying {} -> {}: {e}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
            pb.inc(copied);
        }
    }
    Ok(())
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

fn generate_wrappers(
    bin_dir: &Path,
    device_sdk: &Path,
    sim_sdk: Option<&Path>,
    clang: &Path,
    min_ios: &str,
) -> Result<(), String> {
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
    let device_str = device_sdk.display().to_string();

    write_shim(
        &bin_dir.join("aarch64-apple-ios-clang"),
        &clang_str,
        "aarch64-apple-ios",
        &device_str,
        &format!("-miphoneos-version-min={min_ios}"),
    )?;
    write_shim(
        &bin_dir.join("aarch64-apple-ios-clang++"),
        &clangxx_str,
        "aarch64-apple-ios",
        &device_str,
        &format!("-miphoneos-version-min={min_ios}"),
    )?;

    if let Some(sim) = sim_sdk {
        let sim_str = sim.display().to_string();
        let sim_flag = format!("-mios-simulator-version-min={min_ios}");
        for (shim_name, target) in [
            ("aarch64-apple-ios-sim", "aarch64-apple-ios-simulator"),
            ("x86_64-apple-ios", "x86_64-apple-ios-simulator"),
        ] {
            write_shim(
                &bin_dir.join(format!("{shim_name}-clang")),
                &clang_str,
                target,
                &sim_str,
                &sim_flag,
            )?;
            write_shim(
                &bin_dir.join(format!("{shim_name}-clang++")),
                &clangxx_str,
                target,
                &sim_str,
                &sim_flag,
            )?;
        }
    }

    Ok(())
}

fn write_shim(
    path: &Path,
    compiler: &str,
    target: &str,
    sdk: &str,
    min_ver_flag: &str,
) -> Result<(), String> {
    let contents = format!(
        "#!/bin/sh\nexec \"{compiler}\" --target={target} -isysroot \"{sdk}\" {min_ver_flag} -fuse-ld=lld \"$@\"\n"
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
    fn generates_ios_device_and_simulator_shims() {
        let tmp = std::env::temp_dir().join("xios_shim_test");
        let _ = fs::remove_dir_all(&tmp);
        let bin_dir = tmp.join("bin");
        let dev_sdk = tmp.join("iPhoneOS.sdk");
        let sim_sdk = tmp.join("iPhoneSimulator.sdk");
        fs::create_dir_all(&dev_sdk).unwrap();
        fs::create_dir_all(&sim_sdk).unwrap();

        generate_wrappers(
            &bin_dir,
            &dev_sdk,
            Some(&sim_sdk),
            Path::new("/usr/bin/clang"),
            "15.0",
        )
        .unwrap();

        let dev_cc = fs::read_to_string(bin_dir.join("aarch64-apple-ios-clang")).unwrap();
        assert!(dev_cc.contains("--target=aarch64-apple-ios"));
        assert!(dev_cc.contains("-miphoneos-version-min=15.0"));

        let sim_cc = fs::read_to_string(bin_dir.join("aarch64-apple-ios-sim-clang")).unwrap();
        assert!(sim_cc.contains("--target=aarch64-apple-ios-simulator"));
        assert!(sim_cc.contains("-mios-simulator-version-min=15.0"));

        let _ = fs::remove_dir_all(&tmp);
    }
}

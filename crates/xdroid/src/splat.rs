//! Downloads official Android NDK archives via HTTP Range, extracts only
//! `sysroot/` using `zip`, and generates cross-compilation `clang` / `clang++` shims.

use std::env;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};

use indicatif::ProgressBar;
use zip::ZipArchive;

use crate::ndk::{ANDROID_TRIPLES, host_ndk_os, ndk_download_url};
use crate::remote_zip::{fetch_partial_zip, format_status, local_zip_cd_size, make_bytes_bar};

const ANDROID_LICENSE_URL: &str = "https://developer.android.com/studio/terms";

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub ndk_version: String,
    pub cache_dir: PathBuf,
    pub accept_license: bool,
}

#[derive(Debug, Clone)]
pub struct SplatOptions {
    pub ndk_version: String,
    pub api_level: u32,
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
    PathBuf::from(home).join(".cache").join("xdroid")
}

pub fn default_splat_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("splat")
}

pub fn run_download(opts: &DownloadOptions) -> Result<(), String> {
    ensure_license_accepted(opts.accept_license)?;

    let host_os = host_ndk_os()?;
    let url = ndk_download_url(&opts.ndk_version, host_os);
    let dl_dir = opts.cache_dir.join("dl");
    fs::create_dir_all(&dl_dir).map_err(|e| {
        format!(
            "failed to create download cache dir {}: {e}",
            dl_dir.display()
        )
    })?;

    let archive_name = format!("android-ndk-{}-{host_os}-sysroot.zip", opts.ndk_version);
    let archive_path = dl_dir.join(&archive_name);

    if !archive_path.is_file() {
        let sysroot_pb = fetch_partial_zip(&url, &archive_path, |name| {
            name.contains("/toolchains/llvm/prebuilt/") && name.contains("/sysroot/")
        })?;
        sysroot_pb.finish_with_message(format_status("📥", "downloaded"));
    } else {
        let cd_len = local_zip_cd_size(&archive_path);
        let index_pb = make_bytes_bar("Index", cd_len, "📥", "cached");
        index_pb.set_position(cd_len);
        index_pb.finish_with_message(format_status("📥", "cached"));

        let cached_len = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
        let sysroot_pb = make_bytes_bar("Sysroot", cached_len, "📥", "cached");
        sysroot_pb.set_position(cached_len);
        sysroot_pb.finish_with_message(format_status("📥", "cached"));
    }

    Ok(())
}

pub fn run_splat(opts: &SplatOptions) -> Result<(), String> {
    ensure_license_accepted(opts.accept_license)?;

    let host_os = host_ndk_os()?;
    let url = ndk_download_url(&opts.ndk_version, host_os);
    let dl_dir = opts.cache_dir.join("dl");
    fs::create_dir_all(&dl_dir).map_err(|e| {
        format!(
            "failed to create download cache dir {}: {e}",
            dl_dir.display()
        )
    })?;

    let archive_name = format!("android-ndk-{}-{host_os}-sysroot.zip", opts.ndk_version);
    let archive_path = dl_dir.join(&archive_name);

    let sysroot_pb = if !archive_path.is_file() {
        fetch_partial_zip(&url, &archive_path, |name| {
            name.contains("/toolchains/llvm/prebuilt/") && name.contains("/sysroot/")
        })?
    } else {
        let cd_len = local_zip_cd_size(&archive_path);
        let index_pb = make_bytes_bar("Index", cd_len, "📥", "cached");
        index_pb.set_position(cd_len);
        index_pb.finish_with_message(format_status("📥", "cached"));
        make_bytes_bar("Sysroot", 0, "📦", "unpacking")
    };

    let dest_sysroot = opts.output_dir.join("sysroot");
    if dest_sysroot.exists() {
        fs::remove_dir_all(&dest_sysroot)
            .map_err(|e| format!("failed to clean {}: {e}", dest_sysroot.display()))?;
    }
    fs::create_dir_all(&dest_sysroot)
        .map_err(|e| format!("failed to create {}: {e}", dest_sysroot.display()))?;

    extract_sysroot_zip(&archive_path, &dest_sysroot, &sysroot_pb)?;

    let clang = resolve_clang(opts.clang_bin.as_deref())?;
    let bin_dir = opts.output_dir.join("bin");
    generate_wrappers(&bin_dir, &dest_sysroot, &clang, opts.api_level)?;

    if opts.verbose {
        eprintln!(
            "[xdroid] ready: sysroot at {}, wrappers at {}",
            dest_sysroot.display(),
            bin_dir.display()
        );
    }
    Ok(())
}

fn ensure_license_accepted(flag_accepted: bool) -> Result<(), String> {
    let accepted = flag_accepted
        || matches!(
            env::var("XDROID_ACCEPT_LICENSE").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
    if accepted {
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        return Err(format!(
            "Android NDK download requires accepting the Android SDK License Agreement ({ANDROID_LICENSE_URL}).\n  \
             Re-run with `--accept-license` or set `XDROID_ACCEPT_LICENSE=1`."
        ));
    }

    eprintln!("Do you accept the license at {ANDROID_LICENSE_URL} (yes | no)?");
    let _ = io::stderr().flush();

    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| format!("failed to read license confirmation: {e}"))?;
    if is_affirmative(&answer) {
        eprintln!("license accepted!");
        Ok(())
    } else {
        Err("Android SDK License Agreement was not accepted.".to_string())
    }
}

fn is_affirmative(input: &str) -> bool {
    matches!(input.trim().to_ascii_lowercase().as_str(), "yes" | "y")
}

fn extract_sysroot_zip(
    archive_path: &Path,
    dest_sysroot: &Path,
    pb: &ProgressBar,
) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|e| format!("failed to open {}: {e}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("failed to read zip {}: {e}", archive_path.display()))?;

    let total_entries = archive.len();
    let mut total_uncompressed: u64 = 0;
    for i in 0..total_entries {
        if let Ok(entry) = archive.by_index_raw(i)
            && entry.name().contains("/sysroot/")
        {
            total_uncompressed = total_uncompressed.saturating_add(entry.size());
        }
    }

    pb.reset();
    pb.set_length(total_uncompressed);
    pb.set_message(format_status("📦", "unpacking"));

    let mut extracted = 0usize;
    let mut symlinks = xsysroot_core::SymlinkQueue::new();
    for i in 0..total_entries {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("failed reading zip entry #{i}: {e}"))?;
        let Some((_, rel_str)) = entry.name().split_once("/sysroot/") else {
            continue;
        };
        if rel_str.is_empty() {
            continue;
        }
        let rel_path = sanitize_relative_path(rel_str)?;
        let out_path = dest_sysroot.join(&rel_path);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("failed to create dir {}: {e}", out_path.display()))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create dir {}: {e}", parent.display()))?;
        }

        if entry.is_symlink() {
            let entry_size = entry.size();
            let target = io::read_to_string(&mut entry).map_err(|e| {
                format!(
                    "failed reading symlink target for {}: {e}",
                    rel_path.display()
                )
            })?;
            symlinks.add(&target, &out_path)?;
            pb.inc(entry_size);
            extracted += 1;
            continue;
        }

        let mut out_file = File::create(&out_path)
            .map_err(|e| format!("failed to create {}: {e}", out_path.display()))?;
        let written = io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("failed extracting {}: {e}", out_path.display()))?;
        pb.inc(written);

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode & 0o777));
        }

        extracted += 1;
    }
    symlinks.finish()?;
    pb.finish_with_message(format_status("📦", "splatted"));

    if extracted == 0 {
        return Err(format!(
            "no sysroot/ entries extracted from {}",
            archive_path.display()
        ));
    }
    Ok(())
}

fn sanitize_relative_path(rel: &str) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return Err(format!("unsafe archive entry path: `{rel}`")),
        }
    }
    Ok(clean)
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
    sysroot: &Path,
    clang: &Path,
    api_level: u32,
) -> Result<(), String> {
    fs::create_dir_all(bin_dir)
        .map_err(|e| format!("failed to create {}: {e}", bin_dir.display()))?;

    let clang_dir = clang.parent().unwrap_or_else(|| Path::new(""));
    let clangxx = clang_dir.join(if cfg!(windows) {
        "clang++.exe"
    } else {
        "clang++"
    });
    let clangxx_str = if clangxx.is_file() {
        clangxx.display().to_string()
    } else {
        clang.display().to_string()
    };
    let clang_str = clang.display().to_string();
    let sysroot_str = sysroot.display().to_string();

    for triple in ANDROID_TRIPLES {
        let target_arg = format!("{triple}{api_level}");
        let args = format!("--target={target_arg} --sysroot=\"{sysroot_str}\" -fuse-ld=lld");
        xsysroot_core::write_clang_wrapper(
            bin_dir,
            &format!("{target_arg}-clang"),
            &clang_str,
            &args,
        )?;
        xsysroot_core::write_clang_wrapper(
            bin_dir,
            &format!("{target_arg}-clang++"),
            &clangxx_str,
            &args,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_license_confirmation_answers() {
        assert!(is_affirmative("yes\n"));
        assert!(is_affirmative("  YES \r\n"));
        assert!(is_affirmative("y"));
        assert!(!is_affirmative("no"));
        assert!(!is_affirmative(""));
    }

    #[test]
    fn generates_android_shims_for_all_abis() {
        let tmp = std::env::temp_dir().join("xdroid_shim_test");
        let _ = fs::remove_dir_all(&tmp);
        let bin_dir = tmp.join("bin");
        let sysroot = tmp.join("sysroot");
        fs::create_dir_all(&sysroot).unwrap();

        generate_wrappers(&bin_dir, &sysroot, Path::new("/usr/bin/clang"), 24).unwrap();
        let aarch64_cc = xsysroot_core::wrapper_path(&bin_dir, "aarch64-linux-android24-clang");
        assert!(aarch64_cc.is_file());
        let text = fs::read_to_string(&aarch64_cc).unwrap();
        assert!(text.contains("--target=aarch64-linux-android24"));
        assert!(text.contains("--sysroot="));

        let _ = fs::remove_dir_all(&tmp);
    }
}

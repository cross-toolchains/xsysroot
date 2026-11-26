//! Downloads or reads OpenHarmony `native-*.zip`, extracts only `native/sysroot/`,
//! and generates cross-compilation `clang` / `clang++` wrappers.

use std::env;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use indicatif::ProgressBar;
use zip::ZipArchive;

use crate::sdk::{
    OHOS_TARGETS, extract_native_zip_from_targz_file, fetch_native_zip_from_url, format_status,
    make_bytes_bar, ohos_sdk_url,
};

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub version: String,
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SplatOptions {
    pub version: String,
    pub from: Option<PathBuf>,
    pub output_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub clang_bin: Option<PathBuf>,
    pub verbose: bool,
}

pub fn default_cache_dir() -> PathBuf {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cache").join("xohos")
}

pub fn default_splat_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("splat")
}

pub fn run_download(opts: &DownloadOptions) -> Result<(), String> {
    let dl_dir = opts.cache_dir.join("dl");
    fs::create_dir_all(&dl_dir).map_err(|e| {
        format!(
            "failed to create download cache dir {}: {e}",
            dl_dir.display()
        )
    })?;

    let archive_path = dl_dir.join(format!("ohos-native-{}.zip", opts.version));
    if archive_path.is_file() {
        let len = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
        let pb = make_bytes_bar("Sysroot", len, "📥", "cached");
        pb.set_position(len);
        pb.finish_with_message(format_status("📥", "cached"));
        return Ok(());
    }

    let url = ohos_sdk_url(&opts.version);
    let pb = make_bytes_bar("Sysroot", 0, "📥", "downloading");
    fetch_native_zip_from_url(&url, &archive_path, &pb)?;
    let saved_len = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
    pb.set_length(saved_len);
    pb.set_position(saved_len);
    pb.finish_with_message(format_status("📥", "downloaded"));
    Ok(())
}

pub fn run_splat(opts: &SplatOptions) -> Result<(), String> {
    let dl_dir = opts.cache_dir.join("dl");
    fs::create_dir_all(&dl_dir).map_err(|e| {
        format!(
            "failed to create download cache dir {}: {e}",
            dl_dir.display()
        )
    })?;

    let (native_zip, pb) = if let Some(from_path) = &opts.from {
        if !from_path.is_file() {
            return Err(format!(
                "specified --from does not exist: {}",
                from_path.display()
            ));
        }
        if from_path.extension().and_then(|e| e.to_str()) == Some("zip") {
            (
                from_path.clone(),
                make_bytes_bar("Sysroot", 0, "📦", "unpacking"),
            )
        } else {
            let cached_zip = dl_dir.join(format!("ohos-native-{}.zip", opts.version));
            let pb = make_bytes_bar("Sysroot", 0, "📦", "unpacking");
            extract_native_zip_from_targz_file(from_path, &cached_zip, &pb)?;
            (cached_zip, pb)
        }
    } else {
        let cached_zip = dl_dir.join(format!("ohos-native-{}.zip", opts.version));
        if cached_zip.is_file() {
            (cached_zip, make_bytes_bar("Sysroot", 0, "📦", "unpacking"))
        } else {
            let url = ohos_sdk_url(&opts.version);
            let pb = make_bytes_bar("Sysroot", 0, "📥", "downloading");
            fetch_native_zip_from_url(&url, &cached_zip, &pb)?;
            (cached_zip, pb)
        }
    };

    let dest_sysroot = opts.output_dir.join("sysroot");
    let dest_resource_dir = opts.output_dir.join("resource-dir");
    if dest_sysroot.exists() {
        fs::remove_dir_all(&dest_sysroot)
            .map_err(|e| format!("failed to clean {}: {e}", dest_sysroot.display()))?;
    }
    if dest_resource_dir.exists() {
        fs::remove_dir_all(&dest_resource_dir)
            .map_err(|e| format!("failed to clean {}: {e}", dest_resource_dir.display()))?;
    }
    fs::create_dir_all(&dest_sysroot)
        .map_err(|e| format!("failed to create {}: {e}", dest_sysroot.display()))?;
    fs::create_dir_all(&dest_resource_dir)
        .map_err(|e| format!("failed to create {}: {e}", dest_resource_dir.display()))?;

    extract_sysroot_zip(&native_zip, &dest_sysroot, &dest_resource_dir, &pb)?;

    if let Ok(clang) = resolve_clang(opts.clang_bin.as_deref()) {
        let bin_dir = opts.output_dir.join("bin");
        generate_wrappers(&bin_dir, &dest_sysroot, &dest_resource_dir, &clang)?;
        if opts.verbose {
            eprintln!(
                "[xohos] ready: sysroot at {}, wrappers at {}",
                dest_sysroot.display(),
                bin_dir.display()
            );
        }
    } else if opts.verbose {
        eprintln!("[xohos] ready: sysroot at {}", dest_sysroot.display());
    }

    Ok(())
}

/// Classifies OpenHarmony LLVM runtime entries inside `native-*.zip` that are
/// required to link full C/C++/Rust binaries with a generic host LLVM `clang`.
enum OhosRuntimeTarget {
    /// Extract into `<resource-dir>/lib/<triple>/<file>` (and mirror CRT objects into `sysroot`).
    ResourceLib {
        triple: &'static str,
        file: &'static str,
    },
    /// Extract into `<sysroot>/usr/lib/<triple>/libunwind.a`.
    SysrootLib {
        triple: &'static str,
        file: &'static str,
    },
}

fn classify_ohos_runtime_entry(name: &str) -> Option<OhosRuntimeTarget> {
    const TRIPLES: &[&str] = &["aarch64-linux-ohos", "arm-linux-ohos", "x86_64-linux-ohos"];
    if let Some((_, after)) = name.split_once("/llvm/lib/clang/")
        && let Some((_, rest)) = after.split_once("/lib/")
    {
        for &triple in TRIPLES {
            if let Some(file) = rest.strip_prefix(&format!("{triple}/")) {
                match file {
                    "libclang_rt.builtins.a" => {
                        return Some(OhosRuntimeTarget::ResourceLib {
                            triple,
                            file: "libclang_rt.builtins.a",
                        });
                    }
                    "clang_rt.crtbegin.o" => {
                        return Some(OhosRuntimeTarget::ResourceLib {
                            triple,
                            file: "clang_rt.crtbegin.o",
                        });
                    }
                    "clang_rt.crtend.o" => {
                        return Some(OhosRuntimeTarget::ResourceLib {
                            triple,
                            file: "clang_rt.crtend.o",
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some((_, rest)) = name.split_once("/llvm/lib/") {
        for &triple in TRIPLES {
            if rest == format!("{triple}/libunwind.a") {
                return Some(OhosRuntimeTarget::SysrootLib {
                    triple,
                    file: "libunwind.a",
                });
            }
        }
    }
    None
}

fn extract_sysroot_zip(
    archive_path: &Path,
    dest_sysroot: &Path,
    dest_resource_dir: &Path,
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
            && (strip_ohos_sysroot_prefix(entry.name()).is_some()
                || classify_ohos_runtime_entry(entry.name()).is_some())
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

        if let Some(rt) = classify_ohos_runtime_entry(entry.name()) {
            match rt {
                OhosRuntimeTarget::ResourceLib { triple, file } => {
                    let out_path = dest_resource_dir.join("lib").join(triple).join(file);
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
                    }
                    let mut out_file = File::create(&out_path)
                        .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
                    let written = io::copy(&mut entry, &mut out_file)
                        .map_err(|e| format!("failed extracting {}: {e}", out_path.display()))?;
                    pb.inc(written);

                    // Also place CRT begin/end objects under sysroot/usr/lib/<triple>/ as
                    // crtbegin{,S,T}.o / crtend{,S,T}.o for host clang drivers that look there.
                    let sysroot_lib = dest_sysroot.join("usr/lib").join(triple);
                    fs::create_dir_all(&sysroot_lib)
                        .map_err(|e| format!("failed creating {}: {e}", sysroot_lib.display()))?;
                    if file == "clang_rt.crtbegin.o" {
                        for alias in ["crtbegin.o", "crtbeginS.o", "crtbeginT.o"] {
                            let _ = fs::copy(&out_path, sysroot_lib.join(alias));
                        }
                    } else if file == "clang_rt.crtend.o" {
                        for alias in ["crtend.o", "crtendS.o", "crtendT.o"] {
                            let _ = fs::copy(&out_path, sysroot_lib.join(alias));
                        }
                    }
                }
                OhosRuntimeTarget::SysrootLib { triple, file } => {
                    let out_path = dest_sysroot.join("usr/lib").join(triple).join(file);
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
                    }
                    let mut out_file = File::create(&out_path)
                        .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
                    let written = io::copy(&mut entry, &mut out_file)
                        .map_err(|e| format!("failed extracting {}: {e}", out_path.display()))?;
                    pb.inc(written);
                }
            }
            extracted += 1;
            continue;
        }

        let Some(rel_str) = strip_ohos_sysroot_prefix(entry.name()) else {
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
            "no sysroot/ entries found inside {}",
            archive_path.display()
        ));
    }
    Ok(())
}

pub fn strip_ohos_sysroot_prefix(name: &str) -> Option<&str> {
    if let Some((_, rel)) = name.split_once("/sysroot/") {
        return Some(rel);
    }
    if let Some(rel) = name.strip_prefix("sysroot/") {
        return Some(rel);
    }
    None
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

fn link_host_clang_resource_headers(clang: &Path, resource_dir: &Path) {
    let Ok(output) = std::process::Command::new(clang)
        .arg("-print-resource-dir")
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let host_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if host_dir.is_empty() {
        return;
    }
    let host_include = Path::new(&host_dir).join("include");
    if host_include.is_dir() {
        let dst_include = resource_dir.join("include");
        let _ = xsysroot_core::create_symlink(&host_include.to_string_lossy(), &dst_include);
    }
}

fn generate_wrappers(
    bin_dir: &Path,
    sysroot: &Path,
    resource_dir: &Path,
    clang: &Path,
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

    let resource_flag = if resource_dir.join("lib").is_dir() {
        link_host_clang_resource_headers(clang, resource_dir);
        format!(" -resource-dir \"{}\"", resource_dir.display())
    } else {
        String::new()
    };

    for &(wrapper_prefix, clang_target) in OHOS_TARGETS {
        let args = format!(
            "--target={clang_target} --sysroot=\"{sysroot_str}\"{resource_flag} -fuse-ld=lld"
        );
        xsysroot_core::write_clang_wrapper(
            bin_dir,
            &format!("{wrapper_prefix}-clang"),
            &clang_str,
            &args,
        )?;
        xsysroot_core::write_clang_wrapper(
            bin_dir,
            &format!("{wrapper_prefix}-clang++"),
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
    fn strips_ohos_sysroot_prefix() {
        assert_eq!(
            strip_ohos_sysroot_prefix("native/sysroot/usr/include/stdio.h"),
            Some("usr/include/stdio.h")
        );
        assert_eq!(
            strip_ohos_sysroot_prefix("sysroot/usr/lib/aarch64-linux-ohos/libc.so"),
            Some("usr/lib/aarch64-linux-ohos/libc.so")
        );
        assert_eq!(strip_ohos_sysroot_prefix("native/llvm/bin/clang"), None);
    }

    #[test]
    fn generates_ohos_shims_for_all_abis() {
        let tmp = std::env::temp_dir().join("xohos_shim_test");
        let _ = fs::remove_dir_all(&tmp);
        let bin_dir = tmp.join("bin");
        let sysroot = tmp.join("sysroot");
        let resource_dir = tmp.join("resource-dir");
        fs::create_dir_all(&sysroot).unwrap();

        generate_wrappers(
            &bin_dir,
            &sysroot,
            &resource_dir,
            Path::new("/usr/bin/clang"),
        )
        .unwrap();
        let arm64 = fs::read_to_string(xsysroot_core::wrapper_path(
            &bin_dir,
            "aarch64-unknown-linux-ohos-clang",
        ))
        .unwrap();
        assert!(arm64.contains("--target=aarch64-linux-ohos"));
        assert!(arm64.contains("--sysroot="));

        let _ = fs::remove_dir_all(&tmp);
    }
}

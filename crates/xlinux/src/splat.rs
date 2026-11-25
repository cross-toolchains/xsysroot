//! Downloads and splats minimal Linux `glibc` (Debian) or `musl` (Alpine)
//! sysroots and generates cross-compilation `clang` / `clang++` wrappers.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::archive::{extract_apk, extract_deb, fixup_glibc_linker_scripts};
use crate::repo::{
    RemotePkg, download_to_file, format_status, make_bytes_bar, resolve_alpine_packages,
    resolve_debian_packages,
};
use crate::target::{Libc, LinuxTarget, resolve_debian_suite};

#[derive(Debug, Clone)]
pub struct SplatOptions {
    pub target: LinuxTarget,
    pub glibc: String,
    pub output_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub clang_bin: Option<PathBuf>,
    pub verbose: bool,
}

pub fn default_cache_dir() -> PathBuf {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cache").join("xlinux")
}

pub fn default_splat_dir(cache_dir: &Path, target: &LinuxTarget, glibc: &str) -> PathBuf {
    let tag = match target.libc {
        Libc::Gnu => format!("{}.{glibc}", target.triple),
        Libc::Musl => target.triple.clone(),
    };
    cache_dir.join("splat").join(tag)
}

pub fn run_splat(opts: &SplatOptions) -> Result<(), String> {
    let dl_subdir = match opts.target.libc {
        Libc::Gnu => format!("debian-{}-{}", opts.glibc, opts.target.debian_arch()),
        Libc::Musl => format!("alpine-{}", opts.target.alpine_arch()),
    };
    let dl_dir = opts.cache_dir.join("dl").join(&dl_subdir);
    fs::create_dir_all(&dl_dir)
        .map_err(|e| format!("failed creating {}: {e}", dl_dir.display()))?;

    let pkgs: Vec<RemotePkg> = match opts.target.libc {
        Libc::Gnu => {
            let suite = resolve_debian_suite(&opts.glibc)?;
            resolve_debian_packages(&opts.target, suite)?
        }
        Libc::Musl => resolve_alpine_packages(&opts.target)?,
    };

    let total_download: u64 = pkgs.iter().map(|p| p.size).sum();
    let all_cached = pkgs.iter().all(|p| dl_dir.join(&p.filename).is_file());

    let pb = if all_cached {
        make_bytes_bar("Sysroot", 0, "📦", "unpacking")
    } else {
        let bar = make_bytes_bar("Sysroot", total_download, "📥", "downloading");
        let agent = ureq::Agent::new_with_defaults();
        for pkg in &pkgs {
            let dest = dl_dir.join(&pkg.filename);
            if dest.is_file() {
                bar.inc(pkg.size);
                continue;
            }
            download_to_file(&agent, &pkg.url, &dest, &bar)?;
        }
        bar
    };

    let dest_sysroot = opts.output_dir.join("sysroot");
    if dest_sysroot.exists() {
        fs::remove_dir_all(&dest_sysroot)
            .map_err(|e| format!("failed cleaning {}: {e}", dest_sysroot.display()))?;
    }
    fs::create_dir_all(&dest_sysroot)
        .map_err(|e| format!("failed creating {}: {e}", dest_sysroot.display()))?;

    pb.reset();
    pb.set_length(total_download);
    pb.set_message(format_status("📦", "unpacking"));

    for pkg in &pkgs {
        let archive_path = dl_dir.join(&pkg.filename);
        match opts.target.libc {
            Libc::Gnu => extract_deb(&archive_path, &dest_sysroot, &pb)?,
            Libc::Musl => extract_apk(&archive_path, &dest_sysroot, &pb)?,
        }
    }

    if opts.target.libc == Libc::Gnu {
        fixup_glibc_linker_scripts(&dest_sysroot, opts.target.multiarch_dir())?;
    }

    pb.finish_with_message(format_status("📦", "splatted"));

    if let Ok(clang) = resolve_clang(opts.clang_bin.as_deref()) {
        let bin_dir = opts.output_dir.join("bin");
        generate_wrappers(&bin_dir, &dest_sysroot, &opts.target, &clang)?;
        if opts.verbose {
            eprintln!(
                "[xlinux] ready: sysroot at {}, wrappers at {}",
                dest_sysroot.display(),
                bin_dir.display()
            );
        }
    } else if opts.verbose {
        eprintln!("[xlinux] ready: sysroot at {}", dest_sysroot.display());
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
    sysroot: &Path,
    target: &LinuxTarget,
    clang: &Path,
) -> Result<(), String> {
    fs::create_dir_all(bin_dir)
        .map_err(|e| format!("failed creating {}: {e}", bin_dir.display()))?;

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
    let extra = match target.libc {
        Libc::Gnu => format!(" --gcc-toolchain=\"{sysroot_str}/usr\""),
        Libc::Musl => String::new(),
    };

    let args = format!(
        "--target={} --sysroot=\"{sysroot_str}\"{extra} -fuse-ld=lld",
        target.triple
    );
    xsysroot_core::write_clang_wrapper(
        bin_dir,
        &format!("{}-clang", target.triple),
        &clang_str,
        &args,
    )?;
    xsysroot_core::write_clang_wrapper(
        bin_dir,
        &format!("{}-clang++", target.triple),
        &clangxx_str,
        &args,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_linux_glibc_and_musl_shims() {
        let tmp = std::env::temp_dir().join("xlinux_shim_test");
        let _ = fs::remove_dir_all(&tmp);
        let bin_dir = tmp.join("bin");
        let sysroot = tmp.join("sysroot");
        fs::create_dir_all(&sysroot).unwrap();

        let (gnu, _) = LinuxTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        generate_wrappers(&bin_dir, &sysroot, &gnu, Path::new("/usr/bin/clang")).unwrap();
        let cc = fs::read_to_string(xsysroot_core::wrapper_path(
            &bin_dir,
            "x86_64-unknown-linux-gnu-clang",
        ))
        .unwrap();
        assert!(cc.contains("--target=x86_64-unknown-linux-gnu"));
        assert!(cc.contains("--gcc-toolchain="));

        let _ = fs::remove_dir_all(&tmp);
    }
}

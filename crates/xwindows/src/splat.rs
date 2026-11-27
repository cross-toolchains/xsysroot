//! Unified Windows sysroot download, splat, and Clang/LLD wrapper generation
//! supporting both MSVC (`*-pc-windows-msvc`, adapted from `Jake-Shadle/xwin`)
//! and MinGW / UCRT (`*-pc-windows-gnullvm` / `*-pc-windows-gnu`).

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use camino::Utf8PathBuf;
use indicatif as ia;
use xsysroot_core::{
    default_tool_cache_dir, format_status, resolve_clang, write_executable_script,
};

use crate::mingw::{download_mingw_sysroot, extract_mingw_sysroot};
use crate::msvc;
use crate::target::{DEFAULT_MINGW_RELEASE, MingwCrt, WindowsAbi, WindowsTarget};

const MS_LICENSE_URL: &str = "https://go.microsoft.com/fwlink/?LinkId=2086102";

/// Options for `xwindows download`.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub target: WindowsTarget,
    pub cache_dir: PathBuf,
    pub accept_license: bool,
    pub sdk_version: Option<String>,
    pub crt_version: Option<String>,
    pub include_atl: bool,
    pub mingw_release: String,
    pub mingw_crt: MingwCrt,
}

/// Options for `xwindows splat`.
#[derive(Debug, Clone)]
pub struct SplatOptions {
    pub target: WindowsTarget,
    pub output_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub clang_bin: Option<PathBuf>,
    pub accept_license: bool,
    pub sdk_version: Option<String>,
    pub crt_version: Option<String>,
    pub include_atl: bool,
    pub include_debug_libs: bool,
    pub include_debug_symbols: bool,
    pub disable_symlinks: bool,
    pub preserve_ms_arch_notation: bool,
    pub use_winsysroot_style: bool,
    pub mingw_release: String,
    pub mingw_crt: MingwCrt,
    pub verbose: bool,
}

impl SplatOptions {
    /// Creates sensible defaults for `target` under `~/.cache/xwindows`.
    pub fn for_target(target: WindowsTarget) -> Self {
        let cache_dir = default_cache_dir();
        let output_dir = default_splat_dir(&cache_dir, target);
        Self {
            target,
            output_dir,
            cache_dir,
            clang_bin: None,
            accept_license: false,
            sdk_version: None,
            crt_version: None,
            include_atl: false,
            include_debug_libs: false,
            include_debug_symbols: false,
            disable_symlinks: false,
            preserve_ms_arch_notation: false,
            use_winsysroot_style: false,
            mingw_release: DEFAULT_MINGW_RELEASE.to_string(),
            mingw_crt: MingwCrt::Ucrt,
            verbose: false,
        }
    }
}

/// Default cache root (`~/.cache/xwindows`).
pub fn default_cache_dir() -> PathBuf {
    default_tool_cache_dir("xwindows")
}

/// Default splat output directory (`<cache-dir>/splat/<triple>`).
pub fn default_splat_dir(cache_dir: &Path, target: WindowsTarget) -> PathBuf {
    cache_dir.join("splat").join(target.triple())
}

/// Downloads packages into `<cache-dir>` without splatting.
pub fn run_download(opts: &DownloadOptions) -> Result<(), String> {
    match opts.target.abi {
        WindowsAbi::Msvc => {
            ensure_ms_license_accepted(opts.accept_license)?;
            run_msvc_op(
                &opts.cache_dir,
                opts.target,
                opts.sdk_version.clone(),
                opts.crt_version.clone(),
                opts.include_atl,
                msvc::Ops::Download,
            )
        }
        WindowsAbi::GnuLlvm | WindowsAbi::Gnu => {
            let (_zip, pb) = download_mingw_sysroot(
                opts.target,
                &opts.mingw_release,
                opts.mingw_crt,
                &opts.cache_dir,
            )?;
            if let Some(pb) = pb {
                pb.finish_with_message(format_status("📥", "downloaded"));
            }
            Ok(())
        }
    }
}

/// Downloads, splats, and generates Clang/LLD cross-compilation wrappers.
pub fn run_splat(opts: &SplatOptions) -> Result<(), String> {
    match opts.target.abi {
        WindowsAbi::Msvc => run_splat_msvc(opts),
        WindowsAbi::GnuLlvm | WindowsAbi::Gnu => run_splat_mingw(opts),
    }
}

fn run_splat_msvc(opts: &SplatOptions) -> Result<(), String> {
    ensure_ms_license_accepted(opts.accept_license)?;

    let utf8_output = Utf8PathBuf::from_path_buf(opts.output_dir.clone())
        .map_err(|p| format!("output path {} is not valid UTF-8", p.display()))?;

    let splat_cfg = msvc::SplatConfig {
        include_debug_libs: opts.include_debug_libs,
        include_debug_symbols: opts.include_debug_symbols,
        enable_symlinks: !opts.disable_symlinks,
        preserve_ms_arch_notation: opts.preserve_ms_arch_notation,
        use_winsysroot_style: opts.use_winsysroot_style,
        copy: false,
        map: None,
        output: utf8_output,
    };

    run_msvc_op(
        &opts.cache_dir,
        opts.target,
        opts.sdk_version.clone(),
        opts.crt_version.clone(),
        opts.include_atl,
        msvc::Ops::Splat(splat_cfg),
    )?;

    if let Ok(clang) = resolve_clang(opts.clang_bin.as_deref()) {
        let bin_dir = opts.output_dir.join("bin");
        generate_msvc_wrappers(
            &bin_dir,
            &opts.output_dir,
            opts.target,
            opts.preserve_ms_arch_notation,
            opts.use_winsysroot_style,
            &clang,
        )?;
        if opts.verbose {
            eprintln!(
                "[xwindows] ready: msvc sysroot at {}, wrappers at {}",
                opts.output_dir.display(),
                bin_dir.display()
            );
        }
    } else if opts.verbose {
        eprintln!(
            "[xwindows] ready: msvc sysroot at {}",
            opts.output_dir.display()
        );
    }

    Ok(())
}

fn run_splat_mingw(opts: &SplatOptions) -> Result<(), String> {
    let (zip_path, pb) = download_mingw_sysroot(
        opts.target,
        &opts.mingw_release,
        opts.mingw_crt,
        &opts.cache_dir,
    )?;

    let sysroot_dir = opts.output_dir.join("sysroot");
    extract_mingw_sysroot(&zip_path, &sysroot_dir, opts.target.arch, pb.as_ref())?;

    if let Ok(clang) = resolve_clang(opts.clang_bin.as_deref()) {
        let bin_dir = opts.output_dir.join("bin");
        generate_mingw_wrappers(&bin_dir, &sysroot_dir, opts.target, &clang)?;
        if opts.verbose {
            eprintln!(
                "[xwindows] ready: mingw sysroot at {}, wrappers at {}",
                sysroot_dir.display(),
                bin_dir.display()
            );
        }
    } else if opts.verbose {
        eprintln!(
            "[xwindows] ready: mingw sysroot at {}",
            sysroot_dir.display()
        );
    }

    Ok(())
}

fn run_msvc_op(
    cache_dir: &Path,
    target: WindowsTarget,
    sdk_version: Option<String>,
    crt_version: Option<String>,
    include_atl: bool,
    op: msvc::Ops,
) -> Result<(), String> {
    let msvc_cache = cache_dir.join("msvc");
    fs::create_dir_all(&msvc_cache)
        .map_err(|e| format!("failed to create {}: {e}", msvc_cache.display()))?;
    let utf8_cache = Utf8PathBuf::from_path_buf(msvc_cache)
        .map_err(|p| format!("cache path {} is not valid UTF-8", p.display()))?;

    let tls_config =
        ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier);
    let client = ureq::config::Config::builder()
        .timeout_recv_body(Some(Duration::from_secs(300)))
        .tls_config(tls_config.build())
        .build()
        .new_agent();

    let draw_target = msvc::util::ProgressTarget::Stdout;
    let ctx = Arc::new(
        msvc::Ctx::with_dir(utf8_cache, draw_target, client, 2)
            .map_err(|e| format!("failed initializing MSVC context: {e:#}"))?,
    );

    let manifest_pb = ia::ProgressBar::with_draw_target(Some(0), draw_target.into());
    if let Ok(style) = ia::ProgressStyle::default_bar().template(
        "{spinner:.green} {prefix:<9.bold} [{elapsed:>3}] {wide_bar:.green} {binary_bytes:>10}/{binary_total_bytes:>10}  {msg}",
    ) {
        manifest_pb.set_style(style.progress_chars("█▄ "));
    }
    manifest_pb.set_prefix("Manifest");
    manifest_pb.set_message(format_status("📥", "downloading"));

    let manifest = msvc::manifest::get_manifest(&ctx, 17, "release", manifest_pb.clone())
        .map_err(|e| format!("failed fetching Visual Studio manifest: {e:#}"))?;
    let pkg_manifest = msvc::manifest::get_package_manifest(&ctx, &manifest, manifest_pb.clone())
        .map_err(|e| format!("failed fetching package manifest: {e:#}"))?;
    manifest_pb.finish_with_message(format_status("📥", "downloaded"));

    let arches = target.arch.to_msvc_arch() as u32;
    let variants = msvc::Variant::Desktop as u32;
    let pruned = msvc::prune_pkg_list(
        &pkg_manifest,
        arches,
        variants,
        include_atl,
        false,
        sdk_version,
        crt_version,
    )
    .map_err(|e| format!("failed pruning MSVC package list: {e:#}"))?;

    let mp = ia::MultiProgress::with_draw_target(draw_target.into());
    let work_items: Vec<_> = pruned
        .payloads
        .into_iter()
        .map(|pay| {
            let prefix = match pay.kind {
                msvc::PayloadKind::CrtHeaders => "CRT.hdrs".to_owned(),
                msvc::PayloadKind::AtlHeaders => "ATL.hdrs".to_owned(),
                msvc::PayloadKind::CrtLibs => format!(
                    "CRT.libs.{}",
                    pay.target_arch.map_or("all", |ta| ta.as_str())
                ),
                msvc::PayloadKind::AtlLibs => format!(
                    "ATL.libs.{}",
                    pay.target_arch.map_or("all", |ta| ta.as_str())
                ),
                msvc::PayloadKind::SdkHeaders => "SDK.hdrs".to_owned(),
                msvc::PayloadKind::SdkLibs => format!(
                    "SDK.libs.{}",
                    pay.target_arch.map_or("all", |ta| ta.as_str())
                ),
                msvc::PayloadKind::SdkStoreLibs => "SDK.store".to_owned(),
                msvc::PayloadKind::Ucrt => "SDK.ucrt".to_owned(),
                msvc::PayloadKind::VcrDebug => "VCR.debug".to_owned(),
            };
            let pb = mp.add(
                ia::ProgressBar::with_draw_target(Some(0), draw_target.into())
                    .with_prefix(prefix)
                    .with_style(
                        ia::ProgressStyle::default_bar()
                            .template("{spinner:.green} {prefix:<15.bold} [{elapsed:>3}] {wide_bar:.green} {binary_bytes:>10}/{binary_total_bytes:>10}  {msg}")
                            .unwrap()
                            .progress_chars("█▄ "),
                    ),
            );
            msvc::WorkItem {
                payload: Arc::new(pay),
                progress: pb,
            }
        })
        .collect();

    mp.set_move_cursor(true);

    let pkgs = pkg_manifest.packages;
    ctx.execute(
        pkgs,
        work_items,
        pruned.crt_version,
        pruned.sdk_version,
        pruned.vcr_version,
        arches,
        variants,
        op,
    )
    .map_err(|e| format!("MSVC splat failed: {e:#}"))
}

fn ensure_ms_license_accepted(flag_accepted: bool) -> Result<(), String> {
    let env_accepted = |var: &str| {
        matches!(
            env::var(var).as_deref(),
            Ok("1") | Ok("true") | Ok("yes") | Ok("YES")
        )
    };
    if flag_accepted
        || env_accepted("XWINDOWS_ACCEPT_LICENSE")
        || env_accepted("XWIN_ACCEPT_LICENSE")
    {
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        return Err(format!(
            "MSVC CRT & Windows SDK download requires accepting Microsoft's license ({MS_LICENSE_URL}).\n  \
             Re-run with `--accept-license` or set `XWINDOWS_ACCEPT_LICENSE=1`."
        ));
    }

    eprintln!("Do you accept the license at {MS_LICENSE_URL} (yes | no)?");
    let _ = io::stderr().flush();

    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| format!("failed to read license confirmation: {e}"))?;
    if is_affirmative(&answer) {
        eprintln!("license accepted!");
        Ok(())
    } else {
        Err("Microsoft license was not accepted.".to_string())
    }
}

fn is_affirmative(input: &str) -> bool {
    matches!(input.trim().to_ascii_lowercase().as_str(), "yes" | "y")
}

fn wrapper_path(bin_dir: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        bin_dir.join(format!("{name}.cmd"))
    } else {
        bin_dir.join(name)
    }
}

fn write_clang_wrapper(
    bin_dir: &Path,
    name: &str,
    compiler: &str,
    args: &str,
) -> Result<PathBuf, String> {
    let path = wrapper_path(bin_dir, name);
    let contents = if cfg!(windows) {
        format!("@echo off\r\n\"{compiler}\" {args} %*\r\n")
    } else {
        format!("#!/bin/sh\nexec \"{compiler}\" {args} \"$@\"\n")
    };
    write_executable_script(&path, &contents)?;
    Ok(path)
}

fn resolve_sibling_tool(clang: &Path, name: &str) -> String {
    let dir = clang.parent().unwrap_or_else(|| Path::new(""));
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let candidate = dir.join(&exe);
    if candidate.is_file() {
        candidate.display().to_string()
    } else {
        exe
    }
}

fn generate_msvc_wrappers(
    bin_dir: &Path,
    output_dir: &Path,
    target: WindowsTarget,
    preserve_ms_arch: bool,
    use_winsysroot_style: bool,
    clang: &Path,
) -> Result<(), String> {
    fs::create_dir_all(bin_dir)
        .map_err(|e| format!("failed to create {}: {e}", bin_dir.display()))?;

    let triple = target.triple();
    let clang_target = target.clang_target();
    let clang_str = clang.display().to_string();
    let clangxx_str = resolve_sibling_tool(clang, "clang++");
    let clang_cl_str = resolve_sibling_tool(clang, "clang-cl");
    let lld_link_str = resolve_sibling_tool(clang, "lld-link");

    let arch_dir = target.arch.msvc_lib_dir(preserve_ms_arch);
    let (crt_inc, crt_lib, ucrt_inc, um_inc, shared_inc, ucrt_lib, um_lib) = if use_winsysroot_style
    {
        // Standard `/winsysroot` layout places CRT under `VC/Tools/MSVC/<ver>` and SDK under `Windows Kits/10/...`.
        let out_str = output_dir.display().to_string();
        let cl_args = format!(
            "--target={clang_target} -fuse-ld=lld-link /winsysroot \"{out_str}\" -Wno-unused-command-line-argument"
        );
        write_clang_wrapper(
            bin_dir,
            &format!("{triple}-clang-cl"),
            &clang_cl_str,
            &cl_args,
        )?;
        let cc_args = format!(
            "--target={clang_target} -fuse-ld=lld -winsysroot \"{out_str}\" -Wno-unused-command-line-argument"
        );
        write_clang_wrapper(bin_dir, &format!("{triple}-clang"), &clang_str, &cc_args)?;
        write_clang_wrapper(
            bin_dir,
            &format!("{triple}-clang++"),
            &clangxx_str,
            &cc_args,
        )?;
        return Ok(());
    } else {
        (
            output_dir.join("crt").join("include"),
            output_dir.join("crt").join("lib").join(arch_dir),
            output_dir.join("sdk").join("include").join("ucrt"),
            output_dir.join("sdk").join("include").join("um"),
            output_dir.join("sdk").join("include").join("shared"),
            output_dir
                .join("sdk")
                .join("lib")
                .join("ucrt")
                .join(arch_dir),
            output_dir.join("sdk").join("lib").join("um").join(arch_dir),
        )
    };

    let cc_args = format!(
        "--target={clang_target} -fuse-ld=lld -Wno-unused-command-line-argument \
         -isystem \"{}\" -isystem \"{}\" -isystem \"{}\" -isystem \"{}\" \
         -L\"{}\" -L\"{}\" -L\"{}\"",
        crt_inc.display(),
        ucrt_inc.display(),
        um_inc.display(),
        shared_inc.display(),
        crt_lib.display(),
        ucrt_lib.display(),
        um_lib.display(),
    );
    write_clang_wrapper(bin_dir, &format!("{triple}-clang"), &clang_str, &cc_args)?;
    write_clang_wrapper(
        bin_dir,
        &format!("{triple}-clang++"),
        &clangxx_str,
        &cc_args,
    )?;

    let cl_args = format!(
        "--target={clang_target} -fuse-ld=lld-link -Wno-unused-command-line-argument \
         /imsvc \"{}\" /imsvc \"{}\" /imsvc \"{}\" /imsvc \"{}\" \
         -Xlinker /libpath:\"{}\" -Xlinker /libpath:\"{}\" -Xlinker /libpath:\"{}\"",
        crt_inc.display(),
        ucrt_inc.display(),
        um_inc.display(),
        shared_inc.display(),
        crt_lib.display(),
        ucrt_lib.display(),
        um_lib.display(),
    );
    write_clang_wrapper(
        bin_dir,
        &format!("{triple}-clang-cl"),
        &clang_cl_str,
        &cl_args,
    )?;

    let link_args = format!(
        "/libpath:\"{}\" /libpath:\"{}\" /libpath:\"{}\"",
        crt_lib.display(),
        ucrt_lib.display(),
        um_lib.display(),
    );
    write_clang_wrapper(
        bin_dir,
        &format!("{triple}-lld-link"),
        &lld_link_str,
        &link_args,
    )?;

    Ok(())
}

fn find_clang_resource_dir(sysroot_dir: &Path) -> Option<PathBuf> {
    let clang_lib = sysroot_dir.join("lib").join("clang");
    let entries = fs::read_dir(&clang_lib).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && p.join("include").is_dir() {
            return Some(p);
        }
    }
    None
}

fn generate_mingw_wrappers(
    bin_dir: &Path,
    sysroot_dir: &Path,
    target: WindowsTarget,
    clang: &Path,
) -> Result<(), String> {
    fs::create_dir_all(bin_dir)
        .map_err(|e| format!("failed to create {}: {e}", bin_dir.display()))?;

    let triple = target.triple();
    let clang_target = target.clang_target();
    let clang_str = clang.display().to_string();
    let clangxx_str = resolve_sibling_tool(clang, "clang++");
    let sysroot_str = sysroot_dir.display().to_string();
    let arch_lib = sysroot_dir.join(target.arch.mingw_prefix()).join("lib");
    let arch_inc = sysroot_dir.join(target.arch.mingw_prefix()).join("include");
    let cxx_inc = sysroot_dir.join("include").join("c++").join("v1");

    let resource_flag = match find_clang_resource_dir(sysroot_dir) {
        Some(res_dir) => format!("-resource-dir=\"{}\" ", res_dir.display()),
        None => String::new(),
    };

    let c_args = format!(
        "--target={clang_target} --sysroot=\"{sysroot_str}\" {resource_flag}-fuse-ld=lld \
         --rtlib=compiler-rt --unwindlib=libunwind -Wno-unused-command-line-argument \
         -isystem \"{}\" -L\"{}\"",
        arch_inc.display(),
        arch_lib.display(),
    );
    write_clang_wrapper(bin_dir, &format!("{triple}-clang"), &clang_str, &c_args)?;

    let cxx_args = format!(
        "--target={clang_target} --sysroot=\"{sysroot_str}\" {resource_flag}-fuse-ld=lld \
         --rtlib=compiler-rt --unwindlib=libunwind -stdlib=libc++ -Wno-unused-command-line-argument \
         -isystem \"{}\" -isystem \"{}\" -L\"{}\"",
        cxx_inc.display(),
        arch_inc.display(),
        arch_lib.display(),
    );
    write_clang_wrapper(
        bin_dir,
        &format!("{triple}-clang++"),
        &clangxx_str,
        &cxx_args,
    )?;

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
    fn generates_msvc_and_mingw_wrappers() {
        let tmp = env::temp_dir().join("xwindows_wrapper_test");
        let _ = fs::remove_dir_all(&tmp);
        let msvc_bin = tmp.join("msvc-bin");
        let msvc_out = tmp.join("msvc");
        let msvc_target = WindowsTarget::parse("x86_64-pc-windows-msvc").unwrap();
        generate_msvc_wrappers(
            &msvc_bin,
            &msvc_out,
            msvc_target,
            false,
            false,
            Path::new("/usr/bin/clang"),
        )
        .unwrap();

        let cc = wrapper_path(&msvc_bin, "x86_64-pc-windows-msvc-clang");
        let link = wrapper_path(&msvc_bin, "x86_64-pc-windows-msvc-lld-link");
        assert!(cc.is_file());
        assert!(link.is_file());
        let cc_text = fs::read_to_string(&cc).unwrap();
        assert!(cc_text.contains("--target=x86_64-pc-windows-msvc"));
        assert!(cc_text.contains("-fuse-ld=lld"));

        let mingw_bin = tmp.join("mingw-bin");
        let mingw_sysroot = tmp.join("mingw-sysroot");
        let mingw_target = WindowsTarget::parse("aarch64-pc-windows-gnullvm").unwrap();
        generate_mingw_wrappers(
            &mingw_bin,
            &mingw_sysroot,
            mingw_target,
            Path::new("/usr/bin/clang"),
        )
        .unwrap();
        let mingw_cc = wrapper_path(&mingw_bin, "aarch64-pc-windows-gnullvm-clang");
        assert!(mingw_cc.is_file());
        let mingw_text = fs::read_to_string(&mingw_cc).unwrap();
        assert!(mingw_text.contains("--target=aarch64-w64-windows-gnu"));
        assert!(mingw_text.contains("--sysroot="));

        let _ = fs::remove_dir_all(&tmp);
    }
}

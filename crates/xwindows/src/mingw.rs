//! MinGW / UCRT sysroot fetcher and extractor backed by upstream
//! `mstorsjo/llvm-mingw` GitHub Releases over HTTP `Range` partial ZIP requests.
//!
//! Instead of downloading the full ~190 MiB `llvm-mingw` archive (which is
//! mostly host LLVM binaries), this module reads only the ZIP Central Directory
//! and fetches only the target sysroot entries:
//! - `generic-w64-mingw32/include/` (MinGW-w64 + UCRT C headers)
//! - `<arch>-w64-mingw32/` (CRT objects, Win32 import libs, `libc++`, `libunwind`)
//! - `include/c++/v1/` (`libc++` C++ standard library headers)
//! - `lib/clang/*/lib/windows/` (`libclang_rt.builtins-*.a`)

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use indicatif::ProgressBar;
use xsysroot_core::{format_status, make_bytes_bar, sanitize_relative_path};

use crate::target::{MingwCrt, WindowsArch, WindowsTarget};

const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_CD: u32 = 0x0201_4b50;
const MERGE_GAP_BYTES: u64 = 128 * 1024;
const MAX_RANGE_BODY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
struct CdEntry {
    raw_cd_record: Vec<u8>,
    name: String,
    local_offset: u64,
}

/// Returns the upstream `mstorsjo/llvm-mingw` `.zip` URL for the given release and CRT.
pub fn llvm_mingw_zip_url(release: &str, crt: MingwCrt) -> String {
    let crt_str = crt.as_str();
    format!(
        "https://github.com/mstorsjo/llvm-mingw/releases/download/{release}/llvm-mingw-{release}-{crt_str}-x86_64.zip"
    )
}

/// Returns the cached mini-zip path under `<cache-dir>/dl/`.
pub fn cached_mingw_zip_path(
    cache_dir: &Path,
    release: &str,
    crt: MingwCrt,
    arch: WindowsArch,
) -> PathBuf {
    cache_dir.join("dl").join(format!(
        "llvm-mingw-{release}-{}-{}-sysroot.zip",
        crt.as_str(),
        arch.rust_arch()
    ))
}

/// Downloads only the MinGW/UCRT sysroot entries for `target` into `<cache-dir>/dl/`.
pub fn download_mingw_sysroot(
    target: WindowsTarget,
    release: &str,
    crt: MingwCrt,
    cache_dir: &Path,
) -> Result<(PathBuf, Option<ProgressBar>), String> {
    let dest_zip = cached_mingw_zip_path(cache_dir, release, crt, target.arch);
    if dest_zip.is_file() {
        let len = fs::metadata(&dest_zip).map(|m| m.len()).unwrap_or(0);
        let pb = make_bytes_bar("Sysroot", len, "📥", "cached");
        pb.set_position(len);
        return Ok((dest_zip, Some(pb)));
    }

    let url = llvm_mingw_zip_url(release, crt);
    let prefix = target.arch.mingw_prefix().to_string();
    let rt_arch = target.arch.compiler_rt_arch().to_string();
    let pb = fetch_partial_zip(&url, &dest_zip, |entry_name| {
        is_wanted_mingw_entry(entry_name, &prefix, &rt_arch)
    })?;
    Ok((dest_zip, Some(pb)))
}

/// Extracts the cached mini-zip into `<output_dir>/sysroot/` and normalizes
/// `<arch>-w64-mingw32/include` so Clang `--sysroot=<output_dir>/sysroot` finds
/// headers, import libraries, `libc++`, and `compiler-rt` builtins directly.
pub fn extract_mingw_sysroot(
    zip_path: &Path,
    sysroot_dir: &Path,
    arch: WindowsArch,
    pb: Option<&ProgressBar>,
) -> Result<(), String> {
    if sysroot_dir.exists() {
        fs::remove_dir_all(sysroot_dir)
            .map_err(|e| format!("failed to clean {}: {e}", sysroot_dir.display()))?;
    }
    fs::create_dir_all(sysroot_dir)
        .map_err(|e| format!("failed to create {}: {e}", sysroot_dir.display()))?;

    let file =
        File::open(zip_path).map_err(|e| format!("failed to open {}: {e}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("failed to read zip {}: {e}", zip_path.display()))?;

    let total_uncompressed: u64 = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|e| e.size()))
        .sum();
    if let Some(pb) = pb {
        pb.set_length(total_uncompressed);
        pb.set_position(0);
        pb.set_message(format_status("📦", "unpacking"));
    }

    let mut resource_dir: Option<PathBuf> = None;
    let target_lib_dir = sysroot_dir.join(arch.mingw_prefix()).join("lib");
    let clang_triple = match arch {
        WindowsArch::X86_64 => "x86_64-w64-windows-gnu",
        WindowsArch::Aarch64 => "aarch64-w64-windows-gnu",
        WindowsArch::X86 => "i686-w64-windows-gnu",
        WindowsArch::Armv7 => "armv7-w64-windows-gnu",
    };

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("failed reading zip entry {i}: {e}"))?;
        let raw_name = entry.name().to_string();
        // Strip top-level `llvm-mingw-<ver>-<crt>-x86_64/` directory.
        let Some((_, rel)) = raw_name.split_once('/') else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        let clean_rel = sanitize_relative_path(rel)?;
        let out_path = sysroot_dir.join(&clean_rel);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
        }
        let mut out_file = File::create(&out_path)
            .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
        let copied = io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("failed extracting {}: {e}", out_path.display()))?;
        if let Some(pb) = pb {
            pb.inc(copied);
        }

        // Also mirror `libclang_rt.builtins*.a` into both `<arch>-w64-mingw32/lib/`
        // and `<resource-dir>/lib/<clang-triple>/libclang_rt.builtins.a` so Clang
        // locates compiler-rt builtins natively via `-resource-dir`.
        if let Some(fname) = out_path.file_name().and_then(|n| n.to_str())
            && fname.starts_with("libclang_rt.builtins")
            && fname.ends_with(".a")
        {
            fs::create_dir_all(&target_lib_dir)
                .map_err(|e| format!("failed creating {}: {e}", target_lib_dir.display()))?;
            let mirrored = target_lib_dir.join(fname);
            let _ = fs::copy(&out_path, &mirrored);
            let canonical = target_lib_dir.join("libclang_rt.builtins.a");
            if !canonical.exists() {
                let _ = fs::copy(&out_path, &canonical);
            }

            // `out_path` is `<sysroot>/lib/clang/<ver>/lib/windows/libclang_rt.builtins-<arch>.a`
            if let Some(res_dir) = out_path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
            {
                resource_dir = Some(res_dir.to_path_buf());
                let per_target_dir = res_dir.join("lib").join(clang_triple);
                fs::create_dir_all(&per_target_dir)
                    .map_err(|e| format!("failed creating {}: {e}", per_target_dir.display()))?;
                let _ = fs::copy(&out_path, per_target_dir.join("libclang_rt.builtins.a"));
            }
        }
    }

    let _ = resource_dir;

    // Ensure `<arch>-w64-mingw32/include` exists (pointing to `generic-w64-mingw32/include`
    // or top-level `include/` depending on the `llvm-mingw` release layout).
    let generic_inc = sysroot_dir.join("generic-w64-mingw32").join("include");
    let top_inc = sysroot_dir.join("include");
    let arch_inc = sysroot_dir.join(arch.mingw_prefix()).join("include");
    if !arch_inc.exists() {
        if let Some(parent) = arch_inc.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
        }
        if generic_inc.is_dir() {
            xsysroot_core::create_symlink("../generic-w64-mingw32/include", &arch_inc)?;
        } else if top_inc.is_dir() {
            xsysroot_core::create_symlink("../include", &arch_inc)?;
        }
    }

    // Ensure top-level `include/` exists with C headers accessible via `--sysroot`.

    if let Some(pb) = pb {
        pb.finish_with_message(format_status("📦", "splatted"));
    }
    Ok(())
}

/// Predicate selecting only the target MinGW/UCRT sysroot entries from an
/// upstream `llvm-mingw` archive (skipping host `bin/*.exe` / `bin/*.dll` etc.).
pub fn is_wanted_mingw_entry(entry_name: &str, mingw_prefix: &str, rt_arch: &str) -> bool {
    let Some((_, rel)) = entry_name.split_once('/') else {
        return false;
    };
    if rel.starts_with("include/") || rel.starts_with("generic-w64-mingw32/include/") {
        return true;
    }
    if let Some(rest) = rel.strip_prefix(mingw_prefix)
        && (rest.starts_with("/include/") || rest.starts_with("/lib/"))
    {
        // Skip large DLLs in `bin/`; keep headers, CRT `.o` objects, and `.a` import/static libs.
        return true;
    }
    if rel.starts_with("lib/clang/") {
        if rel.contains("/include/") {
            return true;
        }
        let is_rt_lib =
            rel.contains("/lib/windows/") || rel.contains(&format!("/lib/{mingw_prefix}/"));
        if is_rt_lib
            && rel.ends_with(".a")
            && (rel.contains("builtins") && (rel.contains(rt_arch) || rel.contains("builtins.a")))
        {
            return true;
        }
    }
    false
}

fn fetch_partial_zip<F>(url: &str, dest_zip: &Path, mut predicate: F) -> Result<ProgressBar, String>
where
    F: FnMut(&str) -> bool,
{
    let agent = ureq::Agent::new_with_defaults();

    let index_pb = make_bytes_bar("Index", 0, "📥", "downloading");
    let total_len = fetch_content_length(&agent, url)?;
    let tail_size = total_len.min(65_557);
    let tail_start = total_len - tail_size;
    let tail = fetch_range_streaming(&agent, url, tail_start, total_len - 1, None)?;
    let (cd_offset, cd_size) = parse_eocd(&tail)?;

    index_pb.set_length(cd_size);
    let cd_bytes = fetch_range_streaming(
        &agent,
        url,
        cd_offset,
        cd_offset + cd_size - 1,
        Some(&index_pb),
    )?;
    let all_entries = parse_central_directory(&cd_bytes)?;
    index_pb.finish_with_message(format_status("📥", "downloaded"));

    let mut all_offsets: Vec<u64> = all_entries.iter().map(|e| e.local_offset).collect();
    all_offsets.push(cd_offset);
    all_offsets.sort_unstable();
    all_offsets.dedup();

    let mut matched: Vec<(CdEntry, u64, u64)> = Vec::new();
    for entry in all_entries {
        if predicate(&entry.name) {
            let start = entry.local_offset;
            let pos = all_offsets
                .binary_search(&start)
                .map_err(|_| format!("corrupt offset {start} in central directory"))?;
            let end_exclusive = *all_offsets
                .get(pos + 1)
                .ok_or_else(|| "missing next offset in central directory".to_string())?;
            matched.push((entry, start, end_exclusive));
        }
    }

    if matched.is_empty() {
        return Err(format!("no matching MinGW sysroot entries found in {url}"));
    }

    matched.sort_by_key(|(_, start, _)| *start);

    let mut spans: Vec<(u64, u64)> = Vec::new();
    for &(_, start, end_ex) in &matched {
        if let Some(last) = spans.last_mut()
            && start <= last.1.saturating_add(MERGE_GAP_BYTES)
        {
            if end_ex > last.1 {
                last.1 = end_ex;
            }
            continue;
        }
        spans.push((start, end_ex));
    }

    let payload_bytes: u64 = spans.iter().map(|(s, e)| e - s).sum();

    if let Some(parent) = dest_zip.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let tmp_zip = dest_zip.with_extension("zip.part");
    let mut out = File::create(&tmp_zip)
        .map_err(|e| format!("failed to create {}: {e}", tmp_zip.display()))?;

    let mut new_cd_records: Vec<Vec<u8>> = Vec::with_capacity(matched.len());
    let mut cursor: u64 = 0;
    let mut matched_idx = 0usize;

    let sysroot_pb = make_bytes_bar("Sysroot", payload_bytes, "📥", "downloading");

    for (span_start, span_end_ex) in &spans {
        let buf =
            fetch_range_streaming(&agent, url, *span_start, span_end_ex - 1, Some(&sysroot_pb))?;
        while matched_idx < matched.len() {
            let (entry, start, end_ex) = &matched[matched_idx];
            if *start >= *span_end_ex {
                break;
            }
            let rel_start = (*start - *span_start) as usize;
            let rel_end = (*end_ex - *span_start) as usize;
            out.write_all(&buf[rel_start..rel_end])
                .map_err(|e| format!("failed writing mini zip: {e}"))?;

            let mut cd = entry.raw_cd_record.clone();
            let new_off = u32::try_from(cursor)
                .map_err(|_| "mini zip exceeded 4 GiB offset limit".to_string())?;
            cd[42..46].copy_from_slice(&new_off.to_le_bytes());
            new_cd_records.push(cd);

            cursor += end_ex - start;
            matched_idx += 1;
        }
    }

    let new_cd_offset = u32::try_from(cursor)
        .map_err(|_| "mini zip central directory offset exceeded 4 GiB".to_string())?;
    let mut new_cd_size: u32 = 0;
    for rec in &new_cd_records {
        out.write_all(rec)
            .map_err(|e| format!("failed writing central directory: {e}"))?;
        new_cd_size = new_cd_size
            .checked_add(rec.len() as u32)
            .ok_or_else(|| "mini zip central directory too large".to_string())?;
    }

    let entry_count = u16::try_from(new_cd_records.len())
        .map_err(|_| "mini zip entry count exceeded u16::MAX".to_string())?;
    let mut eocd = [0u8; 22];
    eocd[0..4].copy_from_slice(&SIG_EOCD.to_le_bytes());
    eocd[8..10].copy_from_slice(&entry_count.to_le_bytes());
    eocd[10..12].copy_from_slice(&entry_count.to_le_bytes());
    eocd[12..16].copy_from_slice(&new_cd_size.to_le_bytes());
    eocd[16..20].copy_from_slice(&new_cd_offset.to_le_bytes());
    out.write_all(&eocd)
        .map_err(|e| format!("failed writing EOCD: {e}"))?;
    drop(out);

    fs::rename(&tmp_zip, dest_zip).map_err(|e| {
        format!(
            "failed to finalize {} -> {}: {e}",
            tmp_zip.display(),
            dest_zip.display()
        )
    })?;

    Ok(sysroot_pb)
}

fn fetch_content_length(agent: &ureq::Agent, url: &str) -> Result<u64, String> {
    let resp = agent
        .head(url)
        .header("Accept-Encoding", "identity")
        .call()
        .map_err(|e| format!("HTTP HEAD failed for {url}: {e}"))?;
    resp.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .ok_or_else(|| format!("server did not return Content-Length for {url}"))
}

fn fetch_range_streaming(
    agent: &ureq::Agent,
    url: &str,
    start: u64,
    end_inclusive: u64,
    progress: Option<&ProgressBar>,
) -> Result<Vec<u8>, String> {
    let expected = end_inclusive.saturating_sub(start).saturating_add(1);
    if expected > MAX_RANGE_BODY_BYTES {
        return Err(format!(
            "HTTP Range request size ({expected} bytes) exceeds limit"
        ));
    }
    let range = format!("bytes={start}-{end_inclusive}");
    let resp = agent
        .get(url)
        .header("Accept-Encoding", "identity")
        .header("Range", &range)
        .call()
        .map_err(|e| format!("HTTP Range {range} failed for {url}: {e}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut buf = Vec::with_capacity(expected as usize);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| format!("failed reading HTTP Range {range} body: {e}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() as u64 > MAX_RANGE_BODY_BYTES {
            return Err(format!("HTTP Range {range} body exceeded size limit"));
        }
        if let Some(pb) = progress {
            pb.inc(n as u64);
        }
    }
    Ok(buf)
}

fn parse_eocd(tail: &[u8]) -> Result<(u64, u64), String> {
    if tail.len() < 22 {
        return Err("ZIP tail too small for EOCD".to_string());
    }
    for i in (0..=tail.len() - 22).rev() {
        let sig = u32::from_le_bytes(tail[i..i + 4].try_into().unwrap());
        if sig == SIG_EOCD {
            let cd_size = u32::from_le_bytes(tail[i + 12..i + 16].try_into().unwrap()) as u64;
            let cd_offset = u32::from_le_bytes(tail[i + 16..i + 20].try_into().unwrap()) as u64;
            return Ok((cd_offset, cd_size));
        }
    }
    Err("could not locate ZIP End of Central Directory (EOCD)".to_string())
}

fn parse_central_directory(cd: &[u8]) -> Result<Vec<CdEntry>, String> {
    let mut entries = Vec::new();
    let mut i = 0usize;
    while i + 46 <= cd.len() {
        let sig = u32::from_le_bytes(cd[i..i + 4].try_into().unwrap());
        if sig != SIG_CD {
            return Err(format!(
                "unexpected Central Directory signature at byte {i}"
            ));
        }
        let name_len = u16::from_le_bytes(cd[i + 28..i + 30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(cd[i + 30..i + 32].try_into().unwrap()) as usize;
        let comment_len = u16::from_le_bytes(cd[i + 32..i + 34].try_into().unwrap()) as usize;
        let local_offset = u32::from_le_bytes(cd[i + 42..i + 46].try_into().unwrap()) as u64;

        let rec_end = i + 46 + name_len + extra_len + comment_len;
        if rec_end > cd.len() {
            return Err("truncated Central Directory record".to_string());
        }
        let name = String::from_utf8_lossy(&cd[i + 46..i + 46 + name_len]).into_owned();
        entries.push(CdEntry {
            raw_cd_record: cd[i..rec_end].to_vec(),
            name,
            local_offset,
        });
        i = rec_end;
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_only_target_mingw_sysroot_entries() {
        assert!(is_wanted_mingw_entry(
            "llvm-mingw-20261117-ucrt-x86_64/generic-w64-mingw32/include/windows.h",
            "x86_64-w64-mingw32",
            "x86_64"
        ));
        assert!(is_wanted_mingw_entry(
            "llvm-mingw-20261117-ucrt-x86_64/x86_64-w64-mingw32/lib/libkernel32.a",
            "x86_64-w64-mingw32",
            "x86_64"
        ));
        assert!(is_wanted_mingw_entry(
            "llvm-mingw-20261117-ucrt-x86_64/include/c++/v1/iostream",
            "x86_64-w64-mingw32",
            "x86_64"
        ));
        assert!(is_wanted_mingw_entry(
            "llvm-mingw-20261117-ucrt-x86_64/lib/clang/23/lib/windows/libclang_rt.builtins-x86_64.a",
            "x86_64-w64-mingw32",
            "x86_64"
        ));
        assert!(!is_wanted_mingw_entry(
            "llvm-mingw-20261117-ucrt-x86_64/bin/clang.exe",
            "x86_64-w64-mingw32",
            "x86_64"
        ));
        assert!(!is_wanted_mingw_entry(
            "llvm-mingw-20261117-ucrt-x86_64/aarch64-w64-mingw32/lib/libkernel32.a",
            "x86_64-w64-mingw32",
            "x86_64"
        ));
    }
}

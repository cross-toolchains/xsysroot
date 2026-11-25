//! Pure-Rust `.deb` (`ar` -> `data.tar.{xz,gz}`) and Alpine `.apk` (`tar.gz`)
//! sysroot extractor plus cross-compilation linker script & symlink fixups.

use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};

use flate2::read::MultiGzDecoder;
use indicatif::ProgressBar;

const AR_MAGIC: &[u8; 8] = b"!<arch>\n";

/// Extracts a Debian `.deb` package (`ar` -> `data.tar.{xz,gz}`) into `sysroot`.
pub fn extract_deb(deb_path: &Path, sysroot: &Path, pb: &ProgressBar) -> Result<(), String> {
    let mut file =
        File::open(deb_path).map_err(|e| format!("failed opening {}: {e}", deb_path.display()))?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic)
        .map_err(|e| format!("failed reading ar magic in {}: {e}", deb_path.display()))?;
    if &magic != AR_MAGIC {
        return Err(format!("invalid .deb archive: {}", deb_path.display()));
    }

    let mut hdr = [0u8; 60];
    loop {
        if !read_exact_or_eof(&mut file, &mut hdr)? {
            break;
        }
        let name = std::str::from_utf8(&hdr[0..16])
            .unwrap_or("")
            .trim()
            .trim_end_matches('/');
        let size_str = std::str::from_utf8(&hdr[48..58]).unwrap_or("").trim();
        let size = size_str
            .parse::<u64>()
            .map_err(|e| format!("invalid ar member size `{size_str}`: {e}"))?;

        if name.starts_with("data.tar") {
            let mut payload = vec![0u8; size as usize];
            file.read_exact(&mut payload)
                .map_err(|e| format!("failed reading {name}: {e}"))?;
            if name.ends_with(".xz") {
                let mut tar_bytes = Vec::new();
                let mut cur = Cursor::new(payload);
                lzma_rs::xz_decompress(&mut cur, &mut tar_bytes)
                    .map_err(|e| format!("failed decompressing {name}: {e}"))?;
                extract_tar(Cursor::new(tar_bytes), sysroot, pb)?;
            } else if name.ends_with(".gz") {
                extract_tar(MultiGzDecoder::new(Cursor::new(payload)), sysroot, pb)?;
            } else {
                extract_tar(Cursor::new(payload), sysroot, pb)?;
            }
            return Ok(());
        }

        let skip = size + (size % 2);
        skip_exact(&mut file, skip)?;
    }

    Err(format!("no data.tar.* found inside {}", deb_path.display()))
}

/// Extracts an Alpine `.apk` package (multi-member `.tar.gz`) into `sysroot`.
pub fn extract_apk(apk_path: &Path, sysroot: &Path, pb: &ProgressBar) -> Result<(), String> {
    let file =
        File::open(apk_path).map_err(|e| format!("failed opening {}: {e}", apk_path.display()))?;
    extract_tar(MultiGzDecoder::new(file), sysroot, pb)
}

fn extract_tar<R: Read>(mut reader: R, sysroot: &Path, pb: &ProgressBar) -> Result<(), String> {
    let mut header = [0u8; 512];
    let mut next_long_name: Option<String> = None;
    let mut next_long_link: Option<String> = None;

    loop {
        if !read_exact_or_eof(&mut reader, &mut header)? {
            break;
        }
        if header.iter().all(|&b| b == 0) {
            continue;
        }

        let typeflag = header[156];
        let size = parse_tar_size(&header[124..136])?;
        let pad = (512 - (size % 512)) % 512;

        // GNU long name / long link / PAX extended header
        if typeflag == b'L' {
            let data = read_vec(&mut reader, size)?;
            skip_exact(&mut reader, pad)?;
            next_long_name = Some(
                String::from_utf8_lossy(&data)
                    .trim_end_matches('\0')
                    .to_string(),
            );
            continue;
        }
        if typeflag == b'K' {
            let data = read_vec(&mut reader, size)?;
            skip_exact(&mut reader, pad)?;
            next_long_link = Some(
                String::from_utf8_lossy(&data)
                    .trim_end_matches('\0')
                    .to_string(),
            );
            continue;
        }
        if typeflag == b'x' || typeflag == b'g' {
            let data = read_vec(&mut reader, size)?;
            skip_exact(&mut reader, pad)?;
            if typeflag == b'x' {
                parse_pax_records(&data, &mut next_long_name, &mut next_long_link);
            }
            continue;
        }

        let raw_name = next_long_name
            .take()
            .unwrap_or_else(|| parse_tar_name(&header));
        let raw_link = next_long_link
            .take()
            .unwrap_or_else(|| trim_nul(&header[157..257]).to_string());

        let clean_name = raw_name.trim_start_matches("./").trim_start_matches('/');
        // Skip Alpine APK metadata entries and docs/manpages.
        if clean_name.is_empty()
            || clean_name.starts_with(".SIGN.")
            || clean_name == ".PKGINFO"
            || clean_name == ".trigger"
            || clean_name.starts_with("usr/share/doc/")
            || clean_name.starts_with("usr/share/man/")
        {
            skip_exact(&mut reader, size + pad)?;
            continue;
        }

        let rel_path = sanitize_relative_path(clean_name)?;
        let out_path = sysroot.join(&rel_path);
        #[cfg(unix)]
        let mode = parse_tar_octal(&header[100..108]).unwrap_or(0o644) as u32;

        match typeflag {
            b'5' => {
                fs::create_dir_all(&out_path)
                    .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
                skip_exact(&mut reader, size + pad)?;
            }
            b'2' => {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
                }
                let target = relativize_symlink_target(&rel_path, &raw_link);
                create_symlink(&target, &out_path)?;
                skip_exact(&mut reader, size + pad)?;
            }
            b'1' => {
                // Hardlink inside sysroot -> copy or symlink relative to target
                let link_clean = raw_link.trim_start_matches("./").trim_start_matches('/');
                let link_rel = sanitize_relative_path(link_clean)?;
                let target_path = sysroot.join(&link_rel);
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
                }
                if target_path.is_file() {
                    let _ = fs::copy(&target_path, &out_path);
                }
                skip_exact(&mut reader, size + pad)?;
            }
            b'0' | 0 => {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
                }
                if out_path.symlink_metadata().is_ok() {
                    let _ = fs::remove_file(&out_path);
                }
                let mut out_file = File::create(&out_path)
                    .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
                let mut limited = (&mut reader).take(size);
                let written = io::copy(&mut limited, &mut out_file)
                    .map_err(|e| format!("failed extracting {}: {e}", out_path.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        fs::set_permissions(&out_path, fs::Permissions::from_mode(mode & 0o777));
                }
                skip_exact(&mut reader, pad)?;
                let new_pos = pb.position() + written;
                if new_pos > pb.length().unwrap_or(0) {
                    pb.set_length(new_pos);
                }
                pb.set_position(new_pos);
            }
            _ => {
                skip_exact(&mut reader, size + pad)?;
            }
        }
    }

    Ok(())
}

/// Rewrites absolute paths inside Debian GNU linker scripts (`libc.so`, `libm.so`, etc.)
/// so `clang --sysroot=... -fuse-ld=lld` resolves them inside `sysroot` on any host OS.
pub fn fixup_glibc_linker_scripts(sysroot: &Path, multiarch: &str) -> Result<(), String> {
    let lib_dir = sysroot.join("usr/lib").join(multiarch);
    for name in ["libc.so", "libm.so", "libpthread.so"] {
        let path = lib_dir.join(name);
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        // Only touch ASCII linker scripts (never ELF binaries).
        if bytes.starts_with(b"\x7fELF") {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let fixed = rewrite_linker_script_paths(&text, multiarch);
        if fixed != text {
            fs::write(&path, fixed)
                .map_err(|e| format!("failed updating linker script {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

pub fn rewrite_linker_script_paths(text: &str, multiarch: &str) -> String {
    let abs_usr_lib = format!("/usr/lib/{multiarch}/");
    let abs_lib = format!("/lib/{multiarch}/");
    text.replace(&abs_usr_lib, "")
        .replace(&abs_lib, &format!("../../../lib/{multiarch}/"))
}

/// Converts absolute symlink targets like `/lib/x86_64-linux-gnu/libc.so.6` into
/// relative targets (e.g. `../../../lib/x86_64-linux-gnu/libc.so.6`) so symlinks
/// stay self-contained inside `sysroot`.
pub fn relativize_symlink_target(link_rel_path: &Path, raw_target: &str) -> String {
    let Some(stripped) = raw_target.strip_prefix('/') else {
        return raw_target.to_string();
    };
    let depth = link_rel_path
        .parent()
        .map(|p| p.components().count())
        .unwrap_or(0);
    let mut rel = String::new();
    for _ in 0..depth {
        rel.push_str("../");
    }
    rel.push_str(stripped);
    rel
}

fn parse_pax_records(data: &[u8], name: &mut Option<String>, link: &mut Option<String>) {
    let text = String::from_utf8_lossy(data);
    for line in text.lines() {
        if let Some((_, kv)) = line.split_once(' ') {
            if let Some(v) = kv.strip_prefix("path=") {
                *name = Some(v.to_string());
            } else if let Some(v) = kv.strip_prefix("linkpath=") {
                *link = Some(v.to_string());
            }
        }
    }
}

fn parse_tar_name(header: &[u8; 512]) -> String {
    let name_part = trim_nul(&header[0..100]);
    let prefix_part = trim_nul(&header[345..500]);
    if prefix_part.is_empty() {
        name_part.to_string()
    } else {
        format!("{prefix_part}/{name_part}")
    }
}

fn parse_tar_size(bytes: &[u8]) -> Result<u64, String> {
    // Support GNU base-256 binary size encoding when high bit is set.
    if bytes.first().is_some_and(|&b| b & 0x80 != 0) {
        let mut val: u64 = 0;
        for &b in &bytes[1..] {
            val = (val << 8) | (b as u64);
        }
        return Ok(val);
    }
    parse_tar_octal(bytes)
}

fn parse_tar_octal(bytes: &[u8]) -> Result<u64, String> {
    let s = trim_nul(bytes).trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|e| format!("invalid octal `{s}` in tar header: {e}"))
}

fn trim_nul(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or("")
}

fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<bool, String> {
    let mut read = 0;
    while read < buf.len() {
        let n = r
            .read(&mut buf[read..])
            .map_err(|e| format!("archive read error: {e}"))?;
        if n == 0 {
            if read == 0 {
                return Ok(false);
            }
            return Err("unexpected EOF in archive".to_string());
        }
        read += n;
    }
    Ok(true)
}

fn read_vec<R: Read>(r: &mut R, size: u64) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; size as usize];
    r.read_exact(&mut buf)
        .map_err(|e| format!("archive read error: {e}"))?;
    Ok(buf)
}

fn skip_exact<R: Read>(r: &mut R, mut count: u64) -> Result<(), String> {
    let mut sink = [0u8; 8192];
    while count > 0 {
        let to_read = (count as usize).min(sink.len());
        let n = r
            .read(&mut sink[..to_read])
            .map_err(|e| format!("archive skip error: {e}"))?;
        if n == 0 {
            break;
        }
        count -= n as u64;
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

#[cfg(unix)]
fn create_symlink(target: &str, link: &Path) -> Result<(), String> {
    if link.exists() || link.symlink_metadata().is_ok() {
        let _ = fs::remove_file(link);
    }
    std::os::unix::fs::symlink(target, link).map_err(|e| {
        format!(
            "failed to create symlink {} -> {target}: {e}",
            link.display()
        )
    })
}

#[cfg(not(unix))]
fn create_symlink(target: &str, link: &Path) -> Result<(), String> {
    fs::write(link, target)
        .map_err(|e| format!("failed writing symlink placeholder {}: {e}", link.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_debian_libc_linker_script_to_relative_paths() {
        let orig = "OUTPUT_FORMAT(elf64-x86-64)\nGROUP ( /lib/x86_64-linux-gnu/libc.so.6 /usr/lib/x86_64-linux-gnu/libc_nonshared.a  AS_NEEDED ( /lib64/ld-linux-x86-64.so.2 ) )\n";
        let fixed = rewrite_linker_script_paths(orig, "x86_64-linux-gnu");
        assert!(fixed.contains("../../../lib/x86_64-linux-gnu/libc.so.6"));
        assert!(fixed.contains(" libc_nonshared.a "));
    }

    #[test]
    fn relativizes_absolute_symlinks_inside_sysroot() {
        assert_eq!(
            relativize_symlink_target(
                Path::new("usr/lib/x86_64-linux-gnu/libm.so"),
                "/lib/x86_64-linux-gnu/libm.so.6"
            ),
            "../../../lib/x86_64-linux-gnu/libm.so.6"
        );
        assert_eq!(
            relativize_symlink_target(Path::new("usr/lib/foo.so"), "foo.so.1"),
            "foo.so.1"
        );
    }
}

//! Pure-Rust Apple `.pkg` (`xar` -> `pbzx` -> `cpio`) extractor.
//!
//! Extracts `MacOSX*.sdk/` directly from `CLTools_macOS*_SDK.pkg` without
//! invoking any macOS-specific external binaries (`pkgutil`, `xar`, `pbzx`, `cpio`).

use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use flate2::read::ZlibDecoder;
use indicatif::ProgressBar;

const XAR_MAGIC: &[u8; 4] = b"xar!";
const PBZX_MAGIC: &[u8; 4] = b"pbzx";
const XZ_MAGIC: &[u8; 6] = b"\xfd7zXZ\0";

/// Extracts the `MacOSX*.sdk/` tree from an Apple `.pkg` into `dest_sdk_dir` (e.g. `<out>/MacOSX.sdk`).
pub fn extract_macos_sdk_pkg(
    pkg_path: &Path,
    dest_sdk_dir: &Path,
    pb: &ProgressBar,
) -> Result<usize, String> {
    let mut file =
        File::open(pkg_path).map_err(|e| format!("failed to open {}: {e}", pkg_path.display()))?;

    let (payload_offset, payload_len) = locate_xar_payload(&mut file)?;
    file.seek(SeekFrom::Start(payload_offset))
        .map_err(|e| format!("failed seeking to Payload in {}: {e}", pkg_path.display()))?;

    let limited = file.take(payload_len);
    let mut pbzx = PbzxReader::new(BufReader::new(limited))?;

    pb.reset();
    pb.set_length(payload_len);

    let extracted = extract_cpio_sdk(&mut pbzx, dest_sdk_dir, pb)?;
    if extracted == 0 {
        return Err(format!(
            "no MacOSX*.sdk entries found inside {}",
            pkg_path.display()
        ));
    }
    Ok(extracted)
}

/// Parses the `xar` header and zlib-compressed XML TOC to locate the `Payload` entry.
fn locate_xar_payload<R: Read + Seek>(reader: &mut R) -> Result<(u64, u64), String> {
    let mut hdr = [0u8; 28];
    reader
        .read_exact(&mut hdr)
        .map_err(|e| format!("failed reading xar header: {e}"))?;
    if &hdr[0..4] != XAR_MAGIC {
        return Err("invalid Apple .pkg: missing `xar!` magic".to_string());
    }
    let header_size = u16::from_be_bytes(hdr[4..6].try_into().unwrap()) as u64;
    let toc_len_compressed = u64::from_be_bytes(hdr[8..16].try_into().unwrap());
    let heap_start = header_size + toc_len_compressed;

    reader
        .seek(SeekFrom::Start(header_size))
        .map_err(|e| format!("failed seeking to xar TOC: {e}"))?;
    let mut compressed_toc = vec![0u8; toc_len_compressed as usize];
    reader
        .read_exact(&mut compressed_toc)
        .map_err(|e| format!("failed reading xar TOC: {e}"))?;

    let mut toc_xml = String::new();
    ZlibDecoder::new(Cursor::new(compressed_toc))
        .read_to_string(&mut toc_xml)
        .map_err(|e| format!("failed decompressing xar TOC: {e}"))?;

    let (rel_offset, length) = parse_xar_toc_payload(&toc_xml)?;
    Ok((heap_start + rel_offset, length))
}

fn parse_xar_toc_payload(toc_xml: &str) -> Result<(u64, u64), String> {
    // Locate the `<file ...>...</file>` element enclosing `<name>Payload</name>`.
    let Some(name_pos) = toc_xml.find("<name>Payload</name>") else {
        return Err("xar TOC does not contain a `Payload` entry".to_string());
    };
    let file_start = toc_xml[..name_pos].rfind("<file").unwrap_or(0);
    let file_end = toc_xml[name_pos..]
        .find("</file>")
        .map(|rel| name_pos + rel)
        .unwrap_or(toc_xml.len());
    let section = &toc_xml[file_start..file_end];

    let offset = extract_xml_u64(section, "offset")
        .ok_or_else(|| "missing `<offset>` for Payload in xar TOC".to_string())?;
    let length = extract_xml_u64(section, "length")
        .ok_or_else(|| "missing `<length>` for Payload in xar TOC".to_string())?;
    Ok((offset, length))
}

fn extract_xml_u64(xml: &str, tag: &str) -> Option<u64> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    xml[start..end].trim().parse::<u64>().ok()
}

/// Streaming decompressor for Apple's `pbzx` chunked XZ format.
struct PbzxReader<R: Read> {
    inner: R,
    current_chunk: Vec<u8>,
    pos: usize,
    eof: bool,
}

impl<R: Read> PbzxReader<R> {
    fn new(mut inner: R) -> Result<Self, String> {
        let mut magic = [0u8; 4];
        inner
            .read_exact(&mut magic)
            .map_err(|e| format!("failed reading pbzx magic: {e}"))?;
        if &magic != PBZX_MAGIC {
            return Err("Payload is not in `pbzx` format".to_string());
        }
        let mut flags = [0u8; 8];
        inner
            .read_exact(&mut flags)
            .map_err(|e| format!("failed reading pbzx flags: {e}"))?;
        Ok(Self {
            inner,
            current_chunk: Vec::new(),
            pos: 0,
            eof: false,
        })
    }

    fn fill_next_chunk(&mut self) -> io::Result<bool> {
        if self.eof {
            return Ok(false);
        }
        let mut header = [0u8; 16];
        if !read_exact_or_eof(&mut self.inner, &mut header)? {
            self.eof = true;
            return Ok(false);
        }
        let uncompressed_size = u64::from_be_bytes(header[0..8].try_into().unwrap());
        let compressed_size = u64::from_be_bytes(header[8..16].try_into().unwrap());
        if compressed_size == 0 {
            self.eof = true;
            return Ok(false);
        }

        let mut comp = vec![0u8; compressed_size as usize];
        self.inner.read_exact(&mut comp)?;

        self.current_chunk.clear();
        self.pos = 0;

        if compressed_size == uncompressed_size || !comp.starts_with(XZ_MAGIC) {
            self.current_chunk = comp;
        } else {
            self.current_chunk
                .reserve(uncompressed_size.min(32 * 1024 * 1024) as usize);
            let mut cursor = Cursor::new(comp);
            lzma_rs::xz_decompress(&mut cursor, &mut self.current_chunk).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("pbzx XZ decompression failed: {e}"),
                )
            })?;
        }
        Ok(true)
    }
}

impl<R: Read> Read for PbzxReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.current_chunk.len() {
            if !self.fill_next_chunk()? {
                return Ok(0);
            }
        }
        let avail = &self.current_chunk[self.pos..];
        let n = avail.len().min(buf.len());
        buf[..n].copy_from_slice(&avail[..n]);
        self.pos += n;
        Ok(n)
    }
}

fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<bool> {
    let mut read = 0;
    while read < buf.len() {
        let n = r.read(&mut buf[read..])?;
        if n == 0 {
            if read == 0 {
                return Ok(false);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated pbzx chunk header",
            ));
        }
        read += n;
    }
    Ok(true)
}

/// Extracts `MacOSX*.sdk/` entries from a POSIX ODC (`070707`) or SVR4 Newc (`070701`) cpio stream.
fn extract_cpio_sdk<R: Read>(
    reader: &mut R,
    dest_sdk_dir: &Path,
    pb: &ProgressBar,
) -> Result<usize, String> {
    let mut extracted = 0usize;
    let mut total_unpacked: u64 = 0;

    loop {
        let mut magic = [0u8; 6];
        if !read_exact_or_eof(reader, &mut magic)
            .map_err(|e| format!("failed reading cpio magic: {e}"))?
        {
            break;
        }

        let (mode, namesize, filesize, align4) = if &magic == b"070707" {
            let mut rest = [0u8; 70];
            reader
                .read_exact(&mut rest)
                .map_err(|e| format!("failed reading cpio ODC header: {e}"))?;
            let mode = parse_octal(&rest[12..18])?;
            let namesize = parse_octal(&rest[53..59])? as usize;
            let filesize = parse_octal(&rest[59..70])?;
            (mode, namesize, filesize, false)
        } else if &magic == b"070701" || &magic == b"070702" {
            let mut rest = [0u8; 104];
            reader
                .read_exact(&mut rest)
                .map_err(|e| format!("failed reading cpio newc header: {e}"))?;
            let mode = parse_hex(&rest[8..16])?;
            let filesize = parse_hex(&rest[48..56])?;
            let namesize = parse_hex(&rest[88..96])? as usize;
            (mode, namesize, filesize, true)
        } else {
            return Err(format!(
                "unsupported cpio magic: {:?}",
                String::from_utf8_lossy(&magic)
            ));
        };

        let mut name_buf = vec![0u8; namesize];
        reader
            .read_exact(&mut name_buf)
            .map_err(|e| format!("failed reading cpio filename: {e}"))?;
        if align4 {
            let pad = (4 - ((110 + namesize) % 4)) % 4;
            skip_bytes(reader, pad as u64)?;
        }

        let raw_name = String::from_utf8_lossy(&name_buf)
            .trim_end_matches('\0')
            .to_string();
        if raw_name == "TRAILER!!!" {
            break;
        }

        let rel_opt = strip_macos_sdk_prefix(&raw_name);
        let file_type = mode & 0o170000;

        if let Some(rel_str) = rel_opt {
            if !rel_str.is_empty() {
                let rel_path = sanitize_relative_path(rel_str)?;
                let out_path = dest_sdk_dir.join(&rel_path);

                match file_type {
                    0o040000 => {
                        // Directory
                        fs::create_dir_all(&out_path)
                            .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
                        skip_bytes(reader, filesize)?;
                    }
                    0o120000 => {
                        // Symlink
                        let mut target_buf = vec![0u8; filesize as usize];
                        reader
                            .read_exact(&mut target_buf)
                            .map_err(|e| format!("failed reading symlink target: {e}"))?;
                        let target = String::from_utf8_lossy(&target_buf).into_owned();
                        if let Some(parent) = out_path.parent() {
                            fs::create_dir_all(parent).map_err(|e| {
                                format!("failed creating {}: {e}", parent.display())
                            })?;
                        }
                        create_symlink(&target, &out_path)?;
                        extracted += 1;
                    }
                    _ => {
                        // Regular file
                        if let Some(parent) = out_path.parent() {
                            fs::create_dir_all(parent).map_err(|e| {
                                format!("failed creating {}: {e}", parent.display())
                            })?;
                        }
                        let mut out_file = File::create(&out_path)
                            .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
                        let mut limited = reader.take(filesize);
                        io::copy(&mut limited, &mut out_file).map_err(|e| {
                            format!("failed extracting {}: {e}", out_path.display())
                        })?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = fs::set_permissions(
                                &out_path,
                                fs::Permissions::from_mode((mode as u32) & 0o777),
                            );
                        }
                        extracted += 1;
                    }
                }
            } else {
                skip_bytes(reader, filesize)?;
            }
        } else {
            skip_bytes(reader, filesize)?;
        }

        if align4 {
            let pad = (4 - (filesize % 4)) % 4;
            skip_bytes(reader, pad)?;
        }

        total_unpacked += filesize;
        if total_unpacked > pb.length().unwrap_or(0) {
            pb.set_length(total_unpacked);
        }
        pb.set_position(total_unpacked);
    }

    pb.set_length(total_unpacked);
    pb.set_position(total_unpacked);
    Ok(extracted)
}

fn strip_macos_sdk_prefix(path: &str) -> Option<&str> {
    // Matches paths like `./Library/Developer/CommandLineTools/SDKs/MacOSX15.2.sdk/usr/include/...`
    let idx = path.find("MacOSX")?;
    let rest = &path[idx..];
    let sdk_end = rest.find(".sdk")? + 4;
    let after = &rest[sdk_end..];
    Some(after.strip_prefix('/').unwrap_or(after))
}

fn skip_bytes<R: Read>(reader: &mut R, mut count: u64) -> Result<(), String> {
    let mut sink = [0u8; 8192];
    while count > 0 {
        let to_read = (count as usize).min(sink.len());
        let n = reader
            .read(&mut sink[..to_read])
            .map_err(|e| format!("failed skipping cpio bytes: {e}"))?;
        if n == 0 {
            return Err("unexpected EOF while skipping cpio entry".to_string());
        }
        count -= n as u64;
    }
    Ok(())
}

fn parse_octal(bytes: &[u8]) -> Result<u64, String> {
    let s =
        std::str::from_utf8(bytes).map_err(|_| "invalid UTF-8 in cpio octal field".to_string())?;
    u64::from_str_radix(s.trim(), 8).map_err(|e| format!("invalid octal `{s}` in cpio header: {e}"))
}

fn parse_hex(bytes: &[u8]) -> Result<u64, String> {
    let s =
        std::str::from_utf8(bytes).map_err(|_| "invalid UTF-8 in cpio hex field".to_string())?;
    u64::from_str_radix(s.trim(), 16).map_err(|e| format!("invalid hex `{s}` in cpio header: {e}"))
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
    fn strips_macos_sdk_prefix_cleanly() {
        assert_eq!(
            strip_macos_sdk_prefix(
                "./Library/Developer/CommandLineTools/SDKs/MacOSX15.2.sdk/usr/include/stdio.h"
            ),
            Some("usr/include/stdio.h")
        );
        assert_eq!(
            strip_macos_sdk_prefix("./Library/Developer/CommandLineTools/SDKs/MacOSX.sdk"),
            Some("")
        );
        assert_eq!(strip_macos_sdk_prefix("./Library/Other/foo"), None);
    }

    #[test]
    fn parses_xar_payload_location() {
        let xml = r#"
            <xar><toc><file id="1">
                <name>Payload</name>
                <data><length>55123456</length><offset>4096</offset></data>
            </file></toc></xar>
        "#;
        let (off, len) = parse_xar_toc_payload(xml).unwrap();
        assert_eq!(off, 4096);
        assert_eq!(len, 55_123_456);
    }
}

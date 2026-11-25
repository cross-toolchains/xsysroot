//! Pure-Rust Apple `Xcode.xip` (`xar` -> `pbzx` -> `cpio`) extractor for iOS SDKs.
//!
//! Extracts `iPhoneOS*.sdk/` (and optionally `iPhoneSimulator*.sdk/`) directly
//! from a user-supplied `Xcode.xip` archive without invoking macOS-only tools.

use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use flate2::read::ZlibDecoder;
use indicatif::ProgressBar;

const XAR_MAGIC: &[u8; 4] = b"xar!";
const PBZX_MAGIC: &[u8; 4] = b"pbzx";
const XZ_MAGIC: &[u8; 6] = b"\xfd7zXZ\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IosSdkKind {
    Device,
    Simulator,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ExtractStats {
    pub device_files: usize,
    pub simulator_files: usize,
}

/// Extracts `iPhoneOS.sdk` (and optionally `iPhoneSimulator.sdk`) from an `Xcode.xip`
/// archive into `<output_dir>/iPhoneOS.sdk` and `<output_dir>/iPhoneSimulator.sdk`.
pub fn extract_ios_sdks_from_xip(
    xip_path: &Path,
    output_dir: &Path,
    include_simulator: bool,
    pb: &ProgressBar,
) -> Result<ExtractStats, String> {
    let mut file = File::open(xip_path)
        .map_err(|e| format!("failed to open {}: {e}", xip_path.display()))?;

    let (content_offset, content_len) = locate_xar_content(&mut file)?;
    file.seek(SeekFrom::Start(content_offset))
        .map_err(|e| format!("failed seeking to Content in {}: {e}", xip_path.display()))?;

    let limited = file.take(content_len);
    let mut pbzx = PbzxReader::new(BufReader::new(limited))?;

    pb.reset();
    pb.set_length(content_len);

    let stats = extract_cpio_ios_sdks(&mut pbzx, output_dir, include_simulator, pb)?;
    if stats.device_files == 0 && stats.simulator_files == 0 {
        return Err(format!(
            "no iPhoneOS*.sdk entries found inside {}",
            xip_path.display()
        ));
    }
    Ok(stats)
}

fn locate_xar_content<R: Read + Seek>(reader: &mut R) -> Result<(u64, u64), String> {
    let mut hdr = [0u8; 28];
    reader
        .read_exact(&mut hdr)
        .map_err(|e| format!("failed reading xar header: {e}"))?;
    if &hdr[0..4] != XAR_MAGIC {
        return Err("invalid Xcode .xip: missing `xar!` magic".to_string());
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

    let (rel_offset, length) = parse_xar_toc_content(&toc_xml)?;
    Ok((heap_start + rel_offset, length))
}

fn parse_xar_toc_content(toc_xml: &str) -> Result<(u64, u64), String> {
    let marker = if toc_xml.contains("<name>Content</name>") {
        "<name>Content</name>"
    } else if toc_xml.contains("<name>Payload</name>") {
        "<name>Payload</name>"
    } else {
        return Err("xar TOC does not contain a `Content` or `Payload` entry".to_string());
    };
    let name_pos = toc_xml.find(marker).unwrap();
    let file_start = toc_xml[..name_pos].rfind("<file").unwrap_or(0);
    let file_end = toc_xml[name_pos..]
        .find("</file>")
        .map(|rel| name_pos + rel)
        .unwrap_or(toc_xml.len());
    let section = &toc_xml[file_start..file_end];

    let offset = extract_xml_u64(section, "offset")
        .ok_or_else(|| "missing `<offset>` for Content in xar TOC".to_string())?;
    let length = extract_xml_u64(section, "length")
        .ok_or_else(|| "missing `<length>` for Content in xar TOC".to_string())?;
    Ok((offset, length))
}

fn extract_xml_u64(xml: &str, tag: &str) -> Option<u64> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    xml[start..end].trim().parse::<u64>().ok()
}

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
            return Err("Xcode.xip Content is not in `pbzx` format".to_string());
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

fn extract_cpio_ios_sdks<R: Read>(
    reader: &mut R,
    output_dir: &Path,
    include_simulator: bool,
    pb: &ProgressBar,
) -> Result<ExtractStats, String> {
    let mut stats = ExtractStats::default();
    let mut total_unpacked: u64 = 0;
    let device_root = output_dir.join("iPhoneOS.sdk");
    let sim_root = output_dir.join("iPhoneSimulator.sdk");

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

        let matched = match classify_ios_sdk_entry(&raw_name) {
            Some((IosSdkKind::Device, rel)) => Some((IosSdkKind::Device, &device_root, rel)),
            Some((IosSdkKind::Simulator, rel)) if include_simulator => {
                Some((IosSdkKind::Simulator, &sim_root, rel))
            }
            _ => None,
        };

        if let Some((kind, dest_root, rel_str)) = matched
            && !rel_str.is_empty()
        {
            let rel_path = sanitize_relative_path(rel_str)?;
            let out_path = dest_root.join(&rel_path);
            let file_type = mode & 0o170000;

            match file_type {
                0o040000 => {
                    fs::create_dir_all(&out_path)
                        .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
                    skip_bytes(reader, filesize)?;
                }
                0o120000 => {
                    let mut target_buf = vec![0u8; filesize as usize];
                    reader
                        .read_exact(&mut target_buf)
                        .map_err(|e| format!("failed reading symlink target: {e}"))?;
                    let target = String::from_utf8_lossy(&target_buf).into_owned();
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
                    }
                    create_symlink(&target, &out_path)?;
                    match kind {
                        IosSdkKind::Device => stats.device_files += 1,
                        IosSdkKind::Simulator => stats.simulator_files += 1,
                    }
                }
                _ => {
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
                    }
                    let mut out_file = File::create(&out_path)
                        .map_err(|e| format!("failed creating {}: {e}", out_path.display()))?;
                    let mut limited = reader.take(filesize);
                    io::copy(&mut limited, &mut out_file)
                        .map_err(|e| format!("failed extracting {}: {e}", out_path.display()))?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = fs::set_permissions(
                            &out_path,
                            fs::Permissions::from_mode((mode as u32) & 0o777),
                        );
                    }
                    match kind {
                        IosSdkKind::Device => stats.device_files += 1,
                        IosSdkKind::Simulator => stats.simulator_files += 1,
                    }
                }
            }
            total_unpacked += filesize;
            if total_unpacked > pb.length().unwrap_or(0) {
                pb.set_length(total_unpacked);
            }
            pb.set_position(total_unpacked);
        } else {
            skip_bytes(reader, filesize)?;
        }

        if align4 {
            let pad = (4 - (filesize % 4)) % 4;
            skip_bytes(reader, pad)?;
        }
    }

    pb.set_length(total_unpacked);
    pb.set_position(total_unpacked);
    Ok(stats)
}

pub fn classify_ios_sdk_entry(path: &str) -> Option<(IosSdkKind, &str)> {
    if let Some(idx) = path.find("iPhoneOS.platform/Developer/SDKs/iPhoneOS") {
        let rest = &path[idx..];
        let sdk_end = rest.find(".sdk")? + 4;
        let after = &rest[sdk_end..];
        return Some((IosSdkKind::Device, after.strip_prefix('/').unwrap_or(after)));
    }
    if let Some(idx) = path.find("iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator") {
        let rest = &path[idx..];
        let sdk_end = rest.find(".sdk")? + 4;
        let after = &rest[sdk_end..];
        return Some((
            IosSdkKind::Simulator,
            after.strip_prefix('/').unwrap_or(after),
        ));
    }
    None
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
    let s = std::str::from_utf8(bytes)
        .map_err(|_| "invalid UTF-8 in cpio octal field".to_string())?;
    u64::from_str_radix(s.trim(), 8)
        .map_err(|e| format!("invalid octal `{s}` in cpio header: {e}"))
}

fn parse_hex(bytes: &[u8]) -> Result<u64, String> {
    let s = std::str::from_utf8(bytes)
        .map_err(|_| "invalid UTF-8 in cpio hex field".to_string())?;
    u64::from_str_radix(s.trim(), 16)
        .map_err(|e| format!("invalid hex `{s}` in cpio header: {e}"))
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
pub(crate) fn create_symlink(target: &str, link: &Path) -> Result<(), String> {
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
pub(crate) fn create_symlink(target: &str, link: &Path) -> Result<(), String> {
    fs::write(link, target)
        .map_err(|e| format!("failed writing symlink placeholder {}: {e}", link.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_device_and_simulator_sdk_paths() {
        assert_eq!(
            classify_ios_sdk_entry(
                "Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS18.2.sdk/usr/include/stdio.h"
            ),
            Some((IosSdkKind::Device, "usr/include/stdio.h"))
        );
        assert_eq!(
            classify_ios_sdk_entry(
                "Xcode.app/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator18.2.sdk/SDKSettings.json"
            ),
            Some((IosSdkKind::Simulator, "SDKSettings.json"))
        );
        assert_eq!(
            classify_ios_sdk_entry(
                "Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk/usr/include/stdio.h"
            ),
            None
        );
    }

    #[test]
    fn parses_xar_content_location() {
        let xml = r#"
            <xar><toc><file id="1">
                <data><length>3456789012</length><offset>8192</offset></data>
                <name>Content</name>
            </file></toc></xar>
        "#;
        let (off, len) = parse_xar_toc_content(xml).unwrap();
        assert_eq!(off, 8192);
        assert_eq!(len, 3_456_789_012);
    }
}

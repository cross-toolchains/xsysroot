//! OpenHarmony release metadata and streaming `ohos-sdk-*.tar.gz` -> `native-*.zip` fetcher.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::Duration;

use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};

pub const DEFAULT_OHOS_VERSION: &str = "6.1-Release";

pub const KNOWN_RELEASES: &[&str] = &[
    "7.0-Release",
    "6.1-Release",
    "6.0-Release",
    "5.1.0-Release",
    "5.0.3-Release",
];

/// `(wrapper_prefix, clang_target_triple)` pairs for OpenHarmony ABIs.
pub const OHOS_TARGETS: &[(&str, &str)] = &[
    ("aarch64-unknown-linux-ohos", "aarch64-linux-ohos"),
    ("armv7-unknown-linux-ohos", "arm-linux-ohos"),
    ("x86_64-unknown-linux-ohos", "x86_64-linux-ohos"),
];

/// Returns the official Huawei Cloud OpenHarmony public SDK archive URL.
///
/// We always use the smaller macOS public bundle (~1.3 GiB vs ~4.5 GiB) because
/// `native/sysroot/` is 100% host-independent target headers and `.so` stubs.
pub fn ohos_sdk_url(version: &str) -> String {
    format!("https://repo.huaweicloud.com/openharmony/os/{version}/ohos-sdk-mac-public.tar.gz")
}

pub fn format_status(icon: &str, label: &str) -> String {
    format!("{icon}  {label:>11}")
}

pub fn make_bytes_bar(prefix: &'static str, len: u64, icon: &str, label: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    if let Ok(style) = ProgressStyle::with_template(
        "{spinner:.green} {prefix:<7.bold} [{elapsed:>3}] {wide_bar:.green} {binary_bytes:>10}/{binary_total_bytes:>10}  {msg}",
    ) {
        pb.set_style(style.progress_chars("█▄ "));
    }
    pb.set_prefix(prefix);
    pb.set_message(format_status(icon, label));
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Streams a remote `ohos-sdk-*.tar.gz` and extracts ONLY the inner `native-*.zip`
/// to `dest_zip`, aborting the HTTP download immediately once `native-*.zip` is read.
pub fn fetch_native_zip_from_url(
    url: &str,
    dest_zip: &Path,
    pb: &ProgressBar,
) -> Result<u64, String> {
    let agent = ureq::Agent::new_with_defaults();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("failed to fetch {url}: {e}"))?;
    let content_len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if content_len > 0 {
        pb.set_length(content_len);
    }

    let counting = CountingReader {
        inner: resp.into_body().into_reader(),
        pb,
    };
    extract_native_zip_from_targz_reader(counting, dest_zip)
}

/// Extracts ONLY the inner `native-*.zip` from a local `ohos-sdk-*.tar.gz`.
pub fn extract_native_zip_from_targz_file(
    targz_path: &Path,
    dest_zip: &Path,
    pb: &ProgressBar,
) -> Result<u64, String> {
    let file = File::open(targz_path)
        .map_err(|e| format!("failed to open {}: {e}", targz_path.display()))?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    pb.set_length(len);
    let counting = CountingReader { inner: file, pb };
    extract_native_zip_from_targz_reader(counting, dest_zip)
}

fn extract_native_zip_from_targz_reader<R: Read>(
    reader: R,
    dest_zip: &Path,
) -> Result<u64, String> {
    let mut gz = GzDecoder::new(reader);
    let mut header = [0u8; 512];

    loop {
        if !read_exact_or_eof(&mut gz, &mut header)
            .map_err(|e| format!("failed reading tar header: {e}"))?
        {
            break;
        }
        // Two consecutive zero blocks mark end-of-archive.
        if header.iter().all(|&b| b == 0) {
            break;
        }

        let name = parse_tar_name(&header);
        let size = parse_tar_octal(&header[124..136])?;
        let padded_skip = (512 - (size % 512)) % 512;

        if is_native_zip_entry(&name) {
            if let Some(parent) = dest_zip.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
            }
            let tmp_zip = dest_zip.with_extension("zip.part");
            let mut out = File::create(&tmp_zip)
                .map_err(|e| format!("failed to create {}: {e}", tmp_zip.display()))?;
            let mut limited = (&mut gz).take(size);
            let copied = io::copy(&mut limited, &mut out)
                .map_err(|e| format!("failed extracting {name}: {e}"))?;
            drop(out);
            fs::rename(&tmp_zip, dest_zip).map_err(|e| {
                format!(
                    "failed finalizing {} -> {}: {e}",
                    tmp_zip.display(),
                    dest_zip.display()
                )
            })?;
            return Ok(copied);
        }

        skip_bytes(&mut gz, size + padded_skip)?;
    }

    Err("no `native-*.zip` entry found inside OpenHarmony SDK archive".to_string())
}

pub fn is_native_zip_entry(name: &str) -> bool {
    let file_name = name.rsplit('/').next().unwrap_or(name);
    file_name.starts_with("native-") && file_name.ends_with(".zip")
}

struct CountingReader<'a, R: Read> {
    inner: R,
    pb: &'a ProgressBar,
}

impl<R: Read> Read for CountingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.pb.inc(n as u64);
        }
        Ok(n)
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

fn trim_nul(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or("")
}

fn parse_tar_octal(bytes: &[u8]) -> Result<u64, String> {
    let s = trim_nul(bytes).trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|e| format!("invalid octal `{s}` in tar header: {e}"))
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
                "truncated tar block",
            ));
        }
        read += n;
    }
    Ok(true)
}

fn skip_bytes<R: Read>(reader: &mut R, mut count: u64) -> Result<(), String> {
    let mut sink = [0u8; 16 * 1024];
    while count > 0 {
        let to_read = (count as usize).min(sink.len());
        let n = reader
            .read(&mut sink[..to_read])
            .map_err(|e| format!("failed skipping tar bytes: {e}"))?;
        if n == 0 {
            return Err("unexpected EOF while skipping tar entry".to_string());
        }
        count -= n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_official_openharmony_sdk_url() {
        assert_eq!(
            ohos_sdk_url("5.0.0-Release"),
            "https://repo.huaweicloud.com/openharmony/os/5.0.0-Release/ohos-sdk-mac-public.tar.gz"
        );
    }

    #[test]
    fn identifies_native_zip_entries() {
        assert!(is_native_zip_entry("ohos-sdk/darwin/native-darwin-x64-5.0.0.71-Release.zip"));
        assert!(is_native_zip_entry("native-linux-x64-4.1.7.8-Release.zip"));
        assert!(!is_native_zip_entry("ohos-sdk/darwin/ets-darwin-x64-5.0.0.71-Release.zip"));
    }
}

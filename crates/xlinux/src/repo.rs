//! Upstream Debian (`Packages.gz`) and Alpine (`APKINDEX.tar.gz`) package index
//! parsers and HTTP download helpers.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::time::Duration;

use flate2::read::MultiGzDecoder;
use indicatif::{ProgressBar, ProgressStyle};

use crate::target::{DEFAULT_ALPINE_BRANCH, DebianSuite, LinuxTarget};

#[derive(Debug, Clone)]
pub struct RemotePkg {
    pub name: String,
    pub filename: String,
    pub url: String,
    pub size: u64,
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

/// Resolves minimal glibc sysroot `.deb` packages from Debian's `Packages.gz`.
pub fn resolve_debian_packages(
    target: &LinuxTarget,
    suite: DebianSuite,
) -> Result<Vec<RemotePkg>, String> {
    let index_url = format!(
        "{}/dists/{}/main/binary-{}/Packages.gz",
        suite.mirror,
        suite.codename,
        target.debian_arch()
    );
    let gz_bytes = fetch_url_with_bar("Index", &index_url)?;
    let mut text = String::new();
    MultiGzDecoder::new(Cursor::new(gz_bytes))
        .read_to_string(&mut text)
        .map_err(|e| format!("failed decompressing Debian Packages.gz: {e}"))?;

    let gcc_lib = format!("libgcc-{}-dev", suite.gcc_major);
    let stdcpp_dev = format!("libstdc++-{}-dev", suite.gcc_major);
    let wanted: Vec<&str> = vec![
        "libc6",
        "libc6-dev",
        "linux-libc-dev",
        "libgcc-s1",
        "libgcc1",
        "libstdc++6",
        &gcc_lib,
        &stdcpp_dev,
    ];
    let map = parse_debian_packages_index(&text, &wanted);

    let mut out = Vec::new();
    for pkg_name in wanted {
        if let Some((rel_path, size)) = map.get(pkg_name) {
            let file_name = rel_path.rsplit('/').next().unwrap_or(rel_path).to_string();
            out.push(RemotePkg {
                name: pkg_name.to_string(),
                filename: file_name,
                url: format!("{}/{rel_path}", suite.mirror),
                size: *size,
            });
        }
    }

    if !out.iter().any(|p| p.name == "libc6-dev") {
        return Err(format!(
            "failed to locate libc6-dev in Debian {} index",
            suite.codename
        ));
    }
    Ok(out)
}

/// Resolves minimal musl sysroot `.apk` packages from Alpine's `APKINDEX.tar.gz`.
pub fn resolve_alpine_packages(target: &LinuxTarget) -> Result<Vec<RemotePkg>, String> {
    let base_url = format!(
        "https://dl-cdn.alpinelinux.org/alpine/{DEFAULT_ALPINE_BRANCH}/main/{}",
        target.alpine_arch()
    );
    let index_url = format!("{base_url}/APKINDEX.tar.gz");
    let gz_bytes = fetch_url_with_bar("Index", &index_url)?;
    let index_text = extract_apkindex_text(&gz_bytes)?;

    let wanted = [
        "musl",
        "musl-dev",
        "linux-headers",
        "libgcc",
        "libstdc++",
        "libstdc++-dev",
    ];
    let map = parse_alpine_index(&index_text, &wanted);

    let mut out = Vec::new();
    for pkg_name in wanted {
        if let Some((version, size)) = map.get(pkg_name) {
            let filename = format!("{pkg_name}-{version}.apk");
            out.push(RemotePkg {
                name: pkg_name.to_string(),
                url: format!("{base_url}/{filename}"),
                filename,
                size: *size,
            });
        }
    }

    if !out.iter().any(|p| p.name == "musl-dev") {
        return Err("failed to locate musl-dev in Alpine APKINDEX".to_string());
    }
    Ok(out)
}

pub fn download_to_file(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    pb: &ProgressBar,
) -> Result<u64, String> {
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("failed downloading {url}: {e}"))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
    }
    let tmp = dest.with_extension("part");
    let mut out =
        File::create(&tmp).map_err(|e| format!("failed creating {}: {e}", tmp.display()))?;
    let mut reader = resp.into_body().into_reader();
    let mut chunk = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| format!("failed reading {url}: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&chunk[..n])
            .map_err(|e| format!("failed writing {}: {e}", tmp.display()))?;
        total += n as u64;
        pb.inc(n as u64);
    }
    drop(out);
    fs::rename(&tmp, dest).map_err(|e| {
        format!(
            "failed finalizing {} -> {}: {e}",
            tmp.display(),
            dest.display()
        )
    })?;
    Ok(total)
}

fn fetch_url_with_bar(prefix: &'static str, url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::Agent::new_with_defaults();
    let pb = make_bytes_bar(prefix, 0, "📥", "downloading");
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("failed fetching {url}: {e}"))?;
    let content_len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if content_len > 0 {
        pb.set_length(content_len);
    }

    let mut reader = resp.into_body().into_reader();
    let mut buf = Vec::with_capacity(content_len as usize);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| format!("failed reading {url}: {e}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        pb.inc(n as u64);
    }
    pb.set_length(buf.len() as u64);
    pb.set_position(buf.len() as u64);
    pb.finish_with_message(format_status("📥", "downloaded"));
    Ok(buf)
}

fn extract_apkindex_text(gz_bytes: &[u8]) -> Result<String, String> {
    let mut gz = MultiGzDecoder::new(Cursor::new(gz_bytes));
    let mut header = [0u8; 512];
    loop {
        if !read_exact_or_eof(&mut gz, &mut header)? {
            break;
        }
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let name = trim_nul(&header[0..100]);
        let size_str = trim_nul(&header[124..136]).trim();
        let size = u64::from_str_radix(size_str, 8).unwrap_or(0);
        let pad = (512 - (size % 512)) % 512;
        if name == "APKINDEX" {
            let mut data = vec![0u8; size as usize];
            gz.read_exact(&mut data)
                .map_err(|e| format!("failed reading APKINDEX entry: {e}"))?;
            return Ok(String::from_utf8_lossy(&data).into_owned());
        }
        skip_exact(&mut gz, size + pad)?;
    }
    Err("APKINDEX file not found inside APKINDEX.tar.gz".to_string())
}

fn parse_debian_packages_index(text: &str, wanted: &[&str]) -> HashMap<String, (String, u64)> {
    let mut map = HashMap::new();
    for stanza in text.split("\n\n") {
        let mut pkg = None;
        let mut filename = None;
        let mut size = 0u64;
        for line in stanza.lines() {
            if let Some(v) = line.strip_prefix("Package: ") {
                pkg = Some(v.trim());
            } else if let Some(v) = line.strip_prefix("Filename: ") {
                filename = Some(v.trim());
            } else if let Some(v) = line.strip_prefix("Size: ") {
                size = v.trim().parse::<u64>().unwrap_or(0);
            }
        }
        if let (Some(p), Some(f)) = (pkg, filename)
            && wanted.contains(&p)
        {
            map.insert(p.to_string(), (f.to_string(), size));
        }
    }
    map
}

fn parse_alpine_index(text: &str, wanted: &[&str]) -> HashMap<String, (String, u64)> {
    let mut map = HashMap::new();
    for stanza in text.split("\n\n") {
        let mut pkg = None;
        let mut ver = None;
        let mut size = 0u64;
        for line in stanza.lines() {
            if let Some(v) = line.strip_prefix("P:") {
                pkg = Some(v.trim());
            } else if let Some(v) = line.strip_prefix("V:") {
                ver = Some(v.trim());
            } else if let Some(v) = line.strip_prefix("S:") {
                size = v.trim().parse::<u64>().unwrap_or(0);
            }
        }
        if let (Some(p), Some(v)) = (pkg, ver)
            && wanted.contains(&p)
        {
            map.insert(p.to_string(), (v.to_string(), size));
        }
    }
    map
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
            .map_err(|e| format!("tar read error: {e}"))?;
        if n == 0 {
            return Ok(false);
        }
        read += n;
    }
    Ok(true)
}

fn skip_exact<R: Read>(r: &mut R, mut count: u64) -> Result<(), String> {
    let mut sink = [0u8; 8192];
    while count > 0 {
        let to_read = (count as usize).min(sink.len());
        let n = r
            .read(&mut sink[..to_read])
            .map_err(|e| format!("tar skip error: {e}"))?;
        if n == 0 {
            break;
        }
        count -= n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_debian_packages_stanzas() {
        let sample = "Package: libc6-dev\nSize: 2345678\nFilename: pool/main/g/glibc/libc6-dev_2.31_amd64.deb\n\nPackage: other\nSize: 100\nFilename: pool/other.deb\n";
        let map = parse_debian_packages_index(sample, &["libc6-dev"]);
        assert_eq!(
            map.get("libc6-dev"),
            Some(&(
                "pool/main/g/glibc/libc6-dev_2.31_amd64.deb".to_string(),
                2_345_678
            ))
        );
    }

    #[test]
    fn parses_alpine_apkindex_stanzas() {
        let sample = "P:musl-dev\nV:1.2.5-r9\nS:345678\n\nP:busybox\nV:1.37.0-r0\nS:500000\n";
        let map = parse_alpine_index(sample, &["musl-dev"]);
        assert_eq!(
            map.get("musl-dev"),
            Some(&("1.2.5-r9".to_string(), 345_678))
        );
    }
}

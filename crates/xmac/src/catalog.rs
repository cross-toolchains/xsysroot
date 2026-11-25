//! Apple Software Update Catalog (`.sucatalog`) discovery and HTTP download helpers.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

/// Public Apple Software Update Catalog containing Command Line Tools for Xcode.
pub const APPLE_SUCATALOG_URL: &str = "https://swscan.apple.com/content/catalogs/others/index-15-14-13-12-10.16-10.15-10.14-10.13-10.12-10.11-10.10-10.9-mountainlion-lion-snowleopard-leopard.merged-1.sucatalog";

/// Target triples for generated macOS clang wrappers.
pub const MACOS_TRIPLES: &[&str] = &["aarch64-apple-darwin", "x86_64-apple-darwin"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkPackage {
    pub product_id: String,
    pub post_date: String,
    pub kind: String,
    pub size: u64,
    pub url: String,
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

/// Downloads and parses Apple's public `.sucatalog` for standalone macOS SDK `.pkg` entries.
pub fn fetch_sdk_packages(show_progress: bool) -> Result<Vec<SdkPackage>, String> {
    let agent = ureq::Agent::new_with_defaults();
    let pb = if show_progress {
        Some(make_bytes_bar("Catalog", 0, "📥", "downloading"))
    } else {
        None
    };

    let resp = agent
        .get(APPLE_SUCATALOG_URL)
        .call()
        .map_err(|e| format!("failed to fetch Apple software update catalog: {e}"))?;
    let content_len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if let Some(bar) = &pb
        && content_len > 0
    {
        bar.set_length(content_len);
    }

    let mut reader = resp.into_body().into_reader();
    let mut raw = Vec::with_capacity(content_len as usize);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| format!("failed reading Apple catalog body: {e}"))?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..n]);
        if let Some(bar) = &pb {
            bar.inc(n as u64);
        }
    }

    if let Some(bar) = &pb {
        bar.set_length(raw.len() as u64);
        bar.set_position(raw.len() as u64);
        bar.finish_with_message(format_status("📥", "downloaded"));
    }

    let xml = String::from_utf8_lossy(&raw);
    let pkgs = parse_sucatalog_sdk_packages(&xml);
    if pkgs.is_empty() {
        return Err("no CLTools macOS SDK packages found in Apple software update catalog".to_string());
    }
    Ok(pkgs)
}

/// Streams a remote `.pkg` file to `dest_path` atomically while updating `pb`.
pub fn download_file(url: &str, dest_path: &Path, pb: &ProgressBar) -> Result<u64, String> {
    let agent = ureq::Agent::new_with_defaults();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("failed to download {url}: {e}"))?;
    let content_len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if content_len > 0 {
        pb.set_length(content_len);
    }

    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let tmp_path = dest_path.with_extension("pkg.part");
    let mut out = File::create(&tmp_path)
        .map_err(|e| format!("failed to create {}: {e}", tmp_path.display()))?;

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
            .map_err(|e| format!("failed writing {}: {e}", tmp_path.display()))?;
        total += n as u64;
        pb.inc(n as u64);
    }
    drop(out);

    fs::rename(&tmp_path, dest_path).map_err(|e| {
        format!(
            "failed to finalize {} -> {}: {e}",
            tmp_path.display(),
            dest_path.display()
        )
    })?;

    Ok(total)
}

/// Extracts `CLTools_macOS*_SDK.pkg` entries from Apple's XML `.sucatalog` plist.
pub fn parse_sucatalog_sdk_packages(xml: &str) -> Vec<SdkPackage> {
    let mut results = Vec::new();
    let mut current_product = String::new();
    let mut current_date = String::new();
    let mut current_pkgs: Vec<(String, u64, String)> = Vec::new();
    let mut pkg_size: u64 = 0;
    let mut pkg_url: Option<(String, String)> = None;
    let mut pending_key = "";

    let flush_pkg = |pkg_url: &mut Option<(String, String)>,
                     pkg_size: &mut u64,
                     current_pkgs: &mut Vec<(String, u64, String)>| {
        if let Some((kind, url)) = pkg_url.take() {
            current_pkgs.push((kind, *pkg_size, url));
        }
        *pkg_size = 0;
    };

    let flush_product = |current_product: &str,
                         current_date: &str,
                         current_pkgs: &mut Vec<(String, u64, String)>,
                         results: &mut Vec<SdkPackage>| {
        for (kind, size, url) in current_pkgs.drain(..) {
            results.push(SdkPackage {
                product_id: current_product.to_string(),
                post_date: current_date.to_string(),
                kind,
                size,
                url,
            });
        }
    };

    for line in xml.lines() {
        let trimmed = line.trim();
        if trimmed == "</dict>" {
            flush_pkg(&mut pkg_url, &mut pkg_size, &mut current_pkgs);
            continue;
        }
        if let Some(key) = extract_tag(trimmed, "key") {
            if is_product_id(key) {
                flush_pkg(&mut pkg_url, &mut pkg_size, &mut current_pkgs);
                flush_product(&current_product, &current_date, &mut current_pkgs, &mut results);
                current_product = key.to_string();
                current_date.clear();
            }
            pending_key = key;
            continue;
        }

        match pending_key {
            "PostDate" => {
                if let Some(date) = extract_tag(trimmed, "date") {
                    current_date = date.to_string();
                }
                pending_key = "";
            }
            "Size" => {
                if let Some(val) = extract_tag(trimmed, "integer")
                    && let Ok(parsed) = val.parse::<u64>()
                {
                    pkg_size = parsed;
                }
                pending_key = "";
            }
            "URL" => {
                if let Some(url) = extract_tag(trimmed, "string")
                    && let Some(kind) = classify_sdk_pkg_url(url)
                {
                    pkg_url = Some((kind.to_string(), url.to_string()));
                }
                pending_key = "";
            }
            _ => {}
        }
    }

    flush_pkg(&mut pkg_url, &mut pkg_size, &mut current_pkgs);
    flush_product(&current_product, &current_date, &mut current_pkgs, &mut results);

    // Sort newest first; prefer macOSNMOS_SDK > macOS_SDK > macOSLMOS_SDK for the same date.
    results.sort_by(|a, b| {
        b.post_date
            .cmp(&a.post_date)
            .then_with(|| sdk_kind_rank(&b.kind).cmp(&sdk_kind_rank(&a.kind)))
            .then_with(|| b.product_id.cmp(&a.product_id))
    });
    results
}

fn sdk_kind_rank(kind: &str) -> u8 {
    match kind {
        "macOSNMOS_SDK" => 3,
        "macOS_SDK" => 2,
        "macOSLMOS_SDK" => 1,
        _ => 0,
    }
}

fn classify_sdk_pkg_url(url: &str) -> Option<&'static str> {
    if url.ends_with("/CLTools_macOSNMOS_SDK.pkg") {
        Some("macOSNMOS_SDK")
    } else if url.ends_with("/CLTools_macOS_SDK.pkg") {
        Some("macOS_SDK")
    } else if url.ends_with("/CLTools_macOSLMOS_SDK.pkg") {
        Some("macOSLMOS_SDK")
    } else {
        None
    }
}

fn is_product_id(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 9
        && bytes[0..3].iter().all(|b| b.is_ascii_digit())
        && bytes[3] == b'-'
        && bytes[4..9].iter().all(|b| b.is_ascii_digit())
}

fn extract_tag<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let rest = line.strip_prefix(&open)?;
    rest.strip_suffix(&close)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cltools_sdk_packages_sorted_newest_first() {
        let sample = r#"
            <key>052-59890</key>
            <dict>
                <key>Packages</key>
                <array>
                    <dict>
                        <key>Size</key>
                        <integer>57000000</integer>
                        <key>URL</key>
                        <string>https://swcdn.apple.com/content/downloads/CLTools_macOSNMOS_SDK.pkg</string>
                    </dict>
                </array>
                <key>PostDate</key>
                <date>2025-09-15T17:00:00Z</date>
            </dict>
            <key>012-92431</key>
            <dict>
                <key>Packages</key>
                <array>
                    <dict>
                        <key>Size</key>
                        <integer>52000000</integer>
                        <key>URL</key>
                        <string>https://swcdn.apple.com/content/downloads/CLTools_macOS_SDK.pkg</string>
                    </dict>
                </array>
                <key>PostDate</key>
                <date>2024-05-01T17:00:00Z</date>
            </dict>
        "#;
        let pkgs = parse_sucatalog_sdk_packages(sample);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].product_id, "052-59890");
        assert_eq!(pkgs[0].kind, "macOSNMOS_SDK");
        assert_eq!(pkgs[0].size, 57_000_000);
        assert_eq!(pkgs[1].product_id, "012-92431");
    }
}

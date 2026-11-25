//! HTTP Range partial ZIP fetcher backed by `ureq`.
//!
//! Reads only the remote ZIP's End of Central Directory (EOCD) and Central
//! Directory over HTTP `Range` requests, filters entries matching a predicate,
//! fetches only those compressed byte spans in coalesced requests, and
//! reassembles a valid standalone mini `.zip` archive locally.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_CD: u32 = 0x0201_4b50;
const MERGE_GAP_BYTES: u64 = 128 * 1024;
const MAX_RANGE_BODY_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone)]
struct CdEntry {
    raw_cd_record: Vec<u8>,
    name: String,
    local_offset: u64,
}

/// Fetches only entries matching `predicate` from `url` and writes a valid
/// standalone mini `.zip` archive to `dest_zip`, returning the active `Sysroot`
/// progress bar so the caller can reuse it in-place for splatting.
pub fn fetch_partial_zip<F>(
    url: &str,
    dest_zip: &Path,
    mut predicate: F,
) -> Result<ProgressBar, String>
where
    F: FnMut(&str) -> bool,
{
    let agent = ureq::Agent::new_with_defaults();

    let index_pb = make_bytes_bar("Index", 0, "📥", "downloading");
    let total_len = fetch_content_length(&agent, url)?;
    let tail_size = total_len.min(65_557);
    let tail_start = total_len - tail_size;
    let tail = fetch_range(&agent, url, tail_start, total_len - 1)?;
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

    // Build sorted list of all entry offsets (terminated by `cd_offset`) so we know
    // the exact byte end of every local file record (including optional data descriptors).
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
        return Err("no matching entries found in remote ZIP archive".to_string());
    }

    matched.sort_by_key(|(_, start, _)| *start);

    // Merge adjacent or closely spaced spans to minimize HTTP round-trips.
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
            // Patch the 32-bit relative offset of local header (bytes 42..46).
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

/// Reads the Central Directory byte size from the EOCD record of a local `.zip` file.
pub fn local_zip_cd_size(path: &Path) -> u64 {
    let Ok(mut f) = File::open(path) else {
        return 0;
    };
    if f.seek(SeekFrom::End(-22)).is_err() {
        return 0;
    }
    let mut eocd = [0u8; 22];
    if f.read_exact(&mut eocd).is_err() {
        return 0;
    }
    if u32::from_le_bytes(eocd[0..4].try_into().unwrap()) == SIG_EOCD {
        u32::from_le_bytes(eocd[12..16].try_into().unwrap()) as u64
    } else {
        0
    }
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

fn fetch_content_length(agent: &ureq::Agent, url: &str) -> Result<u64, String> {
    let resp = agent
        .head(url)
        .call()
        .map_err(|e| format!("HTTP HEAD failed for {url}: {e}"))?;
    resp.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .ok_or_else(|| format!("server did not return Content-Length for {url}"))
}

fn fetch_range(
    agent: &ureq::Agent,
    url: &str,
    start: u64,
    end_inclusive: u64,
) -> Result<Vec<u8>, String> {
    fetch_range_streaming(agent, url, start, end_inclusive, None)
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
    fn parses_valid_eocd_record() {
        let mut eocd = vec![0u8; 22];
        eocd[0..4].copy_from_slice(&SIG_EOCD.to_le_bytes());
        eocd[12..16].copy_from_slice(&1024u32.to_le_bytes());
        eocd[16..20].copy_from_slice(&8192u32.to_le_bytes());
        let (off, size) = parse_eocd(&eocd).unwrap();
        assert_eq!(off, 8192);
        assert_eq!(size, 1024);
    }
}

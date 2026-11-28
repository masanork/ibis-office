// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Bounded archive inspection and extraction for `.lzh` (LHA), `.zip`, and `.7z` detection.

use delharc::Read as _;

use crate::{Error, Opened};

const MAX_ENTRIES: usize = 1024;
const MAX_ENTRY_BYTES: usize = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn is_7z(data: &[u8]) -> bool {
    data.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C])
}

pub fn is_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06")
}

pub fn is_lzh(data: &[u8]) -> bool {
    if data.len() < 22 {
        return false;
    }
    let method = &data[2..7];
    matches!(
        method,
        b"-lh0-"
            | b"-lh1-"
            | b"-lh2-"
            | b"-lh3-"
            | b"-lh4-"
            | b"-lh5-"
            | b"-lh6-"
            | b"-lh7-"
            | b"-lhd-"
            | b"-lzs-"
            | b"-lz4-"
            | b"-lz5-"
    )
}

pub fn sanitize_rel_path(raw: &str) -> Option<String> {
    let replaced = raw.replace('\\', "/");
    let mut parts = Vec::new();
    for seg in replaced.split('/') {
        let seg = seg.trim_matches('\0');
        if seg.is_empty() || seg == "." || seg == ".." {
            continue;
        }
        if seg.contains(':') {
            continue;
        }
        parts.push(seg);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

pub fn extract_lzh(data: &[u8]) -> Result<Vec<ArchiveEntry>, Error> {
    let mut reader = delharc::LhaDecodeReader::new(data)
        .map_err(|e| Error::new(format!("invalid LZH archive: {e:?}")))?;
    let mut entries = Vec::new();
    let mut total_bytes = 0usize;
    loop {
        if entries.len() >= MAX_ENTRIES {
            return Err(Error::new("LZH archive exceeds 1024 entries"));
        }
        let header = reader.header();
        let is_dir = header.is_directory();
        let orig_size = header.original_size;
        let raw_name = header.parse_pathname_to_str();
        if !is_dir && let Some(path) = sanitize_rel_path(&raw_name) {
            if orig_size > MAX_ENTRY_BYTES as u64 {
                return Err(Error::new(format!("LZH entry {path} exceeds 64 MiB")));
            }
            if !reader.is_decoder_supported() {
                return Err(Error::new(format!("LZH entry {path} uses unsupported compression")));
            }
            let mut buf = vec![0u8; orig_size as usize];
            reader
                .read_exact(&mut buf)
                .map_err(|e| Error::new(format!("LZH read error for {path}: {e:?}")))?;
            reader
                .crc_check()
                .map_err(|e| Error::new(format!("LZH CRC error for {path}: {e:?}")))?;
            total_bytes = total_bytes.saturating_add(buf.len());
            if total_bytes > MAX_TOTAL_BYTES {
                return Err(Error::new("LZH archive exceeds 128 MiB total extracted size"));
            }
            entries.push(ArchiveEntry { path, bytes: buf });
        }
        if !reader
            .next_file()
            .map_err(|e| Error::new(format!("LZH header error: {e:?}")))?
        {
            break;
        }
    }
    Ok(entries)
}

pub fn extract_zip(data: &[u8]) -> Result<Vec<ArchiveEntry>, Error> {
    let archive = crate::zip::Archive::new(data)?;
    let mut entries = Vec::new();
    let mut total_bytes = 0usize;
    for name in archive.names() {
        if entries.len() >= MAX_ENTRIES {
            return Err(Error::new("ZIP archive exceeds 1024 entries"));
        }
        if name.ends_with('/') {
            continue;
        }
        let Some(path) = sanitize_rel_path(&name) else { continue };
        let Some(bytes) = archive.read(&name)? else { continue };
        if bytes.len() > MAX_ENTRY_BYTES {
            return Err(Error::new(format!("ZIP entry {path} exceeds 64 MiB")));
        }
        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(Error::new("ZIP archive exceeds 128 MiB total extracted size"));
        }
        entries.push(ArchiveEntry { path, bytes });
    }
    Ok(entries)
}

pub fn extract(data: &[u8]) -> Result<Vec<ArchiveEntry>, Error> {
    if is_lzh(data) {
        extract_lzh(data)
    } else if is_zip(data) {
        extract_zip(data)
    } else if is_7z(data) {
        Err(Error::new(
            "7z extraction requires external 7z CLI (detected .7z signature)",
        ))
    } else {
        Err(Error::new("unsupported archive format"))
    }
}

pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let kind = if is_lzh(data) {
        "LZH"
    } else if is_zip(data) {
        "ZIP"
    } else if is_7z(data) {
        return Ok("# 7z Archive\n\n`.7z` archive detected (use `7z x` to extract).\n".into());
    } else {
        return Err(Error::new("not a supported archive"));
    };
    let entries = extract(data)?;
    let mut md = format!(
        "# {kind} Archive ({} files)\n\n| File | Size (bytes) |\n| --- | ---: |\n",
        entries.len()
    );
    for entry in &entries {
        md.push_str(&format!(
            "| `{}` | {} |\n",
            entry.path.replace('|', "\\|"),
            entry.bytes.len()
        ));
    }
    Ok(md)
}

pub fn open_lzh(data: &[u8]) -> Result<Opened, Error> {
    let md = to_markdown(data)?;
    crate::markdown::open_markdown(md.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_archive_signatures() {
        assert!(is_7z(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04]));
        assert!(is_zip(b"PK\x03\x04rest"));
        let mut lzh = vec![0u8; 24];
        lzh[2..7].copy_from_slice(b"-lh5-");
        assert!(is_lzh(&lzh));
        assert!(!is_lzh(b"plain text"));
    }

    #[test]
    fn sanitizes_path_traversal() {
        assert_eq!(sanitize_rel_path("../../etc/passwd"), Some("etc/passwd".into()));
        assert_eq!(sanitize_rel_path("C:\\temp\\a.docx"), Some("temp/a.docx".into()));
        assert_eq!(sanitize_rel_path("/../.."), None);
    }

    #[test]
    fn extracts_zip_and_renders_markdown() {
        let mut w = ibis_ooxml::zip::Writer::new();
        w.add("docs/hello.txt", ibis_ooxml::zip::EntryMeta::default(), b"hello").unwrap();
        let bytes = w.finish().unwrap();
        let entries = extract(&bytes).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "docs/hello.txt");
        assert_eq!(entries[0].bytes, b"hello");
        let md = to_markdown(&bytes).unwrap();
        assert!(md.contains("docs/hello.txt"));
    }
}

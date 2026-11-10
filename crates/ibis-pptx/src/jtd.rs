// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Conservative, read-only JTD text import. No OpenJTD code is used.
//!
//! Format observations: OpenJTD RFC 0003 and 0009 at cfb44c9ac53126e00808c49709fb4dade8a4daae,
//! https://github.com/OpenJTD/rjtd/tree/cfb44c9ac53126e00808c49709fb4dade8a4daae/openjtd-spec/rfc
//! These are incomplete research notes, not a complete specification.
use crate::{Error, Kind};
use ibis_doc_model::{Block, FlowRun, FlowRunKind, FlowRunStyle, Presentation};

const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_STREAM: usize = 32 * 1024 * 1024;

/// Identify a supported JTD by its container and named document stream.
pub fn is_jtd(data: &[u8]) -> bool {
    checked_stream(data).is_ok_and(|s| s.starts_with(b"SsmgV.01"))
}

fn checked_stream(data: &[u8]) -> Result<Vec<u8>, Error> {
    if data.len() > MAX_FILE { return Err(Error::new("JTDファイルが大きすぎます")); }
    if data.len() < 512 || !ibis_ooxml::cfb::is_cfb(data) ||
        !matches!(u16::from_le_bytes([data[30], data[31]]), 9 | 12) ||
        u16::from_le_bytes([data[32], data[33]]) != 6 {
        return Err(Error::new("対応するJTDコンテナではありません"));
    }
    let stream = ibis_ooxml::cfb::stream(data, "DocumentText")
        .map_err(|_| Error::new("JTD本文がないか、暗号化・未対応の形式です"))?;
    if stream.len() > MAX_STREAM { return Err(Error::new("JTD本文が大きすぎます")); }
    Ok(stream)
}

/// Import only confirmed text into the existing document model. This is lossy
/// and must never be used as the source of a JTD save.
pub fn read(data: &[u8]) -> Result<Presentation, Error> {
    let stream = checked_stream(data)?;
    let text = extract(&stream)?;
    let mut presentation = crate::read(&crate::blank_file(Kind::Document)?)?;
    let flow = std::sync::Arc::make_mut(presentation.flow.as_mut().ok_or_else(|| Error::new("文書モデルを作成できません"))?);
    let template = match flow.body.first() { Some(Block::Paragraph(p)) => p.clone(), _ => return Err(Error::new("文書の段落を作成できません")) };
    flow.body.clear();
    for (index, line) in text.split('\n').enumerate() {
        let mut p = template.clone();
        p.id = u32::try_from(index + 1).map_err(|_| Error::new("段落が多すぎます"))?;
        p.runs = vec![FlowRun::new(FlowRunKind::Text(line.to_string()), FlowRunStyle::default())];
        flow.body.push(Block::Paragraph(p));
    }
    flow.id_floor = flow.body.len() as u32;
    Ok(presentation)
}

fn extract(bytes: &[u8]) -> Result<String, Error> {
    if !bytes.starts_with(b"SsmgV.01") || bytes.len() % 2 != 0 { return Err(Error::new("JTD本文形式が未対応か壊れています")); }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect();
    let mut out = String::new();
    // Observed TextV.01 plain-text record: 32-bit BE number of UTF-16 units.
    if bytes.get(20..28) == Some(b"TextV.01") && bytes.len() >= 32 {
        let count = u32::from_be_bytes(bytes[28..32].try_into().unwrap()) as usize;
        if count > 0 && count <= (bytes.len()-32)/2 && units.get(16).is_some_and(|x| *x >= 0x20) {
            let s = String::from_utf16(&units[16..16+count]).map_err(|_| Error::new("JTD文字列が壊れています"))?;
            if s.chars().all(|c| !c.is_control() || matches!(c, '\n'|'\r'|'\t')) { out.push_str(&s); }
        }
    }
    if out.is_empty() {
        let mut i = 4;
        while i + 7 <= units.len() {
            if units[i] != 0x001c { i += 1; continue; }
            let class = units[i + 1];
            let count = units[i + 2] as usize;
            if class == 1 && count == 7 && units[i + 6] == 0x001d {
                // Only the observed visible base/placeholder selectors. Other
                // inline payloads may be ruby or control data, so omit them.
                let visible = units[i + 4] == 0 && matches!(units[i + 5], 1 | 3);
                let start = i + 7;
                let end = (start..units.len().min(start + 256)).find(|&n| units[n] == 0x001e);
                if let Some(end) = end {
                    if visible {
                        let value = String::from_utf16(&units[start..end]).map_err(|_| Error::new("JTD文字列が壊れています"))?;
                        if value.chars().all(|c| !c.is_control() || matches!(c, '\n'|'\r'|'\t')) { out.push_str(&value); }
                    }
                    i = end + 1;
                    continue;
                }
            }
            let end = match i.checked_add(count) { Some(end) if count >= 7 && end <= units.len() => end, _ => { i += 1; continue; } };
            if !matches!(class, 0 | 0x10 | 0x20 | 0x30) || units[end - 4] as usize != count || units[end - 3] != 0 || units[end - 2] != class || units[end - 1] != 0x001f {
                i += 1;
                continue;
            }
            i = end;
            let start = i;
            while i < units.len() && (units[i] >= 0x00a0 || matches!(units[i], 0x09 | 0x0a | 0x0d) || (0x20..0x7f).contains(&units[i])) { i += 1; }
            if start != i {
                let text = String::from_utf16(&units[start..i]).map_err(|_| Error::new("JTD文字列が壊れています"))?;
                out.push_str(&text);
            }
        }
    }
    let out = out.replace("\r\n", "\n").replace('\r', "\n");
    if out.trim().is_empty() { return Err(Error::new("対応するJTD本文を読み取れませんでした")); }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(text: &str) -> Vec<u8> {
        let mut s = b"SsmgV.01".to_vec();
        for n in [0x001c_u16, 0x0010, 7, 7, 0, 0x0010, 0x001f].into_iter().chain(text.encode_utf16()).chain([0u16]) { s.extend_from_slice(&n.to_be_bytes()); }
        ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream("DocumentText", s)])
    }
    #[test]
    fn reads_text_and_does_not_treat_jtd_as_xls() {
        let bytes = fixture("日本語\nsecond");
        assert!(is_jtd(&bytes));
        let result = crate::read(&bytes).unwrap();
        let flow = result.flow.unwrap();
        assert_eq!(flow.body.len(), 2);
        assert!(matches!(&flow.body[0], Block::Paragraph(p) if p.text() == "日本語"));
        assert!(matches!(&flow.body[1], Block::Paragraph(p) if p.text() == "second"));
    }
    #[test]
    fn raw_text_and_inline_selectors_are_bounded() {
        let mut raw = b"SsmgV.01".to_vec();
        raw.extend_from_slice(&[0; 12]);
        raw.extend_from_slice(b"TextV.01");
        raw.extend_from_slice(&2u32.to_be_bytes());
        for unit in "文章".encode_utf16() { raw.extend_from_slice(&unit.to_be_bytes()); }
        assert_eq!(extract(&raw).unwrap(), "文章");
        let mut data = b"SsmgV.01".to_vec();
        for unit in [0x1c_u16, 1, 7, 0, 0, 3, 0x1d, 0x6587, 0x001e, 0x1c, 1, 7, 0, 1, 0x82, 0x1d, 0x3088, 0x001e] { data.extend_from_slice(&unit.to_be_bytes()); }
        assert_eq!(extract(&data).unwrap(), "文");
    }

    #[test]
    #[ignore = "requires a locally authorized JTD sample"]
    fn local_sample_can_be_read() {
        let path = std::env::var_os("IBIS_JTD_SAMPLE").expect("IBIS_JTD_SAMPLE");
        let bytes = std::fs::read(path).unwrap();
        let result = read(&bytes).unwrap();
        let flow = result.flow.unwrap();
        let text = flow.body.iter().filter_map(|b| match b { Block::Paragraph(p) => Some(p.text()), _ => None }).collect::<Vec<_>>().join("\n");
        assert!(!text.trim().is_empty());
        if let Ok(expected) = std::env::var("IBIS_JTD_EXPECT") { assert!(text.contains(&expected)); }
    }
    #[test]
    fn rejects_invalid_and_unsupported() {
        assert!(read(&[]).is_err());
        let b = ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream("DocumentText", b"SsmgV.01".to_vec())]);
        assert!(read(&b).is_err());
        let mut b = fixture("test"); b[30] = 255; assert!(read(&b).is_err());
    }
}

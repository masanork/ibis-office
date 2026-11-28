// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust parser for Microsoft TNEF (`winmail.dat`) and RFC 5322 / MIME (`.eml`) emails.

use base64::Engine as _;

use crate::archive::{ArchiveEntry, sanitize_rel_path};
use crate::{Error, Opened};

const TNEF_SIGNATURE: [u8; 4] = [0x78, 0x9f, 0x3e, 0x22];
const MAX_ATTACHMENTS: usize = 256;
const MAX_ATTACHMENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct ParsedEmail {
    pub subject: String,
    pub from: String,
    pub to: String,
    pub date: String,
    pub body: String,
    pub attachments: Vec<ArchiveEntry>,
}

pub fn is_tnef(data: &[u8]) -> bool {
    data.starts_with(&TNEF_SIGNATURE) && data.len() >= 6
}

pub fn is_eml(data: &[u8]) -> bool {
    let head = &data[..data.len().min(4096)];
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    let mut has_header = false;
    let mut header_lines = 0usize;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        header_lines += 1;
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("from:")
            || lower.starts_with("subject:")
            || lower.starts_with("mime-version:")
            || lower.starts_with("content-type:")
        {
            has_header = true;
        }
        if header_lines > 80 {
            break;
        }
    }
    has_header
}

pub fn parse(data: &[u8]) -> Result<ParsedEmail, Error> {
    if is_tnef(data) {
        parse_tnef(data)
    } else if is_eml(data) {
        parse_eml(data)
    } else {
        Err(Error::new("unsupported email format (expected TNEF winmail.dat or .eml)"))
    }
}

pub fn extract_attachments(data: &[u8]) -> Result<Vec<ArchiveEntry>, Error> {
    Ok(parse(data)?.attachments)
}

pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let parsed = parse(data)?;
    let title = if parsed.subject.trim().is_empty() {
        "Email Message".to_string()
    } else {
        parsed.subject.clone()
    };
    let mut md = format!("# {title}\n\n");
    if !parsed.from.is_empty() {
        md.push_str(&format!("- **From:** {}\n", parsed.from));
    }
    if !parsed.to.is_empty() {
        md.push_str(&format!("- **To:** {}\n", parsed.to));
    }
    if !parsed.date.is_empty() {
        md.push_str(&format!("- **Date:** {}\n", parsed.date));
    }
    if !parsed.attachments.is_empty() {
        md.push_str("\n## Attachments\n\n| File | Size (bytes) |\n| --- | ---: |\n");
        for att in &parsed.attachments {
            md.push_str(&format!(
                "| `{}` | {} |\n",
                att.path.replace('|', "\\|"),
                att.bytes.len()
            ));
        }
    }
    if !parsed.body.trim().is_empty() {
        md.push_str("\n## Body\n\n");
        md.push_str(parsed.body.trim());
        md.push('\n');
    }
    Ok(md)
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    let md = to_markdown(data)?;
    crate::markdown::open_markdown(md.into_bytes())
}

fn parse_tnef(data: &[u8]) -> Result<ParsedEmail, Error> {
    if !is_tnef(data) {
        return Err(Error::new("invalid TNEF signature"));
    }
    let mut pos = 6usize; // 4-byte signature + 2-byte key
    let mut email = ParsedEmail::default();
    let mut current_name: Option<String> = None;
    let mut current_bytes: Option<Vec<u8>> = None;

    let flush_attachment = |email: &mut ParsedEmail,
                            name: &mut Option<String>,
                            bytes: &mut Option<Vec<u8>>| {
        if let Some(data) = bytes.take() {
            let raw_name = name
                .take()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("attachment-{}.bin", email.attachments.len() + 1));
            if let Some(path) = sanitize_rel_path(&raw_name)
                && email.attachments.len() < MAX_ATTACHMENTS
            {
                email.attachments.push(ArchiveEntry { path, bytes: data });
            }
        } else {
            *name = None;
        }
    };

    while pos + 9 <= data.len() {
        let level = data[pos];
        let attr_id = u32::from_le_bytes(data[pos + 1..pos + 5].try_into().unwrap()) & 0xFFFF;
        let len = u32::from_le_bytes(data[pos + 5..pos + 9].try_into().unwrap()) as usize;
        pos += 9;
        if pos + len + 2 > data.len() || len > MAX_ATTACHMENT_BYTES {
            break;
        }
        let val = &data[pos..pos + len];
        pos += len + 2; // skip 2-byte checksum

        match (level, attr_id) {
            (1, 0x8004) => {
                // attSubject
                email.subject = decode_c_string(val);
            }
            (1, 0x800C) => {
                // attBody
                email.body = decode_c_string(val);
            }
            (2, 0x9002) => {
                // attAttachRenddata: start of a new attachment
                flush_attachment(&mut email, &mut current_name, &mut current_bytes);
            }
            (2, 0x8010) => {
                // attAttachTitle
                if current_name.is_none() {
                    let s = decode_c_string(val);
                    if !s.is_empty() {
                        current_name = Some(s);
                    }
                }
            }
            (2, 0x800F) => {
                // attAttachData
                current_bytes = Some(val.to_vec());
            }
            (2, 0x9005) => {
                // attAttachment (MAPI properties)
                if let Some(long_name) = scan_mapi_long_filename(val) {
                    current_name = Some(long_name);
                }
            }
            _ => {}
        }
    }
    flush_attachment(&mut email, &mut current_name, &mut current_bytes);
    Ok(email)
}

fn decode_c_string(bytes: &[u8]) -> String {
    let trimmed = match bytes.iter().position(|&b| b == 0) {
        Some(idx) => &bytes[..idx],
        None => bytes,
    };
    match std::str::from_utf8(trimmed) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(trimmed);
            cow.into_owned()
        }
    }
}

fn scan_mapi_long_filename(data: &[u8]) -> Option<String> {
    // MAPI property stream: count(u32), then props: type(u16), id(u16)...
    if data.len() < 4 {
        return None;
    }
    // Look for PR_ATTACH_LONG_FILENAME_W (type 0x001F, id 0x3707) -> LE bytes [1F 00 07 37]
    let needle = [0x1F, 0x00, 0x07, 0x37];
    let idx = data.windows(4).position(|w| w == needle)?;
    let rest = data.get(idx + 4..)?;
    if rest.len() < 8 {
        return None;
    }
    let count = u32::from_le_bytes(rest[0..4].try_into().ok()?) as usize;
    if count != 1 {
        return None;
    }
    let byte_len = u32::from_le_bytes(rest[4..8].try_into().ok()?) as usize;
    let str_bytes = rest.get(8..8 + byte_len)?;
    let utf16: Vec<u16> = str_bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16(&utf16).ok()
}

fn parse_eml(data: &[u8]) -> Result<ParsedEmail, Error> {
    let raw = String::from_utf8_lossy(data);
    let (headers_raw, body_raw) = split_headers_body(&raw);
    let headers = parse_headers(headers_raw);
    let mut email = ParsedEmail {
        subject: decode_rfc2047(header_value(&headers, "subject").unwrap_or_default()),
        from: decode_rfc2047(header_value(&headers, "from").unwrap_or_default()),
        to: decode_rfc2047(header_value(&headers, "to").unwrap_or_default()),
        date: header_value(&headers, "date").unwrap_or_default().to_string(),
        body: String::new(),
        attachments: Vec::new(),
    };
    let ct = header_value(&headers, "content-type").unwrap_or("text/plain");
    let cte = header_value(&headers, "content-transfer-encoding").unwrap_or("");
    let cd = header_value(&headers, "content-disposition").unwrap_or("");
    collect_mime_part(ct, cte, cd, body_raw, &mut email);
    Ok(email)
}

fn split_headers_body(raw: &str) -> (&str, &str) {
    if let Some(pos) = raw.find("\r\n\r\n") {
        (&raw[..pos], &raw[pos + 4..])
    } else if let Some(pos) = raw.find("\n\n") {
        (&raw[..pos], &raw[pos + 2..])
    } else {
        (raw, "")
    }
}

fn parse_headers(raw: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, val)) = out.last_mut() {
                val.push(' ');
                val.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    out
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn extract_param(header: &str, key: &str) -> Option<String> {
    for part in header.split(';').skip(1) {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn collect_mime_part(
    ct: &str,
    cte: &str,
    cd: &str,
    body: &str,
    email: &mut ParsedEmail,
) {
    let ct_lower = ct.to_ascii_lowercase();
    if ct_lower.starts_with("multipart/")
        && let Some(boundary) = extract_param(ct, "boundary")
    {
        let marker = format!("--{boundary}");
        for chunk in body.split(&marker).skip(1) {
            if chunk.starts_with("--") {
                break;
            }
            let chunk = chunk.strip_prefix("\r\n").or_else(|| chunk.strip_prefix('\n')).unwrap_or(chunk);
            let (h_raw, b_raw) = split_headers_body(chunk);
            let hs = parse_headers(h_raw);
            let sub_ct = header_value(&hs, "content-type").unwrap_or("text/plain");
            let sub_cte = header_value(&hs, "content-transfer-encoding").unwrap_or("");
            let sub_cd = header_value(&hs, "content-disposition").unwrap_or("");
            collect_mime_part(sub_ct, sub_cte, sub_cd, b_raw.trim_end_matches(['\r', '\n']), email);
        }
        return;
    }

    let filename = extract_param(cd, "filename").or_else(|| extract_param(ct, "name"));
    let is_attachment = cd.to_ascii_lowercase().starts_with("attachment") || filename.is_some();
    let decoded = decode_transfer_encoding(cte, body);

    if is_attachment {
        if email.attachments.len() < MAX_ATTACHMENTS && decoded.len() <= MAX_ATTACHMENT_BYTES {
            let raw_name = filename
                .map(|f| decode_rfc2047(&f))
                .unwrap_or_else(|| format!("attachment-{}.bin", email.attachments.len() + 1));
            if let Some(path) = sanitize_rel_path(&raw_name) {
                email.attachments.push(ArchiveEntry { path, bytes: decoded });
            }
        }
    } else if ct_lower.starts_with("text/plain") && email.body.is_empty() {
        let charset = extract_param(ct, "charset").unwrap_or_else(|| "utf-8".into());
        email.body = decode_charset(&charset, &decoded);
    }
}

fn decode_transfer_encoding(cte: &str, body: &str) -> Vec<u8> {
    if cte.eq_ignore_ascii_case("base64") {
        let compact: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(compact.as_bytes())
            .unwrap_or_else(|_| body.as_bytes().to_vec())
    } else if cte.eq_ignore_ascii_case("quoted-printable") {
        decode_qp(body.as_bytes())
    } else {
        body.as_bytes().to_vec()
    }
}

fn decode_qp(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'=' {
            if input.get(i + 1..i + 3) == Some(b"\r\n") {
                i += 3;
                continue;
            }
            if input.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if let (Some(&h1), Some(&h2)) = (input.get(i + 1), input.get(i + 2))
                && let Ok(hex) = std::str::from_utf8(&[h1, h2])
                && let Ok(byte) = u8::from_str_radix(hex, 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

fn decode_charset(charset: &str, bytes: &[u8]) -> String {
    if let Some(enc) = encoding_rs::Encoding::for_label(charset.trim().as_bytes()) {
        let (cow, _, _) = enc.decode(bytes);
        cow.into_owned()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn decode_rfc2047(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("=?") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("?=") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let token = &after[..end];
        let mut parts = token.splitn(3, '?');
        if let (Some(charset), Some(enc), Some(payload)) = (parts.next(), parts.next(), parts.next()) {
            let bytes = if enc.eq_ignore_ascii_case("B") {
                base64::engine::general_purpose::STANDARD
                    .decode(payload.as_bytes())
                    .unwrap_or_else(|_| payload.as_bytes().to_vec())
            } else if enc.eq_ignore_ascii_case("Q") {
                let replaced = payload.replace('_', " ");
                decode_qp(replaced.as_bytes())
            } else {
                payload.as_bytes().to_vec()
            };
            out.push_str(&decode_charset(charset, &bytes));
        } else {
            out.push_str("=?");
            out.push_str(token);
            out.push_str("?=");
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_tnef_winmail_dat() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&TNEF_SIGNATURE);
        buf.extend_from_slice(&[0x01, 0x00]); // key
        let push_attr = |buf: &mut Vec<u8>, level: u8, id: u32, val: &[u8]| {
            buf.push(level);
            buf.extend_from_slice(&id.to_le_bytes());
            buf.extend_from_slice(&(val.len() as u32).to_le_bytes());
            buf.extend_from_slice(val);
            buf.extend_from_slice(&[0, 0]);
        };
        push_attr(&mut buf, 1, 0x8004, b"Test Subject\0");
        push_attr(&mut buf, 1, 0x800C, b"Hello from TNEF\0");
        push_attr(&mut buf, 2, 0x9002, &[0, 0]);
        push_attr(&mut buf, 2, 0x8010, b"report.txt\0");
        push_attr(&mut buf, 2, 0x800F, b"attached-content");

        let parsed = parse(&buf).unwrap();
        assert_eq!(parsed.subject, "Test Subject");
        assert_eq!(parsed.body, "Hello from TNEF");
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].path, "report.txt");
        assert_eq!(parsed.attachments[0].bytes, b"attached-content");
    }

    #[test]
    fn parses_eml_with_rfc2047_and_base64_attachment() {
        let eml = concat!(
            "From: sender@example.jp\r\n",
            "Subject: =?UTF-8?B?44OG44K544OI5Lu25ZCN?=\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=\"B1\"\r\n",
            "\r\n",
            "--B1\r\n",
            "Content-Type: text/plain; charset=UTF-8\r\n",
            "\r\n",
            "本文テスト\r\n",
            "--B1\r\n",
            "Content-Type: application/octet-stream; name=\"data.csv\"\r\n",
            "Content-Disposition: attachment; filename=\"data.csv\"\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "YSxiLGMK\r\n",
            "--B1--\r\n",
        );
        let parsed = parse(eml.as_bytes()).unwrap();
        assert_eq!(parsed.subject, "テスト件名");
        assert_eq!(parsed.body.trim(), "本文テスト");
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].path, "data.csv");
        assert_eq!(parsed.attachments[0].bytes, b"a,b,c\n");
    }
}

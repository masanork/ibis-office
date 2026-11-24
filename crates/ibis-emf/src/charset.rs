// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The single-byte and double-byte text of GDI records, read by the character set of the font (`lfCharSet`).

use encoding_rs::Encoding;

fn encoding(charset: u8) -> &'static Encoding {
    match charset {
        128 => encoding_rs::SHIFT_JIS,
        129 | 130 => encoding_rs::EUC_KR,
        134 => encoding_rs::GBK,
        136 => encoding_rs::BIG5,
        161 => encoding_rs::WINDOWS_1253,
        162 => encoding_rs::WINDOWS_1254,
        177 => encoding_rs::WINDOWS_1255,
        178 => encoding_rs::WINDOWS_1256,
        186 => encoding_rs::WINDOWS_1257,
        204 => encoding_rs::WINDOWS_1251,
        222 => encoding_rs::WINDOWS_874,
        238 => encoding_rs::WINDOWS_1250,
        _ => encoding_rs::WINDOWS_1252,
    }
}

/// A rejected GDI text record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrictTextError {
    Invalid,
    UnsupportedCharset,
}

/// Decode the Windows-1252 and Shift_JIS subsets used by DocuWorks, keeping each character's source byte count.
pub fn decode_text_strict(bytes: &[u8], charset: u8) -> Result<Vec<(char, usize)>, StrictTextError> {
    if !matches!(charset, 0 | 1 | 128) {
        return Err(StrictTextError::UnsupportedCharset);
    }
    let encoding = encoding(charset);
    let mut input = bytes;
    let mut output = Vec::new();
    while let Some(&lead) = input.first() {
        let length = if charset == 128 && matches!(lead, 0x81..=0x9f | 0xe0..=0xfc) {
            2
        } else {
            1
        };
        let (encoded, rest) = input.split_at_checked(length).ok_or(StrictTextError::Invalid)?;
        let decoded = encoding
            .decode_without_bom_handling_and_without_replacement(encoded)
            .ok_or(StrictTextError::Invalid)?;
        let mut chars = decoded.chars();
        let ch = chars.next().ok_or(StrictTextError::Invalid)?;
        if chars.next().is_some() {
            return Err(StrictTextError::Invalid);
        }
        output.push((ch, length));
        input = rest;
    }
    Ok(output)
}

/// The characters of `bytes` and how many bytes each took.
pub(crate) fn decode(bytes: &[u8], charset: u8) -> Vec<(char, usize)> {
    let enc = encoding(charset);
    let (text, _) = enc.decode_without_bom_handling(bytes);
    let mut out = Vec::new();
    let mut used = 0;
    for c in text.chars() {
        let mut buf = [0u8; 4];
        let (encoded, _, unmappable) = enc.encode(c.encode_utf8(&mut buf));
        let n = if unmappable || encoded.is_empty() { 1 } else { encoded.len() };
        used += n;
        out.push((c, n));
    }
    // (Should the counts not add up, the rest is one byte each.)
    if used != bytes.len() {
        return bytes.iter().map(|&b| (char::from(b), 1)).collect();
    }
    out
}

/// The string, with the spacing (one entry per byte) summed per character.
pub(crate) fn decode_with_spacing(bytes: &[u8], charset: u8, dx: &[f64]) -> (String, Vec<f64>) {
    let chars = decode(bytes, charset);
    let text: String = chars.iter().map(|(c, _)| *c).collect();
    if dx.len() != bytes.len() {
        return (text, dx.to_vec());
    }
    let mut at = 0;
    let merged = chars
        .iter()
        .map(|(_, n)| {
            let sum = dx[at..at + n].iter().sum();
            at += n;
            sum
        })
        .collect();
    (text, merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_text_keeps_source_byte_counts_and_rejects_invalid_input() {
        assert_eq!(decode_text_strict(&[0x93, 0xfa, b'A'], 128), Ok(vec![('日', 2), ('A', 1)]));
        assert_eq!(decode_text_strict(&[0x93], 128), Err(StrictTextError::Invalid));
        assert_eq!(decode_text_strict(&[b'A'], 129), Err(StrictTextError::UnsupportedCharset));
        assert_eq!(decode_text_strict(&[0xe9], 1), Ok(vec![('é', 1)]));
    }

    #[test]
    fn shift_jis_is_read_by_the_character_set() {
        // "日本語A" in Shift_JIS.
        let bytes = [0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA, b'A'];
        let chars = decode(&bytes, 128);
        assert_eq!(chars.iter().map(|(c, _)| *c).collect::<String>(), "日本語A");
        assert_eq!(chars.iter().map(|(_, n)| *n).collect::<Vec<_>>(), vec![2, 2, 2, 1]);
        let (text, dx) = decode_with_spacing(&bytes, 128, &[5.0, 5.0, 6.0, 6.0, 7.0, 7.0, 8.0]);
        assert_eq!(text, "日本語A");
        assert_eq!(dx, vec![10.0, 12.0, 14.0, 8.0]);
        // The default character set is Windows-1252.
        assert_eq!(decode(&[0xE9], 0)[0].0, 'é');
    }
}

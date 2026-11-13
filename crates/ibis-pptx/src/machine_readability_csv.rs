// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Byte-stream CSV reader for the native machine-readability engine. This is
//! independent of the editor's CSV import, which performs type conversion.
use ibis_doc_model::machine_readability_csv::{read_sheet, CsvReadError, CsvSheet, Delimiter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsvInputEncoding { Utf8, Utf8Bom, Cp932 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvInput { pub sheet: CsvSheet, pub encoding: CsvInputEncoding }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsvInputError { InvalidCp932, InvalidCsv(CsvReadError) }

/// Strict Python-compatible CP932 decoding. `encoding_rs` uses the WHATWG
/// Shift_JIS mapping, which rejects CP932's four single-byte private-use codes.
/// Treat those codes separately only when they are not part of a lead/trail
/// pair, and disable BOM sniffing. All malformed sequences fail closed.
pub fn decode_cp932(bytes: &[u8]) -> Result<String, CsvInputError> {
    let mut out = String::new();
    let mut start = 0;
    let mut i = 0;
    let append = |out: &mut String, data: &[u8]| -> Result<(), CsvInputError> {
        let (text, errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(data);
        if errors { return Err(CsvInputError::InvalidCp932); }
        out.push_str(&text);
        Ok(())
    };
    while i < bytes.len() {
        let byte = bytes[i];
        if (0x81..=0x9f).contains(&byte) || (0xe0..=0xfc).contains(&byte) {
            i = (i + 2).min(bytes.len());
        } else if byte == 0xa0 || byte >= 0xfd {
            append(&mut out, &bytes[start..i])?;
            let code = if byte == 0xa0 { 0xf8f0 } else { 0xf8f1 + u32::from(byte - 0xfd) };
            out.push(char::from_u32(code).unwrap());
            i += 1;
            start = i;
        } else { i += 1; }
    }
    append(&mut out, &bytes[start..])?;
    Ok(out)
}

/// Match the pinned Harunobu byte-stream path: attempt UTF-8 with an optional
/// single BOM, then fall back to strict CP932. No encoding guess is guaranteed
/// to recover the author's intent when a byte sequence is valid in both.
pub fn read_csv_bytes(bytes: &[u8], delimiter: Delimiter) -> Result<CsvInput, CsvInputError> {
    let (text, encoding) = decode_csv_bytes(bytes)?;
    let sheet = read_sheet(&text, delimiter).map_err(CsvInputError::InvalidCsv)?;
    Ok(CsvInput { sheet, encoding })
}

/// Decode bytes while retaining the detected encoding for raw diagnostics.
pub fn decode_csv_bytes(bytes: &[u8]) -> Result<(std::borrow::Cow<'_, str>, CsvInputEncoding), CsvInputError> {
    let utf8 = bytes.strip_prefix(b"\xef\xbb\xbf");
    let candidate = utf8.unwrap_or(bytes);
    let (text, encoding) = match std::str::from_utf8(candidate) {
        Ok(text) => (std::borrow::Cow::Borrowed(text), if utf8.is_some() { CsvInputEncoding::Utf8Bom } else { CsvInputEncoding::Utf8 }),
        Err(_) => (std::borrow::Cow::Owned(decode_cp932(bytes)?), CsvInputEncoding::Cp932),
    };
    Ok((text, encoding))
}

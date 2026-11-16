// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! CSV / TSV: opened as a workbook of one sheet (the values read as Excel reads
//! them: numbers, dates, percentages and formulas as typed), supporting UTF-8
//! (with or without BOM), UTF-16, and Shift_JIS (CP932); with auto-detected
//! delimiters (comma, tab, semicolon), and writing sheets as CSV or TSV.

use ibis_calc::input::{Typed, parse_imported};
use ibis_doc_model::{CellValue, Presentation};

use crate::Error;

pub use encoding_rs;

/// The actual encoding observed on opening a delimited text file.
/// ASCII bytes alone cannot distinguish UTF-8 from CP932 or other ASCII supersets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalEncoding {
    AsciiAmbiguous,
    /// Valid UTF-8 bytes that also decode as CP932; the intended text is unknown.
    Utf8OrCp932,
    Utf8,
    Utf8Bom,
    Utf16LeBom,
    Utf16BeBom,
    Cp932,
}

/// Properties of the original delimited file retained for same-file saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalCsvFormat {
    pub encoding: OriginalEncoding,
    pub delimiter: char,
    pub record_ending: RecordEnding,
    pub final_record_ending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordEnding { CrLf, Lf, Cr, Mixed, QuotedCarriageReturn }

/// Byte ranges into the original file. Delimiters lie between field ranges;
/// each record's terminator is retained separately, including an empty final
/// terminator. The BOM, when present, precedes the first field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCsvRecord {
    pub fields: Vec<std::ops::Range<usize>>,
    pub terminator: std::ops::Range<usize>,
}

/// Locate original CSV fields without decoding and re-encoding their bytes.
/// This is a building block for lossless same-file editing, not a serializer.
pub fn raw_csv_records(data: &[u8], encoding: OriginalEncoding, delimiter: char) -> Result<Vec<RawCsvRecord>, Error> {
    if !matches!(delimiter, ',' | ';' | '\t') {
        return Err(Error::new("CSV の区切り文字に対応していません"));
    }
    let choice = match encoding {
        OriginalEncoding::Utf8 | OriginalEncoding::Utf8Bom => TextEncoding::Utf8,
        OriginalEncoding::Utf16LeBom | OriginalEncoding::Utf16BeBom => TextEncoding::Utf16,
        OriginalEncoding::Cp932 => TextEncoding::ShiftJis,
        OriginalEncoding::AsciiAmbiguous | OriginalEncoding::Utf8OrCp932 => TextEncoding::Auto,
    };
    let (_, actual) = decode_with_choice(data, choice)?;
    if actual != encoding { return Err(Error::new("CSV の文字コードが元の形式と一致しません")); }
    let (start, width, le) = match encoding {
        OriginalEncoding::Utf8Bom => (3, 1, true),
        OriginalEncoding::Utf16LeBom => (2, 2, true),
        OriginalEncoding::Utf16BeBom => (2, 2, false),
        _ => (0, 1, true),
    };
    let unit = |pos: usize| -> u16 {
        if width == 1 { data[pos] as u16 }
        else if le { u16::from_le_bytes([data[pos], data[pos + 1]]) }
        else { u16::from_be_bytes([data[pos], data[pos + 1]]) }
    };
    let mut records = Vec::new();
    let mut fields = Vec::new();
    let mut pos = start;
    let mut field_start = start;
    let mut empty = true;
    let mut quoted = false;
    while pos < data.len() {
        let c = unit(pos);
        if quoted {
            if c == b'"' as u16 {
                if pos + width < data.len() && unit(pos + width) == c {
                    empty = false;
                    pos += 2 * width;
                    continue;
                }
                quoted = false;
            } else { empty = false; }
            pos += width;
            continue;
        }
        if c == b'"' as u16 && empty {
            quoted = true;
            pos += width;
        } else if c == delimiter as u16 {
            fields.push(field_start..pos);
            pos += width;
            field_start = pos;
            empty = true;
        } else if c == b'\r' as u16 || c == b'\n' as u16 {
            fields.push(field_start..pos);
            let begin = pos;
            pos += width;
            if c == b'\r' as u16 && pos < data.len() && unit(pos) == b'\n' as u16 { pos += width; }
            records.push(RawCsvRecord { fields: std::mem::take(&mut fields), terminator: begin..pos });
            field_start = pos;
            empty = true;
        } else {
            empty = false;
            pos += width;
        }
    }
    if quoted { return Err(Error::new("閉じられていない CSV の引用符があります")); }
    if field_start < data.len() || !fields.is_empty() {
        fields.push(field_start..data.len());
        records.push(RawCsvRecord { fields, terminator: data.len()..data.len() });
    }
    Ok(records)
}

/// Replace selected existing fields while copying every other byte verbatim.
/// Coordinates are zero-based. Structural edits require a separate policy and
/// are deliberately not inferred by this function.
pub fn rewrite_raw_csv_fields(data: &[u8], encoding: OriginalEncoding, delimiter: char, changes: &[(usize, usize, String)]) -> Result<Vec<u8>, Error> {
    let records = raw_csv_records(data, encoding, delimiter)?;
    let mut replacements = Vec::with_capacity(changes.len());
    for (row, column, value) in changes {
        let range = records.get(*row).and_then(|r| r.fields.get(*column))
            .ok_or_else(|| Error::new("元の CSV にないセルはこの方法で保存できません"))?.clone();
        let field = if value.contains([delimiter, '"', '\r', '\n']) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else { value.clone() };
        let mut bytes = encode_original(&field, encoding)?;
        let bom_len = match encoding { OriginalEncoding::Utf8Bom => 3, OriginalEncoding::Utf16LeBom | OriginalEncoding::Utf16BeBom => 2, _ => 0 };
        bytes.drain(..bom_len);
        replacements.push((range, bytes));
    }
    replacements.sort_by_key(|(range, _)| range.start);
    if replacements.windows(2).any(|pair| pair[0].0.end > pair[1].0.start || pair[0].0 == pair[1].0) {
        return Err(Error::new("同じ CSV セルが複数回指定されました"));
    }
    let mut output = Vec::with_capacity(data.len());
    let mut offset = 0;
    for (range, bytes) in replacements {
        output.extend_from_slice(&data[offset..range.start]);
        output.extend_from_slice(&bytes);
        offset = range.end;
    }
    output.extend_from_slice(&data[offset..]);
    Ok(output)
}

/// Determine record separators without mistaking line breaks inside quotes for records.
pub fn original_csv_format(text: &str, encoding: OriginalEncoding, delimiter: char) -> OriginalCsvFormat {
    let mut iter = CsvRecordIter::new(text, delimiter);
    let mut ending = None;
    let mut trailing = false;
    let mut quoted_cr = false;
    while let Some(record) = iter.next() {
        quoted_cr |= record.iter().any(|field| field.contains('\r'));
        trailing = !iter.finished;
        if trailing {
            let end = iter.position();
            let candidate = if text[..end].ends_with("\r\n") { RecordEnding::CrLf }
                else if text[..end].ends_with('\n') { RecordEnding::Lf }
                else { RecordEnding::Cr };
            ending = Some(match ending { None => candidate, Some(old) if old == candidate => old, _ => RecordEnding::Mixed });
        }
    }
    if iter.quoted { ending = Some(RecordEnding::Mixed); }
    if quoted_cr { ending = Some(RecordEnding::QuotedCarriageReturn); }
    OriginalCsvFormat { encoding, delimiter, record_ending: ending.unwrap_or(RecordEnding::CrLf), final_record_ending: trailing }
}

/// Serialize the current sheet and refuse character loss when restoring its encoding.
pub fn sheet_to_original_csv(presentation: &Presentation, slide: usize, format: OriginalCsvFormat) -> Result<Vec<u8>, Error> {
    let ending = match format.record_ending {
        RecordEnding::CrLf => "\r\n", RecordEnding::Lf => "\n", RecordEnding::Cr => "\r",
        RecordEnding::Mixed => return Err(Error::new("CSV の改行が混在しているため上書き保存できません")),
        RecordEnding::QuotedCarriageReturn => return Err(Error::new("CSV のセル内の CR 改行を保持できないため上書き保存できません")),
    };
    let text = sheet_to_delimited_text(presentation, slide, format.delimiter, ending, format.final_record_ending);
    encode_original(&text, format.encoding)
}

/// Decode and keep the encoding necessary for a safe same-file save.
pub fn decode_with_original_encoding(data: &[u8]) -> Result<(String, OriginalEncoding), Error> {
    if let Some(rest) = data.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8(rest.to_vec()).map(|s| (s, OriginalEncoding::Utf8Bom))
            .map_err(|_| Error::new("UTF-8 CSV の読み込みに失敗しました"));
    }
    for (bom, encoding, kind) in [
        (&b"\xFF\xFE"[..], encoding_rs::UTF_16LE, OriginalEncoding::Utf16LeBom),
        (&b"\xFE\xFF"[..], encoding_rs::UTF_16BE, OriginalEncoding::Utf16BeBom),
    ] {
        if let Some(rest) = data.strip_prefix(bom) {
            let (text, _, invalid) = encoding.decode(rest);
            return if invalid { Err(Error::new("UTF-16 CSV の読み込みに失敗しました")) }
                else { Ok((text.into_owned(), kind)) };
        }
    }
    if let Ok(text) = std::str::from_utf8(data) {
        let (_, _, cp932_errors) = encoding_rs::SHIFT_JIS.decode(data);
        let encoding = if data.is_ascii() { OriginalEncoding::AsciiAmbiguous }
            else if !cp932_errors { OriginalEncoding::Utf8OrCp932 }
            else { OriginalEncoding::Utf8 };
        return Ok((text.to_owned(), encoding));
    }
    let (text, _, invalid) = encoding_rs::SHIFT_JIS.decode(data);
    if invalid { Err(Error::new("CSVの文字コードを判定できませんでした")) }
    else { Ok((text.into_owned(), OriginalEncoding::Cp932)) }
}

/// Decode a delimited file using a user-selected encoding, without guessing.
pub fn decode_with_choice(data: &[u8], choice: TextEncoding) -> Result<(String, OriginalEncoding), Error> {
    match choice {
        TextEncoding::Auto => decode_with_original_encoding(data),
        TextEncoding::Utf8 => Ok((decode_as(data, choice)?, if data.starts_with(b"\xEF\xBB\xBF") { OriginalEncoding::Utf8Bom } else { OriginalEncoding::Utf8 })),
        TextEncoding::ShiftJis => Ok((decode_as(data, choice)?, OriginalEncoding::Cp932)),
        TextEncoding::Utf16 => {
            let encoding = if data.starts_with(b"\xFE\xFF") { OriginalEncoding::Utf16BeBom }
                else if data.starts_with(b"\xFF\xFE") { OriginalEncoding::Utf16LeBom }
                else { return Err(Error::new("UTF-16 のバイト順を判定できません")); };
            Ok((decode_as(data, choice)?, encoding))
        }
    }
}

/// Encode without replacement or HTML numeric character references.
pub fn encode_original(text: &str, encoding: OriginalEncoding) -> Result<Vec<u8>, Error> {
    match encoding {
        OriginalEncoding::AsciiAmbiguous | OriginalEncoding::Utf8OrCp932 if !text.is_ascii() => Err(Error::new("元の文字コードを判別できないため、ASCII 以外の文字を上書き保存できません")),
        OriginalEncoding::AsciiAmbiguous | OriginalEncoding::Utf8OrCp932 | OriginalEncoding::Utf8 => Ok(text.as_bytes().to_vec()),
        OriginalEncoding::Utf8Bom => {
            let mut bytes = b"\xEF\xBB\xBF".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            Ok(bytes)
        }
        OriginalEncoding::Utf16LeBom | OriginalEncoding::Utf16BeBom => {
            let le = encoding == OriginalEncoding::Utf16LeBom;
            let mut bytes = if le { b"\xFF\xFE".to_vec() } else { b"\xFE\xFF".to_vec() };
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&if le { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            Ok(bytes)
        }
        OriginalEncoding::Cp932 => {
            let (bytes, _, invalid) = encoding_rs::SHIFT_JIS.encode(text);
            let (decoded, _, decode_error) = encoding_rs::SHIFT_JIS.decode(&bytes);
            if invalid || decode_error || decoded != text {
                Err(Error::new("Shift_JIS で正確に表現できない文字があるため保存できません"))
            } else { Ok(bytes.into_owned()) }
        }
    }
}

#[cfg(test)]
mod original_encoding_tests {
    use super::{decode_with_choice, decode_with_original_encoding, encode_original, raw_csv_records, rewrite_raw_csv_fields, sheet_to_csv_sjis_strict, sheet_to_original_csv, OriginalEncoding, RecordEnding, TextEncoding};

    #[test]
    fn raw_field_rewrites_only_selected_bytes() {
        let original = b"\x87\x90,00123,\r\n\x81\xe0,\"x\",\n";
        assert_eq!(rewrite_raw_csv_fields(original, OriginalEncoding::Cp932, ',', &[]).unwrap(), original);
        let changes = [(1, 1, "a,\"b\"".to_string())];
        assert_eq!(rewrite_raw_csv_fields(original, OriginalEncoding::Cp932, ',', &changes).unwrap(), b"\x87\x90,00123,\r\n\x81\xe0,\"a,\"\"b\"\"\",\n");
        assert!(rewrite_raw_csv_fields(original, OriginalEncoding::Cp932, ',', &[(0, 1, "😀".into())]).is_err());
        assert!(rewrite_raw_csv_fields(original, OriginalEncoding::Cp932, ',', &[(3, 0, "x".into())]).is_err());
        assert!(rewrite_raw_csv_fields(original, OriginalEncoding::Cp932, ',', &[(0, 1, "x".into()), (0, 1, "y".into())]).is_err());
        for encoding in [OriginalEncoding::Utf8Bom, OriginalEncoding::Utf16LeBom, OriginalEncoding::Utf16BeBom] {
            let input = encode_original("00123,\"old\",\r\n", encoding).unwrap();
            let output = rewrite_raw_csv_fields(&input, encoding, ',', &[(0, 1, "名,前".into())]).unwrap();
            assert_eq!(output, encode_original("00123,\"名,前\",\r\n", encoding).unwrap());
        }
    }

    #[test]
    fn raw_ranges_preserve_fields_and_each_record_separator() {
        let data = b"\xef\xbb\xbf\"a,\"\"b\"\"\",00123,\r\nx,\"line\r\nline\"\n\rfinal";
        let records = raw_csv_records(data, OriginalEncoding::Utf8Bom, ',').unwrap();
        let fields: Vec<Vec<&[u8]>> = records.iter().map(|r| r.fields.iter().map(|range| &data[range.clone()]).collect()).collect();
        assert_eq!(fields, vec![vec![&b"\"a,\"\"b\"\"\""[..], &b"00123"[..], &b""[..]], vec![&b"x"[..], &b"\"line\r\nline\""[..]], vec![&b""[..]], vec![&b"final"[..]]]);
        let endings: Vec<&[u8]> = records.iter().map(|r| &data[r.terminator.clone()]).collect();
        assert_eq!(endings, vec![&b"\r\n"[..], &b"\n"[..], &b"\r"[..], &b""[..]]);
        assert!(raw_csv_records(b"\"unterminated", OriginalEncoding::AsciiAmbiguous, ',').is_err());
        assert!(raw_csv_records(&[0xff, 0xfe, 0], OriginalEncoding::Utf16LeBom, ',').is_err());
    }

    #[test]
    fn raw_ranges_keep_noncanonical_cp932_and_utf16() {
        let data = b"\x87\x90,\x81\xe0\r\n";
        let records = raw_csv_records(data, OriginalEncoding::Cp932, ',').unwrap();
        let fields = &records[0].fields;
        assert_eq!(&data[fields[0].clone()], b"\x87\x90");
        assert_eq!(&data[fields[1].clone()], b"\x81\xe0");
        assert_eq!(encoding_rs::SHIFT_JIS.decode(&data[fields[0].clone()]).0, encoding_rs::SHIFT_JIS.decode(&data[fields[1].clone()]).0);
        for encoding in [OriginalEncoding::Utf16LeBom, OriginalEncoding::Utf16BeBom] {
            let data = encode_original("\"名,前\",00123,\r\n", encoding).unwrap();
            let records = raw_csv_records(&data, encoding, ',').unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].fields.len(), 3);
            let bytes = &data[records[0].fields[1].clone()];
            let mut with_bom = data[..2].to_vec();
            with_bom.extend_from_slice(bytes);
            assert_eq!(decode_with_original_encoding(&with_bom).unwrap().0, "00123");
            assert_eq!(records[0].terminator.len(), 4);
        }
    }

    #[test]
    fn cp932_is_retained_and_unrepresentable_text_is_rejected() {
        let (bytes, _, errors) = encoding_rs::SHIFT_JIS.encode("名前,数量\r\nりんご,3\r\n");
        assert!(!errors);
        let (text, encoding) = decode_with_original_encoding(&bytes).unwrap();
        assert_eq!(encoding, OriginalEncoding::Cp932);
        assert_eq!(encode_original(&text, encoding).unwrap(), bytes.as_ref());
        assert!(encode_original("りんご😀", encoding).is_err());
        assert!(encode_original("りんご\u{e0101}", encoding).is_err());
        // Some mappings encode without an error but decode to a different scalar.
        assert!(encode_original("\u{00a5}", encoding).is_err());
    }

    #[test]
    fn bom_and_byte_order_round_trip() {
        for (bytes, encoding) in [
            (b"\xEF\xBB\xBFabc".to_vec(), OriginalEncoding::Utf8Bom),
            (vec![0xff, 0xfe, 0x42, 0x30], OriginalEncoding::Utf16LeBom),
            (vec![0xfe, 0xff, 0x30, 0x42], OriginalEncoding::Utf16BeBom),
        ] {
            let (text, observed) = decode_with_original_encoding(&bytes).unwrap();
            assert_eq!(observed, encoding);
            assert_eq!(encode_original(&text, observed).unwrap(), bytes);
        }
        assert!(decode_with_original_encoding(&[0xff, 0xfe, 0x42]).is_err());
    }

    #[test]
    fn ambiguous_input_cannot_silently_become_utf8() {
        let (text, encoding) = decode_with_original_encoding(b"a,b\r\n").unwrap();
        assert_eq!(encoding, OriginalEncoding::AsciiAmbiguous);
        assert_eq!(encode_original(&text, encoding).unwrap(), b"a,b\r\n");
        assert!(encode_original("a,名前\r\n", encoding).is_err());
        // U+00A2's UTF-8 bytes are also a valid CP932 byte sequence.
        let (_, encoding) = decode_with_original_encoding("¢".as_bytes()).unwrap();
        assert_eq!(encoding, OriginalEncoding::Utf8OrCp932);
        assert!(encode_original("¢", encoding).is_err());
    }

    #[test]
    fn opened_csv_retains_encoding_and_delimiter_for_save() {
        let (bytes, _, invalid) = encoding_rs::SHIFT_JIS.encode("名前;数量\r\nりんご;3\r\n");
        assert!(!invalid);
        let opened = crate::open(bytes.into_owned()).unwrap();
        let format = opened.source.original_csv_format().unwrap();
        assert_eq!(format.encoding, OriginalEncoding::Cp932);
        assert_eq!(format.delimiter, ';');
        let saved = sheet_to_original_csv(&opened.presentation, 0, format).unwrap();
        let (decoded, _, invalid) = encoding_rs::SHIFT_JIS.decode(&saved);
        assert!(!invalid);
        assert!(decoded.contains("名前;数量"));
        assert!(decoded.contains("りんご;3"));
    }

    #[test]
    fn explicit_sjis_export_rejects_unrepresentable_characters() {
        let opened = crate::open("名前,😀\r\n".as_bytes().to_vec()).unwrap();
        assert!(sheet_to_csv_sjis_strict(&opened.presentation, 0).is_err());
    }

    #[test]
    fn same_file_keeps_record_endings_and_quoted_newlines() {
        for text in ["a,b\nx,\"hello\nworld\"", "a,b\rx,y\r", "a,b\r\nx,y\r\n", "a,b"] {
            let opened = crate::open(text.as_bytes().to_vec()).unwrap();
            let format = opened.source.original_csv_format().unwrap();
            let saved = sheet_to_original_csv(&opened.presentation, 0, format).unwrap();
            assert_eq!(saved, text.as_bytes(), "{text:?}");
        }
    }

    #[test]
    fn mixed_or_unclosed_records_are_not_overwritten() {
        for text in ["a,b\nx,y\r\n", "a,\"unfinished\n"] {
            let opened = crate::open(text.as_bytes().to_vec()).unwrap();
            let format = opened.source.original_csv_format().unwrap();
            assert_eq!(format.record_ending, RecordEnding::Mixed);
            assert!(sheet_to_original_csv(&opened.presentation, 0, format).is_err());
        }
    }

    #[test]
    fn quoted_carriage_returns_cannot_be_silently_lost() {
        let text = "a,b\nx,\"hello\r\nworld\"";
        let opened = crate::open(text.as_bytes().to_vec()).unwrap();
        let format = opened.source.original_csv_format().unwrap();
        assert_eq!(format.record_ending, RecordEnding::QuotedCarriageReturn);
        assert!(sheet_to_original_csv(&opened.presentation, 0, format).is_err());
    }

    #[test]
    fn explicit_choice_decodes_ambiguous_bytes_before_editing() {
        let bytes = b"\xc2\xa2\r\n";
        let (utf8, a) = decode_with_choice(bytes, TextEncoding::Utf8).unwrap();
        let (cp932, b) = decode_with_choice(bytes, TextEncoding::ShiftJis).unwrap();
        assert_eq!(utf8, "¢\r\n");
        assert_ne!(utf8, cp932);
        assert_eq!(a, OriginalEncoding::Utf8);
        assert_eq!(b, OriginalEncoding::Cp932);
        assert_eq!(encode_original(&utf8, a).unwrap(), bytes);
        assert_eq!(encode_original(&cp932, b).unwrap(), bytes);
        assert!(decode_with_choice(&[0x81], TextEncoding::ShiftJis).is_err());
        let (opened, _) = crate::open_progressive_with_csv_encoding(bytes.to_vec(), usize::MAX, TextEncoding::ShiftJis).unwrap();
        assert_eq!(opened.source.original_csv_format().unwrap().encoding, OriginalEncoding::Cp932);
        let saved = sheet_to_original_csv(&opened.presentation, 0, opened.source.original_csv_format().unwrap()).unwrap();
        assert_eq!(saved, bytes);
    }
}

/// Character encodings supported for CSV / TSV export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CsvEncoding {
    /// UTF-8 with a BOM (\xEF\xBB\xBF) - Excel default 「CSV UTF-8」
    #[default]
    Utf8Bom,
    /// UTF-8 without a BOM
    Utf8,
    /// Shift_JIS (CP932 / Windows-31J) - Japanese Windows Excel 「CSV (コンマ区切り)」
    ShiftJis,
}

/// Encodes `text` as Shift_JIS (CP932 / Windows-31J) bytes.
pub fn encode_sjis(text: &str) -> Vec<u8> {
    let (cow, _, _) = encoding_rs::SHIFT_JIS.encode(text);
    cow.into_owned()
}


/// Options for exporting a sheet to delimited text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvOptions {
    pub encoding: CsvEncoding,
    pub delimiter: char,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            encoding: CsvEncoding::Utf8Bom,
            delimiter: ',',
        }
    }
}

/// Whether `data` is not a ZIP package (so taken for CSV/text data).
pub fn is_csv(data: &[u8]) -> bool {
    !data.starts_with(b"PK\x03\x04")
}

/// Decodes CSV bytes into a UTF-8 string.
/// Supports UTF-8 (with or without BOM), UTF-16LE/BE (with BOM),
/// and Shift_JIS (CP932 / Windows-31J).
pub fn decode(data: &[u8]) -> Result<String, Error> {
    if let Some(stripped) = data.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8(stripped.to_vec())
            .map_err(|e| Error::new(format!("UTF-8 CSV の読み込みに失敗しました: {e}")));
    }
    if let Some(stripped) = data.strip_prefix(b"\xFF\xFE") {
        let (cow, _, had_errors) = encoding_rs::UTF_16LE.decode(stripped);
        if !had_errors {
            return Ok(cow.into_owned());
        }
    }
    if let Some(stripped) = data.strip_prefix(b"\xFE\xFF") {
        let (cow, _, had_errors) = encoding_rs::UTF_16BE.decode(stripped);
        if !had_errors {
            return Ok(cow.into_owned());
        }
    }
    // Try valid UTF-8
    if let Ok(s) = std::str::from_utf8(data) {
        return Ok(s.to_string());
    }
    // Try Shift_JIS (CP932)
    let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(data);
    if !had_errors {
        return Ok(cow.into_owned());
    }
    Err(Error::new("CSVの文字コードを判定できませんでした（UTF-8 または Shift_JIS 形式である必要があります）"))
}

/// The character encoding the text file is read in (テキスト/CSV から の
/// ファイル元): found from the bytes (自動), or the one chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEncoding {
    #[default]
    Auto,
    Utf8,
    /// Shift_JIS (CP932 / Windows-31J).
    ShiftJis,
    /// UTF-16 (little endian unless the byte order mark says big endian).
    Utf16,
}

/// Decodes `data` in the chosen encoding; a byte order mark of UTF-8 or
/// UTF-16 is left out. A byte that is not valid in it is an error (the
/// text file would be shown garbled).
pub fn decode_as(data: &[u8], encoding: TextEncoding) -> Result<String, Error> {
    let fail = |name: &str| Error::new(format!("{name} として読み込めませんでした"));
    match encoding {
        TextEncoding::Auto => decode(data),
        TextEncoding::Utf8 => {
            let data = data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(data);
            String::from_utf8(data.to_vec()).map_err(|_| fail("UTF-8"))
        }
        TextEncoding::ShiftJis => {
            let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(data);
            if had_errors { Err(fail("Shift_JIS")) } else { Ok(cow.into_owned()) }
        }
        TextEncoding::Utf16 => {
            let (enc, body) = match data.strip_prefix(b"\xFE\xFF") {
                Some(rest) => (encoding_rs::UTF_16BE, rest),
                None => (encoding_rs::UTF_16LE, data.strip_prefix(b"\xFF\xFE").unwrap_or(data)),
            };
            let (cow, _, had_errors) = enc.decode(body);
            if had_errors { Err(fail("UTF-16")) } else { Ok(cow.into_owned()) }
        }
    }
}

/// What a text file is read with (テキスト/CSV から): the encoding, the
/// delimiter (`None`: found from the text), and the row to start from
/// (1 for the first), the rows before it left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    pub encoding: TextEncoding,
    pub delimiter: Option<char>,
    pub start_row: usize,
    /// Fixed-width columns: each line is cut before the characters at these positions (from 0, rising); an empty
    /// list has them found from the text ([`detect_fixed_breaks`]). `None` reads delimited fields.
    pub fixed: Option<Vec<usize>>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self { encoding: TextEncoding::Auto, delimiter: None, start_row: 1, fixed: None }
    }
}

/// Where the columns of a text in fixed-width columns begin: a position where, in every one of the first lines, the
/// character before is blank (or past the line's end) and one of the lines has a character, after something.
pub fn detect_fixed_breaks(text: &str, start_row: usize) -> Vec<usize> {
    let lines: Vec<Vec<char>> = text.lines().skip(start_row.max(1) - 1).take(200).filter(|l| !l.trim().is_empty()).map(|l| l.chars().collect()).collect();
    let width = lines.iter().map(Vec::len).max().unwrap_or(0);
    let blank = |c: usize| lines.iter().all(|l| l.get(c).is_none_or(|ch| ch.is_whitespace()));
    let mut out = Vec::new();
    let mut seen_text = false;
    for c in 0..width {
        if !blank(c) {
            if seen_text && c > 0 && blank(c - 1) {
                out.push(c);
            }
            seen_text = true;
        }
    }
    out
}

/// The line `line` cut before the characters at `breaks`, each part without its blanks at the ends (parts past the end
/// of the line are left out, as Excel does).
pub fn split_fixed(line: &str, breaks: &[usize]) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let (mut out, mut start) = (Vec::new(), 0);
    for &b in breaks {
        if b <= start || b >= chars.len() {
            continue;
        }
        out.push(chars[start..b].iter().collect::<String>().trim().to_string());
        start = b;
    }
    out.push(chars[start..].iter().collect::<String>().trim().to_string());
    out
}

/// A text file read: its records (from the start row), and the delimiter used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub rows: Vec<Vec<String>>,
    pub delimiter: char,
}

/// The records of a text file as `options` say.
pub fn import(data: &[u8], options: &ImportOptions) -> Result<Imported, Error> {
    let text = decode_as(data, options.encoding)?;
    if let Some(breaks) = &options.fixed {
        let skip = options.start_row.max(1) - 1;
        let breaks = if breaks.is_empty() { detect_fixed_breaks(&text, options.start_row) } else { breaks.clone() };
        let rows = text.lines().skip(skip).map(|l| split_fixed(l, &breaks)).collect();
        return Ok(Imported { rows, delimiter: ' ' });
    }
    let delimiter = options.delimiter.unwrap_or_else(|| detect_delimiter(&text));
    let rows = parse_with_delimiter(&text, delimiter).into_iter().skip(options.start_row.max(1) - 1).collect();
    Ok(Imported { rows, delimiter })
}

/// Detects the field delimiter: comma (`,`), tab (`\t`), or semicolon (`;`).
pub fn detect_delimiter(text: &str) -> char {
    let mut comma_count = 0usize;
    let mut tab_count = 0usize;
    let mut semicolon_count = 0usize;
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut lines = 0;

    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                }
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => comma_count += 1,
            '\t' => tab_count += 1,
            ';' => semicolon_count += 1,
            '\n' | '\r' => {
                lines += 1;
                if lines >= 50 {
                    break;
                }
            }
            _ => {}
        }
    }

    if tab_count > 0 && tab_count >= comma_count && tab_count >= semicolon_count {
        '\t'
    } else if semicolon_count > 0 && semicolon_count > comma_count {
        ';'
    } else {
        ','
    }
}

/// Iterator yielding records of fields separated by `delimiter`.
pub struct CsvRecordIter<'a> {
    text: &'a str,
    pos: usize,
    delimiter: char,
    field: String,
    record: Vec<String>,
    quoted: bool,
    finished: bool,
}

impl<'a> CsvRecordIter<'a> {
    pub fn new(text: &'a str, delimiter: char) -> Self {
        Self {
            text,
            pos: 0,
            delimiter,
            field: String::new(),
            record: Vec::new(),
            quoted: false,
            finished: false,
        }
    }

    /// The current byte offset into the underlying text.
    pub fn position(&self) -> usize {
        self.pos
    }
}

impl<'a> Iterator for CsvRecordIter<'a> {
    type Item = Vec<String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let text = self.text;
        let delimiter = self.delimiter;
        while self.pos < text.len() {
            let s = &text[self.pos..];
            let mut chars = s.chars();
            let c = chars.next().unwrap();
            let len = c.len_utf8();
            self.pos += len;

            if self.quoted {
                if c == '"' {
                    if chars.next() == Some('"') {
                        self.field.push('"');
                        self.pos += 1;
                    } else {
                        self.quoted = false;
                    }
                } else {
                    self.field.push(c);
                }
                continue;
            }

            match c {
                '"' if self.field.is_empty() => self.quoted = true,
                c if c == delimiter => self.record.push(std::mem::take(&mut self.field)),
                '\r' if chars.next() == Some('\n') => {
                    self.pos += 1;
                    self.record.push(std::mem::take(&mut self.field));
                    return Some(std::mem::take(&mut self.record));
                }
                '\n' | '\r' => {
                    self.record.push(std::mem::take(&mut self.field));
                    return Some(std::mem::take(&mut self.record));
                }
                _ => self.field.push(c),
            }
        }
        self.finished = true;
        if !self.field.is_empty() || !self.record.is_empty() {
            self.record.push(std::mem::take(&mut self.field));
            Some(std::mem::take(&mut self.record))
        } else {
            None
        }
    }
}

/// Records of fields separated by `delimiter`: quotes around a field, `""` for a
/// quote inside one, CRLF or LF between records.
pub fn parse_with_delimiter(text: &str, delimiter: char) -> Vec<Vec<String>> {
    CsvRecordIter::new(text, delimiter).collect()
}

/// Records of fields: automatically detects delimiter (comma, tab, semicolon).
pub fn parse(text: &str) -> Vec<Vec<String>> {
    let delimiter = detect_delimiter(text);
    parse_with_delimiter(text, delimiter)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub(crate) fn column_name(mut n: u32) -> String {
    let mut s = String::new();
    n += 1;
    while n > 0 {
        let r = (n - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

/// A workbook (`.xlsx` bytes) of one sheet named `name` holding the CSV/TSV values.
pub fn workbook_from(text: &str, name: &str) -> Result<Vec<u8>, Error> {
    let delimiter = detect_delimiter(text);
    workbook_from_with_delimiter(text, name, delimiter)
}

/// A workbook (`.xlsx` bytes) of one sheet named `name` holding the delimited values.
pub fn workbook_from_with_delimiter(text: &str, name: &str, delimiter: char) -> Result<Vec<u8>, Error> {
    let records = parse_with_delimiter(text, delimiter);
    // Number formats the values take (as typing them gives), by code.
    let mut formats: Vec<String> = Vec::new();
    let mut rows = String::new();
    for (r, record) in records.iter().enumerate() {
        let mut cells = String::new();
        for (c, field) in record.iter().enumerate() {
            if field.is_empty() {
                continue;
            }
            let at = format!("{}{}", column_name(c as u32), r + 1);
            match parse_imported(field, false) {
                Typed::Formula(f) => cells.push_str(&format!(r#"<c r="{at}"><f>{}</f></c>"#, escape(&ibis_calc::formula::to_file(&f)))),
                Typed::Value(v, format) => {
                    let style = match format {
                        Some(code) => {
                            let i = formats.iter().position(|f| f == code).unwrap_or_else(|| {
                                formats.push(code.to_string());
                                formats.len() - 1
                            });
                            format!(r#" s="{}""#, i + 1)
                        }
                        None => String::new(),
                    };
                    match v {
                        CellValue::Number(n) => cells.push_str(&format!(r#"<c r="{at}"{style}><v>{}</v></c>"#, ibis_calc::eval::number_to_text(n))),
                        CellValue::Bool(b) => cells.push_str(&format!(r#"<c r="{at}" t="b"><v>{}</v></c>"#, u8::from(b))),
                        CellValue::Text(t) => cells.push_str(&format!(r#"<c r="{at}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#, escape(&t))),
                        CellValue::Error(e) => cells.push_str(&format!(r#"<c r="{at}" t="e"><v>{}</v></c>"#, e.as_str())),
                        CellValue::Empty => {}
                    }
                }
            }
        }
        rows.push_str(&format!(r#"<row r="{}">{cells}</row>"#, r + 1));
    }
    let num_fmts: String = formats.iter().enumerate().map(|(i, f)| format!(r#"<numFmt numFmtId="{}" formatCode="{}"/>"#, 164 + i, escape(f))).collect();
    let xfs: String = (0..formats.len()).map(|i| format!(r#"<xf numFmtId="{}" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" applyAlignment="1"><alignment vertical="center"/></xf>"#, 164 + i)).collect();
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{}<fonts count="1"><font><sz val="12"/><color theme="1"/><name val="游ゴシック"/><family val="2"/><charset val="128"/><scheme val="minor"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"><alignment vertical="center"/></xf></cellStyleXfs><cellXfs count="{}"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"><alignment vertical="center"/></xf>{xfs}</cellXfs><cellStyles count="1"><cellStyle name="標準" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#,
        if formats.is_empty() { String::new() } else { format!(r#"<numFmts count="{}">{num_fmts}</numFmts>"#, formats.len()) },
        formats.len() + 1
    );
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><dimension ref="A1"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews><sheetFormatPr baseColWidth="10" defaultRowHeight="20" customHeight="1"/><sheetData>{rows}</sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#
    );
    let parts: Vec<(&str, String)> = vec![
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#.to_string()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_string()),
        ("xl/workbook.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView/></bookViews><sheets><sheet name="{}" sheetId="1" r:id="rId1"/></sheets><calcPr calcId="191029" fullCalcOnLoad="1"/></workbook>"#, escape(name))),
        ("xl/_rels/workbook.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#.to_string()),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/styles.xml", styles),
    ];
    let mut writer = ibis_ooxml::zip::Writer::new();
    for (name, xml) in parts {
        writer.add(name, ibis_ooxml::zip::EntryMeta::default(), xml.as_bytes()).map_err(|e| Error::new(format!("{e}")))?;
    }
    writer.finish().map_err(|e| Error::new(format!("{e}")))
}

/// The sheet at slide index `slide` of a workbook as CSV: UTF-8 with a
/// BOM, CRLF between records, each cell's text as it shows, quoted when it
/// holds a comma, a quote or a line break.
pub fn sheet_to_csv(presentation: &Presentation, slide: usize) -> Vec<u8> {
    sheet_to_delimited(presentation, slide, CsvOptions::default())
}

/// The sheet at slide index `slide` of a workbook as TSV (tab-separated values):
/// UTF-8 with a BOM, CRLF between records.
pub fn sheet_to_tsv(presentation: &Presentation, slide: usize) -> Vec<u8> {
    sheet_to_delimited(
        presentation,
        slide,
        CsvOptions {
            encoding: CsvEncoding::Utf8Bom,
            delimiter: '\t',
        },
    )
}

/// The sheet at slide index `slide` of a workbook as Shift_JIS CSV (standard Japanese
/// Excel CSV format): CRLF between records.
pub fn sheet_to_csv_sjis(presentation: &Presentation, slide: usize) -> Vec<u8> {
    sheet_to_delimited(
        presentation,
        slide,
        CsvOptions {
            encoding: CsvEncoding::ShiftJis,
            delimiter: ',',
        },
    )
}

/// Shift_JIS export that never silently substitutes unrepresentable characters.
pub fn sheet_to_csv_sjis_strict(presentation: &Presentation, slide: usize) -> Result<Vec<u8>, Error> {
    sheet_to_original_csv(presentation, slide, OriginalCsvFormat { encoding: OriginalEncoding::Cp932, delimiter: ',', record_ending: RecordEnding::CrLf, final_record_ending: true })
}

/// The sheet at slide index `slide` formatted with custom encoding and delimiter options.
pub fn sheet_to_delimited(presentation: &Presentation, slide: usize, options: CsvOptions) -> Vec<u8> {
    let text = sheet_to_delimited_text(presentation, slide, options.delimiter, "\r\n", true);
    match options.encoding {
        CsvEncoding::Utf8Bom => {
            let mut bytes = Vec::with_capacity(3 + text.len());
            bytes.extend_from_slice(b"\xEF\xBB\xBF");
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        CsvEncoding::Utf8 => text.into_bytes(),
        CsvEncoding::ShiftJis => {
            let (cow, _, _) = encoding_rs::SHIFT_JIS.encode(&text);
            cow.into_owned()
        }
    }
}

fn sheet_to_delimited_text(presentation: &Presentation, slide: usize, delimiter: char, ending: &str, final_ending: bool) -> String {
    let mut text = String::new();
    if let Some(sheet) = presentation.slides.get(slide).and_then(ibis_doc_model::sheet_calc::sheet_of_slide) {
        let (rows, columns) = sheet.cells.iter().filter(|c| !c.text.is_empty()).fold((0, 0), |(r, c), x| (r.max(x.row + 1), c.max(x.column + 1)));
        let delimiter_str = delimiter.to_string();
        for row in 0..rows {
            let fields: Vec<String> = (0..columns)
                .map(|column| {
                    let cell_text = sheet.cell(row, column).map_or(String::new(), |c| c.text.clone());
                    if cell_text.contains([delimiter, '"', '\n', '\r']) {
                        format!("\"{}\"", cell_text.replace('"', "\"\""))
                    } else {
                        cell_text
                    }
                })
                .collect();
            text.push_str(&fields.join(&delimiter_str));
            if row + 1 < rows || final_ending { text.push_str(ending); }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_text_file_is_read_in_the_chosen_encoding_and_delimiter() {
        let sjis = encode_sjis("品名;数量\r\nりんご;3\r\n");
        assert!(decode_as(&sjis, TextEncoding::Utf8).is_err(), "Shift_JIS bytes are not UTF-8");
        let got = import(&sjis, &ImportOptions { encoding: TextEncoding::ShiftJis, ..Default::default() }).unwrap();
        assert_eq!(got.delimiter, ';');
        assert_eq!(got.rows, vec![vec!["品名", "数量"], vec!["りんご", "3"]]);
        // The delimiter chosen, and the first rows left out.
        let got = import(b"title\n\na b\nc d\n", &ImportOptions { delimiter: Some(' '), start_row: 3, ..Default::default() }).unwrap();
        assert_eq!(got.rows, vec![vec!["a", "b"], vec!["c", "d"]]);
        // UTF-16 with a byte order mark.
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend("x,y".encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(import(&utf16, &ImportOptions { encoding: TextEncoding::Utf16, ..Default::default() }).unwrap().rows, vec![vec!["x", "y"]]);
        assert_eq!(decode_as(b"\xEF\xBB\xBFabc", TextEncoding::Utf8).unwrap(), "abc");
    }

    use super::*;

    /// The sheet of an opened workbook (the first slide's).
    fn sheet_of(presentation: &Presentation) -> ibis_doc_model::Sheet {
        presentation.slides[0].shapes.iter().find_map(|s| if let ibis_doc_model::ShapeKind::Sheet(sheet) = &s.kind { Some((**sheet).clone()) } else { None }).expect("a sheet")
    }

    /// A text file is opened by making the cells from its records; that must give what writing the workbook and reading it gives.
    #[test]
    fn opening_a_text_file_gives_what_reading_its_workbook_gives() {
        let mut text = String::from("Date,Name,Qty,Rate,Flag,Note,Formula
");
        for i in 0..25_000 {
            text.push_str(&format!("2000/1/{},item {i},{},{}.5%,{},\"line one
line two {i}\",=B{}
", i % 28 + 1, i * 7, i % 100, if i % 2 == 0 { "TRUE" } else { "x" }, i + 1));
        }
        text.push_str(",,,,,
 12,\"a \"\"quoted\"\" cell\",#N/A,1e3,0.1234567890123456789,,
");
        let (old, _) = {
            let o = crate::open(workbook_from(&text, "Sheet1").unwrap()).unwrap();
            (sheet_of(&o.presentation), ())
        };
        let new = crate::open(text.clone().into_bytes()).unwrap();
        let (mut old, new_sheet) = (old, sheet_of(&new.presentation));
        // The formats the old way wrote into its workbook (numbered from 1) are not in the empty workbook the new way saves into: its styles say none.
        for style in &mut old.styles {
            style.xf = style.xf.filter(|&xf| xf == 0);
            // A text file opened sits its cells at the top (issue #17); the workbook written from it does not say so.
            style.vertical = ibis_doc_model::CellVerticalAlign::Top;
            style.vertical_set = true;
        }
        assert_eq!(old.cells.len(), new_sheet.cells.len());
        for (a, b) in old.cells.iter().zip(&new_sheet.cells) {
            assert_eq!(a, b);
        }
        assert_eq!(old.styles, new_sheet.styles);
        assert_eq!(old.rows, new_sheet.rows);
        assert_eq!(old.columns, new_sheet.columns);
        assert!(old == new_sheet, "the rest of the sheet differs");
    }

    /// A cell of about 100 lines (a quoted cell of a text file) makes its row as high as Excel's automatic height goes (409.5 pt), not the 2,000 pt its lines add up to; the rows around it are as high as one line.
    #[test]
    fn a_cell_of_many_lines_makes_a_row_no_higher_than_excels_most() {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let text = format!("a,b\r\nx,y\r\nz,\"{}\"\r\nw,v\r\n", lines.join("\n"));
        let opened = crate::open(text.into_bytes()).unwrap();
        let sheet = sheet_of(&opened.presentation);
        let most = (409.5 * 12700.0) as i64;
        assert_eq!(sheet.rows[2] as i64, most);
        assert!((sheet.rows[1] as i64) < most / 10 && (sheet.rows[3] as i64) < most / 10);
    }

    /// Saving the opened text file as a workbook keeps its cells (the opened workbook holds none of them, they are changes).
    #[test]
    fn a_text_file_saved_as_a_workbook_keeps_its_cells() {
        let text = "a,b,c
1,2.5,x
2000/1/5,10%,\"p
q\"
";
        let opened = crate::open(text.as_bytes().to_vec()).unwrap();
        let saved = opened.source.save(&opened.presentation, crate::save::SaveOptions::default()).unwrap();
        let again = crate::open(saved).unwrap();
        let (a, b) = (sheet_of(&opened.presentation), sheet_of(&again.presentation));
        assert_eq!(a.cells.len(), b.cells.len());
        for (x, y) in a.cells.iter().zip(&b.cells) {
            assert_eq!((x.row, x.column, &x.value, &x.text), (y.row, y.column, &y.value, &y.text));
        }
    }

    #[test]
    fn records() {
        assert_eq!(parse("a,b\r\n\"c,d\",\"e\"\"f\"\n"), vec![vec!["a", "b"], vec!["c,d", "e\"f"]]);
        assert_eq!(parse("1,,3"), vec![vec!["1", "", "3"]]);
    }

    #[test]
    fn round_trip() {
        let bytes = workbook_from("品名,数量,日付\nりんご,3,2026/9/30\n\"み,かん\",1.5,=B2*2\n", "test").unwrap();
        let opened = crate::save::open(bytes).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&opened.presentation.slides[0]).unwrap().clone();
        assert_eq!(sheet.cell(1, 1).unwrap().value, CellValue::Number(3.0));
        assert_eq!(sheet.cell(1, 2).unwrap().text, "2026/9/30");
        assert_eq!(sheet.cell(2, 0).unwrap().value, CellValue::Text("み,かん".into()));
        assert_eq!(sheet.cell(2, 2).unwrap().formula.as_deref(), Some("B2*2"));
        let csv = String::from_utf8(sheet_to_csv(&opened.presentation, 0)).unwrap();
        assert!(csv.starts_with("\u{feff}品名,数量,日付\r\n"));
        assert!(csv.contains("\"み,かん\""));
    }

    #[test]
    fn shift_jis_decoding() {
        // Shift_JIS bytes for "品名,数量\r\nりんご,3\r\n①株式会社,100"
        let (sjis_bytes, _, _) = encoding_rs::SHIFT_JIS.encode("品名,数量\r\nりんご,3\r\n①株式会社,100");
        let decoded = decode(&sjis_bytes).expect("Shift_JIS decode succeeds");
        assert_eq!(decoded, "品名,数量\r\nりんご,3\r\n①株式会社,100");

        // Parse Shift_JIS CSV via open
        let opened = crate::save::open(sjis_bytes.to_vec()).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&opened.presentation.slides[0]).unwrap().clone();
        assert_eq!(sheet.cell(0, 0).unwrap().text, "品名");
        assert_eq!(sheet.cell(1, 0).unwrap().text, "りんご");
        assert_eq!(sheet.cell(2, 0).unwrap().text, "①株式会社");
    }

    #[test]
    fn tsv_delimiter_detection() {
        let tsv = "ID\t名前\t部署\r\n101\t田中\t開発部\r\n102\t佐藤\t営業部\r\n";
        assert_eq!(detect_delimiter(tsv), '\t');
        let records = parse(tsv);
        assert_eq!(records, vec![
            vec!["ID", "名前", "部署"],
            vec!["101", "田中", "開発部"],
            vec!["102", "佐藤", "営業部"],
        ]);

        let opened = crate::save::open(tsv.as_bytes().to_vec()).unwrap();
        let sheet = ibis_doc_model::sheet_calc::sheet_of_slide(&opened.presentation.slides[0]).unwrap().clone();
        assert_eq!(sheet.cell(0, 0).unwrap().text, "ID");
        assert_eq!(sheet.cell(0, 1).unwrap().text, "名前");
        assert_eq!(sheet.cell(1, 1).unwrap().text, "田中");
    }

    #[test]
    fn semicolon_delimiter_detection() {
        let semi = "product;price;tax\r\napple;100;10\r\norange;150;15\r\n";
        assert_eq!(detect_delimiter(semi), ';');
        let records = parse(semi);
        assert_eq!(records[0], vec!["product", "price", "tax"]);
        assert_eq!(records[1], vec!["apple", "100", "10"]);
    }

    #[test]
    fn delimited_export_formats() {
        let bytes = workbook_from("品名,単価\nバナナ,120\n", "test").unwrap();
        let opened = crate::save::open(bytes).unwrap();

        // 1. TSV export
        let tsv_bytes = sheet_to_tsv(&opened.presentation, 0);
        let tsv_str = decode(&tsv_bytes).unwrap();
        assert!(tsv_str.contains("品名\t単価\r\n"));
        assert!(tsv_str.contains("バナナ\t120\r\n"));

        // 2. Shift_JIS CSV export
        let sjis_bytes = sheet_to_csv_sjis(&opened.presentation, 0);
        assert!(!sjis_bytes.starts_with(b"\xEF\xBB\xBF"));
        let decoded_sjis = decode(&sjis_bytes).unwrap();
        assert!(decoded_sjis.contains("品名,単価\r\n"));
        assert!(decoded_sjis.contains("バナナ,120\r\n"));
    }

    #[test]
    fn fixed_width_columns_are_found_and_cut() {
        let text = "Code  Name       Qty\nA001  Apple        3\nB020  Mandarin    12\n";
        assert_eq!(detect_fixed_breaks(text, 1), [6, 17]);
        assert_eq!(split_fixed("A001  Apple        3", &[6, 17]), ["A001", "Apple", "3"]);
        // Past the end of a short line: left out.
        assert_eq!(split_fixed("A001", &[6, 17]), ["A001"]);
        let imported = import(text.as_bytes(), &ImportOptions { fixed: Some(Vec::new()), start_row: 2, ..Default::default() }).unwrap();
        assert_eq!(imported.rows, [vec!["A001", "Apple", "3"], vec!["B020", "Mandarin", "12"]]);
    }
}

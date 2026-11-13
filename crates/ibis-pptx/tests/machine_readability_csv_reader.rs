// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Compare the in-process CSV reader with pinned Harunobu fixtures.
use ibis_doc_model::machine_readability_csv::{read_sheet, CsvReadError, Delimiter};
use ibis_doc_model::machine_readability_layout::LayoutValue;

#[test]
fn csv_reader_matches_reference_and_preserves_text() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-csv-reader-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let delimiter = match case["delimiter"].as_str().unwrap() { "," => Delimiter::Comma, "\t" => Delimiter::Tab, _ => panic!("bad delimiter") };
        let result = read_sheet(case["text"].as_str().unwrap(), delimiter);
        if case["error"].is_string() {
            assert_eq!(result, Err(CsvReadError::UnquotedCarriageReturn), "case {index}: {case}");
            continue;
        }
        let sheet = result.unwrap_or_else(|e| panic!("case {index}: {case}: {e:?}"));
        let expected: Vec<Vec<Option<String>>> = case["records"].as_array().unwrap().iter().map(|row| row.as_array().unwrap().iter().map(|v| v.as_str().map(str::to_owned)).collect()).collect();
        assert_eq!(sheet.records, expected, "case {index}: {case}");
        assert_eq!(sheet.max_columns, case["max_columns"].as_u64().unwrap() as usize, "case {index}");
        let expected_cells: Vec<_> = expected.iter().enumerate().flat_map(|(r, row)| row.iter().enumerate().filter_map(move |(c, v)| v.as_ref().map(|v| (r as u32, c as u32, v.as_str())))).collect();
        let actual_cells = sheet.layout_cells().unwrap();
        let actual: Vec<_> = actual_cells.iter().map(|c| (c.row, c.column, match &c.value { Some(LayoutValue::Text(v)) => v.as_str(), _ => panic!("coerced input") })).collect();
        assert_eq!(actual, expected_cells, "case {index}");
    }
    assert!(read_sheet(&"あ".repeat(131_072), Delimiter::Comma).is_ok());
    assert_eq!(read_sheet(&"あ".repeat(131_073), Delimiter::Comma), Err(CsvReadError::FieldTooLong));
}

#[test]
fn cp932_matches_python_for_every_one_and_two_byte_sequence() {
    use ibis_pptx::machine_readability_csv::{decode_cp932, CsvInputError};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/cp932-python-pairs.bin");
    let fixture = std::fs::read(path).unwrap();
    assert_eq!(fixture.len(), (256 + 65_536) * 8);
    for size in [1usize, 2] {
        let count = if size == 1 { 256 } else { 65_536 };
        let offset = if size == 1 { 0 } else { 256 };
        for n in 0..count {
            let at = (offset + n) * 8;
            let a = u32::from_le_bytes(fixture[at..at + 4].try_into().unwrap());
            let b = u32::from_le_bytes(fixture[at + 4..at + 8].try_into().unwrap());
            let bytes = [(n >> 8) as u8, n as u8];
            let result = decode_cp932(&bytes[2 - size..]);
            if a == 0x110001 { assert_eq!(result, Err(CsvInputError::InvalidCp932), "{size} {n:04x}"); }
            else {
                let mut expected = char::from_u32(a).unwrap().to_string();
                if b != 0x110000 { expected.push(char::from_u32(b).unwrap()); }
                assert_eq!(result, Ok(expected), "{size} {n:04x}");
            }
        }
    }
}

#[test]
fn byte_reader_keeps_encoding_and_cell_values() {
    use ibis_pptx::machine_readability_csv::{read_csv_bytes, CsvInputEncoding, CsvInputError};
    let input = read_csv_bytes(b"\xef\xbb\xbf01,=1+1\n", Delimiter::Comma).unwrap();
    assert_eq!(input.encoding, CsvInputEncoding::Utf8Bom);
    assert_eq!(input.sheet.records, vec![vec![Some("01".into()), Some("=1+1".into())]]);
    let input = read_csv_bytes(&[0x82, 0xa0, b',', 0xa0], Delimiter::Comma).unwrap();
    assert_eq!(input.encoding, CsvInputEncoding::Cp932);
    assert_eq!(input.sheet.records, vec![vec![Some("あ".into()), Some("\u{f8f0}".into())]]);
    let input = read_csv_bytes(&[0xc2, 0xa2], Delimiter::Comma).unwrap();
    assert_eq!(input.encoding, CsvInputEncoding::Utf8);
    assert_eq!(input.sheet.records, vec![vec![Some("¢".into())]]);
    // A malformed UTF-8 BOM input falls back to CP932 and remains invalid.
    assert_eq!(read_csv_bytes(&[0xef, 0xbb, 0xbf, 0xff], Delimiter::Comma), Err(CsvInputError::InvalidCp932));
    assert_eq!(read_csv_bytes(&[0x81], Delimiter::Comma), Err(CsvInputError::InvalidCp932));
}

#[test]
fn csv_bytes_through_native_layout_match_reference_examples() {
    use ibis_doc_model::machine_readability_layout::{merge_column_adjacent_regions, merge_form_fragments, preliminary_islands, OtherRegion};
    use ibis_pptx::machine_readability_csv::{read_csv_bytes, CsvInputEncoding};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-csv-layout-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for (index, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let hex = case["bytes"].as_str().unwrap();
        let bytes: Vec<_> = hex.as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect();
        let delimiter = if case["delimiter"] == "," { Delimiter::Comma } else { Delimiter::Tab };
        let input = read_csv_bytes(&bytes, delimiter).unwrap();
        let encoding = match input.encoding { CsvInputEncoding::Utf8 => "utf-8", CsvInputEncoding::Utf8Bom => "utf-8-sig", CsvInputEncoding::Cp932 => "cp932" };
        assert_eq!(encoding, case["encoding"].as_str().unwrap(), "case {index}");
        let cells = input.sheet.layout_cells().unwrap();
        let preliminary = preliminary_islands(&cells, &[]).unwrap();
        let merged = merge_column_adjacent_regions(&preliminary.candidates, &cells, &[]).unwrap();
        let forms = merge_form_fragments(&merged, &cells, &[]).unwrap();
        let actual: Vec<_> = forms.iter().map(|t| (t.candidate.range.first_row, t.candidate.range.first_column, t.candidate.range.last_row, t.candidate.range.last_column, t.candidate.confidence, t.headers.rows.clone())).collect();
        let expected: Vec<_> = case["tables"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_u64().unwrap() as u32, v[3].as_u64().unwrap() as u32, v[4].as_f64().unwrap(), v[5].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>())).collect();
        assert_eq!(actual, expected, "tables case {index}");
        let actual: Vec<_> = preliminary.others.iter().map(|v| (v.row, v.column, match v.kind { OtherRegion::Title => "title", OtherRegion::Total => "total", OtherRegion::Note => "note", OtherRegion::Source => "source", OtherRegion::Isolated => "isolated" })).collect();
        let expected: Vec<_> = case["others"].as_array().unwrap().iter().map(|v| (v[0].as_u64().unwrap() as u32, v[1].as_u64().unwrap() as u32, v[2].as_str().unwrap())).collect();
        assert_eq!(actual, expected, "others case {index}");
    }
}

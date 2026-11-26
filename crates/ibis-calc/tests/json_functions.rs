// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork

use ibis_calc::eval::Book;
use ibis_calc::formula::to_file;
use ibis_calc::recalc::evaluate;
use ibis_calc::{CellError, CellValue};

struct RawBook {
    rows: Vec<&'static str>,
}

impl Book for RawBook {
    fn sheet_count(&self) -> usize {
        1
    }
    fn sheet_name(&self, _: usize) -> &str {
        "Raw"
    }
    fn value(&self, _: usize, row: u32, column: u32) -> CellValue {
        if column == 0 {
            match self.rows.get(row as usize) {
                Some(&text) if !text.is_empty() => CellValue::Text(text.into()),
                _ => CellValue::Empty,
            }
        } else if row == 0 {
            match column {
                1 => CellValue::Text("answers.name".into()),
                2 => CellValue::Text("answers.party_size".into()),
                3 => CellValue::Text("answers.shelter".into()),
                _ => CellValue::Empty,
            }
        } else {
            CellValue::Empty
        }
    }
    fn formula(&self, _: usize, _: u32, _: u32) -> Option<&str> {
        None
    }
    fn for_each_cell(&self, _: usize, range: (u32, u32, u32, u32), f: &mut dyn FnMut(u32, u32, &CellValue)) {
        for r in range.0..=range.2 {
            for c in range.1..=range.3 {
                let v = self.value(0, r, c);
                if !matches!(v, CellValue::Empty) {
                    f(r, c, &v);
                }
            }
        }
    }
    fn extent(&self, _: usize) -> (u32, u32) {
        (self.rows.len() as u32 + 1, 4)
    }
    fn defined_name(&self, _: &str, _: Option<usize>) -> Option<String> {
        None
    }
}

#[test]
fn json_value_extracts_scalars_arrays_and_defaults() {
    let book = RawBook {
        rows: vec![r#"{"answers":{"name":"山田 太郎","party_size":3,"vip":true},"tags":["a","b"],"a.b":"dot"}"#],
    };
    assert_eq!(
        evaluate(&book, (0, 1, 0), r#"SORANE.JSONVALUE(A1,"answers.name")"#),
        (CellValue::Text("山田 太郎".into()), false),
    );
    assert_eq!(
        evaluate(&book, (0, 1, 0), r#"SORANE.JSONVALUE(A1,"answers.party_size")"#),
        (CellValue::Number(3.0), false),
    );
    assert_eq!(
        evaluate(&book, (0, 1, 0), r#"SORANE.JSONVALUE(A1,"answers.vip")"#),
        (CellValue::Bool(true), false),
    );
    assert_eq!(
        evaluate(&book, (0, 1, 0), r#"SORANE.JSONVALUE(A1,"tags[1]")"#),
        (CellValue::Text("b".into()), false),
    );
    assert_eq!(
        evaluate(&book, (0, 1, 0), r#"SORANE.JSONVALUE(A1,"[""a.b""]")"#),
        (CellValue::Text("dot".into()), false),
    );
    assert_eq!(
        evaluate(&book, (0, 1, 0), r#"SORANE.JSONVALUE(A1,"missing")"#),
        (CellValue::Text("".into()), false),
    );
    assert_eq!(
        evaluate(&book, (0, 1, 0), r#"SORANE.JSONVALUE(A1,"missing","N/A")"#),
        (CellValue::Text("N/A".into()), false),
    );
    assert_eq!(
        evaluate(&book, (0, 1, 0), r#"SORANE.JSONVALUE("{bad","a")"#),
        (CellValue::Error(CellError::Value), false),
    );
}

#[test]
fn json_table_spills_range_with_empty_row_preservation_and_headers() {
    let book = RawBook {
        rows: vec![
            r#"{"answers":{"name":"山田 太郎","party_size":3,"shelter":"第一小"}}"#,
            "",
            r#"{"answers":{"name":"佐藤 花子","party_size":2}}"#,
        ],
    };

    assert_eq!(
        evaluate(
            &book,
            (0, 5, 0),
            r#"TEXTJOIN("|",FALSE,INDEX(SORANE.JSONTABLE(A1:A3,B1:D1),1,0))"#,
        ),
        (CellValue::Text("山田 太郎|3|第一小".into()), false),
    );
    // Empty row in A2 is preserved as empty strings.
    assert_eq!(
        evaluate(
            &book,
            (0, 5, 0),
            r#"TEXTJOIN("|",FALSE,INDEX(SORANE.JSONTABLE(A1:A3,B1:D1),2,0))"#,
        ),
        (CellValue::Text("||".into()), false),
    );
    // Missing field in A3 falls back to "".
    assert_eq!(
        evaluate(
            &book,
            (0, 5, 0),
            r#"TEXTJOIN("|",FALSE,INDEX(SORANE.JSONTABLE(A1:A3,B1:D1),3,0))"#,
        ),
        (CellValue::Text("佐藤 花子|2|".into()), false),
    );
    // Header row option.
    assert_eq!(
        evaluate(
            &book,
            (0, 5, 0),
            r#"TEXTJOIN(",",FALSE,INDEX(SORANE.JSONTABLE(A1:A1,B1:D1,TRUE),1,0))"#,
        ),
        (CellValue::Text("answers.name,answers.party_size,answers.shelter".into()), false),
    );
    // Single JSON array string input.
    assert_eq!(
        evaluate(
            &book,
            (0, 5, 0),
            r#"SUM(INDEX(SORANE.JSONTABLE("[{""n"":10},{""n"":25}]","n"),0,1))"#,
        ),
        (CellValue::Number(35.0), false),
    );
}

#[test]
fn json_keys_discovers_unique_paths_and_stores_xlfn_prefix() {
    let book = RawBook {
        rows: vec![
            r#"{"answers":{"name":"A","party_size":1},"submitted_at":"2026-11-26"}"#,
            r#"{"answers":{"name":"B","needs":"薬"},"submitted_at":"2026-11-26"}"#,
        ],
    };
    assert_eq!(
        evaluate(&book, (0, 5, 0), r#"TEXTJOIN(",",FALSE,SORANE.JSONKEYS(A1:A2))"#),
        (CellValue::Text("answers.name,answers.party_size,submitted_at,answers.needs".into()), false),
    );

    let stored = to_file(r#"SORANE.JSONTABLE(A1:A2,{"answers.name"})"#);
    assert!(stored.starts_with("_xlfn.SORANE.JSONTABLE("), "{stored}");
    let stored_val = to_file(r#"SORANE.JSONVALUE(A1,"answers.name")"#);
    assert!(stored_val.starts_with("_xlfn.SORANE.JSONVALUE("), "{stored_val}");
}

#[test]
fn spot_roster_pipeline_evaluates_jsontable_fieldmatch_and_summary() {
    let book = RawBook {
        rows: vec![
            r#"{"answers":{"name":"山田 太郎","shelter":"第一小","group_id":"grp-a1b2"},"verification":{"trust_status":"unverified"}}"#,
            r#"{"answers":{"name":"山田 花子","shelter":"第一小","group_id":"grp-a1b2"},"verification":{"trust_status":"unverified"}}"#,
            r#"{"answers":{"name":"髙橋 一郎","shelter":"第二中","group_id":""},"verification":{"trust_status":"verified"}}"#,
        ],
    };

    // Expand 3 records into 2 columns (name, shelter) and count by shelter.
    assert_eq!(
        evaluate(
            &book,
            (0, 5, 0),
            r#"COUNTIF(INDEX(SORANE.JSONTABLE(A1:A3,{"answers.shelter"}),0,1),"第一小")"#,
        ),
        (CellValue::Number(2.0), false),
    );
    // Count verified records at 第二中.
    assert_eq!(
        evaluate(
            &book,
            (0, 5, 0),
            r#"COUNTIFS(INDEX(SORANE.JSONTABLE(A1:A3,{"answers.shelter","verification.trust_status"}),0,1),"第二中",INDEX(SORANE.JSONTABLE(A1:A3,{"answers.shelter","verification.trust_status"}),0,2),"verified")"#,
        ),
        (CellValue::Number(1.0), false),
    );
    // Match extracted names against reference name with SORANE.FIELDMATCH.
    assert_eq!(
        evaluate(
            &book,
            (0, 5, 0),
            r#"TEXTJOIN(",",FALSE,SORANE.FIELDMATCH(INDEX(SORANE.JSONTABLE(A1:A3,{"answers.name"}),0,1),"山田 太郎","name"))"#,
        ),
        (CellValue::Text("2,0,0".into()), false),
    );
}

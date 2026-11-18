// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork

use ibis_calc::eval::Book;
use ibis_calc::formula::to_file;
use ibis_calc::recalc::evaluate;
use ibis_calc::{CellError, CellValue};

struct Sheet {
    date1904: bool,
}

impl Book for Sheet {
    fn sheet_count(&self) -> usize {
        1
    }
    fn sheet_name(&self, _: usize) -> &str {
        "Sheet1"
    }
    fn value(&self, _: usize, row: u32, column: u32) -> CellValue {
        match (row, column) {
            (0, 0) => CellValue::Text("東京都千代田区霞が関二丁目１番２号".into()),
            (0, 1) => CellValue::Text("東京都千代田区霞が関2-1-2".into()),
            _ => CellValue::Empty,
        }
    }
    fn formula(&self, _: usize, _: u32, _: u32) -> Option<&str> {
        None
    }
    fn for_each_cell(&self, _: usize, _: (u32, u32, u32, u32), _: &mut dyn FnMut(u32, u32, &CellValue)) {}
    fn extent(&self, _: usize) -> (u32, u32) {
        (1, 2)
    }
    fn defined_name(&self, _: &str, _: Option<usize>) -> Option<String> {
        None
    }
    fn date1904(&self) -> bool {
        self.date1904
    }
}

fn check(args: &str, expected: CellValue) {
    let formula = format!("IBIS.FIELDMATCH({args})");
    let sheet = Sheet { date1904: false };
    assert_eq!(evaluate(&sheet, (0, 2, 0), &formula), (expected.clone(), false), "{formula}");
    let stored = to_file(&formula);
    assert!(stored.starts_with("_xlfn.IBIS.FIELDMATCH("), "{stored}");
    assert_eq!(evaluate(&sheet, (0, 2, 0), &stored), (expected, false), "{stored}");
}

#[test]
fn date_precision_and_era_bounds() {
    for (args, rank) in [
        (r#""令和元年5月1日",DATE(2019,5,1),"date""#, 2.0),
        (r#""昭和60年4月","1985-04-01","date""#, 1.0),
        (r#""2019-05","平成31年","date""#, 0.0),
        (r#""平成元年1月","1989-01-07","date""#, 0.0),
        (r#""平成元年1月","1989-01-08","date""#, 1.0),
        (r#""平成31年","令和元年","date""#, 0.0),
        (r#""2019","平成31年","date""#, 1.0),
        (r#""令和元年4月","2019-04-01","date""#, -1.0),
        (r#""1985-04-01","85/4/1","date""#, 1.0),
        (r#""2023-02-29","2023-02","date""#, -1.0),
        (r#"60,"1900-02","date""#, -1.0),
    ] {
        check(
            args,
            if rank < 0.0 {
                CellValue::Error(CellError::NA)
            } else {
                CellValue::Number(rank)
            },
        );
    }
    assert_eq!(
        evaluate(&Sheet { date1904: true }, (0, 2, 0), r#"IBIS.FIELDMATCH(0,"1904-01-01","date")"#),
        (CellValue::Number(2.0), false)
    );
}

#[test]
fn address_tokens_preserve_location_details() {
    for (args, rank) in [
        (r#"A1,B1,"ADDRESS""#, 2.0),
        (r#""霞が関二丁目十二番二十号","霞が関 2‐12−20","address""#, 2.0),
        (r#""霞が関2-1-2","霞が関2-1-3","address""#, 0.0),
        (r#""霞が関2-1-2 A棟101","霞が関2-1-2 A棟102","address""#, 0.0),
        (r#""霞ヶ関2-1-2","霞が関2-1-2","address""#, 1.0),
        (r#""霞が関1 2丁目1番2号","霞が関12-1-2","address""#, 0.0),
        (r#""霞が関2ー1ー2","霞が関2-1-2","address""#, 0.0),
        (r#""霞が関二十十丁目1番2号","霞が関30-1-2","address""#, 0.0),
    ] {
        check(args, CellValue::Number(rank));
    }
}

#[test]
fn missing_values_and_errors_are_not_matches() {
    check(r#""男性","Ｍ","gender""#, CellValue::Number(2.0));
    check(r#""女性","MALE","gender""#, CellValue::Number(0.0));
    check(r#"1,1,"gender""#, CellValue::Error(CellError::NA));
    check(r#""不明","不明","gender""#, CellValue::Error(CellError::NA));
    check(r#"C1," ","address""#, CellValue::Error(CellError::NA));
    check(r#"1/0,2,"date""#, CellValue::Error(CellError::Div0));
    check(r#"A1,B1,"other""#, CellValue::Error(CellError::Value));
}

#[test]
fn arrays_can_filter_candidates() {
    let result = evaluate(
        &Sheet { date1904: false },
        (0, 2, 0),
        r#"SUM(IBIS.FIELDMATCH({"1985-04-01","1985-04","1985-05"},"1985-04-01","date"))"#,
    );
    assert_eq!(result, (CellValue::Number(3.0), false));
}

#[test]
fn address_place_variants_and_wildcards_are_possible_matches() {
    for (args, result) in [
        (r#""霞ケ関2-1-2","霞が関2-1-2","address""#, 1.0),
        (r#""○町大字山田字上1","○町山田上1","address""#, 1.0),
        (r#""○町字山田1","○町山田1","address""#, 1.0),
        (r#""東京都八丈島八丈町大賀郷2551-2","東京都八丈町大賀郷2551-2","address""#, 1.0),
        (r#""東京都八丈島三宅村阿古497","東京都三宅村阿古497","address""#, 0.0),
        (r#""霞ヶ●2-1-2","霞が関2-1-2","address""#, 1.0),
        (r#""霞〓関2-1-2","霞が関2-1-2","address""#, 1.0),
        (r#""霞?関2-1-2","霞？関2-1-2","address""#, 1.0),
        (r#""●●●2-1-2","霞が関2-1-2","address""#, 0.0),
        (r#""霞●2-1-2","霞が関2-1-2","address""#, 0.0),
        (r#""霞●関2-1-2","霞が関2-1-3","address""#, 0.0),
        (r#""霞●関2-1-2 A棟101","霞が関2-1-2 A棟102","address""#, 0.0),
        (r#""霞●関二丁目 A棟?","霞が関二丁目 A棟101","address""#, 0.0),
    ] {
        check(args, CellValue::Number(result));
    }
    let external = '\u{e123}';
    check(
        &format!(r#""霞{external}関2-1-2","霞が関2-1-2","address","{external}""#),
        CellValue::Number(1.0),
    );
    check(&format!(r#""霞{external}関2-1-2","霞が関2-1-2","address""#), CellValue::Number(0.0));
    check(r#""男性","Ｍ","gender","●""#, CellValue::Error(CellError::Value));
}

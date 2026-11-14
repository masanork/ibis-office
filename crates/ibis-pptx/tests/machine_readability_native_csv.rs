// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Compare experimental native CSV checks with pinned Harunobu outcomes.
use ibis_doc_model::machine_readability_csv::Delimiter;
use ibis_doc_model::machine_readability_rules::FindingSeverity;
use ibis_doc_model::machine_readability_scoring::Severity;
use ibis_pptx::machine_readability_csv::CsvInputEncoding;
use ibis_pptx::machine_readability_native::{analyze_csv_partial, NativeCsvError, UNIMPLEMENTED_CSV_LEVEL1};

#[test]
fn native_csv_checks_match_reference_without_claiming_complete_coverage() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-native-csv-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes: Vec<_> = case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect();
        let delimiter = if case["delimiter"] == "," { Delimiter::Comma } else { Delimiter::Tab };
        let actual = analyze_csv_partial(&bytes, delimiter).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(!actual.complete, "{name}");
        let encoding = match actual.encoding { CsvInputEncoding::Utf8 => "utf-8", CsvInputEncoding::Utf8Bom => "utf-8-sig", CsvInputEncoding::Cp932 => "cp932" };
        assert_eq!(encoding, case["encoding"].as_str().unwrap(), "{name}");
        let r = actual.table.range;
        assert_eq!([r.first_row, r.first_column, r.last_row, r.last_column], std::array::from_fn(|i| case["range"][i].as_u64().unwrap() as u32), "{name}");
        assert_eq!(actual.table.confidence, case["confidence"].as_f64().unwrap(), "{name}");
        assert_eq!(actual.header_rows, case["headers"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>(), "{name}");
        for (got, expected) in actual.checks.iter().chain(actual.level2_checks.iter()).chain(actual.level3_checks.iter()).zip(case["checks"].as_array().unwrap().iter().chain(case["level2_checks"].as_array().unwrap().iter()).chain(case["level3_checks"].as_array().unwrap().iter())) {
            assert_eq!(got.result.rule_id, expected["id"].as_str().unwrap(), "{name}");
            assert_eq!(got.result.passed, expected["passed"].as_bool().unwrap(), "{name}: {}", got.result.rule_id);
            assert_eq!(got.result.confidence, expected["confidence"].as_f64().unwrap(), "{name}: {}", got.result.rule_id);
            let severity = match got.result.severity.unwrap() { Severity::Fatal => "fatal", Severity::Critical => "critical", Severity::Major => "major", Severity::Minor => "minor", Severity::Info => "info" };
            assert_eq!(severity, expected["severity"].as_str().unwrap(), "{name}: {}", got.result.rule_id);
            let findings: Vec<_> = got.findings.iter().map(|f| (f.location.as_str(), match f.severity { FindingSeverity::Error => "error", FindingSeverity::Warning => "warning", FindingSeverity::Info => "info" })).collect();
            let wanted: Vec<_> = expected["findings"].as_array().unwrap().iter().map(|v| (v[0].as_str().unwrap(), v[1].as_str().unwrap())).collect();
            assert_eq!(findings, wanted, "{name}: {}", got.result.rule_id);
        }
        assert_eq!(actual.checks.len(), 11);
        assert_eq!(actual.level2_checks.len(), 5);
        assert_eq!(actual.level3_checks.len(), 9);
    }
    assert_eq!(UNIMPLEMENTED_CSV_LEVEL1, &["L1-09"]);
    assert_eq!(analyze_csv_partial(b"", Delimiter::Comma), Err(NativeCsvError::UnsupportedTableCount(0)));
    assert_eq!(analyze_csv_partial(b"A,B\n1,2\n\n\n\nA,B\n3,4\n", Delimiter::Comma), Err(NativeCsvError::UnsupportedTableCount(2)));
}

#[test]
fn native_multitable_csv_checks_match_reference() {
    use ibis_pptx::machine_readability_native::analyze_csv_tables_partial;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-multitable-csv-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["harunobu_commit"], "88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f");
    for case in data["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes: Vec<_> = case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect();
        let actual = analyze_csv_tables_partial(&bytes, Delimiter::Comma).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let tables = case["tables"].as_array().unwrap();
        assert_eq!(actual.len(), tables.len(), "{name}");
        for (got, expected) in actual.iter().zip(tables) {
            assert!(!got.complete);
            let r = got.table.range;
            assert_eq!([r.first_row, r.first_column, r.last_row, r.last_column], std::array::from_fn(|i| expected["range"][i].as_u64().unwrap() as u32), "{name}");
            assert_eq!(got.header_rows, expected["headers"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect::<Vec<_>>(), "{name}");
            let labels:Vec<_>=got.column_headers.iter().map(|h|serde_json::json!({"column":h.column,"rows":h.rows,"label":h.label,"merged":h.merged})).collect();
            assert_eq!(labels,*expected["column_headers"].as_array().unwrap(),"{name}: column headers");
            assert_eq!(got.table.confidence, expected["confidence"].as_f64().unwrap(), "{name}");
            let checks: Vec<_> = got.checks.iter().chain(&got.level2_checks).chain(&got.level3_checks).collect();
            let wanted = expected["checks"].as_array().unwrap();
            assert_eq!(checks.len(), wanted.len(), "{name}");
            for (a, b) in checks.iter().zip(wanted) {
                assert_eq!(a.result.rule_id, b["id"].as_str().unwrap(), "{name}");
                assert_eq!(a.result.passed, b["passed"].as_bool().unwrap(), "{name}: {}", a.result.rule_id);
                assert_eq!(a.result.confidence, b["confidence"].as_f64().unwrap(), "{name}: {}", a.result.rule_id);
                let severity = match a.result.severity.unwrap() { Severity::Fatal => "fatal", Severity::Critical => "critical", Severity::Major => "major", Severity::Minor => "minor", Severity::Info => "info" };
                assert_eq!(severity, b["severity"].as_str().unwrap(), "{name}: {}", a.result.rule_id);
                let findings: Vec<_> = a.findings.iter().map(|f| (f.location.as_str(), match f.severity { FindingSeverity::Error => "error", FindingSeverity::Warning => "warning", FindingSeverity::Info => "info" })).collect();
                let expected: Vec<_> = b["findings"].as_array().unwrap().iter().map(|v| (v[0].as_str().unwrap(), v[1].as_str().unwrap())).collect();
                assert_eq!(findings, expected, "{name}: {}", a.result.rule_id);
            }
        }
    }
}

#[test]
fn partial_csv_scores_match_reference_aggregation() {
    use ibis_pptx::machine_readability_native::{analyze_csv_tables_partial, score_csv_tables_partial};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-multitable-csv-reference.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let name=case["name"].as_str().unwrap();
        let bytes: Vec<_> = case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect();
        let tables=analyze_csv_tables_partial(&bytes, Delimiter::Comma).unwrap();
        for (got,wanted) in score_csv_tables_partial(&tables).iter().zip(case["scores"].as_array().unwrap()) {
            assert_eq!(got.score.map(u64::from), wanted["score"].as_u64(), "{name}");
            assert_eq!(got.passed as u64, wanted["passed"].as_u64().unwrap(), "{name}");
            assert_eq!(got.total as u64, wanted["total"].as_u64().unwrap(), "{name}");
            let mut ids: Vec<_> = got.failed_rules.iter().map(|v| v.rule_id.as_str()).collect();
            let mut expected: Vec<_> = wanted["failed_rules"].as_array().unwrap().iter().map(|v| v["rule_id"].as_str().unwrap()).collect();
            ids.sort(); expected.sort();
            assert_eq!(ids, expected, "{name}");
            assert_eq!(!got.forced_zero_rules.is_empty(), wanted["forced_zero"].as_bool().unwrap(), "{name}");
        }
    }
}

#[test]
fn native_csv_reports_match_reference_for_all_modes() {
    use ibis_pptx::machine_readability_native::analyze_csv_report_experimental;
    fn normalize(report:&mut serde_json::Value) {
        for level in ["1","2","3"] { if let Some(score)=report["レベル別スコア"][level].as_object_mut() {
            for key in ["失敗ルール","強制0点ルール"] { if let Some(rules)=score.get_mut(key).and_then(|v|v.as_array_mut()) { rules.sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string()); } }
        }}
        for sheet in report["sheets"].as_array_mut().unwrap() { for table in sheet["評価対象エリア"].as_array_mut().unwrap() {
            table["評価内容"].as_array_mut().unwrap().sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string());
        }}
    }
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-multitable-csv-reference.json");
    let data:serde_json::Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for case in data["cases"].as_array().unwrap() {
        let name=case["name"].as_str().unwrap();
        let bytes:Vec<_>=case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
        for mode in ["lite","standard","thorough"] {
            let mut got=analyze_csv_report_experimental(&bytes,"fixture.csv",mode).unwrap_or_else(|e|panic!("{name} {mode}: {e:?}"));
            let compressed:Vec<_>=case["reports"][mode].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
            let reference=miniz_oxide::inflate::decompress_to_vec_zlib(&compressed).unwrap();
            let mut expected:serde_json::Value=serde_json::from_slice(&reference).unwrap(); normalize(&mut got); normalize(&mut expected);
            assert_eq!(got,expected,"{name} {mode}");
        }
    }
}

#[test]
fn native_csv_and_tsv_reports_match_encoded_reference_inputs() {
    use ibis_pptx::machine_readability_native::analyze_csv_report_experimental;
    fn normalize(report:&mut serde_json::Value) {
        for level in ["1","2","3"] { if let Some(score)=report["レベル別スコア"][level].as_object_mut() {
            for key in ["失敗ルール","強制0点ルール"] { if let Some(rules)=score.get_mut(key).and_then(|v|v.as_array_mut()) { rules.sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string()); } }
        }}
        for sheet in report["sheets"].as_array_mut().unwrap() { for table in sheet["評価対象エリア"].as_array_mut().unwrap() {
            table["評価内容"].as_array_mut().unwrap().sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string());
        }}
    }
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-csv-report-e2e-reference.json");
    let data:serde_json::Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for (i,case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let bytes:Vec<_>=case["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
        let mut got=analyze_csv_report_experimental(&bytes,case["name"].as_str().unwrap(),"thorough").unwrap_or_else(|e|panic!("case {i}: {e:?}"));
        let mut expected=case["report"].clone(); normalize(&mut got); normalize(&mut expected);
        assert_eq!(got,expected,"case {i}");
    }
}

#[test]
fn mixed_bulk_reports_match_pinned_harunobu() {
    use ibis_pptx::machine_readability_native::analyze_csv_report_experimental;
    use ibis_pptx::machine_readability_xlsx::analyze_xlsx_report_experimental;
    use ibis_pptx::machine_readability_report::combine_native_reports;
    fn normalize(report:&mut serde_json::Value) {
        for file in report["ファイル別結果"].as_array_mut().unwrap() {
            for level in ["1","2","3"] { if let Some(score)=file["レベル別スコア"][level].as_object_mut() {
                for key in ["失敗ルール","強制0点ルール"] { if let Some(rules)=score.get_mut(key).and_then(|v|v.as_array_mut()) { rules.sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string()); } }
            }}
            for sheet in file["sheets"].as_array_mut().unwrap() { for table in sheet["評価対象エリア"].as_array_mut().unwrap() {
                table["評価内容"].as_array_mut().unwrap().sort_by_key(|v|v["ルールID"].as_str().unwrap().to_string());
            }}
        }
    }
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/machine-readability/harunobu-bulk-e2e-reference.json");
    let data:serde_json::Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let inputs=data["inputs"].as_array().unwrap();
    for (i,case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let mode=case["mode"].as_str().unwrap();let mut files=Vec::new();
        for index in case["indices"].as_array().unwrap() {
            let input=&inputs[index.as_u64().unwrap() as usize];let name=input["name"].as_str().unwrap();
            let bytes:Vec<_>=input["bytes"].as_str().unwrap().as_bytes().chunks_exact(2).map(|p|u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect();
            files.push(if name.ends_with(".xlsx") { analyze_xlsx_report_experimental(&bytes,name,mode).unwrap() } else { analyze_csv_report_experimental(&bytes,name,mode).unwrap() });
        }
        let mut got=combine_native_reports(&files).unwrap();let mut expected=case["report"].clone();normalize(&mut got);normalize(&mut expected);
        assert_eq!(got,expected,"case {i}");
    }
}

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
        assert_eq!(actual.level2_checks.len(), 4);
        assert_eq!(actual.level3_checks.len(), 5);
    }
    assert_eq!(UNIMPLEMENTED_CSV_LEVEL1, &["L1-09"]);
    assert_eq!(analyze_csv_partial(b"", Delimiter::Comma), Err(NativeCsvError::UnsupportedTableCount(0)));
    assert_eq!(analyze_csv_partial(b"A,B\n1,2\n\n\n\nA,B\n3,4\n", Delimiter::Comma), Err(NativeCsvError::UnsupportedTableCount(2)));
}

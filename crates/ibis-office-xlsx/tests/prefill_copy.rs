// SPDX-License-Identifier: AGPL-3.0-only
#![cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]

#[path = "../src/prefill_copy.rs"]
mod prefill_copy;

use ibis_doc_model::sheet_calc::sheet_of_slide;
use prefill_copy::{ReviewInput, save_reviewed_copy};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

#[test]
fn desktop_copy_writes_selected_cells_and_rejects_stale_review() {
    let folder = tempfile::tempdir().unwrap();
    let workbook = folder.path().join("original.xlsx");
    let source = folder.path().join("training.sd-jwt");
    let mapping = folder.path().join("mapping.json");
    let initial = include_bytes!("../../../test/fixtures/structured-preview/site-entry-form.xlsx");
    let source_text = include_str!("../../../test/fixtures/structured-preview/training.sd-jwt");
    let mapping_text =
        include_str!("../../../test/fixtures/structured-preview/prefill-sd-jwt.json");
    std::fs::write(&workbook, initial).unwrap();
    std::fs::write(&source, source_text).unwrap();
    std::fs::write(&mapping, mapping_text).unwrap();
    let original = ibis_formats::open(initial.as_slice()).unwrap();
    let slide = original.presentation.slides[0].id;
    let settings: Value = serde_json::from_str(mapping_text).unwrap();
    let report = ibis_doc_service::review_prefill(
        &original.presentation,
        slide,
        false,
        "sd-jwt",
        source_text,
        &settings["mappings"],
    )
    .unwrap();
    let review = ReviewInput {
        source,
        mapping,
        source_text: source_text.into(),
        mapping_text: mapping_text.into(),
        format: "sd-jwt".into(),
        report,
    };
    let targets = BTreeSet::from(["training_name".into()]);
    let output = folder.path().join("result.xlsx");
    let save = |path: &Path| {
        save_reviewed_copy(
            &review,
            original.presentation.clone(),
            &original.source,
            slide,
            &workbook,
            path,
            &targets,
        )
    };
    save(&output).unwrap();
    assert_eq!(std::fs::read(&workbook).unwrap(), initial);
    let reopened = ibis_formats::open(std::fs::read(&output).unwrap()).unwrap();
    let sheet = sheet_of_slide(&reopened.presentation.slides[0]).unwrap();
    assert_eq!(
        sheet.cell(4, 1).unwrap().value,
        ibis_doc_model::CellValue::Text("安全研修（サンプル）".into())
    );
    assert!(
        sheet
            .cell(5, 1)
            .is_none_or(|cell| cell.value == ibis_doc_model::CellValue::Empty)
    );
    let written = std::fs::read(&output).unwrap();
    assert!(save(&output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), written);
    let refused = folder.path().join("refused.xlsx");
    assert!(
        save_reviewed_copy(
            &review,
            reopened.presentation,
            &original.source,
            slide,
            &workbook,
            &refused,
            &targets,
        )
        .is_err()
    );
    assert!(!refused.exists());
    std::fs::write(&review.source, format!("{source_text}\n")).unwrap();
    assert!(save(&refused).is_err());
    assert!(!refused.exists());
}

#[test]
fn desktop_copy_allows_explicit_overwrite_of_occupied_cell() {
    let folder = tempfile::tempdir().unwrap();
    let workbook = folder.path().join("filled-once.xlsx");
    let source = folder.path().join("training.sd-jwt");
    let mapping = folder.path().join("mapping.json");
    let initial = include_bytes!("../../../test/fixtures/structured-preview/site-entry-form.xlsx");
    let source_text = include_str!("../../../test/fixtures/structured-preview/training.sd-jwt");
    let mapping_text =
        include_str!("../../../test/fixtures/structured-preview/prefill-sd-jwt.json");
    std::fs::write(&source, source_text).unwrap();
    std::fs::write(&mapping, mapping_text).unwrap();

    // First prefill training_name into a new file so cell B5 becomes occupied.
    let first_wb = folder.path().join("orig.xlsx");
    std::fs::write(&first_wb, initial).unwrap();
    let orig_open = ibis_formats::open(initial.as_slice()).unwrap();
    let slide = orig_open.presentation.slides[0].id;
    let settings: Value = serde_json::from_str(mapping_text).unwrap();
    let report = ibis_doc_service::review_prefill(
        &orig_open.presentation,
        slide,
        false,
        "sd-jwt",
        source_text,
        &settings["mappings"],
    )
    .unwrap();
    let review = ReviewInput {
        source: source.clone(),
        mapping: mapping.clone(),
        source_text: source_text.into(),
        mapping_text: mapping_text.into(),
        format: "sd-jwt".into(),
        report,
    };
    let targets = BTreeSet::from(["training_name".into()]);
    save_reviewed_copy(
        &review,
        orig_open.presentation,
        &orig_open.source,
        slide,
        &first_wb,
        &workbook,
        &targets,
    )
    .unwrap();

    // Reopen the filled workbook and preview a JSON source with a new value for the occupied cell.
    let updated_source = folder.path().join("updated.json");
    let updated_source_text = r#"{"training":"更新済み安全研修"}"#;
    let updated_mapping = folder.path().join("updated-mapping.json");
    let updated_mapping_text = serde_json::json!({
        "sheet": settings["sheet"],
        "mappings": [{"target": "training_name", "source_path": "/training"}]
    })
    .to_string();
    std::fs::write(&updated_source, updated_source_text).unwrap();
    std::fs::write(&updated_mapping, &updated_mapping_text).unwrap();

    let filled_bytes = std::fs::read(&workbook).unwrap();
    let filled_open = ibis_formats::open(filled_bytes.as_slice()).unwrap();
    let filled_slide = filled_open.presentation.slides[0].id;
    let updated_settings: Value = serde_json::from_str(&updated_mapping_text).unwrap();
    let occ_report = ibis_doc_service::review_prefill(
        &filled_open.presentation,
        filled_slide,
        false,
        "json",
        updated_source_text,
        &updated_settings["mappings"],
    )
    .unwrap();
    assert_eq!(occ_report["changes"][0]["status"], "occupied");

    let occ_review = ReviewInput {
        source: updated_source,
        mapping: updated_mapping,
        source_text: updated_source_text.into(),
        mapping_text: updated_mapping_text,
        format: "json".into(),
        report: occ_report,
    };
    let refused = folder.path().join("refused-overwrite.xlsx");
    assert!(
        save_reviewed_copy(
            &occ_review,
            filled_open.presentation.clone(),
            &filled_open.source,
            filled_slide,
            &workbook,
            &refused,
            &targets,
        )
        .is_err()
    );
    assert!(!refused.exists());

    let overwritten = folder.path().join("overwritten.xlsx");
    prefill_copy::save_reviewed_copy_with_options(
        &occ_review,
        filled_open.presentation,
        &filled_open.source,
        filled_slide,
        &workbook,
        &overwritten,
        &targets,
        true,
    )
    .unwrap();
    let reopened = ibis_formats::open(std::fs::read(&overwritten).unwrap()).unwrap();
    let sheet = sheet_of_slide(&reopened.presentation.slides[0]).unwrap();
    assert_eq!(
        sheet.cell(4, 1).unwrap().value,
        ibis_doc_model::CellValue::Text("更新済み安全研修".into())
    );
}

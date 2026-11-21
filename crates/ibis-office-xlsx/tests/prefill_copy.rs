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

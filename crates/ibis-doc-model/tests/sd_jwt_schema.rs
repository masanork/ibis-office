// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use ibis_doc_model::{
    Document, EmuRect, Presentation, SdJwtSchema, SdJwtSchemaValueType, SdJwtTemplateKind,
    Shape, ShapeId, ShapeKind, Sheet, SheetState, Slide, SlideId, Workbook, WorkbookSheet,
    build_sd_jwt_template_commands, extract_sheet_claims,
};

fn blank_worksheet() -> (Presentation, SlideId) {
    let slide_id = SlideId(1);
    let mut slide = Slide::new(slide_id);
    slide.shapes.push(Shape::new(
        ShapeId(1),
        EmuRect::new(0, 0, 1_000_000, 1_000_000),
        ShapeKind::Sheet(Arc::new(Sheet::default())),
    ));
    let mut wb = Workbook::default();
    wb.sheets.push(WorkbookSheet {
        name: "Sheet1".to_string(),
        state: SheetState::Visible,
        slide: Some(slide_id),
        hidden: None,
        other: false,
    });
    let mut pres = Presentation::new();
    pres.slides = vec![slide];
    pres.workbook = Some(Arc::new(wb));
    (pres, slide_id)
}

#[test]
fn schema_json_roundtrip_and_type_metadata_compat() {
    let custom_json = r#"{
        "vct": "urn:example:employee:1",
        "name": "Employee Badge",
        "claims": [
            { "path": ["employee_id"], "sd": "never", "type": "string", "sample": "EMP-007", "display": [{ "label": "社員番号" }] },
            { "path": "address.city", "sd": "allowed", "type": "string", "disclose_default": false, "sample": "Tokyo", "label": "都市" },
            { "path": "roles", "sd": true, "type": "string_array", "sample": "admin, reviewer" },
            {
                "path": "projects",
                "sd": true,
                "type": "object_table",
                "columns": [
                    { "key": "code", "type": "string" },
                    { "key": "active", "type": "boolean" }
                ]
            }
        ]
    }"#;

    let parsed = SdJwtSchema::from_json(custom_json).expect("valid schema");
    assert_eq!(parsed.vct, "urn:example:employee:1");
    assert_eq!(parsed.name, "Employee Badge");
    assert_eq!(parsed.claims.len(), 4);
    assert!(!parsed.claims[0].selective);
    assert_eq!(parsed.claims[0].label.as_deref(), Some("社員番号"));
    assert!(parsed.claims[1].selective);
    assert!(!parsed.claims[1].disclose_default);
    assert_eq!(parsed.claims[2].value_type, SdJwtSchemaValueType::StringArray);
    assert_eq!(parsed.claims[3].value_type, SdJwtSchemaValueType::ObjectTable);
    assert_eq!(parsed.claims[3].columns.len(), 2);

    let re_encoded = parsed.to_json_pretty();
    let reparsed = SdJwtSchema::from_json(&re_encoded).expect("roundtrip");
    assert_eq!(reparsed, parsed);
}

#[test]
fn infer_from_mdl_template_and_scaffold_custom_schema() {
    let (pres, slide) = blank_worksheet();
    let mut doc = Document::new(pres);
    let cmds = build_sd_jwt_template_commands(
        doc.presentation(),
        slide,
        SdJwtTemplateKind::Iso18013Mdl,
    );
    doc.execute(cmds).unwrap();

    let inferred = SdJwtSchema::infer_from_sheet(doc.presentation(), slide, Some("mDL Inferred"))
        .expect("schema inferred from mDL sheet");
    assert_eq!(inferred.vct, "org.iso.18013.5.1.mDL");
    let priv_claim = inferred
        .claims
        .iter()
        .find(|c| c.path == "driving_privileges")
        .expect("driving_privileges inferred");
    assert_eq!(priv_claim.value_type, SdJwtSchemaValueType::ObjectTable);
    assert!(!priv_claim.columns.is_empty());

    // Scaffold a fresh sheet from the inferred schema and re-extract claims
    let (pres2, slide2) = blank_worksheet();
    let mut doc2 = Document::new(pres2);
    let scaffold_cmds = inferred.build_sheet_commands(doc2.presentation(), slide2);
    doc2.execute(scaffold_cmds).unwrap();
    let extracted = extract_sheet_claims(doc2.presentation(), slide2).expect("extracted claims");
    assert!(extracted.claims.iter().any(|c| c.path_str == "vct"));
    assert!(extracted.claims.iter().any(|c| c.path_str == "driving_privileges"));
}

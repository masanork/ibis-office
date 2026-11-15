// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use ibis_doc_model::{
    CellKind, CellStyle, CellValue, ClaimValue, ControlKind, DefinedName, Document, EmuRect,
    FormControl, Presentation, Shape, ShapeId, ShapeKind, Sheet, SheetCell, Slide, SlideId,
    Workbook, WorkbookSheet, extract_sheet_claims,
};

fn put_cell(sheet: &mut Sheet, row: u32, column: u32, value: CellValue, text: &str, style: u32) {
    let mut cells = (*sheet.cells).clone();
    cells.push(SheetCell {
        row,
        column,
        kind: CellKind::of(&value),
        value,
        text: text.to_string(),
        formula: None,
        style,
        phonetic: None,
        runs: None,
    });
    cells.sort_by_key(|c| (c.row, c.column));
    sheet.cells = cells.into();
}

fn sample_presentation() -> (Presentation, SlideId, ShapeId) {
    let slide_id = SlideId(1);
    let mut sheet = Sheet::default();
    let general_style = CellStyle::default();
    let mut date_style = CellStyle::default();
    date_style.number_format = "yyyy-mm-dd".to_string();
    let mut checkbox_style = CellStyle::default();
    checkbox_style.checkbox = true;
    sheet.styles = vec![general_style, date_style, checkbox_style];

    put_cell(
        &mut sheet,
        0,
        1,
        CellValue::Text("https://credentials.example.com/identity_credential".into()),
        "https://credentials.example.com/identity_credential",
        0,
    );
    put_cell(
        &mut sheet,
        1,
        1,
        CellValue::Text("https://issuer.example.com".into()),
        "https://issuer.example.com",
        0,
    );
    put_cell(&mut sheet, 2, 1, CellValue::Number(1_700_000_000.0), "1700000000", 1);
    put_cell(&mut sheet, 3, 1, CellValue::Text("山田".into()), "山田", 0);
    put_cell(&mut sheet, 4, 1, CellValue::Text("太郎".into()), "太郎", 0);
    let serial_1990_05_15 = ibis_calc::eval::convert::serial_from_date(1990, 5, 15, false) as f64;
    put_cell(&mut sheet, 5, 1, CellValue::Number(serial_1990_05_15), "1990-05-15", 1);
    put_cell(&mut sheet, 6, 1, CellValue::Text("JP".into()), "JP", 0);
    put_cell(&mut sheet, 7, 1, CellValue::Number(2.0), "2", 0);
    put_cell(&mut sheet, 8, 1, CellValue::Bool(true), "TRUE", 2);
    put_cell(&mut sheet, 9, 1, CellValue::Empty, "", 0);
    put_cell(&mut sheet, 4, 2, CellValue::Bool(false), "FALSE", 0);

    put_cell(&mut sheet, 0, 5, CellValue::Text("東京都".into()), "東京都", 0);
    put_cell(&mut sheet, 1, 5, CellValue::Text("大阪府".into()), "大阪府", 0);
    put_cell(&mut sheet, 2, 5, CellValue::Text("北海道".into()), "北海道", 0);

    let mut slide = Slide::new(slide_id);
    slide.shapes.push(Shape::new(
        ShapeId(1),
        EmuRect::new(0, 0, 1_000_000, 1_000_000),
        ShapeKind::Sheet(Arc::new(sheet)),
    ));

    let mut drop = FormControl::new(ControlKind::Drop, "sd.address.region");
    drop.link = Some("$B$8".into());
    drop.range = Some("$F$1:$F$3".into());
    drop.value = 2;
    slide.shapes.push(Shape::new(
        ShapeId(10),
        EmuRect::new(0, 0, 500_000, 200_000),
        ShapeKind::Control(Arc::new(drop)),
    ));

    let mut radio1 = FormControl::new(ControlKind::Radio, "sd.gender");
    radio1.first_button = true;
    radio1.text = "male".into();
    radio1.checked = 0;
    slide.shapes.push(Shape::new(
        ShapeId(11),
        EmuRect::new(0, 0, 400_000, 200_000),
        ShapeKind::Control(Arc::new(radio1)),
    ));
    let mut radio2 = FormControl::new(ControlKind::Radio, "Radio Button 2");
    radio2.first_button = false;
    radio2.text = "female".into();
    radio2.checked = 1;
    slide.shapes.push(Shape::new(
        ShapeId(12),
        EmuRect::new(0, 0, 400_000, 200_000),
        ShapeKind::Control(Arc::new(radio2)),
    ));

    let disclose_gn_id = ShapeId(20);
    let mut chk = FormControl::new(ControlKind::CheckBox, "disclose.given_name");
    chk.text = "名を開示".into();
    chk.link = Some("$C$5".into());
    chk.checked = 0;
    slide.shapes.push(Shape::new(
        disclose_gn_id,
        EmuRect::new(0, 0, 400_000, 200_000),
        ShapeKind::Control(Arc::new(chk)),
    ));

    let mut wb = Workbook::default();
    wb.sheets.push(WorkbookSheet {
        name: "Sheet1".into(),
        state: ibis_doc_model::SheetState::Visible,
        slide: Some(slide_id),
        hidden: None,
        other: false,
    });
    wb.names = vec![
        DefinedName { name: "claim.vct".into(), sheet: None, formula: "Sheet1!$B$1".into(), hidden: false },
        DefinedName { name: "claim.iss".into(), sheet: None, formula: "Sheet1!$B$2".into(), hidden: false },
        DefinedName { name: "claim.iat".into(), sheet: None, formula: "Sheet1!$B$3".into(), hidden: false },
        DefinedName { name: "sd.family_name".into(), sheet: None, formula: "Sheet1!$B$4".into(), hidden: false },
        DefinedName { name: "sd.given_name".into(), sheet: None, formula: "Sheet1!$B$5".into(), hidden: false },
        DefinedName { name: "sd.birthdate".into(), sheet: None, formula: "Sheet1!$B$6".into(), hidden: false },
        DefinedName { name: "sd.address.country".into(), sheet: None, formula: "Sheet1!$B$7".into(), hidden: false },
        DefinedName { name: "sd.prefecture_from_cell".into(), sheet: None, formula: "Sheet1!$B$8".into(), hidden: false },
        DefinedName { name: "sd.age_over_18".into(), sheet: None, formula: "Sheet1!$B$9".into(), hidden: false },
        DefinedName { name: "sd.middle_name".into(), sheet: None, formula: "Sheet1!$B$10".into(), hidden: false },
    ];

    let mut pres = Presentation::new();
    pres.slides = vec![slide];
    pres.workbook = Some(Arc::new(wb));
    (pres, slide_id, disclose_gn_id)
}

#[test]
fn extracts_claims_nested_objects_dates_controls_and_disclosure_toggles() {
    let (pres, slide_id, _chk_id) = sample_presentation();
    let bindings = extract_sheet_claims(&pres, slide_id).expect("extract bindings");

    assert!(bindings.claim("middle_name").is_none());

    let vct = bindings.claim("vct").unwrap();
    assert!(!vct.selective);
    assert_eq!(vct.value.as_str(), Some("https://credentials.example.com/identity_credential"));
    let iat = bindings.claim("iat").unwrap();
    assert!(!iat.selective);
    assert_eq!(iat.value, ClaimValue::Int(1_700_000_000));

    let family_name = bindings.claim("family_name").unwrap();
    assert!(family_name.selective);
    assert!(family_name.disclose);
    assert_eq!(family_name.value.as_str(), Some("山田"));

    let given_name = bindings.claim("given_name").unwrap();
    assert!(given_name.selective);
    assert!(!given_name.disclose);
    assert_eq!(given_name.value.as_str(), Some("太郎"));

    let birthdate = bindings.claim("birthdate").unwrap();
    assert_eq!(birthdate.value.as_str(), Some("1990-05-15"));

    let region = bindings.claim("address.region").unwrap();
    assert_eq!(region.value.as_str(), Some("大阪府"));
    let pref = bindings.claim("prefecture_from_cell").unwrap();
    assert_eq!(pref.value.as_str(), Some("大阪府"));

    let gender = bindings.claim("gender").unwrap();
    assert_eq!(gender.value.as_str(), Some("female"));

    let age_over_18 = bindings.claim("age_over_18").unwrap();
    assert_eq!(age_over_18.value, ClaimValue::Bool(true));

    let plain = bindings.to_plain_claims_json();
    let disclosed = bindings.to_disclosed_claims_json();
    assert!(plain.as_object().unwrap().contains_key("given_name"));
    assert!(!disclosed.as_object().unwrap().contains_key("given_name"));

    let addr = disclosed.as_object().unwrap().get("address").unwrap().as_object().unwrap();
    assert_eq!(addr.get("country").and_then(ClaimValue::as_str), Some("JP"));
    assert_eq!(addr.get("region").and_then(ClaimValue::as_str), Some("大阪府"));
    assert!(disclosed.to_json().contains(r#""address":{"country":"JP","region":"大阪府"}"#));
}

#[test]
fn set_disclose_commands_updates_checkbox_control_and_linked_cell() {
    let (pres, slide_id, chk_id) = sample_presentation();
    let bindings = extract_sheet_claims(&pres, slide_id).unwrap();
    assert!(!bindings.claim("given_name").unwrap().disclose);

    let cmds = bindings.set_disclose_commands(&pres, &["given_name"], false);
    assert_eq!(cmds.len(), 2);

    let mut doc = Document::new(pres);
    doc.execute(cmds).unwrap();

    let ShapeKind::Control(c) = &doc.presentation().slide(slide_id).unwrap().shape(chk_id).unwrap().kind else {
        panic!("expected control");
    };
    assert_eq!(c.checked, 1);

    let updated = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
    assert!(updated.claim("given_name").unwrap().disclose);
}

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Form controls of the Excel 2007 form: a worksheet with no `controls` element, the controls only in its VML
//! drawing (`x:ClientData ObjectType=...`) and `ctrlProp` parts (GitHub issue #95).

use std::sync::Arc;

use ibis_doc_model::{ControlKind, FormControl, Shape, ShapeKind};
use ibis_pptx::SaveOptions;

fn fixture() -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/editing/xlsx-form-controls-2007.xlsx")).expect("read xlsx-form-controls-2007.xlsx")
}

fn controls(p: &ibis_doc_model::Presentation) -> Vec<(&Shape, &FormControl, bool)> {
    let slide = &p.slides[0];
    slide
        .shapes
        .iter()
        .filter_map(|s| match &s.kind {
            ShapeKind::Control(c) => Some((s, c.as_ref(), slide.hidden_shapes.contains(&s.id))),
            _ => None,
        })
        .collect()
}

fn zip_text(bytes: &[u8], name: &str) -> String {
    String::from_utf8(ibis_ooxml::Package::open(bytes.to_vec()).unwrap().read(name).expect("part")).unwrap()
}

#[test]
fn reads_the_controls_that_only_the_vml_describes() {
    let loaded = ibis_pptx::open(fixture()).expect("open");
    let found = controls(&loaded.presentation);
    let kinds: Vec<ControlKind> = found.iter().map(|(_, c, _)| c.kind).collect();
    assert_eq!(kinds, [ControlKind::Radio, ControlKind::Radio, ControlKind::GroupBox, ControlKind::CheckBox, ControlKind::Drop]);
    // The text, the linked cells and the state come from the VML and the ctrlProp parts.
    let (_, radio, _) = found[0];
    assert_eq!(radio.text, "Large");
    assert_eq!(radio.link.as_deref(), Some("$B$2"));
    assert!(radio.first_button);
    assert!(!radio.part.is_empty(), "the ctrlProp lined up with the shape");
    let (_, drop, _) = found[4];
    assert_eq!(drop.range.as_deref(), Some("$D$1:$D$3"));
    // The group box is hidden (`visibility:hidden`); the others are shown.
    assert_eq!(found.iter().map(|(_, _, h)| *h).collect::<Vec<_>>(), [false, false, true, false, false]);
    // The place: the style's `margin-left`, `margin-top`, `width` and `height` in points.
    let frame = found[0].0.frame;
    assert_eq!((frame.x, frame.y, frame.width, frame.height), (96 * 12700, 60 * 12700, 48 * 12700, 15 * 12700));
    // The linked cells decide the state: B2 = 1 chooses the first option button; B5 = 2 the second item.
    assert_eq!(found[0].1.checked, 1);
    assert_eq!(found[4].1.value, 2);
}

#[test]
fn a_changed_state_is_written_to_the_vml_and_the_ctrl_prop_and_the_rest_is_kept() {
    let mut loaded = ibis_pptx::open(fixture()).expect("open");
    let id = controls(&loaded.presentation)[1].0.id;
    {
        let slide = loaded.presentation.slides[0].shapes.iter_mut().find(|s| s.id == id).unwrap();
        let ShapeKind::Control(c) = &mut slide.kind else { unreachable!() };
        let mut next = (**c).clone();
        next.checked = 1;
        next.name = "sd.size_option".to_string();
        *c = Arc::new(next);
    }
    let saved = loaded.source.save(&loaded.presentation, SaveOptions::default()).expect("save");
    let vml = zip_text(&saved, "xl/drawings/vmlDrawing1.vml");
    assert!(vml.contains("<x:Checked>1</x:Checked>"), "the second option button's VML says it is chosen:\n{vml}");
    assert!(vml.contains("id=\"sd.size_option\" o:spid=\"_x0000_s1026\""), "custom control name saved in VML id:\n{vml}");
    assert!(zip_text(&saved, "xl/ctrlProps/ctrlProp2.xml").contains("checked=\"Checked\""));
    // Reading the saved file gives the same controls; the group box is still hidden.
    let again = ibis_pptx::open(saved).expect("reopen");
    let found = controls(&again.presentation);
    assert_eq!(found.len(), 5);
    assert_eq!(found[1].1.checked, 1);
    assert_eq!(found[1].1.name, "sd.size_option");
    assert!(found[2].2);
}

#[test]
fn deleting_a_control_takes_its_shape_and_ctrl_prop_with_it() {
    let mut loaded = ibis_pptx::open(fixture()).expect("open");
    let slide = &mut loaded.presentation.slides[0];
    let at = slide.shapes.iter().position(|s| matches!(&s.kind, ShapeKind::Control(c) if c.kind == ControlKind::CheckBox)).unwrap();
    slide.shapes.remove(at);
    let saved = loaded.source.save(&loaded.presentation, SaveOptions::default()).expect("save");
    assert!(!zip_text(&saved, "xl/drawings/vmlDrawing1.vml").contains("_x0000_s1028"));
    let again = ibis_pptx::open(saved).expect("reopen");
    assert_eq!(controls(&again.presentation).len(), 4);
}

#[test]
fn custom_control_name_roundtrips_for_modern_and_legacy_controls() {
    use ibis_doc_model::EmuRect;

    // 1. Modern DrawingML + <controls> form control rename roundtrip on a blank workbook
    let mut opened = ibis_pptx::open(ibis_pptx::blank_file(ibis_pptx::Kind::Workbook).unwrap()).unwrap();
    let slide_id = opened.presentation.slides[0].id;
    let mut ctrl = FormControl::new(ControlKind::CheckBox, "Check Box 1");
    ctrl.text = "メールを開示".to_string();
    ctrl.link = Some("$B$2".to_string());
    ctrl.checked = 1;
    opened.presentation.slide_mut(slide_id).unwrap().shapes.push(Shape::new(
        ibis_doc_model::ShapeId(500),
        EmuRect::new(914_400, 457_200, 1_828_800, 274_320),
        ShapeKind::Control(Arc::new(ctrl)),
    ));
    let saved1 = opened.source.save(&opened.presentation, SaveOptions::default()).unwrap();
    let mut reopened1 = ibis_pptx::open(saved1).unwrap();
    {
        let sh = reopened1
            .presentation
            .slides[0]
            .shapes
            .iter_mut()
            .find(|s| matches!(s.kind, ShapeKind::Control(_)))
            .unwrap();
        let ShapeKind::Control(c) = &mut sh.kind else { unreachable!() };
        let mut next = (**c).clone();
        assert_eq!(next.name, "Check Box 1");
        next.name = "disclose.email".to_string();
        *c = Arc::new(next);
    }
    let saved2 = reopened1.source.save(&reopened1.presentation, SaveOptions::default()).unwrap();
    let sheet_xml = zip_text(&saved2, "xl/worksheets/sheet1.xml");
    let drawing_xml = zip_text(&saved2, "xl/drawings/drawing1.xml");
    assert!(sheet_xml.contains(r#"name="disclose.email""#), "sheet1.xml should contain renamed control:\n{sheet_xml}");
    assert!(drawing_xml.contains(r#"name="disclose.email""#), "drawing1.xml should contain renamed control:\n{drawing_xml}");
    let reopened2 = ibis_pptx::open(saved2.clone()).unwrap();
    let found = controls(&reopened2.presentation);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.name, "disclose.email");
    assert_eq!(found[0].1.text, "メールを開示");

    // 2. Legacy VML-only control rename roundtrip (stripped <controls> + <drawing>, keeping vmlDrawing1.vml)
    let mut pkg = ibis_ooxml::Package::open(saved2).unwrap();
    let vml_bytes = br##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel">
<v:shape id="_x0000_s1043" type="#_x0000_t201" style="position:absolute;margin-left:72pt;margin-top:36pt;width:120pt;height:18pt">
 <v:textbox><div>Option A</div></v:textbox>
 <x:ClientData ObjectType="CheckBox"><x:Checked>1</x:Checked><x:FmlaLink>$B$2</x:FmlaLink></x:ClientData>
</v:shape>
</xml>"##;
    pkg.set_bytes("xl/drawings/vmlDrawing1.vml", vml_bytes.to_vec());
    pkg.add_relationship(
        "xl/worksheets/sheet1.xml",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing",
        "../drawings/vmlDrawing1.vml",
        false,
    )
    .unwrap();
    {
        let doc = pkg.xml_mut("xl/worksheets/sheet1.xml").unwrap();
        let root = doc.root_element().unwrap();
        for name in ["drawing", "controls"] {
            if let Some(node) = doc.children(root).into_iter().copied().find(|&n| doc.is_element(n) && doc.local_name(n) == name) {
                doc.detach(node);
            }
        }
    }
    let vml_only_bytes = pkg.save(SaveOptions::default()).unwrap();
    let mut vml_loaded = ibis_pptx::open(vml_only_bytes).unwrap();
    {
        let sh = vml_loaded
            .presentation
            .slides[0]
            .shapes
            .iter_mut()
            .find(|s| matches!(s.kind, ShapeKind::Control(_)))
            .expect("legacy VML control loaded");
        let ShapeKind::Control(c) = &mut sh.kind else { unreachable!() };
        let mut next = (**c).clone();
        assert_eq!(next.vml_id, "_x0000_s1043");
        next.name = "sd.size_option".to_string();
        *c = Arc::new(next);
    }
    let vml_saved = vml_loaded.source.save(&vml_loaded.presentation, SaveOptions::default()).unwrap();
    let vml_text = zip_text(&vml_saved, "xl/drawings/vmlDrawing1.vml");
    assert!(
        vml_text.contains(r#"id="sd.size_option" o:spid="_x0000_s1043""#),
        "VML shape should store custom name in id and preserve o:spid:\n{vml_text}"
    );
    let vml_reopened = ibis_pptx::open(vml_saved).unwrap();
    let vml_controls = controls(&vml_reopened.presentation);
    assert_eq!(vml_controls.len(), 1);
    assert_eq!(vml_controls[0].1.name, "sd.size_option");
    assert_eq!(vml_controls[0].1.vml_id, "_x0000_s1043");
}

#[test]
fn synthetic_xlsx_sd_jwt_bindings_roundtrip() {
    use ibis_doc_model::{
        CellEdit, CellInput, CellRange, CellValue, ClaimValue, Command, Document,
        EmuRect, SheetCommand, StyleChange, extract_sheet_claims,
    };

    let opened = ibis_pptx::open(ibis_pptx::blank_file(ibis_pptx::Kind::Workbook).unwrap()).unwrap();
    let slide = opened.presentation.slides[0].id;
    let mut doc = Document::new(opened.presentation.clone());

    let b4_range = CellRange { first_row: 3, first_column: 1, last_row: 3, last_column: 1 };
    let b5_range = CellRange { first_row: 4, first_column: 1, last_row: 4, last_column: 1 };
    let serial_1995_12_01 = ibis_calc::eval::convert::serial_from_date(1995, 12, 1, false) as f64;

    doc.execute(vec![
        Command::Sheet(SheetCommand::SetCells {
            slide,
            edits: vec![
                CellEdit { row: 0, column: 1, input: CellInput::Value(CellValue::Text("urn:eu.europa.ec.eudi:pid:1".into())) },
                CellEdit { row: 1, column: 1, input: CellInput::Value(CellValue::Text("山田".into())) },
                CellEdit { row: 2, column: 1, input: CellInput::Value(CellValue::Text("花子".into())) },
                CellEdit { row: 3, column: 1, input: CellInput::Value(CellValue::Number(serial_1995_12_01)) },
                CellEdit { row: 4, column: 1, input: CellInput::Value(CellValue::Bool(true)) },
                CellEdit { row: 5, column: 1, input: CellInput::Value(CellValue::Number(2.0)) },
                CellEdit { row: 0, column: 4, input: CellInput::Value(CellValue::Text("東京都".into())) },
                CellEdit { row: 1, column: 4, input: CellInput::Value(CellValue::Text("京都府".into())) },
                CellEdit { row: 2, column: 4, input: CellInput::Value(CellValue::Text("福岡県".into())) },
            ],
        }),
        Command::Sheet(SheetCommand::SetFormat {
            slide,
            ranges: vec![b4_range],
            change: StyleChange::NumberFormat("yyyy-mm-dd".into()),
        }),
        Command::Sheet(SheetCommand::SetFormat {
            slide,
            ranges: vec![b5_range],
            change: StyleChange::Checkbox(true),
        }),
        Command::Sheet(SheetCommand::SetName { name: "claim.vct".into(), sheet: None, formula: Some("Sheet1!$B$1".into()) }),
        Command::Sheet(SheetCommand::SetName { name: "sd.family_name".into(), sheet: None, formula: Some("Sheet1!$B$2".into()) }),
        Command::Sheet(SheetCommand::SetName { name: "sd.given_name".into(), sheet: None, formula: Some("Sheet1!$B$3".into()) }),
        Command::Sheet(SheetCommand::SetName { name: "sd.birthdate".into(), sheet: None, formula: Some("Sheet1!$B$4".into()) }),
        Command::Sheet(SheetCommand::SetName { name: "sd.age_over_18".into(), sheet: None, formula: Some("Sheet1!$B$5".into()) }),
    ])
    .unwrap();

    // Add a Drop control (`sd.address.region`) and a CheckBox control (`disclose.given_name`)
    let mut drop = FormControl::new(ControlKind::Drop, "sd.address.region");
    drop.link = Some("$B$6".into());
    drop.range = Some("$E$1:$E$3".into());
    drop.value = 2;
    let mut chk = FormControl::new(ControlKind::CheckBox, "disclose.given_name");
    chk.text = "名を開示".into();
    chk.checked = 0;

    let mut pres = doc.presentation().clone();
    pres.slide_mut(slide).unwrap().shapes.push(Shape::new(
        ibis_doc_model::ShapeId(701),
        EmuRect::new(914_400, 457_200, 1_828_800, 274_320),
        ShapeKind::Control(Arc::new(drop)),
    ));
    pres.slide_mut(slide).unwrap().shapes.push(Shape::new(
        ibis_doc_model::ShapeId(702),
        EmuRect::new(914_400, 914_400, 1_828_800, 274_320),
        ShapeKind::Control(Arc::new(chk)),
    ));

    let saved = opened.source.save(&pres, SaveOptions::default()).unwrap();
    let reopened = ibis_pptx::open(saved).unwrap();
    let bindings = extract_sheet_claims(&reopened.presentation, slide).expect("extract_sheet_claims");

    assert_eq!(bindings.claim("vct").unwrap().value.as_str(), Some("urn:eu.europa.ec.eudi:pid:1"));
    assert!(!bindings.claim("vct").unwrap().selective);
    assert_eq!(bindings.claim("family_name").unwrap().value.as_str(), Some("山田"));
    assert!(bindings.claim("family_name").unwrap().disclose);
    assert_eq!(bindings.claim("given_name").unwrap().value.as_str(), Some("花子"));
    assert!(!bindings.claim("given_name").unwrap().disclose);
    assert_eq!(bindings.claim("birthdate").unwrap().value.as_str(), Some("1995-12-01"));
    assert_eq!(bindings.claim("age_over_18").unwrap().value, ClaimValue::Bool(true));
    assert_eq!(bindings.claim("address.region").unwrap().value.as_str(), Some("京都府"));
}

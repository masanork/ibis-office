// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::{Command, Document, SheetCommand};
use ibis_formats::SaveOptions;

#[test]
fn added_sheet_declares_relationship_namespace_when_siblings_declare_their_own() {
    let data = include_bytes!("fixtures/ods_features.xlsx");
    let mut package = ibis_ooxml::Package::open(data.to_vec()).unwrap();
    let part = "xl/workbook.xml";
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let declaration =
        " xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
    assert!(xml.contains(declaration));
    let xml =
        xml.replacen(declaration, "", 1)
            .replacen("<sheet ", &format!("<sheet{declaration} "), 1);
    package.set_bytes(part, xml.into_bytes());
    let data = package.save(Default::default()).unwrap();

    let opened = ibis_formats::open(data).unwrap();
    let mut document = Document::new(opened.presentation.clone());
    let sheet = document.ids().new_slide_id();
    let shape = document.ids().new_shape_id();
    document
        .execute(vec![Command::Sheet(SheetCommand::AddSheet {
            index: 0,
            name: "New sheet".into(),
            sheet,
            shape,
        })])
        .unwrap();
    let bytes = opened
        .source
        .save(document.presentation(), SaveOptions::default())
        .unwrap();
    let reopened = ibis_formats::read(&bytes).unwrap();
    let names: Vec<_> = reopened
        .slides
        .iter()
        .map(|slide| slide.name.as_deref())
        .collect();
    assert_eq!(names, [Some("New sheet"), Some("Data")]);
}

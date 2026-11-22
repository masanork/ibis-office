// SPDX-License-Identifier: AGPL-3.0-only
use ibis_formats::{form_template, read_docx, student_record};
use serde_json::{Value, json};

const TEMPLATE: &[u8] =
    include_bytes!("../../../test/fixtures/docx-form/elementary-school-template.docx");

fn inputs() -> (Value, Value) {
    (
        serde_json::from_str(include_str!("../../../test/fixtures/docx-form/form.json")).unwrap(),
        serde_json::from_str(include_str!("../../../test/fixtures/docx-form/record.json")).unwrap(),
    )
}

#[test]
fn fills_repeated_names_and_preserves_multiline_value() {
    let counts = form_template::inspect(TEMPLATE).unwrap();
    assert_eq!(counts["pupil.name"], 4);
    let (definition, mut data) = inputs();
    data["data"]["pupil.name"] = json!("青葉 <遥> & 葵");
    let bytes = form_template::fill(TEMPLATE, &definition, &data).unwrap();
    let flow = read_docx(&bytes).unwrap().flow.unwrap();
    let extracted = student_record::extract(&flow).unwrap();
    assert_eq!(extracted.name.text, "青葉 <遥> & 葵");
    assert_eq!(extracted.birth_date.text, "平成31年4月2日");
    assert_eq!(
        extracted.next_school.text,
        "架空市立そよかぜ中学校\n架空市ひなた町１丁目"
    );
    let repetitions = flow.body.iter().filter(|block| {
        matches!(block, ibis_doc_model::Block::Paragraph(p)
            if p.runs.iter().any(|r| matches!(&r.kind, ibis_doc_model::FlowRunKind::Text(s) if s.contains("青葉 <遥> & 葵"))))
    }).count();
    assert_eq!(repetitions, 3);
}

#[test]
fn rejects_wrong_version_invalid_date_and_missing_field() {
    let (definition, mut data) = inputs();
    data["form"]["version"] = json!(2);
    assert!(form_template::fill(TEMPLATE, &definition, &data).is_err());
    data["form"]["version"] = json!(1);
    data["data"]["pupil.birthDate"] = json!("2019-02-29");
    assert!(form_template::fill(TEMPLATE, &definition, &data).is_err());
    data["data"]
        .as_object_mut()
        .unwrap()
        .remove("pupil.birthDate");
    assert!(form_template::fill(TEMPLATE, &definition, &data).is_err());
}

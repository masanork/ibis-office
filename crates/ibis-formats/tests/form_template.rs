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

#[test]
fn generates_two_complete_school_records_with_the_same_form() {
    const TEMPLATE: &[u8] =
        include_bytes!("../../../test/fixtures/docx-form/full-school-record/template.docx");
    let definition: Value = serde_json::from_str(include_str!(
        "../../../test/fixtures/docx-form/full-school-record/form.json"
    ))
    .unwrap();
    let sources = [
        (
            include_str!("../../../test/fixtures/docx-form/full-school-record/record-a.json"),
            "紅葉 葵",
            "架空市立そよかぜ中学校",
            "班の意見を整理して",
        ),
        (
            include_str!("../../../test/fixtures/docx-form/full-school-record/record-b.json"),
            "青葉 凪",
            "架空市立みずき中学校",
            "図書委員会で貸出の分担を",
        ),
    ];
    assert_eq!(form_template::inspect(TEMPLATE).unwrap().len(), 372);
    for (source, name, school, remarks) in sources {
        let data = serde_json::from_str(source).unwrap();
        let generated = form_template::fill(TEMPLATE, &definition, &data).unwrap();
        let flow = read_docx(&generated).unwrap().flow.unwrap();
        let record = student_record::extract(&flow).unwrap();
        assert_eq!(record.name.text, name);
        assert!(record.next_school.text.starts_with(school));
        let all = ibis_doc_model::flow_command::all_paragraphs(&flow);
        assert!(all.iter().any(|p| {
            p.runs.iter().any(|r|
            matches!(&r.kind, ibis_doc_model::FlowRunKind::Text(value) if value.contains(remarks))
        )
        }));
    }
}

#[test]
fn rejects_a_flat_key_that_duplicates_a_nested_value() {
    let (definition, mut data) = inputs();
    data["data"]["pupil"] = json!({"name": "duplicate"});
    assert!(form_template::fill(TEMPLATE, &definition, &data).is_err());
}

#[test]
fn yaml_matches_json_and_limits_visible_graphemes_in_both_outputs() {
    let (mut definition, mut data) = inputs();
    definition["fields"]["pupil.name"] = json!({"type": "text", "maxChars": 2});
    data["data"]["pupil.name"] = json!("か\u{3099}嵜\u{e0100}余分");
    let result = form_template::fill(TEMPLATE, &definition, &data).unwrap();
    let flow = read_docx(&result).unwrap().flow.unwrap();
    assert_eq!(
        student_record::extract(&flow).unwrap().name.text,
        "か\u{3099}嵜\u{e0100}"
    );
    assert_eq!(
        form_template::fill_html("<p>{{pupil.name}}</p>", &definition, &data).unwrap(),
        "<p>か\u{3099}嵜\u{e0100}</p>"
    );
    assert_eq!(data["data"]["pupil.name"], "か\u{3099}嵜\u{e0100}余分");

    let yaml = include_str!("../../../test/fixtures/docx-form/full-school-record/record-b.yaml");
    let json = include_str!("../../../test/fixtures/docx-form/full-school-record/record-b.json");
    assert_eq!(
        form_template::parse_input(yaml, true).unwrap(),
        form_template::parse_input(json, false).unwrap()
    );
    assert!(form_template::parse_input("name: one\nname: two", true).is_err());
}

#[test]
fn html_escapes_inserted_text_and_rejects_unknown_markers() {
    let (definition, mut data) = inputs();
    data["data"]["pupil.name"] = json!("海 <川> & 山");
    assert_eq!(
        form_template::fill_html("<p>{{pupil.name}}</p>", &definition, &data).unwrap(),
        "<p>海 &lt;川&gt; &amp; 山</p>"
    );
    assert!(form_template::fill_html("<p>{{unknown}}</p>", &definition, &data).is_err());
}

#[test]
fn preflight_reports_limits_for_used_fields_without_exposing_values() {
    let (mut definition, mut data) = inputs();
    definition["fields"]["pupil.name"] = json!({"type": "text", "maxChars": 2});
    definition["fields"]["nextSchool"] = json!({"type": "text", "maxChars": 2});
    data["data"]["pupil.name"] = json!("嵜\u{e0100}か\u{3099}余分");
    data["data"]["nextSchool"] = json!("山中学校");
    let (html, diagnostics) = form_template::fill_html_with_diagnostics(
        "<p>{{pupil.name}}</p><p>{{pupil.name}}</p>",
        &definition,
        &data,
    )
    .unwrap();
    assert_eq!(
        html,
        "<p>嵜\u{e0100}か\u{3099}</p><p>嵜\u{e0100}か\u{3099}</p>"
    );
    assert_eq!(
        diagnostics,
        [form_template::Truncation {
            field_id: "pupil.name".into(),
            actual_chars: 4,
            max_chars: 2,
        }]
    );
    let (_, diagnostics) =
        form_template::fill_with_diagnostics(TEMPLATE, &definition, &data).unwrap();
    assert_eq!(diagnostics.len(), 2);
}

#[test]
fn municipal_notice_fixture_reports_only_the_overlong_note() {
    const LAYOUT: &str =
        include_str!("../../../test/fixtures/docx-form/enrollment-notice/layout.html");
    let definition = form_template::parse_input(
        include_str!("../../../test/fixtures/docx-form/enrollment-notice/form.yaml"),
        true,
    )
    .unwrap();
    let data = form_template::parse_input(
        include_str!("../../../test/fixtures/docx-form/enrollment-notice/record.yaml"),
        true,
    )
    .unwrap();
    let (html, diagnostics) =
        form_template::fill_html_with_diagnostics(LAYOUT, &definition, &data).unwrap();
    assert!(html.contains("架空市立みずき中学校"));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].field_id, "notice.note");
    assert_eq!(diagnostics[0].max_chars, 40);
    assert!(!html.contains("後日学校からお知らせします。"));
}

#[test]
fn tagged_fields_map_to_their_first_body_occurrence() {
    let flow = read_docx(TEMPLATE).unwrap().flow.unwrap();
    let positions = form_template::field_positions(&flow);
    let at = positions["pupil.name"];
    let paragraph = ibis_doc_model::flow_command::paragraph(&flow, at.para).unwrap();
    let mut offset = 0;
    let found = paragraph
        .runs
        .iter()
        .find(|run| {
            if run
                .control
                .as_ref()
                .is_some_and(|c| c.tag == "ibis.form.pupil.name")
            {
                return true;
            }
            offset += run.len();
            false
        })
        .unwrap();
    assert_eq!(at.offset, offset);
    assert!(found.control.is_some());
    assert_eq!(
        positions.len(),
        form_template::inspect(TEMPLATE).unwrap().len()
    );
}

#[test]
fn saved_template_snapshot_reports_the_warning_at_the_sixth_grade_remarks() {
    const DOCX: &[u8] =
        include_bytes!("../../../test/fixtures/docx-form/full-school-record/template.docx");
    let opened = ibis_formats::open(DOCX).unwrap();
    let template = opened
        .source
        .save(&opened.presentation, ibis_formats::SaveOptions::default())
        .unwrap();
    assert_eq!(template, DOCX);
    let definition = form_template::parse_input(
        include_str!("../../../test/fixtures/docx-form/full-school-record/form.yaml"),
        true,
    )
    .unwrap();
    let mut data = form_template::parse_input(
        include_str!("../../../test/fixtures/docx-form/full-school-record/record-b.yaml"),
        true,
    )
    .unwrap();
    data["data"]["grades"]["6"]["remarks"] = json!("図書委員会の活動を振り返った。".repeat(15));
    let (_, warnings) =
        form_template::fill_with_diagnostics(&template, &definition, &data).unwrap();
    assert_eq!(
        warnings,
        [form_template::Truncation {
            field_id: "grades.6.remarks".into(),
            actual_chars: 225,
            max_chars: 160,
        }]
    );
    let flow = opened.presentation.flow.unwrap();
    let target = form_template::field_positions(&flow)[&warnings[0].field_id];
    let paragraph = ibis_doc_model::flow_command::paragraph(&flow, target.para).unwrap();
    assert!(paragraph.runs.iter().any(|run| {
        run.control
            .as_ref()
            .is_some_and(|c| c.tag == "ibis.form.grades.6.remarks")
    }));
}

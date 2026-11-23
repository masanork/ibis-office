// SPDX-License-Identifier: AGPL-3.0-only
use ibis_formats::lawtext::{input_to_markdown, parse_law_xml, parse_lawtext, to_lawtext};

#[test]
fn egov_constitution_opens_as_markdown_and_lawtext() {
    let xml = include_str!("lawtext/321CONSTITUTION.xml");
    let law = parse_law_xml(xml).unwrap();
    assert_eq!(law.law_title, "日本国憲法");
    let markdown = input_to_markdown(xml.as_bytes()).unwrap().unwrap();
    assert!(markdown.contains("われらとわれらの子孫のために"));
    let lawtext = to_lawtext(&law);
    let reparsed = parse_lawtext(&lawtext).unwrap();
    assert_eq!(reparsed.law_title, law.law_title);
    assert_eq!(reparsed.main_provision, law.main_provision);
}

#[test]
fn upstream_lawtext_sample_keeps_the_split_title_and_both_paragraphs() {
    let source = include_str!("lawtext/administrative-procedure-act.lawtext");
    let markdown = input_to_markdown(source.as_bytes()).unwrap().unwrap();
    assert!(markdown.contains("# 行政手続法"));
    assert!(markdown.contains("法令番号: 平成五年法律第八十八号"));
    assert!(markdown.contains("第一条 （目的等）"));
    assert!(markdown.contains("他の法律に特別の定めがある場合"));
}

#[test]
fn verifies_amendment_after_xml_and_lawtext_round_trips() {
    use std::{fs, process::Command};

    use ibis_formats::lawtext::{parse_markdown_law, to_law_xml, to_markdown};
    use serde_json::Value;

    const OLD_XML: &str = include_str!("lawtext/supplementary-amendment.xml");
    const AMENDMENT: &str = "令和八年法律第三号";
    const HISTORICAL: &str = "令和七年法律第二号";
    const BILL_SUPPLEMENT: &str = r#"
    <SupplProvision AmendLawNum="令和八年法律第三号">
      <SupplProvisionLabel>附　則</SupplProvisionLabel>
      <Article Num="1">
        <ArticleTitle>第一条</ArticleTitle>
        <Paragraph Num="1">
          <ParagraphNum/>
          <ParagraphSentence><Sentence>公布の日から施行する。</Sentence></ParagraphSentence>
        </Paragraph>
      </Article>
    </SupplProvision>
"#;

    let fixture = tempfile::tempdir().unwrap();
    let historical_block = format!(r#"<SupplProvision AmendLawNum="{HISTORICAL}""#);
    let new_xml = OLD_XML.replace("旧基準", "新基準").replace(
        &historical_block,
        &format!("{BILL_SUPPLEMENT}{historical_block}"),
    );
    for (name, xml) in [("old", OLD_XML), ("new", new_xml.as_str())] {
        let original = parse_law_xml(xml).unwrap();
        let exported_xml = to_law_xml(&original);
        assert!(exported_xml.contains("<Ruby>例<Rt>れい</Rt></Ruby>外"));
        assert!(exported_xml.contains("<TOCArticle Num=\"1\">"));
        assert!(exported_xml.contains("<TOCSupplProvision>"));
        let lawtext = to_lawtext(&original);
        let roundtrip = parse_law_xml(&to_law_xml(&parse_lawtext(&lawtext).unwrap())).unwrap();
        assert_eq!(roundtrip, original, "{name} round trip");
        assert_eq!(
            to_law_xml(&roundtrip),
            to_law_xml(&original),
            "{name} XML structure"
        );
        let markdown = to_markdown(&original);
        let markdown_roundtrip = parse_markdown_law(&markdown).unwrap();
        assert_eq!(
            to_law_xml(&markdown_roundtrip),
            to_law_xml(&original),
            "{name} Markdown structure"
        );
        fs::write(fixture.path().join(format!("{name}.md")), markdown).unwrap();
        fs::write(fixture.path().join(format!("{name}.lawtext")), lawtext).unwrap();
        fs::write(fixture.path().join(format!("{name}.xml")), xml).unwrap();
    }

    let run = |extension: &str, amendment: Option<&str>| -> Value {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ibis-lawtext-5set"));
        command
            .current_dir(fixture.path())
            .args([format!("old.{extension}"), format!("new.{extension}")])
            .args(["--json", "--require-match"]);
        if let Some(number) = amendment {
            command.args(["--amendment-law-num", number]);
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };

    let xml = run("xml", Some(AMENDMENT));
    assert_eq!(xml, run("lawtext", Some(AMENDMENT)));
    assert_eq!(xml, run("md", Some(AMENDMENT)));
    let changes = xml["amendment_ir"]["suppl_changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["amend_law_num"], AMENDMENT);
    let comparison = xml["documents"]["shinkyu"].as_str().unwrap();
    assert!(comparison.contains("公布の日から施行する。"));
    assert!(!comparison.contains("経過措置を定める。"));
    let verification = &xml["deterministic_outputs"]["aratame_verification"];
    assert_eq!(verification["applied_ops"], 1);
    assert_eq!(verification["is_exact_match"], false);
    assert_eq!(verification["bill_supplement_law_num"], AMENDMENT);
    assert_eq!(verification["diagnostics"], serde_json::json!([]));
    assert_eq!(
        verification["residual_diff"]["suppl_changes"],
        serde_json::json!([])
    );

    for amendment in [None, Some(HISTORICAL)] {
        let unclassified = run("xml", amendment);
        let verification = &unclassified["deterministic_outputs"]["aratame_verification"];
        assert!(verification["bill_supplement_law_num"].is_null());
        assert_eq!(
            verification["residual_diff"]["suppl_changes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            verification["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| {
                    diagnostic
                        .as_str()
                        .unwrap()
                        .contains("附則に逆適用後の差分")
                })
        );
    }
}

#[test]
fn audit_reports_xml_model_changes_and_preserves_required_lawtext_attributes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.xml");
    let xml = include_str!("lawtext/supplementary-amendment.xml").replace(
        "<TOC>",
        "<EnactStatement>試験法を公布する。</EnactStatement><TOC>",
    );
    let xml = xml.replace("<LawBody>", "<LawBody Subject=\"試験法の件名\">");
    std::fs::write(&path, xml).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ibis-lawtext-audit"))
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let model = report["xml_model_changes"].as_array().unwrap();
    let lawtext = report["lawtext_roundtrip_changes"].as_array().unwrap();
    assert!(model.iter().any(|change| {
        change["kind"] == "attribute_removed" && change["path"] == "/Law/LawBody[1]/@Subject"
    }));
    for path in ["/Law/@PromulgateMonth", "/Law/@PromulgateDay"] {
        assert!(model.iter().all(|change| change["path"] != path));
    }
    assert_eq!(
        lawtext,
        &[serde_json::json!({
            "kind": "element_removed",
            "path": "/Law/LawBody[1]/EnactStatement[1]",
        })],
    );
    assert!(report["output_xsd"].is_null());
}

#[test]
fn reads_combined_lawtext_root_attributes_before_a_split_title() {
    let source = concat!(
        "[Era=\"Reiwa\"][Year=\"07\"] [Num=\"001\"]\r\n",
        "[PromulgateMonth=\"06\"][PromulgateDay=\"09\"]\r\n",
        "[LawType=\"Act\"][Lang=\"en\"]\r\n\r\n",
        "試験法\r\n（令和七年法律第一号）\r\n\r\n第一条 規定を定める。",
    );
    assert!(ibis_formats::lawtext::is_lawtext(source.as_bytes()));
    let doc = parse_lawtext(source).unwrap();
    assert_eq!(doc.law_title, "試験法");
    assert_eq!(doc.law_num, "令和七年法律第一号");
    assert_eq!(doc.era.as_deref(), Some("Reiwa"));
    assert_eq!(doc.year.as_deref(), Some("07"));
    assert_eq!(doc.promulgate_month.as_deref(), Some("06"));
    assert_eq!(doc.promulgate_day.as_deref(), Some("09"));
    assert_eq!(doc.num.as_deref(), Some("001"));
    assert_eq!(doc.law_type.as_deref(), Some("Act"));
    assert_eq!(doc.lang.as_deref(), Some("en"));
    assert_eq!(doc, parse_lawtext(&to_lawtext(&doc)).unwrap());
    let markdown = input_to_markdown(source.as_bytes()).unwrap().unwrap();
    assert_eq!(
        doc,
        ibis_formats::lawtext::parse_markdown_law(&markdown).unwrap()
    );
}

#[test]
fn markdown_keeps_visible_text_and_discards_stale_sentence_metadata() {
    use ibis_formats::lawtext::{parse_markdown_law, to_law_xml, to_markdown};

    let source = include_str!("lawtext/supplementary-amendment.xml");
    let markdown = to_markdown(&parse_law_xml(source).unwrap());
    let plain = "旧基準を適用する。ただし、例外を認める。";
    assert!(markdown.contains(&format!("{plain} <!-- ibis-law-sentences ")));
    let presentation = ibis_formats::markdown::read_markdown(markdown.as_bytes()).unwrap();
    let rendered = ibis_formats::markdown::export_presentation_to_markdown(&presentation).unwrap();
    assert!(rendered.contains(plain));
    assert!(!rendered.contains("ibis-law-sentences"));
    let edited = markdown.replacen(plain, "新基準を適用する。", 1);
    let xml = to_law_xml(&parse_markdown_law(&edited).unwrap());
    assert!(
        xml.contains("<ParagraphCaption CommonCaption=\"true\">（適用基準）</ParagraphCaption>")
    );
    assert!(xml.contains("<ParagraphNum>１</ParagraphNum>"));
    assert!(xml.contains("<ParagraphSentence><Sentence>新基準を適用する。</Sentence>"));
    assert!(!xml.contains("ただし、例外を認める。"));
    assert!(xml.contains("Function=\"proviso\">ただし、除外例を設ける。"));
    let edited_item = markdown.replacen(
        "特例を定める。ただし、除外例を設ける。",
        "号を編集する。",
        1,
    );
    let item_xml = to_law_xml(&parse_markdown_law(&edited_item).unwrap());
    assert!(item_xml.contains("<ItemSentence><Sentence>号を編集する。</Sentence>"));
    assert!(!item_xml.contains("ただし、除外例を設ける。"));
    assert!(!xml.contains("ibis-law-sentences"));
    let columns = "給付の種類 特例給付及び通常給付";
    assert!(markdown.contains(&format!("{columns} <!-- ibis-law-sentences ")));
    let edited_columns = markdown.replacen(columns, "給付の区分を編集する。", 1);
    let columns_xml = to_law_xml(&parse_markdown_law(&edited_columns).unwrap());
    assert!(columns_xml.contains("<ItemSentence><Sentence>給付の区分を編集する。</Sentence>"));
    assert!(!columns_xml.contains("<Column"));
    let ai = ibis_formats::lawtext::export_ai_law(source.as_bytes())
        .unwrap()
        .unwrap();
    assert!(
        !ai["markdown"]
            .as_str()
            .unwrap()
            .contains("ibis-law-sentences")
    );
}

#[test]
fn lawtext_title_metadata_keeps_empty_attributes_and_plain_title() {
    let source = concat!(
        ":law-title-attrs: <LawTitle Kana=\"しけんほう\" Abbrev=\"\" AbbrevKana=\"よみ&#10;かた\"/>\n",
        "試験法（令和七年法律第一号）\n\n第一条 規定を定める。",
    );
    let law = ibis_formats::lawtext::parse_law_input(source).unwrap();
    assert!(ibis_formats::lawtext::is_lawtext(source.as_bytes()));
    assert_eq!(law.law_title, "試験法");
    assert_eq!(law.law_title_attributes.abbrev.as_deref(), Some(""));
    assert_eq!(
        law.law_title_attributes.abbrev_kana.as_deref(),
        Some("よみ\nかた")
    );
    assert_eq!(law, parse_lawtext(&to_lawtext(&law)).unwrap());
    let markdown = input_to_markdown(source.as_bytes()).unwrap().unwrap();
    assert!(markdown.contains("# 試験法\n"));
    assert_eq!(
        law,
        ibis_formats::lawtext::parse_markdown_law(&markdown).unwrap()
    );
}

#[test]
fn public_food_labeling_samples_expose_table_and_form_limits() {
    use ibis_formats::lawtext::{parse_markdown_law, to_law_xml, to_markdown};

    let table = include_str!("lawtext/food-labeling-table.xml");
    let form = include_str!("lawtext/food-inspection-form.xml");
    assert!(table.contains("rowspan=\"2\""));
    assert!(form.contains("<AppdxStyle>") && form.contains("<Fig src="));

    for (xml, title) in [
        (table, "食品表示基準"),
        (
            form,
            "食品表示法第六条第三項の内閣府令・財務省令で定める表示事項及び遵守事項等を定める命令",
        ),
    ] {
        let original = parse_law_xml(xml).unwrap();
        let lawtext = parse_lawtext(&to_lawtext(&original)).unwrap();
        let markdown = parse_markdown_law(&to_markdown(&original)).unwrap();
        assert_eq!(lawtext.appdx_tables, original.appdx_tables);
        for edited in [lawtext, markdown] {
            assert_eq!(edited.law_title, title);
            assert_eq!(edited.law_num, original.law_num);
            assert_eq!(edited.main_provision, original.main_provision);
            assert_eq!(edited.appdx_tables.len(), original.appdx_tables.len());
            for (actual, expected) in edited.appdx_tables.iter().zip(&original.appdx_tables) {
                assert_eq!(actual.title, expected.title);
                assert_eq!(actual.related_article_num, expected.related_article_num);
                assert_eq!(actual.rows.len(), expected.rows.len());
                for (actual_row, expected_row) in actual.rows.iter().zip(&expected.rows) {
                    assert!(actual_row.starts_with(expected_row));
                    assert!(
                        actual_row[expected_row.len()..]
                            .iter()
                            .all(String::is_empty)
                    );
                }
            }
            assert_eq!(
                parse_law_xml(&to_law_xml(&edited)).unwrap().law_title,
                title
            );
        }
    }

    let table_output = to_law_xml(&parse_law_xml(table).unwrap());
    assert!(table_output.contains("別表第十八"));
    assert!(!table_output.contains("rowspan="));
    let form_output = to_law_xml(&parse_law_xml(form).unwrap());
    assert!(!form_output.contains("<AppdxStyle>"));
}

#[test]
fn audit_reports_reordered_siblings_without_counting_removals_as_reordering() {
    let fixture = include_str!("lawtext/supplementary-amendment.xml");
    let title_start = fixture.find("<LawTitle ").unwrap();
    let toc_start = fixture.find("<TOC>").unwrap();
    let after_toc = fixture.find("</TOC>").unwrap() + "</TOC>".len();
    let xml = format!(
        "{}{}\n{}{}",
        &fixture[..title_start],
        &fixture[toc_start..after_toc],
        &fixture[title_start..toc_start],
        &fixture[after_toc..]
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("order.xml");
    std::fs::write(&path, xml).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ibis-lawtext-audit"))
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let changes = report["xml_model_changes"].as_array().unwrap();
    let [change] = changes.as_slice() else {
        panic!("expected one difference, got {changes:?}")
    };
    assert_eq!(change["kind"], "child_order_changed");
    assert_eq!(change["path"], "/Law/LawBody[1]");
    assert_eq!(change["before"][0], "TOC[1]");
    assert_eq!(change["after"][0], "LawTitle[1]");
    assert_eq!(report["lawtext_roundtrip_changes"], serde_json::json!([]));
}

#[test]
fn generates_and_reapplies_the_two_food_labeling_tables_added_in_2024() {
    use ibis_doc_model::flow::Block;
    use ibis_formats::lawtext::{ShinkyuOptions, build_5set_package};

    let old = parse_law_xml(include_str!("lawtext/food-labeling-2024-before.xml")).unwrap();
    let new = parse_law_xml(include_str!("lawtext/food-labeling-2024-after.xml")).unwrap();
    let package = build_5set_package(&old, &new, "", &ShinkyuOptions::default());
    let clauses = &package.aratame_draft.clauses;
    assert_eq!(clauses.len(), 2);
    for (clause, anchor, title) in [
        (&clauses[0], "別表第二十五", "別表第二十六"),
        (&clauses[1], "別表第二十六", "別表第二十七"),
    ] {
        assert_eq!(clause.target, title);
        assert!(clause.text.starts_with(&format!(
            "{anchor}の次に次の別表を加える。\n{title}\n（第二条関係）"
        )));
        assert!(clause.text.contains("| --- | --- |"));
    }
    let verification = &package.aratame_verification;
    assert!(
        verification.is_exact_match,
        "{:?}",
        verification.diagnostics
    );
    assert_eq!(verification.applied_ops, 2);
    assert_eq!(verification.simulated_new.appdx_tables, new.appdx_tables);

    let docx = ibis_formats::open(package.to_print_docx().unwrap()).unwrap();
    let flow = docx.presentation.flow.unwrap();
    let rows: Vec<_> = flow
        .body
        .iter()
        .filter_map(|block| {
            if let Block::Table(table) = block {
                Some(table.rows.len())
            } else {
                None
            }
        })
        .take(2)
        .collect();
    assert_eq!(rows, [2, 4]);
}

#[test]
fn reapplies_nested_quotation_in_the_2024_food_labeling_table_change() {
    use ibis_formats::lawtext::{generate_aratame_draft, verify_aratame};

    let old = parse_law_xml(include_str!("lawtext/table-twenty-before.xml")).unwrap();
    let new = parse_law_xml(include_str!("lawtext/table-twenty-after.xml")).unwrap();
    let draft = generate_aratame_draft(&old, &new);
    assert_eq!(draft.clauses.len(), 1);
    assert_eq!(draft.clauses[0].target, "別表第二十");
    assert!(
        draft
            .full_text
            .contains("「機能性表示食品」の文字を枠で囲んで表示する")
    );
    let verified = verify_aratame(&old, &draft.full_text, &new);
    assert!(verified.is_exact_match, "{:?}", verified.diagnostics);
    assert_eq!(verified.applied_ops, 1);
    assert_eq!(verified.simulated_new.appdx_tables, new.appdx_tables);
}

#[test]
fn reapplies_new_subitems_in_the_2024_food_labeling_amendment() {
    use ibis_formats::lawtext::{generate_aratame_draft, verify_aratame};

    let old = parse_law_xml(include_str!("lawtext/food-subitems-2024-before.xml")).unwrap();
    let new = parse_law_xml(include_str!("lawtext/food-subitems-2024-after.xml")).unwrap();
    let draft = generate_aratame_draft(&old, &new);
    assert!(draft.full_text.contains("第二条第十号に次のように加える。"));
    assert!(draft.full_text.contains("    （５）"));
    assert!(
        draft
            .full_text
            .contains("第二十二条第四号に次のように加える。")
    );
    let verified = verify_aratame(&old, &draft.full_text, &new);
    assert!(verified.is_exact_match, "{:?}", verified.diagnostics);
    assert_eq!(verified.simulated_new.main_provision, new.main_provision);
}

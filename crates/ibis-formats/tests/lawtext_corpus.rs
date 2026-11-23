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
    assert!(lawtext.is_empty(), "{lawtext:?}");
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
fn public_food_labeling_samples_preserve_table_and_form_markup() {
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
        assert!(original.enact_statement.is_some());
        let serialized = to_lawtext(&original);
        assert!(ibis_formats::lawtext::is_lawtext(serialized.as_bytes()));
        let lawtext = parse_lawtext(&serialized).unwrap();
        let markdown = parse_markdown_law(&to_markdown(&original)).unwrap();
        assert_eq!(lawtext.appdx_tables, original.appdx_tables);
        for edited in [lawtext, markdown] {
            assert_eq!(edited.law_title, title);
            assert_eq!(edited.law_num, original.law_num);
            assert_eq!(edited.main_provision, original.main_provision);
            assert_eq!(edited.appdx_tables, original.appdx_tables);
            if !original.appdx_tables.is_empty() {
                assert_eq!(
                    to_law_xml(&edited).split_once("<AppdxTable>").unwrap().1,
                    to_law_xml(&original).split_once("<AppdxTable>").unwrap().1
                );
            }
            let output = to_law_xml(&edited);
            if let Some((_, fragment)) = xml.split_once("<AppdxStyle>") {
                let expected = fragment.split_once("</AppdxStyle>").unwrap().0;
                let actual = output.split_once("<AppdxStyle>").unwrap().1;
                assert_eq!(actual.split_once("</AppdxStyle>").unwrap().0, expected);
            }
            let restored = parse_law_xml(&output).unwrap();
            assert_eq!(restored.law_title, title);
            assert_eq!(restored.enact_statement, original.enact_statement);
        }
    }

    let table_output = to_law_xml(&parse_law_xml(table).unwrap());
    assert!(table_output.contains("別表第十八"));
    assert!(table_output.contains("rowspan=\"2\""));
    assert!(table_output.contains("BorderBottom=\"solid\""));
    let source = parse_law_xml(table).unwrap();
    let expected = table_output.replace(">形質</Sentence>", ">形質区分</Sentence>");
    let lawtext = to_lawtext(&source).replace("| 形質 |", "| 形質区分 |");
    let markdown = to_markdown(&source).replace("| 形質 |", "| 形質区分 |");
    let mut rust_edit = source.clone();
    let cell = rust_edit.appdx_tables[0].rows.iter_mut().flatten()
        .find(|cell| cell.as_str() == "形質").unwrap();
    *cell = "形質区分".to_owned();
    for edited in [
        rust_edit,
        parse_lawtext(&lawtext).unwrap(),
        parse_markdown_law(&markdown).unwrap(),
    ] {
        assert_eq!(to_law_xml(&edited), expected);
        assert_eq!(to_law_xml(&parse_lawtext(&to_lawtext(&edited)).unwrap()), expected);
    }

    let changed_shape = lawtext.replace("| 形質区分 |", "| 形質区分 | 追加セル |");
    let output = to_law_xml(&parse_lawtext(&changed_shape).unwrap());
    assert!(output.contains("追加セル"));
    assert!(!output.contains("rowspan="));
}

#[test]
fn appendix_style_metadata_keeps_multiple_forms_and_rejects_invalid_fragments() {
    use ibis_formats::lawtext::{parse_markdown_law, to_law_xml};

    let first = "<AppdxStyle><AppdxStyleTitle>様式一</AppdxStyleTitle><FigStruct><Fig src=\"./pict/a.pdf\"/></FigStruct></AppdxStyle>";
    let second = "<AppdxStyle><AppdxStyleTitle>様式二</AppdxStyleTitle><FigStruct><Fig src=\"./pict/b.pdf\"/></FigStruct></AppdxStyle>";
    let metadata = serde_json::to_string(&[first, "<AppdxStyle>", "<MainProvision/>", second]).unwrap();
    let lawtext = format!(":law-appdx-styles: {metadata}\n:law-appdx-style-positions: [0,0]\n\n試験法（令和七年法律第一号）\n\n第一条 規定する。");
    let markdown = format!("---\ntype: lawtext\nlaw_appdx_styles: {metadata}\nlaw_appdx_style_positions: [0,0]\n---\n\n# 試験法\n\n## 第一条 規定する。");
    for doc in [parse_lawtext(&lawtext).unwrap(), parse_markdown_law(&markdown).unwrap()] {
        let xml = to_law_xml(&doc);
        assert_eq!(xml.matches("<AppdxStyle>").count(), 2);
        assert!(xml.find(first).unwrap() < xml.find(second).unwrap());
        assert!(!xml.contains("<MainProvision/>"));
        assert!(parse_law_xml(&xml).is_ok());
    }
}

#[test]
fn mixed_appendices_keep_their_order_even_when_a_table_cell_is_edited() {
    use ibis_formats::lawtext::{parse_markdown_law, to_law_xml, to_markdown};

    let source = include_str!("lawtext/food-labeling-table.xml");
    let (before, appendix) = source.split_once("<AppdxTable>").unwrap();
    let (table, after) = appendix.split_once("</AppdxTable>").unwrap();
    let form = include_str!("lawtext/food-inspection-form.xml");
    let form = form.split_once("<AppdxStyle>").unwrap().1;
    let form = form.split_once("</AppdxStyle>").unwrap().0;
    let figure = r#"<AppdxFig Num="1"><AppdxFigTitle>図一</AppdxFigTitle><FigStruct><Fig src="./pict/figure.pdf"/></FigStruct></AppdxFig>"#;
    let note = r#"<AppdxNote Num="2"><AppdxNoteTitle>注一</AppdxNoteTitle><FigStruct><Fig src="./pict/note.pdf"/></FigStruct></AppdxNote>"#;
    let format = r#"<AppdxFormat Num="3"><AppdxFormatTitle WritingMode="horizontal">書式一</AppdxFormatTitle></AppdxFormat>"#;
    let formula = "<Appdx><ArithFormulaNum>算式一</ArithFormulaNum><ArithFormula/></Appdx>";
    let supplement = r#"<SupplProvision AmendLawNum="令和七年内閣府令第一号"><SupplProvisionLabel>附則</SupplProvisionLabel><Paragraph Num="1"><ParagraphNum/><ParagraphSentence><Sentence>公布の日から施行する。</Sentence></ParagraphSentence></Paragraph></SupplProvision>"#;
    let xml = format!(
        "{before}{figure}<AppdxTable>{table}</AppdxTable>{supplement}{note}{format}\n<AppdxStyle>{form}</AppdxStyle><AppdxTable>{}</AppdxTable>{formula}{after}",
        table.replace("別表第十八", "別表第十九"),
    );
    let source = parse_law_xml(&xml).unwrap();
    let lawtext = to_lawtext(&source);
    assert!(ibis_formats::lawtext::is_lawtext(lawtext.as_bytes()));
    let markdown = to_markdown(&source).replacen("| 形質 |", "| 形質区分 |", 1);
    for (roundtrip, expected_cell) in [
        (source, "形質"),
        (parse_lawtext(&lawtext).unwrap(), "形質"),
        (parse_markdown_law(&markdown).unwrap(), "形質区分"),
    ] {
        let output = to_law_xml(&roundtrip);
        let xml = roxmltree::Document::parse(&output).unwrap();
        let body = xml.root_element().children().find(|node| node.has_tag_name("LawBody")).unwrap();
        let entries: Vec<_> = body.children()
            .filter(roxmltree::Node::is_element)
            .map(|node| node.tag_name().name())
            .filter(|tag| *tag == "SupplProvision" || tag.starts_with("Appdx"))
            .collect();
        assert!(entries.windows(4).any(|items| items == ["AppdxFig", "AppdxTable", "SupplProvision", "AppdxNote"]));
        let appendices: Vec<_> = body.children()
            .filter(|node| node.is_element() && node.tag_name().name().starts_with("Appdx"))
            .collect();
        let tags: Vec<_> = appendices.iter().map(|node| node.tag_name().name()).collect();
        assert_eq!(tags, ["AppdxFig", "AppdxTable", "AppdxNote", "AppdxFormat", "AppdxStyle", "AppdxTable", "Appdx"]);
        for fragment in [figure, note, format, formula] {
            assert!(output.contains(fragment));
        }
        assert!(appendices[1].descendants().any(|node| node.is_text() && node.text() == Some(expected_cell)));
        assert!(appendices[5].descendants().any(|node| node.text() == Some("別表第十九")));
    }
}

#[test]
fn downstream_code_can_construct_a_supplement_with_a_struct_literal() {
    use ibis_formats::lawtext::{LawDoc, SupplProvision, to_law_xml};

    let mut doc = LawDoc::default();
    doc.suppl_provisions.push(SupplProvision {
        amend_law_num: Some("令和七年法律第二号".to_string()),
        extract: true,
        label: "附則".to_string(),
        nodes: Vec::new(),
    });
    assert!(to_law_xml(&doc).contains(
        "<SupplProvision AmendLawNum=\"令和七年法律第二号\" Extract=\"true\">",
    ));
}

#[test]
fn supplementary_appendices_roundtrip_under_their_own_headings_in_order() {
    use ibis_formats::lawtext::{parse_markdown_law, to_law_xml, to_markdown};

    let table = concat!(
        "<SupplProvisionAppdxTable Num=\"1\"><SupplProvisionAppdxTableTitle>附則別表</SupplProvisionAppdxTableTitle>",
        "<TableStruct><Table><TableRow><TableColumn rowspan=\"2\"><Sentence>甲</Sentence></TableColumn>",
        "<TableColumn><Sentence>乙</Sentence></TableColumn></TableRow>",
        "<TableRow><TableColumn><Sentence>丙</Sentence></TableColumn></TableRow></Table></TableStruct>",
        "</SupplProvisionAppdxTable>",
    );
    let style = concat!(
        "<SupplProvisionAppdxStyle><SupplProvisionAppdxStyleTitle>附則様式</SupplProvisionAppdxStyleTitle>",
        "<StyleStruct><Style><Fig src=\"./pict/sample--form.pdf\"/></Style></StyleStruct></SupplProvisionAppdxStyle>",
    );
    let formula = "<SupplProvisionAppdx><ArithFormulaNum>附則算式</ArithFormulaNum><ArithFormula/></SupplProvisionAppdx>";
    let supplement = |number: &str, appendices: &str| format!(
        "<SupplProvision AmendLawNum=\"{number}\"><SupplProvisionLabel>附　則</SupplProvisionLabel>\n",
    ) + "<Paragraph Num=\"1\"><ParagraphNum/><ParagraphSentence><Sentence>公布の日から施行する。</Sentence></ParagraphSentence></Paragraph>" + appendices + "</SupplProvision>";
    let first = "令和七年法律第二号";
    let second = "令和七年法律第三号";
    let third = "令和七年法律第四号";
    let source = include_str!("lawtext/supplementary-amendment.xml");
    let (before, _) = source.split_once("<SupplProvision AmendLawNum=").unwrap();
    let input = format!(
        "{before}{}{}{}\n</LawBody>\n</Law>",
        supplement(first, &format!("{style}{table}{formula}")),
        supplement(second, ""),
        supplement(third, &format!("{formula}{style}")),
    );
    let document = parse_law_xml(&input).unwrap();
    let before_amendment = format!(
        "{before}{}{}\n</LawBody>\n</Law>",
        supplement(first, &format!("{style}{table}{formula}")),
        supplement(third, &format!("{formula}{style}")),
    );
    let old = parse_law_xml(&before_amendment).unwrap();
    let verification = ibis_formats::lawtext::verify_aratame_with_bill_supplement(
        &old, "", &document, Some(second),
    );
    assert!(verification.diagnostics.is_empty(), "{:?}", verification.diagnostics);

    let mut reversed = document.clone();
    reversed.suppl_provisions.reverse();
    let repeated_input = input.replace(
        &supplement(third, &format!("{formula}{style}")),
        &supplement(first, &format!("{formula}{style}"))
            .replace("公布の日から施行する。", "翌日から施行する。"),
    );
    let mut repeated = parse_law_xml(&repeated_input).unwrap();
    repeated.suppl_provisions.swap(0, 2);
    for (document, expected) in [
        (document, [(first, vec![style, table, formula]), (second, vec![]), (third, vec![formula, style])]),
        (reversed, [(third, vec![formula, style]), (second, vec![]), (first, vec![style, table, formula])]),
        (repeated, [(first, vec![formula, style]), (second, vec![]), (first, vec![style, table, formula])]),
    ] {
        let lawtext = to_lawtext(&document);
        let markdown = to_markdown(&document);
        assert!(markdown.contains("sample\\u002d\\u002dform.pdf"));
        for restored in [document, parse_lawtext(&lawtext).unwrap(), parse_markdown_law(&markdown).unwrap()] {
            let output = to_law_xml(&restored);
            let xml = roxmltree::Document::parse(&output).unwrap();
            let body = xml.root_element().children().find(|node| node.has_tag_name("LawBody")).unwrap();
            let actual: Vec<_> = body.children()
                .filter(|node| node.has_tag_name("SupplProvision"))
                .map(|node| {
                    let fragments: Vec<_> = node.children()
                        .filter(|child| child.is_element() && child.tag_name().name().starts_with("SupplProvisionAppdx"))
                        .map(|child| &output[child.range()])
                        .collect();
                    (node.attribute("AmendLawNum").unwrap(), fragments)
                })
                .collect();
            assert_eq!(actual, expected);
        }
    }
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

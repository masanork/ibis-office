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

    use ibis_formats::lawtext::to_law_xml;
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
        let lawtext = to_lawtext(&original);
        let roundtrip = parse_law_xml(&to_law_xml(&parse_lawtext(&lawtext).unwrap())).unwrap();
        assert_eq!(
            roundtrip.main_provision, original.main_provision,
            "{name} 本則"
        );
        assert_eq!(
            roundtrip.suppl_provisions, original.suppl_provisions,
            "{name} 附則"
        );
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
fn audit_separates_xml_model_changes_from_lawtext_losses() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.xml");
    let xml = include_str!("lawtext/supplementary-amendment.xml").replace(
        "<MainProvision>",
        "<TOC><TOCLabel>目次</TOCLabel></TOC><MainProvision>",
    );
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
        change["kind"] == "element_removed" && change["path"] == "/Law/LawBody[1]/TOC[1]"
    }));
    for attr in ["Era", "Year", "Num", "LawType"] {
        let path = format!("/Law/@{attr}");
        assert!(model.iter().all(|change| change["path"] != path));
        assert!(
            lawtext
                .iter()
                .any(|change| { change["kind"] == "attribute_removed" && change["path"] == path })
        );
    }
    assert!(
        lawtext
            .iter()
            .all(|change| { !change["path"].as_str().unwrap().contains("/SupplProvision") })
    );
    assert!(report["output_xsd"].is_null());
}

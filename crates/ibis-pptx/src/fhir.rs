// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust HL7 FHIR (Bundle JSON/XML, JP-Core 3文書6情報) and HL7 v2.x
//! faithful document formatter for ibisOffice.
//!
//! Non-SaMD boundary: performs strictly faithful structural transcription and
//! layout formatting into Markdown/DOCX without any diagnostic inference,
//! risk scoring, or treatment recommendation.

use std::fmt::Write as _;

use serde_json::Value;

use crate::Error;

/// Convert HL7 FHIR (Bundle JSON/XML) or HL7 v2.x bytes into structured Markdown
/// if the input matches one of those formats; returns `Ok(None)` otherwise.
pub fn input_to_markdown(data: &[u8]) -> Result<Option<String>, Error> {
    if data.is_empty() || data.starts_with(b"PK\x03\x04") {
        return Ok(None);
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return Ok(None);
    };
    let trimmed = text.trim_start_matches('\u{FEFF}').trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if trimmed.starts_with('{') && looks_like_fhir_json(trimmed) {
        return render_fhir_json_to_markdown(trimmed).map(Some);
    }
    if trimmed.starts_with('<') && looks_like_fhir_xml(trimmed) {
        return render_fhir_xml_to_markdown(trimmed).map(Some);
    }
    if looks_like_hl7v2(trimmed) {
        return Ok(Some(render_hl7v2_to_markdown(trimmed)));
    }
    Ok(None)
}

fn looks_like_fhir_json(trimmed: &str) -> bool {
    trimmed.contains("\"resourceType\"")
        && (trimmed.contains("\"Bundle\"")
            || trimmed.contains("\"Composition\"")
            || trimmed.contains("\"Patient\""))
}

fn looks_like_fhir_xml(trimmed: &str) -> bool {
    trimmed.contains("http://hl7.org/fhir")
        && (trimmed.contains("<Bundle") || trimmed.contains("<Composition"))
}

fn looks_like_hl7v2(trimmed: &str) -> bool {
    let first_line = trimmed.lines().next().unwrap_or("");
    first_line.starts_with("MSH|") && first_line.len() >= 9 && first_line[4..].starts_with("^~\\&")
}

fn render_fhir_json_to_markdown(text: &str) -> Result<String, Error> {
    let root: Value = serde_json::from_str(text)
        .map_err(|e| Error::new(format!("FHIR JSON のパースに失敗しました: {e}")))?;

    let rtype = root
        .get("resourceType")
        .and_then(Value::as_str)
        .unwrap_or("Unknown");
    let resources: Vec<&Value> = if rtype == "Bundle" {
        root.get("entry")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.get("resource"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        vec![&root]
    };

    let composition = resources
        .iter()
        .copied()
        .find(|r| res_type(r) == "Composition");
    let title = composition
        .and_then(|c| c.get("title").and_then(Value::as_str))
        .unwrap_or("HL7 FHIR 文書");
    let doc_date = composition
        .and_then(|c| c.get("date").and_then(Value::as_str))
        .or_else(|| root.get("timestamp").and_then(Value::as_str))
        .unwrap_or("");
    let bundle_id = root.get("id").and_then(Value::as_str).unwrap_or("");

    let mut md = String::new();
    let _ = writeln!(md, "---");
    let _ = writeln!(md, "title: {}", yaml_escape(title));
    let _ = writeln!(md, "fhir_resource_type: {}", yaml_escape(rtype));
    if !bundle_id.is_empty() {
        let _ = writeln!(md, "fhir_id: {}", yaml_escape(bundle_id));
    }
    if !doc_date.is_empty() {
        let _ = writeln!(md, "date: {}", yaml_escape(doc_date));
    }
    let _ = writeln!(md, "---\n");
    let _ = writeln!(md, "# {title}\n");
    let _ = writeln!(
        md,
        "> 本表示は HL7 FHIR データの忠実な構造化転記であり、診断・治療の判断支援を行うものではありません。\n"
    );

    // Patient(s)
    let patients: Vec<&Value> = resources
        .iter()
        .copied()
        .filter(|r| res_type(r) == "Patient")
        .collect();
    if !patients.is_empty() {
        let _ = writeln!(md, "## 患者情報\n");
        let _ = writeln!(md, "| 項目 | 内容 |");
        let _ = writeln!(md, "| --- | --- |");
        for p in patients {
            let name = extract_human_name(p);
            let gender = p.get("gender").and_then(Value::as_str).unwrap_or("-");
            let birth = p.get("birthDate").and_then(Value::as_str).unwrap_or("-");
            let pid = p
                .get("identifier")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|i| i.get("value").and_then(Value::as_str))
                .or_else(|| p.get("id").and_then(Value::as_str))
                .unwrap_or("-");
            let _ = writeln!(md, "| 患者ID | {} |", md_cell(pid));
            let _ = writeln!(md, "| 氏名 | {} |", md_cell(&name));
            let _ = writeln!(md, "| 性別 | {} |", md_cell(gender));
            let _ = writeln!(md, "| 生年月日 | {} |", md_cell(birth));
        }
        md.push('\n');
    }

    // Organizations / Practitioners
    let orgs: Vec<&Value> = resources
        .iter()
        .copied()
        .filter(|r| matches!(res_type(r), "Organization" | "Practitioner"))
        .collect();
    if !orgs.is_empty() {
        let _ = writeln!(md, "## 発行元・担当者\n");
        let _ = writeln!(md, "| 種別 | 名称 |");
        let _ = writeln!(md, "| --- | --- |");
        for o in orgs {
            let kind = res_type(o);
            let name = o
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| extract_human_name(o));
            let _ = writeln!(md, "| {kind} | {} |", md_cell(&name));
        }
        md.push('\n');
    }

    // 6情報: 傷病名 (Condition)
    let conditions: Vec<&Value> = resources
        .iter()
        .copied()
        .filter(|r| res_type(r) == "Condition")
        .collect();
    if !conditions.is_empty() {
        let _ = writeln!(md, "## 傷病名 (Condition)\n");
        let _ = writeln!(md, "| 傷病名 | コード | 臨床状態 | 発症/記録日 |");
        let _ = writeln!(md, "| --- | --- | --- | --- |");
        for c in conditions {
            let (disp, code) = extract_codeable(c.get("code"));
            let status = extract_codeable(c.get("clinicalStatus")).0;
            let onset = c
                .get("onsetDateTime")
                .or_else(|| c.get("recordedDate"))
                .and_then(Value::as_str)
                .unwrap_or("-");
            let _ = writeln!(
                md,
                "| {} | {} | {} | {} |",
                md_cell(&disp),
                md_cell(&code),
                md_cell(&status),
                md_cell(onset)
            );
        }
        md.push('\n');
    }

    // 6情報: アレルギー・薬剤禁忌 (AllergyIntolerance)
    let allergies: Vec<&Value> = resources
        .iter()
        .copied()
        .filter(|r| res_type(r) == "AllergyIntolerance")
        .collect();
    if !allergies.is_empty() {
        let _ = writeln!(md, "## アレルギー・薬剤禁忌 (AllergyIntolerance)\n");
        let _ = writeln!(md, "| 原因物質 | 種別 | 重要度 | 臨床状態 |");
        let _ = writeln!(md, "| --- | --- | --- | --- |");
        for a in allergies {
            let (disp, _) = extract_codeable(a.get("code"));
            let cat = a
                .get("category")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(Value::as_str)
                .unwrap_or("-");
            let crit = a.get("criticality").and_then(Value::as_str).unwrap_or("-");
            let status = extract_codeable(a.get("clinicalStatus")).0;
            let _ = writeln!(
                md,
                "| {} | {} | {} | {} |",
                md_cell(&disp),
                md_cell(cat),
                md_cell(crit),
                md_cell(&status)
            );
        }
        md.push('\n');
    }

    // 6情報: 検査・感染症 (Observation)
    let observations: Vec<&Value> = resources
        .iter()
        .copied()
        .filter(|r| res_type(r) == "Observation")
        .collect();
    if !observations.is_empty() {
        let _ = writeln!(md, "## 検査結果・所見 (Observation)\n");
        let _ = writeln!(md, "| 項目名 | コード | 測定値 | 基準日時 |");
        let _ = writeln!(md, "| --- | --- | --- | --- |");
        for o in observations {
            let (disp, code) = extract_codeable(o.get("code"));
            let val = extract_observation_value(o);
            let dt = o
                .get("effectiveDateTime")
                .and_then(Value::as_str)
                .unwrap_or("-");
            let _ = writeln!(
                md,
                "| {} | {} | {} | {} |",
                md_cell(&disp),
                md_cell(&code),
                md_cell(&val),
                md_cell(dt)
            );
        }
        md.push('\n');
    }

    // 6情報: 処方 (MedicationRequest / MedicationStatement)
    let meds: Vec<&Value> = resources
        .iter()
        .copied()
        .filter(|r| matches!(res_type(r), "MedicationRequest" | "MedicationStatement"))
        .collect();
    if !meds.is_empty() {
        let _ = writeln!(md, "## 処方・投薬 (Medication)\n");
        let _ = writeln!(md, "| 医薬品名 | コード | 用法・用量 | ステータス |");
        let _ = writeln!(md, "| --- | --- | --- | --- |");
        for m in meds {
            let (disp, code) = extract_codeable(m.get("medicationCodeableConcept"));
            let dosage = m
                .get("dosageInstruction")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|d| d.get("text").and_then(Value::as_str))
                .unwrap_or("-");
            let status = m.get("status").and_then(Value::as_str).unwrap_or("-");
            let _ = writeln!(
                md,
                "| {} | {} | {} | {} |",
                md_cell(&disp),
                md_cell(&code),
                md_cell(dosage),
                md_cell(status)
            );
        }
        md.push('\n');
    }

    Ok(md)
}

fn render_fhir_xml_to_markdown(text: &str) -> Result<String, Error> {
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| Error::new(format!("FHIR XML のパースに失敗しました: {e}")))?;
    let root = doc.root_element();
    let root_name = root.tag_name().name();

    let title = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Composition")
        .and_then(|c| {
            c.children()
                .find(|n| n.is_element() && n.tag_name().name() == "title")
                .and_then(|t| t.attribute("value"))
        })
        .unwrap_or("HL7 FHIR XML 文書");

    let mut md = String::new();
    let _ = writeln!(md, "---\ntitle: {}\nfhir_resource_type: {root_name}\n---\n", yaml_escape(title));
    let _ = writeln!(md, "# {title}\n");
    let _ = writeln!(md, "## 収録リソース一覧\n");
    let _ = writeln!(md, "| # | リソース種別 | ID |");
    let _ = writeln!(md, "| --- | --- | --- |");

    let mut idx = 0usize;
    for entry in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "entry")
    {
        if let Some(res_container) = entry
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "resource")
        {
            for res in res_container.children().filter(|n| n.is_element()) {
                idx += 1;
                let id = res
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == "id")
                    .and_then(|n| n.attribute("value"))
                    .unwrap_or("-");
                let _ = writeln!(md, "| {idx} | {} | {} |", res.tag_name().name(), md_cell(id));
            }
        }
    }
    Ok(md)
}

fn render_hl7v2_to_markdown(text: &str) -> String {
    let mut md = String::new();
    let mut msg_type = String::from("-");
    let mut msg_ctrl = String::from("-");
    let mut segments: Vec<(&str, Vec<&str>)> = Vec::new();

    for raw_line in text.split(['\r', '\n']) {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('|').collect();
        let seg = fields.first().copied().unwrap_or("");
        if seg == "MSH" {
            msg_type = fields.get(8).copied().unwrap_or("-").to_string();
            msg_ctrl = fields.get(9).copied().unwrap_or("-").to_string();
        }
        segments.push((seg, fields));
    }

    let _ = writeln!(md, "---\ntitle: HL7 v2 電文インスペクタ\nhl7_message_type: {}\n---\n", yaml_escape(&msg_type));
    let _ = writeln!(md, "# HL7 v2 電文 ({msg_type})\n");
    let _ = writeln!(md, "- メッセージ制御 ID: `{msg_ctrl}`");
    let _ = writeln!(md, "- セグメント数: {}\n", segments.len());
    let _ = writeln!(md, "## セグメント一覧\n");
    let _ = writeln!(md, "| # | セグメント | 主要フィールド |");
    let _ = writeln!(md, "| --- | --- | --- |");
    for (i, (seg, fields)) in segments.iter().enumerate() {
        let preview = fields
            .iter()
            .skip(1)
            .take(6)
            .copied()
            .collect::<Vec<_>>()
            .join(" | ");
        let _ = writeln!(md, "| {} | `{seg}` | {} |", i + 1, md_cell(&preview));
    }
    md
}

fn res_type(v: &Value) -> &str {
    v.get("resourceType").and_then(Value::as_str).unwrap_or("")
}

fn extract_human_name(v: &Value) -> String {
    if let Some(arr) = v.get("name").and_then(Value::as_array)
        && let Some(first) = arr.first()
    {
        if let Some(t) = first.get("text").and_then(Value::as_str) {
            return t.to_string();
        }
        let fam = first.get("family").and_then(Value::as_str).unwrap_or("");
        let given = first
            .get("given")
            .and_then(Value::as_array)
            .and_then(|g| g.first())
            .and_then(Value::as_str)
            .unwrap_or("");
        let joined = format!("{fam} {given}").trim().to_string();
        if !joined.is_empty() {
            return joined;
        }
    }
    "-".to_string()
}

fn extract_codeable(v: Option<&Value>) -> (String, String) {
    let Some(v) = v else {
        return ("-".to_string(), "-".to_string());
    };
    let text = v.get("text").and_then(Value::as_str);
    let coding = v
        .get("coding")
        .and_then(Value::as_array)
        .and_then(|a| a.first());
    let disp = text
        .or_else(|| coding.and_then(|c| c.get("display").and_then(Value::as_str)))
        .unwrap_or("-")
        .to_string();
    let code = coding
        .and_then(|c| c.get("code").and_then(Value::as_str))
        .unwrap_or("-")
        .to_string();
    (disp, code)
}

fn extract_observation_value(o: &Value) -> String {
    if let Some(vq) = o.get("valueQuantity") {
        let val = vq
            .get("value")
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string());
        let unit = vq.get("unit").and_then(Value::as_str).unwrap_or("");
        return format!("{val} {unit}").trim().to_string();
    }
    if let Some(vs) = o.get("valueString").and_then(Value::as_str) {
        return vs.to_string();
    }
    if let Some(vc) = o.get("valueCodeableConcept") {
        return extract_codeable(Some(vc)).0;
    }
    "-".to_string()
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn yaml_escape(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNTHETIC_FHIR_BUNDLE: &str = include_str!("../tests/fixtures/synthetic_fhir_3doc6info.json");

    #[test]
    fn converts_fhir_3doc6info_bundle_to_markdown_and_docx_model() {
        let md = input_to_markdown(SYNTHETIC_FHIR_BUNDLE.as_bytes())
            .expect("fhir parse")
            .expect("should detect FHIR Bundle");
        assert!(md.contains("# 診療情報提供書（テストサンプル）"));
        assert!(md.contains("山田 太郎"));
        assert!(md.contains("2型糖尿病"));
        assert!(md.contains("ペニシリン"));
        assert!(md.contains("6.8 %"));
        assert!(md.contains("メトホルミン塩酸塩錠250mg"));

        let opened = crate::markdown::open_markdown(md.into_bytes()).expect("open_markdown");
        assert!(opened.presentation.flow.is_some());
    }

    #[test]
    fn converts_fhir_xml_and_hl7v2() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Bundle xmlns="http://hl7.org/fhir">
  <entry>
    <resource>
      <Composition>
        <id value="comp-1"/>
        <title value="退院時サマリー"/>
      </Composition>
    </resource>
  </entry>
</Bundle>"#;
        let md_xml = input_to_markdown(xml.as_bytes()).unwrap().unwrap();
        assert!(md_xml.contains("# 退院時サマリー"));
        assert!(md_xml.contains("| 1 | Composition | comp-1 |"));

        let hl7 = "MSH|^~\\&|HIS|HOSP|LAB|HOSP|202611161200||ORU^R01|MSG001|P|2.5\rPID|1||PAT001||YAMADA^TARO\rOBX|1|NM|GLU||98|mg/dL\r";
        let md_v2 = input_to_markdown(hl7.as_bytes()).unwrap().unwrap();
        assert!(md_v2.contains("ORU^R01"));
        assert!(md_v2.contains("`PID`"));
        assert!(md_v2.contains("`OBX`"));
    }

    #[test]
    fn ignores_non_fhir_json_and_plain_text() {
        assert_eq!(input_to_markdown(b"{\"hello\": \"world\"}").unwrap(), None);
        assert_eq!(input_to_markdown(b"# Normal Markdown\nhello").unwrap(), None);
    }
}

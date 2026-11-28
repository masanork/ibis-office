//! 国税電子申告 (e-Tax `.xtx`/`.xml`)・地方税 (eLTAX) XML の判定と Markdown サマリ出力。

use roxmltree::{Document, Node};

use crate::Error;

#[derive(Debug, Clone)]
pub struct EtaxField {
    pub path: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct EtaxSignatureInfo {
    pub signature_method: Option<String>,
    pub digest_method: Option<String>,
    pub reference_uri: Option<String>,
    pub has_x509: bool,
}

#[derive(Debug, Clone)]
pub struct EtaxReport {
    pub system_name: String,
    pub root_tag: String,
    pub namespace: String,
    pub version: Option<String>,
    pub software: Option<String>,
    pub author: Option<String>,
    pub created_day: Option<String>,
    pub forms: Vec<(String, Option<String>)>,
    pub signature: Option<EtaxSignatureInfo>,
    pub fields: Vec<EtaxField>,
}

fn decode_xml_bytes(data: &[u8]) -> String {
    let head_ascii: String = data
        .iter()
        .take(256)
        .map(|&b| if b.is_ascii() { b as char } else { ' ' })
        .collect();
    let lower = head_ascii.to_ascii_lowercase();
    if lower.contains("encoding=\"shift_jis\"")
        || lower.contains("encoding='shift_jis'")
        || lower.contains("encoding=\"windows-31j\"")
    {
        let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(data);
        return cow.into_owned();
    }
    String::from_utf8_lossy(data)
        .trim_start_matches('\u{FEFF}')
        .to_string()
}

/// バイト列が e-Tax (`.xtx`/`.xml`) または eLTAX XML かどうかを判定する。
pub fn is_etax(data: &[u8]) -> bool {
    let text = decode_xml_bytes(data);
    let head: String = text.chars().take(4096).collect();
    if !head.contains('<') {
        return false;
    }
    head.contains("xml.e-tax.nta.go.jp/") || head.contains("eltax.jp/")
}

fn format_date_node(node: Node<'_, '_>) -> Option<String> {
    let mut yyyy = None;
    let mut era = None;
    let mut yy = None;
    let mut mm = None;
    let mut dd = None;
    let mut other_child = false;

    for child in node.children().filter(|n| n.is_element()) {
        let name = child.tag_name().name();
        let val = child.text().map(str::trim).unwrap_or("");
        match name {
            "yyyy" => yyyy = Some(val.to_string()),
            "era" => era = Some(val.to_string()),
            "yy" => yy = Some(val.to_string()),
            "mm" => mm = Some(val.to_string()),
            "dd" => dd = Some(val.to_string()),
            _ => other_child = true,
        }
    }
    if other_child {
        return None;
    }
    if let Some(y) = yyyy {
        return match (mm, dd) {
            (Some(m), Some(d)) => Some(format!("{y}-{m:0>2}-{d:0>2}")),
            (Some(m), None) => Some(format!("{y}-{m:0>2}")),
            _ => Some(y),
        };
    }
    if let (Some(e), Some(y)) = (era, yy) {
        let era_label = match e.as_str() {
            "1" => "明治",
            "2" => "大正",
            "3" => "昭和",
            "4" => "平成",
            "5" => "令和",
            _ => "元号",
        };
        return match (mm, dd) {
            (Some(m), Some(d)) => Some(format!("{era_label}{y}年{m}月{d}日")),
            _ => Some(format!("{era_label}{y}年")),
        };
    }
    None
}

fn collect_fields(node: Node<'_, '_>, prefix: &str, out: &mut Vec<EtaxField>) {
    if out.len() >= 300 {
        return;
    }
    let name = node.tag_name().name();
    if name == "Signature" {
        return;
    }
    let path = if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    };

    if let Some(date_str) = format_date_node(node) {
        out.push(EtaxField {
            path,
            value: date_str,
        });
        return;
    }

    let elem_children: Vec<Node<'_, '_>> = node.children().filter(|n| n.is_element()).collect();
    if elem_children.is_empty() {
        if let Some(txt) = node.text().map(str::trim).filter(|s| !s.is_empty()) {
            out.push(EtaxField {
                path,
                value: txt.to_string(),
            });
        }
        return;
    }

    for child in elem_children {
        collect_fields(child, &path, out);
    }
}

/// e-Tax / eLTAX XML を解析して構造化レポートを返す。
pub fn inspect_etax(data: &[u8]) -> Result<EtaxReport, Error> {
    let text = decode_xml_bytes(data);
    let doc = Document::parse(&text).map_err(|e| Error::xml("e-Tax XML", e))?;
    let root = doc.root_element();
    let namespace = root.default_namespace().unwrap_or("").to_string();
    let system_name = if namespace.contains("eltax.jp") {
        "eLTAX (地方税電子申告 XML)".to_string()
    } else {
        "e-Tax (国税電子申告・申請 XML)".to_string()
    };

    let version = root.attribute("VR").map(str::to_string);
    let software = root.attribute("softNM").map(str::to_string);
    let author = root.attribute("sakuseiNM").map(str::to_string);
    let created_day = root.attribute("sakuseiDay").map(str::to_string);

    let mut forms = Vec::new();
    for child in root.children().filter(|n| n.is_element()) {
        let tag = child.tag_name().name();
        if tag != "Signature" {
            if let Some(vr) = child.attribute("VR") {
                forms.push((tag.to_string(), Some(vr.to_string())));
            } else if tag == "IT" {
                forms.push(("IT (基本メタ情報)".to_string(), None));
            }
        }
    }

    let sig_node = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Signature");
    let signature = sig_node.map(|sn| {
        let signature_method = sn
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "SignatureMethod")
            .and_then(|n| n.attribute("Algorithm"))
            .map(str::to_string);
        let digest_method = sn
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "DigestMethod")
            .and_then(|n| n.attribute("Algorithm"))
            .map(str::to_string);
        let reference_uri = sn
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "Reference")
            .and_then(|n| n.attribute("URI"))
            .map(str::to_string);
        let has_x509 = sn
            .descendants()
            .any(|n| n.is_element() && n.tag_name().name() == "X509Certificate");
        EtaxSignatureInfo {
            signature_method,
            digest_method,
            reference_uri,
            has_x509,
        }
    });

    let mut fields = Vec::new();
    for child in root.children().filter(|n| n.is_element()) {
        collect_fields(child, "", &mut fields);
    }

    Ok(EtaxReport {
        system_name,
        root_tag: root.tag_name().name().to_string(),
        namespace,
        version,
        software,
        author,
        created_day,
        forms,
        signature,
        fields,
    })
}

/// e-Tax / eLTAX XML の概要と抽出項目を Markdown で返す。
pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let r = inspect_etax(data)?;
    let mut out = format!("# {}\n\n", r.system_name);
    out.push_str(&format!("- **ルート要素**: `<{}>`\n", r.root_tag));
    if !r.namespace.is_empty() {
        out.push_str(&format!("- **名前空間**: `{}`\n", r.namespace));
    }
    if let Some(vr) = &r.version {
        out.push_str(&format!("- **様式バージョン (`VR`)**: `{vr}`\n"));
    }
    if let Some(sw) = &r.software {
        out.push_str(&format!("- **作成ソフト (`softNM`)**: {sw}\n"));
    }
    if let Some(author) = &r.author {
        out.push_str(&format!("- **作成者 (`sakuseiNM`)**: {author}\n"));
    }
    if let Some(day) = &r.created_day {
        out.push_str(&format!("- **作成日 (`sakuseiDay`)**: {day}\n"));
    }

    if !r.forms.is_empty() {
        let form_list: Vec<String> = r
            .forms
            .iter()
            .map(|(name, vr)| match vr {
                Some(v) => format!("`{name}` (VR {v})"),
                None => format!("`{name}`"),
            })
            .collect();
        out.push_str(&format!("- **収録様式**: {}\n", form_list.join(", ")));
    }

    if let Some(sig) = &r.signature {
        out.push_str("- **電子署名 (XMLDSig)**: 付与あり");
        if sig.has_x509 {
            out.push_str(" (X.509 証明書含む)");
        }
        out.push('\n');
        if let Some(sm) = &sig.signature_method {
            out.push_str(&format!("  - 署名方式: `{sm}`\n"));
        }
        if let Some(dm) = &sig.digest_method {
            out.push_str(&format!("  - ダイジェスト: `{dm}`\n"));
        }
    } else {
        out.push_str("- **電子署名 (XMLDSig)**: なし\n");
    }

    if !r.fields.is_empty() {
        out.push_str("\n## 抽出項目一覧\n\n");
        out.push_str("| 項目パス | 値 |\n|---|---|\n");
        for f in &r.fields {
            out.push_str(&format!(
                "| `{}` | {} |\n",
                f.path.replace('|', "\\|"),
                f.value.replace('|', "\\|").replace(['\r', '\n'], " ")
            ));
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_etax_life_insurance_fixture_and_renders_markdown() {
        let xml = include_bytes!("../../../test/fixtures/structured-preview/etax/life-insurance.xml");
        assert!(is_etax(xml));
        let report = inspect_etax(xml).unwrap();
        assert_eq!(report.root_tag, "TEG800");
        assert_eq!(report.version.as_deref(), Some("1.1"));
        assert!(report.signature.is_some());
        assert!(report.fields.iter().any(|f| f.value == "架空保険株式会社"));
        assert!(report.fields.iter().any(|f| f.value == "2026-10-01"));

        let md = to_markdown(xml).unwrap();
        assert!(md.contains("e-Tax"));
        assert!(md.contains("生命保険料控除証明書"));
        assert!(md.contains("電子署名 (XMLDSig)"));
    }

    #[test]
    fn parses_etax_data_envelope_with_it_and_era_date() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<DATA xmlns="http://xml.e-tax.nta.go.jp/XSD/hojin" xmlns:gen="http://xml.e-tax.nta.go.jp/XSD/general">
  <IT>
    <NOZEISHA_NM>テスト株式会社</NOZEISHA_NM>
    <TEISYUTSU_DAY><gen:era>5</gen:era><gen:yy>08</gen:yy><gen:mm>05</gen:mm><gen:dd>31</gen:dd></TEISYUTSU_DAY>
  </IT>
  <HOZ600 VR="9.0"><ZAA00010>100000</ZAA00010></HOZ600>
</DATA>"#;
        assert!(is_etax(xml.as_bytes()));
        let md = to_markdown(xml.as_bytes()).unwrap();
        assert!(md.contains("テスト株式会社"));
        assert!(md.contains("令和08年05月31日"));
        assert!(md.contains("`HOZ600` (VR 9.0)"));
    }
}

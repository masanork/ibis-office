//! Adobe InDesign Markup Language (`.idml`) inspector and Markdown exporter.

use std::collections::BTreeSet;

use roxmltree::Document;

use crate::{Error, zip::Archive};

/// Summary of an Adobe InDesign IDML package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdmlSummary {
    pub document_name: String,
    pub dom_version: String,
    pub spread_count: usize,
    pub master_spread_count: usize,
    pub layer_count: usize,
    pub story_count: usize,
    pub stories: Vec<(String, String)>,
}

/// Returns true when `data` is a ZIP package containing Adobe IDML markers
/// (`mimetype` == `application/vnd.adobe.indesign-idml-package` or `designmap.xml`).
pub fn is_idml(data: &[u8]) -> bool {
    if !data.starts_with(b"PK\x03\x04") {
        return false;
    }
    let Ok(archive) = Archive::new(data) else {
        return false;
    };
    if let Ok(Some(mime_bytes)) = archive.read("mimetype") {
        if let Ok(mime) = std::str::from_utf8(&mime_bytes) {
            if mime.trim() == "application/vnd.adobe.indesign-idml-package" {
                return true;
            }
        }
    }
    matches!(archive.read("designmap.xml"), Ok(Some(_)))
}

/// Inspects an IDML ZIP package, parsing `designmap.xml` and `Stories/Story_*.xml`.
pub fn inspect_idml(data: &[u8]) -> Result<IdmlSummary, Error> {
    if !is_idml(data) {
        return Err(Error::new("Not an Adobe InDesign IDML package"));
    }

    let archive = Archive::new(data)?;
    let mut summary = IdmlSummary {
        document_name: String::new(),
        dom_version: String::new(),
        spread_count: 0,
        master_spread_count: 0,
        layer_count: 0,
        story_count: 0,
        stories: Vec::new(),
    };

    let mut story_order: Vec<String> = Vec::new();

    if let Ok(Some(designmap_bytes)) = archive.read("designmap.xml") {
        let designmap_xml = std::str::from_utf8(&designmap_bytes)
            .map_err(|_| Error::new("designmap.xml is not valid UTF-8"))?;
        let doc = Document::parse(designmap_xml).map_err(|e| Error::xml("designmap.xml", e))?;
        let root = doc.root_element();

        summary.document_name = root.attribute("Name").unwrap_or("").trim().to_string();
        summary.dom_version = root.attribute("DOMVersion").unwrap_or("").trim().to_string();

        if let Some(list) = root.attribute("StoryList") {
            for id in list.split_whitespace() {
                if !id.is_empty() {
                    story_order.push(format!("Stories/Story_{id}.xml"));
                }
            }
        }

        for node in doc.descendants().filter(|n| n.is_element()) {
            match node.tag_name().name() {
                "Spread" => summary.spread_count += 1,
                "MasterSpread" => summary.master_spread_count += 1,
                "Layer" => summary.layer_count += 1,
                "Story" => {
                    if let Some(src) = node.attribute("src") {
                        if !story_order.iter().any(|s| s == src) {
                            story_order.push(src.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let archive_names = archive.names();
    let mut all_story_paths: BTreeSet<String> = archive_names
        .iter()
        .filter(|name| name.starts_with("Stories/") && name.ends_with(".xml"))
        .cloned()
        .collect();

    if summary.spread_count == 0 {
        summary.spread_count = archive_names
            .iter()
            .filter(|name| name.starts_with("Spreads/") && name.ends_with(".xml"))
            .count();
    }
    if summary.master_spread_count == 0 {
        summary.master_spread_count = archive_names
            .iter()
            .filter(|name| name.starts_with("MasterSpreads/") && name.ends_with(".xml"))
            .count();
    }

    let mut ordered_paths = Vec::new();
    for path in story_order {
        if all_story_paths.remove(&path) {
            ordered_paths.push(path);
        }
    }
    ordered_paths.extend(all_story_paths);

    for path in ordered_paths {
        let Ok(Some(bytes)) = archive.read(&path) else {
            continue;
        };
        let Ok(xml) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let Ok(doc) = Document::parse(xml) else {
            continue;
        };
        let text = extract_story_text(&doc);
        let story_id = path
            .rsplit('/')
            .next()
            .unwrap_or(&path)
            .trim_end_matches(".xml")
            .to_string();
        if !text.is_empty() {
            summary.stories.push((story_id, text));
        }
    }

    summary.story_count = summary.stories.len();
    Ok(summary)
}

/// Renders an IDML package as a Markdown document containing metadata and extracted story text.
pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let summary = inspect_idml(data)?;
    let mut out = String::new();

    if summary.document_name.is_empty() {
        out.push_str("# Adobe InDesign (IDML) ドキュメント\n\n");
    } else {
        out.push_str(&format!("# {}\n\n", summary.document_name));
    }

    out.push_str("## メタデータ\n\n");
    out.push_str("| 項目 | 内容 |\n| --- | --- |\n");
    if !summary.dom_version.is_empty() {
        out.push_str(&format!("| InDesign DOM バージョン | {} |\n", summary.dom_version));
    }
    out.push_str(&format!("| スプレッド数 | {} |\n", summary.spread_count));
    out.push_str(&format!("| マスタースプレッド数 | {} |\n", summary.master_spread_count));
    out.push_str(&format!("| レイヤー数 | {} |\n", summary.layer_count));
    out.push_str(&format!("| テキストありストーリー数 | {} |\n\n", summary.story_count));

    if !summary.stories.is_empty() {
        out.push_str("## 本文ストーリー\n\n");
        for (story_id, text) in &summary.stories {
            out.push_str(&format!("### {story_id}\n\n{text}\n\n"));
        }
    }

    Ok(out)
}

fn extract_story_text(doc: &Document<'_>) -> String {
    let mut out = String::new();
    for node in doc.descendants() {
        if !node.is_element() {
            continue;
        }
        match node.tag_name().name() {
            "Content" => {
                if let Some(text) = node.text() {
                    out.push_str(text);
                }
            }
            "Br" => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &b in data {
                crc ^= b as u32;
                for _ in 0..8 {
                    let mask = (crc & 1).wrapping_neg();
                    crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
                }
            }
            !crc
        }

        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, content) in files {
            let offset = out.len() as u32;
            let name_bytes = name.as_bytes();
            let crc = crc32(content);
            let size = content.len() as u32;

            // Local file header
            out.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(content);

            // Central directory header
            central.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name_bytes);
        }

        let cd_offset = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);

        // End of central directory
        out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn parses_idml_designmap_and_stories_to_markdown() {
        let designmap = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="19.0" Name="koho_2026.indd" StoryList="u100 u200">
  <Layer Self="u1" Name="本文"/>
  <idPkg:MasterSpread src="MasterSpreads/MasterSpread_u10.xml"/>
  <idPkg:Spread src="Spreads/Spread_u20.xml"/>
  <idPkg:Spread src="Spreads/Spread_u21.xml"/>
  <idPkg:Story src="Stories/Story_u100.xml"/>
  <idPkg:Story src="Stories/Story_u200.xml"/>
</Document>"#;

        let story1 = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="19.0">
  <Story Self="u100">
    <ParagraphStyleRange>
      <CharacterStyleRange><Content>令和8年度 広報誌</Content></CharacterStyleRange>
      <Br/>
      <CharacterStyleRange><Content>第1特集</Content></CharacterStyleRange>
    </ParagraphStyleRange>
  </Story>
</idPkg:Story>"#;

        let story2 = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="19.0">
  <Story Self="u200">
    <ParagraphStyleRange>
      <CharacterStyleRange><Content>お知らせ一覧</Content></CharacterStyleRange>
    </ParagraphStyleRange>
  </Story>
</idPkg:Story>"#;

        let zip = build_stored_zip(&[
            ("mimetype", b"application/vnd.adobe.indesign-idml-package"),
            ("designmap.xml", designmap.as_bytes()),
            ("Stories/Story_u100.xml", story1.as_bytes()),
            ("Stories/Story_u200.xml", story2.as_bytes()),
        ]);

        assert!(is_idml(&zip));
        let summary = inspect_idml(&zip).expect("inspect_idml");
        assert_eq!(summary.document_name, "koho_2026.indd");
        assert_eq!(summary.dom_version, "19.0");
        assert_eq!(summary.spread_count, 2);
        assert_eq!(summary.master_spread_count, 1);
        assert_eq!(summary.layer_count, 1);
        assert_eq!(summary.story_count, 2);
        assert_eq!(summary.stories[0].1, "令和8年度 広報誌\n第1特集");

        let md = to_markdown(&zip).expect("to_markdown");
        assert!(md.contains("# koho_2026.indd"));
        assert!(md.contains("スプレッド数 | 2"));
        assert!(md.contains("令和8年度 広報誌"));
        assert!(md.contains("お知らせ一覧"));
    }
}

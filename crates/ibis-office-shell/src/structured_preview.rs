// SPDX-License-Identifier: AGPL-3.0-only
//! Render the parsed preview as literal, selectable document text.

use ibis_doc_model::{Block, FlowParagraph, FlowRun, FlowRunKind, FlowRunStyle};
use ibis_formats::{Kind, Opened, structured_preview};
use std::sync::Arc;

pub fn open(
    extension: &str,
    data: &[u8],
    definition: Option<&std::path::Path>,
) -> Result<Opened, String> {
    use std::io::Read as _;
    let view = definition
        .filter(|_| extension.eq_ignore_ascii_case("xml"))
        .map(std::fs::File::open)
        .transpose();
    let preview = match view {
        Ok(Some(file)) => {
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            structured_preview::xml_view::read(data, &bytes)?
        }
        Ok(None) => structured_preview::read(extension, data)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            structured_preview::read(extension, data)?
        }
        Err(error) => return Err(error.to_string()),
    };
    let bytes = ibis_formats::blank_file(Kind::Document).map_err(|e| e.to_string())?;
    let mut opened = ibis_formats::open(bytes).map_err(|e| e.to_string())?;
    let flow = opened
        .presentation
        .flow
        .as_mut()
        .ok_or("Cannot create structured preview")?;
    let mut body = Vec::new();
    let mut line = |label: &str, value: &str, path: Option<&str>| {
        let mut runs = vec![FlowRun::new(
            FlowRunKind::Text(literal(label)),
            FlowRunStyle {
                bold: true,
                ..Default::default()
            },
        )];
        if !value.is_empty() {
            runs.push(FlowRun::new(
                FlowRunKind::Text(format!("  {}", literal(value))),
                FlowRunStyle::default(),
            ));
        }
        if let Some(path) = path.filter(|path| *path != label) {
            runs.push(FlowRun::new(
                FlowRunKind::Text(format!("  ·  {}", literal(path))),
                FlowRunStyle {
                    size: 900,
                    color: ibis_doc_model::Rgb(0x595959),
                    ..Default::default()
                },
            ));
        }
        body.push(Block::Paragraph(FlowParagraph {
            id: body.len() as u32 + 1,
            runs,
            ..Default::default()
        }));
    };
    let t = crate::i18n::text;
    line(&t("structured-preview.title"), preview.format, None);
    line(&t("structured-preview.read-only"), "", None);
    if let Some(credential) = preview.credential {
        if let Some(credential_type) = credential.credential_type {
            line(
                &t("structured-preview.credential-type"),
                &credential_type,
                None,
            );
        }
        if let Some(issuer) = credential.issuer {
            line(&t("structured-preview.issuer"), &issuer, None);
        }
        line(&t("structured-preview.received-only"), "", None);
        line(&t("structured-preview.unverified"), "", None);
        line(
            &t("structured-preview.matching-digests"),
            &credential.matching_digests.to_string(),
            None,
        );
    }
    for field in preview.fields {
        line(
            &field.name,
            &field.value,
            (!field.path.is_empty()).then_some(&field.path),
        );
    }
    Arc::make_mut(flow).body = body;
    Ok(opened)
}

// Show controls visibly, including bidi controls that can disguise a field’s name.
fn literal(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars();
    for ch in chars.by_ref().take(4_096) {
        if ch.is_control()
            || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            result.extend(ch.escape_default());
        } else {
            result.push(ch);
        }
    }
    if chars.next().is_some() {
        result.push_str(" …");
    }
    result
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[test]
    fn etax_xml_reopen_uses_the_current_sibling_definition() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test/fixtures/structured-preview/etax");
        let directory = std::env::temp_dir().join(format!("ibis-etax-view-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let xml = directory.join("sample.xml");
        let definition = directory.join("sample.view.json");
        let source = std::fs::read(fixture.join("life-insurance.xml")).unwrap();
        std::fs::write(&xml, &source).unwrap();

        let read = || {
            let opened = crate::read_file::read_file(&xml).unwrap();
            assert!(opened.structured_preview);
            opened
                .presentation
                .flow
                .as_ref()
                .unwrap()
                .body
                .iter()
                .filter_map(|block| {
                    let Block::Paragraph(paragraph) = block else {
                        return None;
                    };
                    Some(
                        paragraph
                            .runs
                            .iter()
                            .filter_map(|run| match &run.kind {
                                FlowRunKind::Text(text) => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<String>(),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let plain = read();
        let mut profile: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture.join("life-insurance.view.json")).unwrap(),
        )
        .unwrap();
        std::fs::write(&definition, serde_json::to_vec(&profile).unwrap()).unwrap();
        let before = read();
        profile["entries"].as_array_mut().unwrap().swap(0, 1);
        std::fs::write(&definition, serde_json::to_vec(&profile).unwrap()).unwrap();
        let after = read();
        assert_eq!(std::fs::read(&xml).unwrap(), source);
        std::fs::remove_dir_all(directory).unwrap();

        assert!(!plain.contains("保険会社名"));
        assert!(before.find("保険会社名").unwrap() < before.find("契約者").unwrap());
        assert!(after.find("契約者").unwrap() < after.find("保険会社名").unwrap());
        for text in [&before, &after] {
            for value in ["明細 1", "48,000 円", "明細 2", "126,000 円"] {
                assert!(text.contains(value), "{value}");
            }
        }
    }
}

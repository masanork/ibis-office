// SPDX-License-Identifier: AGPL-3.0-only
//! Render the parsed preview as literal, selectable document text.

use ibis_doc_model::{Block, FlowParagraph, FlowRun, FlowRunKind, FlowRunStyle};
use ibis_formats::{Kind, Opened, structured_preview};
use std::sync::Arc;

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use ibis_formats::structured_preview::{Preview, xml_view};

pub(crate) const XML_VIEW_CONFLICT: &str = "\u{1}xml-view-conflict:";

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn open_shared_directory(cx: &gpui_kit::App) -> Result<(), String> {
    let path = shared_directory()
        .ok_or_else(|| crate::i18n::text("settings.xml-views.unavailable").to_string())?;
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    crate::external::open_with_system(&path, cx);
    Ok(())
}

pub(crate) fn shared_directory() -> Option<std::path::PathBuf> {
    Some(
        crate::app_data::base()?
            .join("ibisOffice")
            .join("xml-views"),
    )
}

pub fn open(extension: &str, data: &[u8], definition: Option<&Path>) -> Result<Opened, String> {
    let shared = shared_directory();
    let preview = read_preview(extension, data, definition, shared.as_deref())?;
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

fn read_preview(
    extension: &str,
    data: &[u8],
    definition: Option<&Path>,
    shared: Option<&Path>,
) -> Result<Preview, String> {
    if extension.eq_ignore_ascii_case("xml") {
        if let Some(path) = definition {
            match read_definition(path) {
                Ok(bytes) => return xml_view::read(data, &bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        if let Some(directory) = shared {
            match fs::read_dir(directory) {
                Ok(entries) => {
                    let definitions = entries.flatten().filter_map(|entry| {
                        let name = entry.file_name().into_string().ok()?;
                        if !name.ends_with(".view.json") || !entry.file_type().ok()?.is_file() {
                            return None;
                        }
                        let path = entry.path();
                        match read_definition(&path) {
                            Ok(bytes) => Some((name, bytes)),
                            Err(error) => {
                                log::warn!("{}: {error}", path.display());
                                None
                            }
                        }
                    });
                    let selected =
                        xml_view::select_shared(data, definitions).map_err(
                            |error| match error {
                                xml_view::SharedError::Invalid(message) => message,
                                xml_view::SharedError::Conflict(names) => {
                                    let files = names
                                        .iter()
                                        .map(|name| literal(name))
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    let message = crate::i18n::format(
                                        "structured-preview.xml-definition-conflict",
                                        &[("files", &files)],
                                    );
                                    format!("{XML_VIEW_CONFLICT}{message}")
                                }
                            },
                        )?;
                    if let Some(preview) = selected {
                        return Ok(preview);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    structured_preview::read(extension, data)
}

fn read_definition(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(xml_view::MAX_DEFINITION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
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
    fn etax_shared_views_match_namespace_root_and_version() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test/fixtures/structured-preview/etax");
        let catalog = std::env::temp_dir().join(format!("ibis-xml-catalog-{}", std::process::id()));
        fs::create_dir(&catalog).unwrap();
        let life = fs::read_to_string(fixture.join("life-insurance.xml")).unwrap();
        let quake = fs::read(fixture.join("earthquake-insurance.xml")).unwrap();
        let life_profile = fs::read_to_string(fixture.join("life-insurance.view.json")).unwrap();
        fs::write(catalog.join("profile-a.view.json"), &life_profile).unwrap();
        fs::copy(
            fixture.join("earthquake-insurance.view.json"),
            catalog.join("profile-b.view.json"),
        )
        .unwrap();
        fs::write(catalog.join("unrelated.view.json"), "{").unwrap();
        let local = catalog.join("local.json");
        let read =
            |xml: &[u8], sidecar: Option<&Path>| read_preview("xml", xml, sidecar, Some(&catalog));
        let first = read(life.as_bytes(), None).unwrap();
        let second = read(&quake, None).unwrap();
        assert_eq!(first.fields[0].name, "生命保険料控除証明書（合成サンプル）");
        assert_eq!(
            second.fields[0].name,
            "地震保険料控除証明書（合成サンプル）"
        );
        for (preview, amounts) in [
            (&first, ["48,000 円", "126,000 円"]),
            (&second, ["23,500 円", "31,750 円"]),
        ] {
            for amount in amounts {
                assert!(preview.fields.iter().any(|field| field.value == amount));
            }
            assert!(preview.fields.iter().any(|field| field.name == "明細 2"));
        }
        let older = life.replace("VR=\"1.1\"", "VR=\"1.0\"");
        let other_namespace = life.replace("/XSD/kyotsu", "/XSD/other");
        for xml in [&older, &other_namespace] {
            let fallback = read(xml.as_bytes(), None).unwrap();
            assert!(
                !fallback
                    .fields
                    .iter()
                    .any(|field| field.name == "保険会社名")
            );
        }
        let older_profile = life_profile
            .replace("1.1", "1.0")
            .replace("合成サンプル", "旧版");
        fs::write(catalog.join("older.view.json"), older_profile).unwrap();
        assert_eq!(
            read(older.as_bytes(), None).unwrap().fields[0].name,
            "生命保険料控除証明書（旧版）"
        );
        fs::write(catalog.join("duplicate.view.json"), &life_profile).unwrap();
        fs::write(catalog.join("third.view.json"), &life_profile).unwrap();
        let Err(error) = read(life.as_bytes(), None) else {
            panic!("conflicting shared definitions must stop the preview");
        };
        assert!(
            error.contains("duplicate.view.json, profile-a.view.json, third.view.json"),
            "{error}"
        );
        assert!(error.starts_with(XML_VIEW_CONFLICT), "{error}");
        assert!(!error.contains("profile-b.view.json"), "{error}");
        fs::write(&local, life_profile.replace("合成サンプル", "個別定義")).unwrap();
        assert_eq!(
            read(life.as_bytes(), Some(&local)).unwrap().fields[0].name,
            "生命保険料控除証明書（個別定義）"
        );
        fs::remove_file(catalog.join("duplicate.view.json")).unwrap();
        fs::remove_file(catalog.join("third.view.json")).unwrap();
        assert_eq!(
            read(life.as_bytes(), None).unwrap().fields[0].name,
            first.fields[0].name
        );
        fs::remove_dir_all(catalog).unwrap();
    }

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

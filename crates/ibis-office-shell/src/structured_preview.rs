// SPDX-License-Identifier: AGPL-3.0-only
//! Render the parsed preview as literal, selectable document text.

use ibis_doc_model::{
    Block, BorderStyle, FlowAlign, FlowBorder, FlowCell, FlowParagraph, FlowRow, FlowRun,
    FlowRunKind, FlowRunStyle, FlowTable, ParaFormat, Rgb, TableLook, TableStyleRef,
};
use ibis_formats::{Kind, Opened, structured_preview};
use std::sync::Arc;

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use ibis_formats::structured_preview::{
    CalloutTone, Preview, PreviewBlock, TableCellValue, ValueKind, xml_view,
};

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
    let mut next_id: u32 = 1;
    let mut fresh_id = || {
        let id = next_id;
        next_id += 1;
        id
    };
    let mut body = Vec::new();
    let t = crate::i18n::text;

    // Header title & read-only notice
    body.push(Block::Paragraph(FlowParagraph {
        id: fresh_id(),
        runs: vec![
            FlowRun::new(
                FlowRunKind::Text(format!("{}  ", literal(&t("structured-preview.title")))),
                FlowRunStyle {
                    bold: true,
                    size: 1300,
                    color: Rgb(0x0F172A),
                    ..Default::default()
                },
            ),
            FlowRun::new(
                FlowRunKind::Text(format!("[{}]", literal(preview.format))),
                FlowRunStyle {
                    bold: true,
                    size: 1100,
                    color: Rgb(0x1D4ED8),
                    ..Default::default()
                },
            ),
        ],
        direct: ParaFormat {
            after: Some(60_000),
            ..Default::default()
        },
        ..Default::default()
    }));

    // Summary card table
    let mut summary_rows: Vec<(String, String)> = vec![(
        t("structured-preview.read-only").to_string(),
        if let Some(schema) = &preview.summary.detected_schema {
            schema.clone()
        } else {
            preview.format.to_string()
        },
    )];
    if preview.summary.byte_size > 0 {
        summary_rows.push((
            "Size / Fields".to_string(),
            format!(
                "{} B · {} fields · depth {} · {} tables",
                preview.summary.byte_size,
                preview.summary.field_count,
                preview.summary.max_depth,
                preview.summary.table_count
            ),
        ));
    }
    if let Some(credential) = &preview.credential {
        if let Some(credential_type) = &credential.credential_type {
            summary_rows.push((
                t("structured-preview.credential-type").to_string(),
                credential_type.clone(),
            ));
        }
        if let Some(issuer) = &credential.issuer {
            summary_rows.push((t("structured-preview.issuer").to_string(), issuer.clone()));
        }
        summary_rows.push((t("structured-preview.received-only").to_string(), String::new()));
        summary_rows.push((t("structured-preview.unverified").to_string(), String::new()));
        summary_rows.push((
            t("structured-preview.matching-digests").to_string(),
            credential.matching_digests.to_string(),
        ));
    }
    let summary_table_rows: Vec<Vec<TableCellValue>> = summary_rows
        .into_iter()
        .map(|(k, v)| {
            vec![
                TableCellValue {
                    text: k,
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: v,
                    kind: ValueKind::String,
                },
            ]
        })
        .collect();
    body.push(build_styled_table(
        &["Property", "Detail"],
        &summary_table_rows,
        0,
        &mut fresh_id,
    ));

    // Render visual blocks (or fallback to flat fields if blocks is empty).
    if preview.blocks.is_empty() {
        for field in &preview.fields {
            body.push(build_kv_paragraph(
                0,
                &field.name,
                &field.value,
                (!field.path.is_empty()).then_some(field.path.as_str()),
                ValueKind::String,
                &mut fresh_id,
            ));
        }
    } else {
        for block in &preview.blocks {
            match block {
                PreviewBlock::Section {
                    title,
                    badge,
                    depth,
                } => {
                    let mut runs = vec![FlowRun::new(
                        FlowRunKind::Text(literal(title)),
                        FlowRunStyle {
                            bold: true,
                            size: 1100,
                            color: Rgb(0x0F172A),
                            ..Default::default()
                        },
                    )];
                    if let Some(b) = badge {
                        runs.push(FlowRun::new(
                            FlowRunKind::Text(format!("  {}", literal(b))),
                            FlowRunStyle {
                                size: 950,
                                color: Rgb(0x475569),
                                ..Default::default()
                            },
                        ));
                    }
                    let indent = (*depth as i64) * 180_000;
                    body.push(Block::Paragraph(FlowParagraph {
                        id: fresh_id(),
                        runs,
                        direct: ParaFormat {
                            left: (indent > 0).then_some(indent),
                            before: Some(60_000),
                            after: Some(20_000),
                            shading: Some(Some(Rgb(0xF1F5F9))),
                            borders: [
                                None,
                                None,
                                None,
                                Some(Some(FlowBorder {
                                    color: Rgb(0x94A3B8),
                                    width: 12,
                                    style: BorderStyle::Single,
                                    space: 6,
                                    art: None,
                                })),
                            ],
                            ..Default::default()
                        },
                        ..Default::default()
                    }));
                }
                PreviewBlock::KeyValue {
                    depth,
                    key,
                    path,
                    value,
                    kind,
                } => {
                    body.push(build_kv_paragraph(
                        *depth,
                        key,
                        value,
                        (!path.is_empty() && path != key).then_some(path.as_str()),
                        *kind,
                        &mut fresh_id,
                    ));
                }
                PreviewBlock::Table {
                    title,
                    depth,
                    headers,
                    rows,
                } => {
                    if let Some(t) = title {
                        let indent = (*depth as i64) * 180_000;
                        body.push(Block::Paragraph(FlowParagraph {
                            id: fresh_id(),
                            runs: vec![FlowRun::new(
                                FlowRunKind::Text(literal(t)),
                                FlowRunStyle {
                                    bold: true,
                                    size: 1050,
                                    color: Rgb(0x1E293B),
                                    ..Default::default()
                                },
                            )],
                            direct: ParaFormat {
                                left: (indent > 0).then_some(indent),
                                before: Some(60_000),
                                after: Some(20_000),
                                ..Default::default()
                            },
                            ..Default::default()
                        }));
                    }
                    let hdr_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
                    body.push(build_styled_table(&hdr_refs, rows, *depth, &mut fresh_id));
                }
                PreviewBlock::Callout { tone, title, body: msg } => {
                    let (bg, border, fg) = match tone {
                        CalloutTone::Info => (Rgb(0xEFF6FF), Rgb(0x3B82F6), Rgb(0x1E3A8A)),
                        CalloutTone::Warning => (Rgb(0xFEFCE8), Rgb(0xEAB308), Rgb(0x854D0E)),
                        CalloutTone::Success => (Rgb(0xF0FDF4), Rgb(0x22C55E), Rgb(0x166534)),
                        CalloutTone::Danger => (Rgb(0xFEF2F2), Rgb(0xEF4444), Rgb(0x991B1B)),
                    };
                    body.push(Block::Paragraph(FlowParagraph {
                        id: fresh_id(),
                        runs: vec![
                            FlowRun::new(
                                FlowRunKind::Text(format!("{}: ", literal(title))),
                                FlowRunStyle {
                                    bold: true,
                                    color: fg,
                                    ..Default::default()
                                },
                            ),
                            FlowRun::new(
                                FlowRunKind::Text(literal(msg)),
                                FlowRunStyle {
                                    color: fg,
                                    ..Default::default()
                                },
                            ),
                        ],
                        direct: ParaFormat {
                            before: Some(40_000),
                            after: Some(40_000),
                            shading: Some(Some(bg)),
                            borders: [
                                None,
                                None,
                                None,
                                Some(Some(FlowBorder {
                                    color: border,
                                    width: 18,
                                    style: BorderStyle::Single,
                                    space: 8,
                                    art: None,
                                })),
                            ],
                            ..Default::default()
                        },
                        ..Default::default()
                    }));
                }
            }
        }
    }

    Arc::make_mut(flow).body = body;
    Ok(opened)
}

fn value_run_style(kind: ValueKind) -> FlowRunStyle {
    match kind {
        ValueKind::String => FlowRunStyle {
            color: Rgb(0x047857),
            ..Default::default()
        },
        ValueKind::Number => FlowRunStyle {
            color: Rgb(0x1D4ED8),
            ..Default::default()
        },
        ValueKind::Boolean => FlowRunStyle {
            bold: true,
            color: Rgb(0xB45309),
            ..Default::default()
        },
        ValueKind::Null | ValueKind::Empty => FlowRunStyle {
            italic: true,
            color: Rgb(0x64748B),
            ..Default::default()
        },
        ValueKind::Attribute => FlowRunStyle {
            bold: true,
            color: Rgb(0x7C3AED),
            ..Default::default()
        },
        ValueKind::Container => FlowRunStyle {
            color: Rgb(0x334155),
            ..Default::default()
        },
    }
}

fn build_kv_paragraph(
    depth: usize,
    key: &str,
    value: &str,
    path: Option<&str>,
    kind: ValueKind,
    fresh_id: &mut dyn FnMut() -> u32,
) -> Block {
    let key_color = if kind == ValueKind::Attribute {
        Rgb(0x7C3AED)
    } else {
        Rgb(0x0F172A)
    };
    let mut runs = vec![FlowRun::new(
        FlowRunKind::Text(literal(key)),
        FlowRunStyle {
            bold: true,
            color: key_color,
            ..Default::default()
        },
    )];
    if !value.is_empty() {
        runs.push(FlowRun::new(
            FlowRunKind::Text(format!("  {}", literal(value))),
            value_run_style(kind),
        ));
    }
    if let Some(p) = path {
        runs.push(FlowRun::new(
            FlowRunKind::Text(format!("  ·  {}", literal(p))),
            FlowRunStyle {
                size: 850,
                color: Rgb(0x94A3B8),
                ..Default::default()
            },
        ));
    }
    let indent = (depth as i64) * 180_000;
    let left_border = (depth > 0).then_some(Some(FlowBorder {
        color: Rgb(0xCBD5E1),
        width: 6,
        style: BorderStyle::Single,
        space: 6,
        art: None,
    }));
    Block::Paragraph(FlowParagraph {
        id: fresh_id(),
        runs,
        direct: ParaFormat {
            left: (indent > 0).then_some(indent),
            before: Some(15_000),
            after: Some(15_000),
            borders: [None, None, None, left_border],
            ..Default::default()
        },
        ..Default::default()
    })
}

fn build_styled_table(
    headers: &[&str],
    rows: &[Vec<TableCellValue>],
    depth: usize,
    fresh_id: &mut dyn FnMut() -> u32,
) -> Block {
    let ncols = headers.len().max(1);
    let total_w: i64 = (8_200_000 - (depth as i64) * 180_000).max(3_600_000);
    let col_w = total_w / ncols as i64;
    let columns = vec![col_w; ncols];
    let border = FlowBorder {
        color: Rgb(0xCBD5E1),
        width: 4,
        style: BorderStyle::Single,
        space: 0,
        art: None,
    };

    let make_cell = |text: &str, style: FlowRunStyle, shading: Option<Rgb>, fresh: &mut dyn FnMut() -> u32| {
        let para = FlowParagraph {
            id: fresh(),
            runs: vec![FlowRun::new(FlowRunKind::Text(literal(text)), style)],
            ..Default::default()
        };
        FlowCell {
            id: fresh(),
            blocks: vec![Block::Paragraph(para)],
            span: 1,
            shading,
            borders: [Some(border), Some(border), Some(border), Some(border)],
            margins: [40 * 635, 90 * 635, 40 * 635, 90 * 635],
            ..FlowCell::default()
        }
    };

    let mut flow_rows = Vec::with_capacity(1 + rows.len());
    let header_cells: Vec<FlowCell> = headers
        .iter()
        .map(|h| {
            make_cell(
                h,
                FlowRunStyle {
                    bold: true,
                    size: 950,
                    color: Rgb(0x0F172A),
                    ..Default::default()
                },
                Some(Rgb(0xE2E8F0)),
                fresh_id,
            )
        })
        .collect();
    flow_rows.push(FlowRow {
        id: fresh_id(),
        header: true,
        cells: header_cells,
        ..FlowRow::default()
    });

    for (ri, row) in rows.iter().enumerate() {
        let row_bg = (ri % 2 == 1).then_some(Rgb(0xF8FAFC));
        let cells: Vec<FlowCell> = row
            .iter()
            .map(|cell| make_cell(&cell.text, value_run_style(cell.kind), row_bg, fresh_id))
            .collect();
        flow_rows.push(FlowRow {
            id: fresh_id(),
            header: false,
            cells,
            ..FlowRow::default()
        });
    }

    Block::Table(FlowTable {
        id: fresh_id(),
        columns,
        rows: flow_rows,
        indent: (depth as i64) * 180_000,
        align: FlowAlign::Left,
        borders: [Some(Some(border)); 6],
        style: Some(TableStyleRef {
            id: "TableGrid".to_string(),
            look: TableLook {
                first_row: true,
                last_row: false,
                first_column: false,
                last_column: false,
                no_hband: false,
                no_vband: true,
            },
            row_band: 1,
            col_band: 1,
        }),
        ..FlowTable::default()
    })
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

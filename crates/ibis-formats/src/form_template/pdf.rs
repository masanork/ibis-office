// SPDX-License-Identifier: AGPL-3.0-only
//! Merge a versioned form record into existing PDF/A-2b text fields.

use std::{collections::BTreeMap, io::Write, path::Path, sync::Arc};

use ibis_pdf::{
    CosDict, CosObj, ObjectId, PdfContainer, SaveMode, StreamData, acroform, pdfa, scene,
};
use krilla::{
    Document, SerializeSettings,
    color::rgb,
    configure::{Archival, ConfigurationBuilder},
    geom::{PathBuilder, Point, Rect},
    page::PageSettings,
    paint::{Fill, Stroke},
    tagging::{Artifact, ArtifactType, ContentTag},
    text::{Font, TextDirection},
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Truncation, rendered_values};
use crate::pdfa_validation;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// List the qualified names and widget counts of the editable text fields.
pub fn inspect(bytes: Vec<u8>) -> Result<BTreeMap<String, usize>> {
    Ok(acroform::text_fields(&mut open(bytes)?)?
        .into_iter()
        .map(|(name, field)| (name, field.widget_count()))
        .collect())
}

/// Check both the input and the filled PDF with the same independent validator.
pub fn fill_validated(
    template: &Path,
    definition: &Value,
    record: &Value,
    font: Vec<u8>,
    validator: &Path,
) -> Result<(Vec<u8>, Vec<Truncation>)> {
    let bytes = std::fs::read(template)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if definition["templateSha256"].as_str() != Some(digest.as_str()) {
        return Err("PDF template does not match the form definition".into());
    }
    // Validate the exact bytes we are about to use, even if the template is replaced.
    validate(&bytes, validator, "Input template")?;
    let result = merge(bytes, definition, record, font)?;
    validate(&result.0, validator, "Filled PDF")?;
    Ok(result)
}

fn validate(bytes: &[u8], validator: &Path, label: &str) -> Result<()> {
    let mut file = tempfile::Builder::new().suffix(".pdf").tempfile()?;
    file.write_all(bytes)?;
    let (compliant, report) = pdfa_validation::validate(file.path(), "2b", validator)?;
    if !compliant {
        return Err(format!("{label} failed PDF/A-2b validation: {report}").into());
    }
    Ok(())
}

fn open(bytes: Vec<u8>) -> Result<PdfContainer> {
    let mut pdf = PdfContainer::from_bytes(bytes)?;
    if pdfa::inspect(&mut pdf)?.declared_profile() != Some("PDF/A-2b") {
        return Err("Expected a PDF/A-2b template".into());
    }
    let (_, catalog) = pdf.catalog()?;
    if catalog.get(b"Perms").is_some() {
        return Err("PDF templates with document permissions are unsupported".into());
    }
    Ok(pdf)
}

fn merge(
    bytes: Vec<u8>,
    definition: &Value,
    record: &Value,
    font_bytes: Vec<u8>,
) -> Result<(Vec<u8>, Vec<Truncation>)> {
    let mut pdf = open(bytes)?;
    let fields = acroform::text_fields(&mut pdf)?;
    let values = rendered_values(definition, record)?;
    if fields.is_empty() || !fields.keys().eq(values.keys()) {
        return Err(
            "PDF template and definition must have the same supported text field IDs".into(),
        );
    }

    let font_bytes = Arc::new(font_bytes);
    let face = rustybuzz::Face::from_slice(&font_bytes, 0).ok_or("Invalid appearance font")?;
    if !face.is_outline_embedding_allowed()
        || !face.is_subsetting_allowed()
        || face.permissions() == Some(rustybuzz::ttf_parser::Permissions::Restricted)
    {
        return Err("Appearance font forbids outline embedding or subsetting".into());
    }
    let font = Font::new(font_bytes.clone().into(), 0).ok_or("Unsupported appearance font")?;
    let configuration = ConfigurationBuilder::new()
        .with_archival_validator(Archival::A2_B)
        .finish()
        .map_err(|error| format!("{error:?}"))?;
    let mut appearances = Document::new_with(SerializeSettings {
        configuration,
        ..Default::default()
    });
    let mut sizes = Vec::new();
    for (name, field) in &fields {
        let text = &values[name].text;
        if (!field.multiline && text.contains('\n')) || text.contains('\t') {
            return Err(format!("Unsupported whitespace in field {name}").into());
        }
        if field
            .max_len
            .is_some_and(|limit| text.encode_utf16().count() > limit)
        {
            return Err(format!("Field {name} exceeds the PDF /MaxLen").into());
        }
        for style in field.widget_appearances(&mut pdf)? {
            draw_widget(&mut appearances, &style, &font, &face, field, name, text)?;
            sizes.push((style.width, style.height));
        }
    }
    let generated = appearances
        .finish()
        .map_err(|error| format!("PDF appearance export: {error:?}"))?;
    let mut source = PdfContainer::from_bytes(generated)?;
    let pages = scene::collect_pages(&mut source)?;
    if pages.len() != sizes.len() {
        return Err("Appearance page count does not match the widget count".into());
    }
    let mut imported = BTreeMap::new();
    let mut ids = Vec::with_capacity(sizes.len());
    for (page, (width, height)) in pages.into_iter().zip(sizes) {
        let CosObj::Dict(page) = source.resolve(page.object_id)? else {
            return Err("Invalid generated appearance page".into());
        };
        let resources = page
            .get(b"Resources")
            .cloned()
            .unwrap_or(CosObj::Dict(CosDict::new()));
        let contents = page
            .get(b"Contents")
            .ok_or("Generated page has no contents")?;
        let CosObj::Stream { mut dict, data } = source.deref(contents)? else {
            return Err("Expected a single generated content stream".into());
        };
        let content = source.decode_stream(&dict, &data)?;
        let marker = b"/Artifact BMC";
        let position = content
            .windows(marker.len())
            .position(|bytes| bytes == marker)
            .ok_or("Generated appearance lacks a variable-text section")?;
        let data = StreamData::Owned {
            bytes: [
                &content[..position],
                b"/Tx BMC",
                &content[position + marker.len()..],
            ]
            .concat(),
            encoded: false,
        };
        for key in [b"Length".as_slice(), b"Filter", b"DecodeParms"] {
            dict.remove(key);
        }
        dict.set(b"Type", CosObj::Name(b"XObject".to_vec()));
        dict.set(b"Subtype", CosObj::Name(b"Form".to_vec()));
        dict.set(
            b"BBox",
            CosObj::Array(vec![
                CosObj::Int(0),
                CosObj::Int(0),
                CosObj::Real(width.into()),
                CosObj::Real(height.into()),
            ]),
        );
        dict.set(b"Resources", resources);
        let stream = import(
            &mut source,
            &mut pdf,
            &mut imported,
            CosObj::Stream { dict, data },
        )?;
        ids.push(pdf.alloc_object(stream));
    }
    let mut ids = ids.into_iter();
    for (name, field) in fields {
        let field_ids = ids.by_ref().take(field.widget_count()).collect();
        field.set_text(&mut pdf, &values[&name].text, field_ids)?;
    }

    let warnings = values
        .iter()
        .filter_map(|(id, value)| value.truncation(id))
        .collect();
    Ok((pdf.save(SaveMode::Incremental)?, warnings))
}

fn draw_widget(
    appearances: &mut Document,
    style: &acroform::WidgetAppearance,
    font: &Font,
    face: &rustybuzz::Face<'_>,
    field: &acroform::TextField,
    name: &str,
    text: &str,
) -> Result<()> {
    let (width, height) = (style.width, style.height);
    let settings = PageSettings::from_wh(width, height).ok_or("Invalid widget size")?;
    let mut page = appearances.start_page_with(settings);
    let mut surface = page.surface();
    const FONT_SIZE: f32 = 11.0;
    let padding = 2.0 + style.border.map_or(0.0, |(_, width)| width);
    if style.background.is_some() || style.border.is_some() {
        let inset = style.border.map_or(0.0, |(_, width)| width / 2.0);
        let mut path = PathBuilder::new();
        path.push_rect(
            Rect::from_ltrb(inset, inset, width - inset, height - inset)
                .ok_or("Invalid widget decoration")?,
        );
        surface.set_fill(style.background.map(|[r, g, b]| Fill {
            paint: rgb::Color::new(r, g, b).into(),
            ..Default::default()
        }));
        surface.set_stroke(style.border.map(|([r, g, b], width)| Stroke {
            paint: rgb::Color::new(r, g, b).into(),
            width,
            ..Default::default()
        }));
        surface.draw_path(&path.finish().ok_or("Invalid widget decoration")?);
        surface.set_fill(Some(Fill::default()));
        surface.set_stroke(None);
    }
    let scale = FONT_SIZE / face.units_per_em() as f32;
    let ascent = face.ascender() as f32 * scale;
    let descent = -face.descender() as f32 * scale;
    let line_height = (ascent + descent).max(FONT_SIZE);
    let lines: Vec<_> = text.split('\n').collect();
    let text_height = line_height * lines.len() as f32;
    if !text.is_empty() && text_height + 2.0 * padding > height {
        return Err(format!("Text in {name} exceeds the widget height").into());
    }
    let top = if field.multiline {
        padding
    } else {
        (height - line_height) / 2.0
    };
    let mut positioned = Vec::new();
    for (index, line) in lines
        .into_iter()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
    {
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(line);
        buffer.guess_segment_properties();
        let shaped = rustybuzz::shape(face, &[], buffer);
        if shaped.glyph_infos().iter().any(|glyph| glyph.glyph_id == 0) {
            return Err(format!("Appearance font lacks a glyph for field {name}").into());
        }
        let advance: f32 = shaped
            .glyph_positions()
            .iter()
            .map(|p| p.x_advance as f32)
            .sum();
        if advance * scale + 2.0 * padding > width {
            return Err(format!("Text in {name} exceeds the widget width").into());
        }
        let x = match field.alignment {
            0 => padding,
            1 => (width - advance * scale) / 2.0,
            2 => width - padding - advance * scale,
            _ => return Err(format!("Invalid /Q alignment for field {name}").into()),
        };
        let baseline = top + ascent + line_height * index as f32;
        let mut pen = 0.0;
        for (glyph, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let glyph_id = rustybuzz::ttf_parser::GlyphId(glyph.glyph_id as u16);
            if let Some(bbox) = face.glyph_bounding_box(glyph_id) {
                let left = x + (pen + position.x_offset as f32 + bbox.x_min as f32) * scale;
                let right = x + (pen + position.x_offset as f32 + bbox.x_max as f32) * scale;
                let upper = baseline - (position.y_offset as f32 + bbox.y_max as f32) * scale;
                let lower = baseline - (position.y_offset as f32 + bbox.y_min as f32) * scale;
                if left < 0.0 || right > width || upper < 0.0 || lower > height {
                    return Err(format!("A glyph in {name} exceeds the widget rectangle").into());
                }
            }
            pen += position.x_advance as f32;
        }
        positioned.push((Point::from_xy(x, baseline), line));
    }
    // Krilla provides an unqualified marked-content section through an
    // untyped artifact. Retag that section as /Tx when it becomes an AP.
    surface.start_tagged(ContentTag::Artifact(Artifact::with_kind(
        ArtifactType::Other,
    )));
    for (position, line) in positioned {
        surface.draw_text(
            position,
            font.clone(),
            FONT_SIZE,
            line,
            false,
            TextDirection::Auto,
        );
    }
    surface.end_tagged();
    surface.finish();
    page.finish();
    Ok(())
}

// Import the generated content and its resource graph. In particular the font
// subset, descriptor, Unicode map and colour profile keep their shared references.
fn import(
    source: &mut PdfContainer,
    target: &mut PdfContainer,
    imported: &mut BTreeMap<ObjectId, ObjectId>,
    object: CosObj,
) -> Result<CosObj> {
    let result = match object {
        CosObj::Ref(id) => {
            if let Some(new_id) = imported.get(&id) {
                return Ok(CosObj::Ref(*new_id));
            }
            let new_id = target.alloc_object(CosObj::Null);
            imported.insert(id, new_id);
            let object = source.resolve(id)?;
            let object = import(source, target, imported, object)?;
            target.set_object(new_id, object);
            CosObj::Ref(new_id)
        }
        CosObj::Dict(dict) => CosObj::Dict(import_dict(source, target, imported, dict)?),
        CosObj::Array(items) => CosObj::Array(
            items
                .into_iter()
                .map(|item| import(source, target, imported, item))
                .collect::<Result<_>>()?,
        ),
        CosObj::Stream { mut dict, data } => {
            dict.remove(b"Length");
            let data = match data {
                StreamData::RawSlice { range, encoded } => StreamData::Owned {
                    bytes: source.raw_bytes()[range].to_vec(),
                    encoded,
                },
                owned => owned,
            };
            CosObj::Stream {
                dict: import_dict(source, target, imported, dict)?,
                data,
            }
        }
        other => other,
    };
    Ok(result)
}

fn import_dict(
    source: &mut PdfContainer,
    target: &mut PdfContainer,
    imported: &mut BTreeMap<ObjectId, ObjectId>,
    dict: CosDict,
) -> Result<CosDict> {
    let mut result = CosDict::new();
    for (key, value) in dict.entries() {
        result.set(
            key.clone(),
            import(source, target, imported, value.clone())?,
        );
    }
    Ok(result)
}

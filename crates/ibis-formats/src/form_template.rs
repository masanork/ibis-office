// SPDX-License-Identifier: AGPL-3.0-only
//! Populate tagged plain-text fields in a DOCX form template.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use chrono::{Datelike, NaiveDate};
use ibis_ooxml::zip::{Archive, Writer};
use roxmltree::{Document, Node};
use serde_json::Value;
use sha2::{Digest, Sha256};
use unicode_segmentation::UnicodeSegmentation;

use crate::Error;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub(crate) const PREFIX: &str = "ibis.form.";

pub(crate) fn valid_field_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|n| n.has_tag_name((W, name)))
}

struct Field<'a, 'input> {
    id: &'a str,
    control: Node<'a, 'input>,
    run: Node<'a, 'input>,
    text: Node<'a, 'input>,
}

fn fields<'a, 'input>(xml: &'a Document<'input>) -> Result<Vec<Field<'a, 'input>>, Error> {
    let mut fields = Vec::new();
    for node in xml.descendants().filter(|n| n.has_tag_name((W, "sdt"))) {
        let Some(id) = child(node, "sdtPr")
            .and_then(|props| child(props, "tag"))
            .and_then(|tag| tag.attribute((W, "val")))
            .and_then(|tag| tag.strip_prefix(PREFIX))
        else {
            continue;
        };
        let unsupported = || Error::new(format!("Unsupported form field: {id}"));
        let content = child(node, "sdtContent").ok_or_else(unsupported)?;
        let run = child(content, "r").ok_or_else(unsupported)?;
        let text = child(run, "t").ok_or_else(unsupported)?;
        if !valid_field_id(id)
            || !node
                .parent()
                .is_some_and(|parent| parent.has_tag_name((W, "p")))
            || content.children().filter(Node::is_element).count() != 1
            || run
                .children()
                .filter(Node::is_element)
                .any(|n| n != text && !n.has_tag_name((W, "rPr")))
        {
            return Err(unsupported());
        }
        fields.push(Field {
            id,
            control: node,
            run,
            text,
        });
    }
    if fields.is_empty() {
        return Err(Error::new("No tagged DOCX form fields"));
    }
    Ok(fields)
}

fn package(bytes: &[u8]) -> Result<(Archive, String), Error> {
    let archive = Archive::new(Arc::from(bytes)).map_err(|e| Error::new(e.to_string()))?;
    let mut parts = archive
        .entries()
        .iter()
        .filter(|entry| entry.name == "word/document.xml");
    let document = parts
        .next()
        .ok_or_else(|| Error::new("Missing word/document.xml"))?;
    if parts.next().is_some() || document.size > 16 * 1024 * 1024 {
        return Err(Error::new("Unsupported DOCX main part"));
    }
    let xml = archive
        .read(document)
        .map_err(|e| Error::new(e.to_string()))?;
    let xml = String::from_utf8(xml).map_err(|e| Error::new(e.to_string()))?;
    Ok((archive, xml))
}

/// First body position of each tagged field in a loaded DOCX.
pub fn field_positions(
    document: &ibis_doc_model::FlowDocument,
) -> BTreeMap<String, ibis_doc_model::FlowPos> {
    let mut positions = BTreeMap::new();
    for paragraph in ibis_doc_model::flow_command::all_paragraphs(document) {
        let mut offset = 0;
        for run in &paragraph.runs {
            if let Some(id) = run
                .control
                .as_ref()
                .and_then(|control| control.tag.strip_prefix(PREFIX))
            {
                positions
                    .entry(id.to_owned())
                    .or_insert(ibis_doc_model::FlowPos::new(paragraph.id, offset));
            }
            offset += run.len();
        }
    }
    positions
}

/// List the field IDs in the main document, including occurrence counts.
pub fn inspect(bytes: &[u8]) -> Result<BTreeMap<String, usize>, Error> {
    let (_, xml) = package(bytes)?;
    let xml = Document::parse(&xml).map_err(|e| Error::new(e.to_string()))?;
    let mut counts = BTreeMap::new();
    for field in fields(&xml)? {
        *counts.entry(field.id.to_owned()).or_default() += 1;
    }
    Ok(counts)
}

fn display(kind: &str, value: &str) -> Option<String> {
    match kind {
        "text" => Some(value.replace("\r\n", "\n").replace('\r', "\n")),
        "date-ja" => {
            let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
            if date.format("%Y-%m-%d").to_string() != value {
                return None;
            }
            let (era, base) = if date >= NaiveDate::from_ymd_opt(2019, 5, 1)? {
                ("令和", 2018)
            } else if date >= NaiveDate::from_ymd_opt(1989, 1, 8)? {
                ("平成", 1988)
            } else {
                return None;
            };
            let year = date.year() - base;
            let year = if year == 1 {
                "元".into()
            } else {
                year.to_string()
            };
            Some(format!("{era}{year}年{}月{}日", date.month(), date.day()))
        }
        _ => None,
    }
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn write_text(value: &str) -> String {
    value
        .split('\n')
        .map(|line| {
            let text = line
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            format!(r#"<w:t xmlns:w="{W}" xml:space="preserve">{text}</w:t>"#)
        })
        .collect::<Vec<_>>()
        .join(&format!(r#"<w:br xmlns:w="{W}"/>"#))
}

fn same_form(definition: &Value, data: &Value) -> bool {
    let form = &definition["form"];
    form["id"].as_str().is_some_and(|id| !id.is_empty())
        && form["version"].as_u64().is_some_and(|v| v > 0)
        && *form == data["form"]
}

fn collect_values<'a>(
    object: &'a serde_json::Map<String, Value>,
    prefix: &str,
    values: &mut BTreeMap<String, &'a Value>,
) -> Result<(), Error> {
    for (key, value) in object {
        let id = format!("{prefix}{key}");
        if let Some(children) = value.as_object() {
            if children.is_empty() {
                return Err(Error::new(format!("Empty field group: {id}")));
            }
            collect_values(children, &format!("{id}."), values)?;
        } else if values.insert(id.clone(), value).is_some() {
            return Err(Error::new(format!("Duplicate record field: {id}")));
        }
    }
    Ok(())
}

/// Read a JSON or YAML form definition or record.
pub fn parse_input(text: &str, yaml: bool) -> Result<Value, Error> {
    if yaml {
        serde_saphyr::from_str(text).map_err(|e| Error::new(e.to_string()))
    } else {
        serde_json::from_str(text).map_err(|e| Error::new(e.to_string()))
    }
}

/// One field whose formatted value was cut to its configured limit.
#[derive(Debug, PartialEq, Eq)]
pub struct Truncation {
    pub field_id: String,
    pub actual_chars: usize,
    pub max_chars: usize,
}

#[cfg(not(target_family = "wasm"))]
pub mod pdf;

struct RenderedValue {
    text: String,
    lengths: Option<(usize, usize)>,
}

impl RenderedValue {
    fn truncation(&self, field_id: &str) -> Option<Truncation> {
        let (actual_chars, max_chars) = self.lengths?;
        Some(Truncation {
            field_id: field_id.to_owned(),
            actual_chars,
            max_chars,
        })
    }
}

fn rendered_values(
    definition: &Value,
    data: &Value,
) -> Result<BTreeMap<String, RenderedValue>, Error> {
    if !same_form(definition, data) {
        return Err(Error::new(
            "Record references a different form ID or version",
        ));
    }
    let declared = definition["fields"]
        .as_object()
        .ok_or_else(|| Error::new("Missing field definitions"))?;
    let object = data["data"]
        .as_object()
        .ok_or_else(|| Error::new("Missing record data"))?;
    let mut values = BTreeMap::new();
    collect_values(object, "", &mut values)?;
    if declared.len() != values.len() || !declared.keys().all(|id| values.contains_key(id)) {
        return Err(Error::new(
            "Definition and record must have the same field IDs",
        ));
    }
    declared
        .iter()
        .map(|(id, field)| {
            let invalid = || Error::new(format!("Invalid value or field type: {id}"));
            let kind = field
                .as_str()
                .or_else(|| field["type"].as_str())
                .ok_or_else(invalid)?;
            let value = if kind == "nonnegative-integer" {
                values[id].as_u64().map(|value| value.to_string())
            } else {
                values[id].as_str().and_then(|value| display(kind, value))
            }
            .filter(|s| {
                s.chars().all(|c| {
                    matches!(c, '\t' | '\n') || c >= ' ' && !matches!(c, '\u{fffe}' | '\u{ffff}')
                })
            })
            .ok_or_else(invalid)?;
            let mut rendered = RenderedValue {
                text: value,
                lengths: None,
            };
            if let Some(limit) = field.get("maxChars") {
                let limit = limit
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| Error::new(format!("Invalid maxChars: {id}")))?;
                let mut graphemes = rendered.text.grapheme_indices(true);
                if let Some((cut, _)) = graphemes.nth(limit) {
                    rendered.lengths = Some((limit + 1 + graphemes.count(), limit));
                    rendered.text.truncate(cut);
                }
            }
            Ok((id.clone(), rendered))
        })
        .collect()
}

/// Fill text placeholders in a trusted HTML print layout. Markers belong in text nodes.
pub fn fill_html(layout: &str, definition: &Value, data: &Value) -> Result<String, Error> {
    fill_html_with_diagnostics(layout, definition, data).map(|(document, _)| document)
}

/// Assembles an HTML layout and reports truncations for the fields it uses.
pub fn fill_html_with_diagnostics(
    layout: &str,
    definition: &Value,
    data: &Value,
) -> Result<(String, Vec<Truncation>), Error> {
    let (document, diagnostics, _) = render_html(layout, definition, data)?;
    Ok((document, diagnostics))
}

/// Preview an HTML print form together with unsigned, typed claim candidates.
pub fn preview_html_claims(
    layout: &str,
    definition: &Value,
    record: &Value,
    mapping: &Value,
) -> Result<Value, Error> {
    if !same_form(definition, mapping) {
        return Err(Error::new(
            "Claim mapping references a different form ID or version",
        ));
    }
    let bindings = mapping["claims"]
        .as_object()
        .filter(|claims| !claims.is_empty())
        .ok_or_else(|| Error::new("Missing claim mappings"))?;
    let (html, truncations, printed) = render_html(layout, definition, record)?;
    let mut values = BTreeMap::new();
    let data = record["data"]
        .as_object()
        .ok_or_else(|| Error::new("Missing record data"))?;
    collect_values(data, "", &mut values)?;
    let labels = mapping
        .get("fieldLabels")
        .map(|labels| {
            labels
                .as_object()
                .ok_or_else(|| Error::new("fieldLabels must map field IDs to labels"))
        })
        .transpose()?;
    if let Some(labels) = labels {
        for (field, label) in labels {
            if !values.contains_key(field)
                || !label.as_str().is_some_and(|text| {
                    !text.trim().is_empty() && !text.chars().any(char::is_control)
                })
            {
                return Err(Error::new(format!("Invalid field label: {field}")));
            }
        }
    }
    let mut claims = serde_json::Map::new();
    for (claim, field) in bindings {
        let field = field
            .as_str()
            .filter(|_| !claim.trim().is_empty())
            .ok_or_else(|| Error::new(format!("Invalid mapping for claim: {claim}")))?;
        let field_name = || {
            labels
                .and_then(|labels| labels.get(field))
                .and_then(Value::as_str)
                .map_or_else(|| field.to_owned(), |label| format!("{label} ({field})"))
        };
        if !printed.contains(field) {
            return Err(Error::new(format!(
                "Claim field is absent from the print layout: {}",
                field_name()
            )));
        }
        if let Some(item) = truncations.iter().find(|item| item.field_id == field) {
            return Err(Error::new(format!(
                "Claim field is truncated in the print layout: {} ({} characters, limit {})",
                field_name(),
                item.actual_chars,
                item.max_chars
            )));
        }
        let value = values[field];
        if value.as_str().is_some_and(|text| text.trim().is_empty()) {
            return Err(Error::new(format!(
                "Required claim field is blank: {}",
                field_name()
            )));
        }
        claims.insert(claim.clone(), value.clone());
    }
    let truncations: Vec<_> = truncations
        .into_iter()
        .map(|item| {
            serde_json::json!({
                "fieldId": item.field_id,
                "actualChars": item.actual_chars,
                "maxChars": item.max_chars,
            })
        })
        .collect();
    Ok(serde_json::json!({ "html": html, "claims": claims, "truncations": truncations }))
}

fn render_html<'a>(
    layout: &'a str,
    definition: &Value,
    data: &Value,
) -> Result<(String, Vec<Truncation>, BTreeSet<&'a str>), Error> {
    let values = rendered_values(definition, data)?;
    let mut used = BTreeSet::new();
    let mut result = String::with_capacity(layout.len());
    let mut tail = layout;
    while let Some((before, field)) = tail.split_once("{{") {
        let (id, after) = field
            .split_once("}}")
            .ok_or_else(|| Error::new("Unclosed HTML form field"))?;
        let value = values
            .get(id.trim())
            .ok_or_else(|| Error::new(format!("Unknown HTML form field: {id}")))?;
        result.push_str(before);
        used.insert(id.trim());
        result.push_str(&escape_text(&value.text));
        tail = after;
    }
    result.push_str(tail);
    let diagnostics = used
        .iter()
        .copied()
        .filter_map(|id| values[id].truncation(id))
        .collect();
    Ok((result, diagnostics, used))
}

/// Merge a text form definition and versioned record into DOCX.
pub fn fill(bytes: &[u8], definition: &Value, data: &Value) -> Result<Vec<u8>, Error> {
    fill_with_diagnostics(bytes, definition, data).map(|(document, _)| document)
}

/// Assembles a DOCX and reports one truncation per affected field.
pub fn fill_with_diagnostics(
    bytes: &[u8],
    definition: &Value,
    data: &Value,
) -> Result<(Vec<u8>, Vec<Truncation>), Error> {
    let values = rendered_values(definition, data)?;
    verify_template_hash(bytes, &definition["templateSha256"], "DOCX")?;
    fill_docx_values(bytes, values)
}

/// List the fields as they will appear in the compiled DOCX.
pub fn inspect_markdown(bytes: &[u8]) -> Result<BTreeMap<String, usize>, Error> {
    inspect(&crate::markdown::import_markdown_to_docx_bytes(bytes)?)
}

/// Fill a versioned Markdown template and return a DOCX and its truncation diagnostics.
pub fn fill_markdown_with_diagnostics(
    bytes: &[u8],
    definition: &Value,
    data: &Value,
) -> Result<(Vec<u8>, Vec<Truncation>), Error> {
    let values = rendered_values(definition, data)?;
    verify_template_hash(bytes, &definition["markdownTemplateSha256"], "Markdown")?;
    let docx = crate::markdown::import_markdown_to_docx_bytes(bytes)?;
    fill_docx_values(&docx, values)
}

fn verify_template_hash(bytes: &[u8], expected: &Value, format: &str) -> Result<(), Error> {
    let digest = format!("{:x}", Sha256::digest(bytes));
    if expected.as_str() == Some(digest.as_str()) {
        Ok(())
    } else {
        Err(Error::new(format!("{format} template does not match the form definition")))
    }
}

fn fill_docx_values(
    bytes: &[u8],
    values: BTreeMap<String, RenderedValue>,
) -> Result<(Vec<u8>, Vec<Truncation>), Error> {
    let (archive, mut xml) = package(bytes)?;
    let document = Document::parse(&xml).map_err(|e| Error::new(e.to_string()))?;
    let fields = fields(&document)?;
    let ids: BTreeSet<&str> = fields.iter().map(|field| field.id).collect();
    if !ids.into_iter().eq(values.keys().map(String::as_str)) {
        return Err(Error::new(
            "Template and definition must have the same field IDs",
        ));
    }
    let mut replacements = Vec::with_capacity(fields.len());
    for Field {
        id,
        control,
        run,
        text,
    } in fields
    {
        let run_range = run.range();
        let text_range = text.range();
        let mut result = xml[run_range.start..text_range.start].to_owned();
        result.push_str(&write_text(&values[id].text));
        result.push_str(&xml[text_range.end..run_range.end]);
        replacements.push((control.range(), result));
    }
    for (range, value) in replacements.into_iter().rev() {
        xml.replace_range(range, &value);
    }

    Document::parse(&xml).map_err(|e| Error::new(e.to_string()))?;
    let mut output = Writer::new();
    output.set_comment(archive.comment());
    for entry in archive.entries() {
        if entry.name.starts_with("_xmlsignatures/") {
            return Err(Error::new("Signed DOCX templates are unsupported"));
        }
        let result = if entry.name == "word/document.xml" {
            output.add(&entry.name, entry.meta, xml.as_bytes())
        } else {
            output.copy(&archive, entry)
        };
        result.map_err(|e| Error::new(e.to_string()))?;
    }
    let document = output.finish().map_err(|e| Error::new(e.to_string()))?;
    let diagnostics = values
        .iter()
        .filter_map(|(id, value)| value.truncation(id))
        .collect();
    Ok((document, diagnostics))
}

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

use crate::Error;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const PREFIX: &str = "ibis.form.";

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
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
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

/// Merge a text form definition and versioned record into DOCX.
pub fn fill(bytes: &[u8], definition: &Value, data: &Value) -> Result<Vec<u8>, Error> {
    if !same_form(definition, data) {
        return Err(Error::new(
            "Record references a different form ID or version",
        ));
    }
    let digest = format!("{:x}", Sha256::digest(bytes));
    if definition["templateSha256"].as_str() != Some(digest.as_str()) {
        return Err(Error::new(
            "DOCX template does not match the form definition",
        ));
    }
    let (archive, mut xml) = package(bytes)?;
    let document = Document::parse(&xml).map_err(|e| Error::new(e.to_string()))?;
    let fields = fields(&document)?;
    let declared = definition["fields"]
        .as_object()
        .ok_or_else(|| Error::new("Missing field definitions"))?;
    let values = data["data"]
        .as_object()
        .ok_or_else(|| Error::new("Missing record data"))?;
    let ids: BTreeSet<&str> = fields.iter().map(|field| field.id).collect();
    if ids != declared.keys().map(String::as_str).collect()
        || ids != values.keys().map(String::as_str).collect()
    {
        return Err(Error::new(
            "Template, definition and record must have the same field IDs",
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
        let replacement = declared[id]
            .as_str()
            .zip(values[id].as_str())
            .and_then(|(kind, value)| display(kind, value))
            .filter(|s| {
                s.chars().all(|c| {
                    matches!(c, '\t' | '\n') || c >= ' ' && !matches!(c, '\u{fffe}' | '\u{ffff}')
                })
            })
            .ok_or_else(|| Error::new(format!("Invalid value or field type: {id}")))?;
        let run_range = run.range();
        let text_range = text.range();
        let mut result = xml[run_range.start..text_range.start].to_owned();
        result.push_str(&write_text(&replacement));
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
    output.finish().map_err(|e| Error::new(e.to_string()))
}

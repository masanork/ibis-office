// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::process::ExitCode;

use ibis_formats::lawtext::{parse_law_xml, parse_lawtext, to_law_xml, to_lawtext};
use roxmltree::{Document, Node};
use serde_json::{Value, json};

const USAGE: &str = "Usage: ibis-lawtext-audit INPUT.xml [--xsd SCHEMA.xsd]\n    Write a JSON report of XML model and Lawtext round-trip differences.\n    Exit 2 if an XSD was supplied and the final XML is invalid.";

fn node_key(node: Node<'_, '_>) -> String {
    let tag = node.tag_name().name();
    let identity = match tag {
        "SupplProvision" => node
            .attribute("AmendLawNum")
            .map(|value| ("AmendLawNum", value)),
        "Part" | "Chapter" | "Section" | "Subsection" | "Division" | "Article" | "Paragraph"
        | "Item" => node.attribute("Num").map(|value| ("Num", value)),
        tag if tag.starts_with("Subitem") => node.attribute("Num").map(|value| ("Num", value)),
        _ => None,
    };
    match identity {
        Some((name, value)) => format!("{tag}[@{name}={value:?}]"),
        None => tag.to_owned(),
    }
}

fn children_by_location<'a, 'input>(node: Node<'a, 'input>) -> BTreeMap<String, Node<'a, 'input>> {
    let mut occurrences = HashMap::new();
    node.children()
        .filter(Node::is_element)
        .map(|child| {
            let key = node_key(child);
            let occurrence = occurrences.entry(key.clone()).or_insert(0);
            *occurrence += 1;
            (format!("{key}[{occurrence}]"), child)
        })
        .collect()
}

fn compare(old: Node<'_, '_>, new: Node<'_, '_>, path: &str, changes: &mut Vec<Value>) {
    for attr in old.attributes() {
        let other = match attr.namespace() {
            Some(namespace) => new.attribute((namespace, attr.name())),
            None => new.attribute(attr.name()),
        };
        if other != Some(attr.value()) {
            changes.push(json!({
                "kind": if other.is_none() { "attribute_removed" } else { "attribute_changed" },
                "path": format!("{path}/@{}", attr.name()),
                "before": attr.value(),
                "after": other,
            }));
        }
    }
    for attr in new.attributes() {
        let previous = match attr.namespace() {
            Some(namespace) => old.attribute((namespace, attr.name())),
            None => old.attribute(attr.name()),
        };
        if previous.is_none() {
            changes.push(json!({
                "kind": "attribute_added",
                "path": format!("{path}/@{}", attr.name()),
                "after": attr.value(),
            }));
        }
    }

    let direct_text = |node: Node<'_, '_>| {
        node.children()
            .filter(Node::is_text)
            .filter_map(|child| child.text().map(str::trim))
            .collect::<String>()
    };
    if direct_text(old) != direct_text(new) {
        changes.push(json!({"kind": "text_changed", "path": path}));
    }

    let old_children = children_by_location(old);
    let mut new_children = children_by_location(new);
    for (key, old_child) in old_children {
        let path = format!("{path}/{key}");
        if let Some(new_child) = new_children.remove(&key) {
            compare(old_child, new_child, &path, changes);
        } else {
            changes.push(json!({"kind": "element_removed", "path": path}));
        }
    }
    for key in new_children.into_keys() {
        changes.push(json!({"kind": "element_added", "path": format!("{path}/{key}")}));
    }
}

fn structural_changes(old: &str, new: &str) -> Result<Vec<Value>, roxmltree::Error> {
    let old = Document::parse(old)?;
    let new = Document::parse(new)?;
    let mut changes = Vec::new();
    compare(old.root_element(), new.root_element(), "/Law", &mut changes);
    Ok(changes)
}

#[cfg(not(target_family = "wasm"))]
fn validate_xsd(xml: &str, schema: &std::ffi::OsStr) -> Result<Value, String> {
    use std::io::{Seek, Write};
    use std::process::Command;

    let mut input = tempfile::tempfile().map_err(|e| e.to_string())?;
    input.write_all(xml.as_bytes()).map_err(|e| e.to_string())?;
    input.rewind().map_err(|e| e.to_string())?;
    let output = Command::new("xmllint")
        .args(["--nonet", "--noout", "--schema"])
        .arg(schema)
        .arg("-")
        .stdin(input)
        .output()
        .map_err(|e| format!("xmllint: {e}"))?;
    let messages = String::from_utf8_lossy(&output.stderr);
    match output.status.code() {
        Some(0 | 3) => Ok(json!({
            "valid": output.status.success(),
            "messages": messages.trim(),
        })),
        _ => Err(format!("xmllint: {}", messages.trim())),
    }
}

#[cfg(target_family = "wasm")]
fn validate_xsd(_: &str, _: &std::ffi::OsStr) -> Result<Value, String> {
    Err("XSD validation requires the native CLI and xmllint".into())
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().ok_or(USAGE)?;
    if input == "--help" {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    let schema = match args.next() {
        Some(flag) if flag == "--xsd" => Some(args.next().ok_or(USAGE)?),
        None => None,
        _ => return Err(USAGE.into()),
    };
    if args.next().is_some() {
        return Err(USAGE.into());
    }

    let original = fs::read_to_string(input)?;
    let law = parse_law_xml(&original)?;
    let model_xml = to_law_xml(&law);
    let roundtrip_xml = to_law_xml(&parse_lawtext(&to_lawtext(&law))?);
    let schema_result = schema
        .as_deref()
        .map(|schema| validate_xsd(&roundtrip_xml, schema))
        .transpose()?;
    let invalid = schema_result
        .as_ref()
        .is_some_and(|result| result["valid"] == false);
    let report = json!({
        "xml_model_changes": structural_changes(&original, &model_xml)?,
        "lawtext_roundtrip_changes": structural_changes(&model_xml, &roundtrip_xml)?,
        "output_xsd": schema_result,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(if invalid {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

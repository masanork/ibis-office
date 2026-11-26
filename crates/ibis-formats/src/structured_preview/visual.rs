// SPDX-License-Identifier: AGPL-3.0-only
//! Visual hierarchy, type classification, and automatic table grouping for
//! structured previews (JSON, YAML, XML, and credential payloads).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{Field, MAX_DEPTH, check_depth, pointer_segment, xml_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    String,
    Number,
    Boolean,
    Null,
    Attribute,
    Empty,
    Container,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutTone {
    Info,
    Warning,
    Success,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCellValue {
    pub text: String,
    pub kind: ValueKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewBlock {
    Section {
        title: String,
        badge: Option<String>,
        depth: usize,
    },
    KeyValue {
        depth: usize,
        key: String,
        path: String,
        value: String,
        kind: ValueKind,
    },
    Table {
        title: Option<String>,
        depth: usize,
        headers: Vec<String>,
        rows: Vec<Vec<TableCellValue>>,
    },
    Callout {
        tone: CalloutTone,
        title: String,
        body: String,
    },
    ChatTurn {
        role: String,
        content: String,
        tool_calls: Option<String>,
        depth: usize,
    },
    PreferencePair {
        prompt: String,
        chosen: String,
        rejected: String,
        warnings: Vec<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewSummary {
    pub byte_size: usize,
    pub field_count: usize,
    pub max_depth: usize,
    pub table_count: usize,
    pub detected_schema: Option<String>,
    pub warnings: Vec<String>,
}

pub(super) fn value_kind_of_json(value: &Value) -> ValueKind {
    match value {
        Value::String(_) => ValueKind::String,
        Value::Number(_) => ValueKind::Number,
        Value::Bool(_) => ValueKind::Boolean,
        Value::Null => ValueKind::Null,
        Value::Array(_) | Value::Object(_) => ValueKind::Container,
    }
}

pub(super) fn summarize_blocks(
    byte_size: usize,
    field_count: usize,
    blocks: &[PreviewBlock],
    detected_schema: Option<String>,
    warnings: Vec<String>,
) -> PreviewSummary {
    let mut max_depth = 0usize;
    let mut table_count = 0usize;
    for block in blocks {
        match block {
            PreviewBlock::Section { depth, .. }
            | PreviewBlock::KeyValue { depth, .. }
            | PreviewBlock::ChatTurn { depth, .. } => {
                max_depth = max_depth.max(*depth);
            }
            PreviewBlock::Table { depth, .. } => {
                max_depth = max_depth.max(*depth);
                table_count += 1;
            }
            PreviewBlock::PreferencePair { .. } => {
                table_count += 1;
            }
            PreviewBlock::Callout { .. } => {}
        }
    }
    PreviewSummary {
        byte_size,
        field_count,
        max_depth,
        table_count,
        detected_schema,
        warnings,
    }
}

pub(super) fn blocks_from_fields(fields: &[Field]) -> Vec<PreviewBlock> {
    fields
        .iter()
        .map(|f| {
            let kind = if f.value.is_empty() {
                ValueKind::Empty
            } else if f.path.contains("/@") {
                ValueKind::Attribute
            } else if f.value == "null" {
                ValueKind::Null
            } else if f.value == "true" || f.value == "false" {
                ValueKind::Boolean
            } else if f.value.starts_with('"') && f.value.ends_with('"') {
                ValueKind::String
            } else if f.value.parse::<f64>().is_ok() {
                ValueKind::Number
            } else {
                ValueKind::String
            };
            PreviewBlock::KeyValue {
                depth: 0,
                key: f.name.clone(),
                path: f.path.clone(),
                value: f.value.clone(),
                kind,
            }
        })
        .collect()
}

/// Build hierarchical visual blocks for JSON/YAML, automatically promoting uniform arrays
/// of flat/compact objects into native tables.
pub(super) fn build_json_blocks(
    value: &Value,
    path: &str,
    name: &str,
    depth: usize,
    blocks: &mut Vec<PreviewBlock>,
) -> Result<(), String> {
    check_depth(depth)?;
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                blocks.push(PreviewBlock::KeyValue {
                    depth,
                    key: name.to_string(),
                    path: if path.is_empty() {
                        "/".into()
                    } else {
                        path.into()
                    },
                    value: "{}".into(),
                    kind: ValueKind::Empty,
                });
                return Ok(());
            }
            if depth > 0 {
                blocks.push(PreviewBlock::Section {
                    title: name.to_string(),
                    badge: Some(format!("{{{} keys}}", map.len())),
                    depth: depth - 1,
                });
            }
            for (k, v) in map {
                let child_path = format!("{path}/{}", pointer_segment(k));
                build_json_blocks(v, &child_path, k, depth + 1, blocks)?;
            }
        }
        Value::Array(list) => {
            if list.is_empty() {
                blocks.push(PreviewBlock::KeyValue {
                    depth,
                    key: name.to_string(),
                    path: if path.is_empty() {
                        "/".into()
                    } else {
                        path.into()
                    },
                    value: "[]".into(),
                    kind: ValueKind::Empty,
                });
                return Ok(());
            }
            if let Some((headers, rows)) = try_json_array_table(list) {
                blocks.push(PreviewBlock::Table {
                    title: (!name.is_empty()).then(|| format!("{name} [{} items]", list.len())),
                    depth: depth.saturating_sub(1),
                    headers,
                    rows,
                });
                return Ok(());
            }
            if depth > 0 {
                blocks.push(PreviewBlock::Section {
                    title: name.to_string(),
                    badge: Some(format!("[{} items]", list.len())),
                    depth: depth - 1,
                });
            }
            for (idx, item) in list.iter().enumerate() {
                let child_path = format!("{path}/{idx}");
                let item_name = format!("[{idx}]");
                build_json_blocks(item, &child_path, &item_name, depth + 1, blocks)?;
            }
        }
        scalar => {
            blocks.push(PreviewBlock::KeyValue {
                depth: depth.saturating_sub(1),
                key: name.to_string(),
                path: if path.is_empty() {
                    "/".into()
                } else {
                    path.into()
                },
                value: scalar.to_string(),
                kind: value_kind_of_json(scalar),
            });
        }
    }
    Ok(())
}

fn try_json_array_table(list: &[Value]) -> Option<(Vec<String>, Vec<Vec<TableCellValue>>)> {
    if list.len() < 2 || list.len() > 200 {
        return None;
    }
    let mut headers: Vec<String> = Vec::new();
    let mut header_set: BTreeSet<String> = BTreeSet::new();
    for item in list {
        let Value::Object(map) = item else {
            return None;
        };
        if map.is_empty() {
            return None;
        }
        for (k, v) in map {
            // Only promote to table when cells are scalars or short compact summaries.
            if matches!(v, Value::Object(m) if m.len() > 3)
                || matches!(v, Value::Array(a) if a.len() > 4)
            {
                return None;
            }
            if header_set.insert(k.clone()) {
                headers.push(k.clone());
                if headers.len() > 12 {
                    return None;
                }
            }
        }
    }
    if headers.is_empty() {
        return None;
    }
    let mut rows = Vec::with_capacity(list.len());
    for item in list {
        let Value::Object(map) = item else {
            return None;
        };
        let row = headers
            .iter()
            .map(|h| match map.get(h) {
                Some(v) => TableCellValue {
                    text: compact_json_cell(v),
                    kind: value_kind_of_json(v),
                },
                None => TableCellValue {
                    text: String::new(),
                    kind: ValueKind::Empty,
                },
            })
            .collect();
        rows.push(row);
    }
    Some((headers, rows))
}

fn compact_json_cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(compact_json_cell).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, val)| format!("{k}: {}", compact_json_cell(val)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        other => other.to_string(),
    }
}

/// Build hierarchical visual blocks for XML, automatically promoting repeated flat sibling
/// elements into native tables.
pub(super) fn build_xml_blocks(
    node: roxmltree::Node<'_, '_>,
    path: &str,
    depth: usize,
    blocks: &mut Vec<PreviewBlock>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("Structured preview is limited to 32 levels".into());
    }
    let attrs: Vec<_> = node.attributes().collect();
    let elem_children: Vec<_> = node.children().filter(|c| c.is_element()).collect();
    let text_parts: Vec<&str> = node
        .children()
        .filter_map(|c| c.text().filter(|t| c.is_text() && !t.trim().is_empty()))
        .collect();

    let has_nested = !attrs.is_empty() || !elem_children.is_empty();
    if has_nested {
        let mut badge_parts = Vec::new();
        if !attrs.is_empty() {
            badge_parts.push(format!("@{} attrs", attrs.len()));
        }
        if !elem_children.is_empty() {
            badge_parts.push(format!("{} children", elem_children.len()));
        }
        blocks.push(PreviewBlock::Section {
            title: format!("<{}>", node.tag_name().name()),
            badge: (!badge_parts.is_empty()).then(|| badge_parts.join(" · ")),
            depth,
        });
    }

    for attr in attrs {
        let aname = xml_name(attr.namespace(), attr.name());
        blocks.push(PreviewBlock::KeyValue {
            depth: depth + 1,
            key: format!("@{}", attr.name()),
            path: format!("{path}/@{aname}"),
            value: attr.value().to_string(),
            kind: ValueKind::Attribute,
        });
    }

    // Group consecutive repeated sibling elements with the same tag name when they form a flat table.
    let mut i = 0;
    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    while i < elem_children.len() {
        let child = elem_children[i];
        let full_name = xml_name(child.tag_name().namespace(), child.tag_name().name());
        let local_name = child.tag_name().name();

        let mut run_end = i + 1;
        while run_end < elem_children.len()
            && xml_name(
                elem_children[run_end].tag_name().namespace(),
                elem_children[run_end].tag_name().name(),
            ) == full_name
        {
            run_end += 1;
        }
        let run = &elem_children[i..run_end];
        if run.len() >= 2
            && let Some((headers, rows)) = try_xml_elements_table(run)
        {
            let count = tag_counts.entry(full_name).or_default();
            *count += run.len();
            blocks.push(PreviewBlock::Table {
                title: Some(format!("<{local_name}> [{} items]", run.len())),
                depth: depth + 1,
                headers,
                rows,
            });
            i = run_end;
            continue;
        }

        let index = tag_counts.entry(full_name.clone()).or_default();
        *index += 1;
        build_xml_blocks(
            child,
            &format!("{path}/{full_name}[{index}]"),
            depth + 1,
            blocks,
        )?;
        i += 1;
    }

    for (ti, text) in text_parts.iter().enumerate() {
        if has_nested {
            blocks.push(PreviewBlock::KeyValue {
                depth: depth + 1,
                key: "#text".to_string(),
                path: format!("{path}/#text[{}]", ti + 1),
                value: (*text).to_string(),
                kind: ValueKind::String,
            });
        } else {
            blocks.push(PreviewBlock::KeyValue {
                depth,
                key: node.tag_name().name().to_string(),
                path: format!("{path}/#text[{}]", ti + 1),
                value: (*text).to_string(),
                kind: ValueKind::String,
            });
        }
    }

    if !has_nested && text_parts.is_empty() {
        blocks.push(PreviewBlock::KeyValue {
            depth,
            key: node.tag_name().name().to_string(),
            path: path.to_string(),
            value: String::new(),
            kind: ValueKind::Empty,
        });
    }
    Ok(())
}

fn try_xml_elements_table(
    nodes: &[roxmltree::Node<'_, '_>],
) -> Option<(Vec<String>, Vec<Vec<TableCellValue>>)> {
    if nodes.len() < 2 || nodes.len() > 200 {
        return None;
    }
    let mut headers: Vec<String> = Vec::new();
    let mut header_set: BTreeSet<String> = BTreeSet::new();
    let mut extracted: Vec<BTreeMap<String, (String, ValueKind)>> = Vec::with_capacity(nodes.len());

    for &node in nodes {
        let mut row_map = BTreeMap::new();
        for attr in node.attributes() {
            let k = format!("@{}", attr.name());
            if header_set.insert(k.clone()) {
                headers.push(k.clone());
                if headers.len() > 12 {
                    return None;
                }
            }
            row_map.insert(k, (attr.value().to_string(), ValueKind::Attribute));
        }
        let mut child_seen: BTreeSet<String> = BTreeSet::new();
        for child in node.children().filter(|c| c.is_element()) {
            // Only promote flat records (child elements must be leaves without nested elements).
            if child.children().any(|gc| gc.is_element()) || child.attributes().next().is_some() {
                return None;
            }
            let k = child.tag_name().name().to_string();
            if !child_seen.insert(k.clone()) {
                return None;
            }
            if header_set.insert(k.clone()) {
                headers.push(k.clone());
                if headers.len() > 12 {
                    return None;
                }
            }
            let txt: String = child
                .children()
                .filter(|c| c.is_text())
                .filter_map(|c| c.text())
                .collect();
            let kind = if txt.trim().is_empty() {
                ValueKind::Empty
            } else if txt.trim().parse::<f64>().is_ok() {
                ValueKind::Number
            } else if matches!(txt.trim(), "true" | "false") {
                ValueKind::Boolean
            } else {
                ValueKind::String
            };
            row_map.insert(k, (txt, kind));
        }
        let own_text: String = node
            .children()
            .filter_map(|c| c.text().filter(|t| c.is_text() && !t.trim().is_empty()))
            .collect();
        if !own_text.is_empty() {
            let k = "#text".to_string();
            if header_set.insert(k.clone()) {
                headers.push(k.clone());
                if headers.len() > 12 {
                    return None;
                }
            }
            row_map.insert(k, (own_text, ValueKind::String));
        }
        if row_map.is_empty() {
            return None;
        }
        extracted.push(row_map);
    }

    if headers.len() < 2 {
        return None;
    }
    let rows = extracted
        .into_iter()
        .map(|map| {
            headers
                .iter()
                .map(|h| match map.get(h) {
                    Some((text, kind)) => TableCellValue {
                        text: text.clone(),
                        kind: *kind,
                    },
                    None => TableCellValue {
                        text: String::new(),
                        kind: ValueKind::Empty,
                    },
                })
                .collect()
        })
        .collect();
    Some((headers, rows))
}

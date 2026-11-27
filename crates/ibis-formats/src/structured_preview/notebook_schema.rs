// SPDX-License-Identifier: AGPL-3.0-only
//! Structured previews for Jupyter Notebooks (`.ipynb`), TOML (`.toml`),
//! and JSON Schema / OpenAPI specifications.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::visual::{
    CalloutTone, PreviewBlock, PreviewSummary, TableCellValue, ValueKind, summarize_blocks,
};
use super::{Field, json_fields, push};

// ============================================================================
// 1. Jupyter Notebook (`.ipynb`)
// ============================================================================

pub(super) fn read_ipynb(
    bytes: &[u8],
    byte_size: usize,
) -> Result<(Vec<Field>, PreviewSummary, Vec<PreviewBlock>), String> {
    let data: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid .ipynb JSON")?;
    let Some(cells) = data.get("cells").and_then(Value::as_array) else {
        return Err("Invalid .ipynb: missing `cells` array".into());
    };

    let mut fields = Vec::new();
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();

    let lang = data
        .pointer("/metadata/kernelspec/language")
        .or_else(|| data.pointer("/metadata/language_info/name"))
        .and_then(Value::as_str)
        .unwrap_or("python")
        .to_string();
    let kernel_name = data
        .pointer("/metadata/kernelspec/display_name")
        .and_then(Value::as_str)
        .unwrap_or(&lang)
        .to_string();
    let nbformat = format!(
        "{}.{}",
        data.get("nbformat").and_then(Value::as_u64).unwrap_or(4),
        data.get("nbformat_minor")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );

    let mut code_cells = 0usize;
    let mut md_cells = 0usize;
    let mut raw_cells = 0usize;
    let mut output_count = 0usize;
    let mut error_count = 0usize;

    for cell in cells {
        match cell.get("cell_type").and_then(Value::as_str) {
            Some("code") => {
                code_cells += 1;
                if let Some(outs) = cell.get("outputs").and_then(Value::as_array) {
                    output_count += outs.len();
                    error_count += outs
                        .iter()
                        .filter(|o| o.get("output_type").and_then(Value::as_str) == Some("error"))
                        .count();
                }
            }
            Some("markdown") => md_cells += 1,
            _ => raw_cells += 1,
        }
    }

    if error_count > 0 {
        let msg = format!("{error_count} code cell(s) recorded execution errors.");
        warnings.push(msg.clone());
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Danger,
            title: "Notebook execution errors".into(),
            body: msg,
        });
    }

    let _ = push(&mut fields, "/metadata/kernel", "kernel", kernel_name.clone());
    let _ = push(&mut fields, "/nbformat", "nbformat", nbformat.clone());

    blocks.push(PreviewBlock::Table {
        title: Some("Notebook Summary".into()),
        depth: 0,
        headers: vec!["Property".into(), "Value".into()],
        rows: vec![
            vec![
                TableCellValue {
                    text: "Kernel / Language".into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: format!("{kernel_name} ({lang})"),
                    kind: ValueKind::String,
                },
            ],
            vec![
                TableCellValue {
                    text: "Format".into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: format!("nbformat {nbformat}"),
                    kind: ValueKind::String,
                },
            ],
            vec![
                TableCellValue {
                    text: "Cells".into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: format!(
                        "{} total ({md_cells} markdown, {code_cells} code, {raw_cells} raw)",
                        cells.len()
                    ),
                    kind: ValueKind::String,
                },
            ],
            vec![
                TableCellValue {
                    text: "Outputs / Errors".into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: format!("{output_count} outputs, {error_count} errors"),
                    kind: if error_count > 0 {
                        ValueKind::Boolean
                    } else {
                        ValueKind::Number
                    },
                },
            ],
        ],
    });

    for (idx, cell) in cells.iter().take(100).enumerate() {
        let cell_no = idx + 1;
        let ctype = cell
            .get("cell_type")
            .and_then(Value::as_str)
            .unwrap_or("raw");
        let src = join_nb_lines(cell.get("source"));
        let exec_badge = cell
            .get("execution_count")
            .and_then(Value::as_u64)
            .map(|n| format!("In [{n}]"));

        if fields.len() < super::MAX_FIELDS {
            let _ = push(
                &mut fields,
                &format!("/cells/{idx}/source"),
                &format!("cell_{cell_no}_{ctype}"),
                src.clone(),
            );
        }

        blocks.push(PreviewBlock::Section {
            title: format!("Cell #{cell_no} ({ctype})"),
            badge: exec_badge,
            depth: 0,
        });

        let role = match ctype {
            "markdown" => "system",
            "code" => "user",
            _ => "developer",
        };
        blocks.push(PreviewBlock::ChatTurn {
            role: role.into(),
            content: src,
            tool_calls: None,
            depth: 1,
        });

        if let Some(outs) = cell.get("outputs").and_then(Value::as_array) {
            for out in outs.iter().take(10) {
                let otype = out
                    .get("output_type")
                    .and_then(Value::as_str)
                    .unwrap_or("output");
                match otype {
                    "error" => {
                        let ename = out
                            .get("ename")
                            .and_then(Value::as_str)
                            .unwrap_or("Error");
                        let evalue = out
                            .get("evalue")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        blocks.push(PreviewBlock::Callout {
                            tone: CalloutTone::Danger,
                            title: ename.into(),
                            body: evalue.into(),
                        });
                    }
                    "stream" => {
                        let txt = join_nb_lines(out.get("text"));
                        if !txt.trim().is_empty() {
                            blocks.push(PreviewBlock::ChatTurn {
                                role: "assistant".into(),
                                content: txt,
                                tool_calls: None,
                                depth: 1,
                            });
                        }
                    }
                    _ => {
                        if let Some(data_obj) = out.get("data").and_then(Value::as_object) {
                            let plain = join_nb_lines(data_obj.get("text/plain"));
                            let has_img = data_obj.contains_key("image/png")
                                || data_obj.contains_key("image/jpeg")
                                || data_obj.contains_key("image/svg+xml");
                            let content = match (plain.trim().is_empty(), has_img) {
                                (false, true) => format!("{plain}\n[Embedded chart/image output]"),
                                (false, false) => plain,
                                (true, true) => "[Embedded chart/image output]".into(),
                                (true, false) => "(rich output)".into(),
                            };
                            blocks.push(PreviewBlock::ChatTurn {
                                role: "assistant".into(),
                                content,
                                tool_calls: None,
                                depth: 1,
                            });
                        }
                    }
                }
            }
        }
    }

    let summary = summarize_blocks(
        byte_size,
        fields.len(),
        &blocks,
        Some(format!("Jupyter Notebook ({lang})")),
        warnings,
    );
    Ok((fields, summary, blocks))
}

fn join_nb_lines(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

// ============================================================================
// 2. Pure-Rust TOML Parser (`.toml`) -> `serde_json::Value`
// ============================================================================

pub(super) fn parse_toml_to_value(src: &str) -> Result<Value, String> {
    let mut root = Map::new();
    let mut current_path: Vec<String> = Vec::new();
    let mut current_is_array = false;
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let stripped = strip_toml_comment(lines[i]).trim();
        if stripped.is_empty() {
            i += 1;
            continue;
        }
        // Array of tables `[[a.b]]`
        if stripped.starts_with("[[") && stripped.ends_with("]]") {
            let inner = &stripped[2..stripped.len() - 2];
            current_path = split_toml_key_path(inner)?;
            current_is_array = true;
            ensure_toml_table_array_item(&mut root, &current_path);
            i += 1;
            continue;
        }
        // Table `[a.b]`
        if stripped.starts_with('[') && stripped.ends_with(']') {
            let inner = &stripped[1..stripped.len() - 1];
            current_path = split_toml_key_path(inner)?;
            current_is_array = false;
            ensure_toml_table(&mut root, &current_path);
            i += 1;
            continue;
        }
        // Key = Value (supporting multi-line `[...]` arrays)
        let Some((raw_k, raw_v)) = split_toml_kv(stripped) else {
            return Err(format!("Invalid TOML syntax at line {}", i + 1));
        };
        let mut val_buf = raw_v.trim().to_string();
        while !toml_brackets_balanced(&val_buf) && i + 1 < lines.len() {
            i += 1;
            let next_line = strip_toml_comment(lines[i]).trim();
            if !next_line.is_empty() {
                val_buf.push(' ');
                val_buf.push_str(next_line);
            }
        }
        let key_parts = split_toml_key_path(raw_k)?;
        let val = parse_toml_value(val_buf.trim())?;
        insert_toml_value(&mut root, &current_path, current_is_array, &key_parts, val);
        i += 1;
    }

    Ok(Value::Object(root))
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_dq = false;
    let mut in_sq = false;
    let mut escaped = false;
    for (idx, b) in line.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match b {
            b'\\' if in_dq => escaped = true,
            b'"' if !in_sq => in_dq = !in_dq,
            b'\'' if !in_dq => in_sq = !in_sq,
            b'#' if !in_dq && !in_sq => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn split_toml_kv(line: &str) -> Option<(&str, &str)> {
    let mut in_dq = false;
    let mut in_sq = false;
    let mut escaped = false;
    for (idx, b) in line.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match b {
            b'\\' if in_dq => escaped = true,
            b'"' if !in_sq => in_dq = !in_dq,
            b'\'' if !in_dq => in_sq = !in_sq,
            b'=' if !in_dq && !in_sq => return Some((line[..idx].trim(), line[idx + 1..].trim())),
            _ => {}
        }
    }
    None
}

fn toml_brackets_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_dq = false;
    let mut in_sq = false;
    let mut escaped = false;
    for b in s.bytes() {
        if escaped {
            escaped = false;
            continue;
        }
        match b {
            b'\\' if in_dq => escaped = true,
            b'"' if !in_sq => in_dq = !in_dq,
            b'\'' if !in_dq => in_sq = !in_sq,
            b'[' | b'{' if !in_dq && !in_sq => depth += 1,
            b']' | b'}' if !in_dq && !in_sq => depth -= 1,
            _ => {}
        }
    }
    depth <= 0 && !in_dq && !in_sq
}

fn split_toml_key_path(raw: &str) -> Result<Vec<String>, String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut in_dq = false;
    let mut in_sq = false;
    for ch in raw.chars() {
        match ch {
            '"' if !in_sq => in_dq = !in_dq,
            '\'' if !in_dq => in_sq = !in_sq,
            '.' if !in_dq && !in_sq => {
                let t = cur.trim().trim_matches(['"', '\'']).to_string();
                if t.is_empty() {
                    return Err("Empty TOML key segment".into());
                }
                parts.push(t);
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let last = cur.trim().trim_matches(['"', '\'']).to_string();
    if last.is_empty() {
        return Err("Empty TOML key".into());
    }
    parts.push(last);
    Ok(parts)
}

fn parse_toml_value(s: &str) -> Result<Value, String> {
    if s == "true" {
        return Ok(Value::Bool(true));
    }
    if s == "false" {
        return Ok(Value::Bool(false));
    }
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        return Ok(Value::String(s[1..s.len() - 1].replace("\\\"", "\"")));
    }
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        let items = split_top_level_commas(inner)
            .into_iter()
            .filter(|x| !x.is_empty())
            .map(parse_toml_value)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Value::Array(items));
    }
    if s.starts_with('{') && s.ends_with('}') {
        let inner = &s[1..s.len() - 1];
        let mut map = Map::new();
        for item in split_top_level_commas(inner) {
            if item.is_empty() {
                continue;
            }
            if let Some((k, v)) = split_toml_kv(item) {
                let clean_k = k.trim_matches(['"', '\'']).to_string();
                map.insert(clean_k, parse_toml_value(v)?);
            }
        }
        return Ok(Value::Object(map));
    }
    let no_underscore = s.replace('_', "");
    if let Ok(n) = no_underscore.parse::<i64>() {
        return Ok(Value::Number(n.into()));
    }
    if let Ok(f) = no_underscore.parse::<f64>()
        && let Some(num) = serde_json::Number::from_f64(f)
    {
        return Ok(Value::Number(num));
    }
    Ok(Value::String(s.to_string()))
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_dq = false;
    let mut in_sq = false;
    let mut escaped = false;
    for (idx, b) in s.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match b {
            b'\\' if in_dq => escaped = true,
            b'"' if !in_sq => in_dq = !in_dq,
            b'\'' if !in_dq => in_sq = !in_sq,
            b'[' | b'{' if !in_dq && !in_sq => depth += 1,
            b']' | b'}' if !in_dq && !in_sq => depth -= 1,
            b',' if !in_dq && !in_sq && depth == 0 => {
                parts.push(s[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts
}

fn ensure_toml_table<'a>(root: &'a mut Map<String, Value>, path: &[String]) -> &'a mut Map<String, Value> {
    let mut cur = root;
    for seg in path {
        let entry = cur
            .entry(seg.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        cur = entry.as_object_mut().unwrap();
    }
    cur
}

fn ensure_toml_table_array_item(root: &mut Map<String, Value>, path: &[String]) {
    if path.is_empty() {
        return;
    }
    let parent = ensure_toml_table(root, &path[..path.len() - 1]);
    let arr = parent
        .entry(path.last().unwrap().clone())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !arr.is_array() {
        *arr = Value::Array(Vec::new());
    }
    arr.as_array_mut().unwrap().push(Value::Object(Map::new()));
}

fn insert_toml_value(
    root: &mut Map<String, Value>,
    table_path: &[String],
    is_array: bool,
    key_parts: &[String],
    val: Value,
) {
    let target = if is_array && !table_path.is_empty() {
        let parent = ensure_toml_table(root, &table_path[..table_path.len() - 1]);
        parent
            .get_mut(table_path.last().unwrap())
            .and_then(Value::as_array_mut)
            .and_then(|a| a.last_mut())
            .and_then(Value::as_object_mut)
    } else {
        Some(ensure_toml_table(root, table_path))
    };
    if let Some(tbl) = target {
        let leaf_parent = if key_parts.len() > 1 {
            ensure_toml_table(tbl, &key_parts[..key_parts.len() - 1])
        } else {
            tbl
        };
        if let Some(k) = key_parts.last() {
            leaf_parent.insert(k.clone(), val);
        }
    }
}

// ============================================================================
// 3. JSON Schema & OpenAPI Specification Detection
// ============================================================================

pub(super) fn try_build_schema_or_openapi_blocks(
    val: &Value,
    byte_size: usize,
    fields: &[Field],
) -> Option<(PreviewSummary, Vec<PreviewBlock>)> {
    let Value::Object(map) = val else {
        return None;
    };
    if (map.contains_key("openapi") || map.contains_key("swagger"))
        && (map.get("paths").is_some_and(Value::is_object)
            || map.get("components").is_some_and(Value::is_object))
    {
        return Some(build_openapi_preview(map, byte_size, fields.len()));
    }
    if map.contains_key("$schema")
        || (map.get("properties").is_some_and(Value::is_object)
            && (map.contains_key("type") || map.contains_key("title") || map.contains_key("required")))
    {
        return Some(build_json_schema_preview(map, byte_size, fields.len()));
    }
    None
}

fn build_json_schema_preview(
    map: &Map<String, Value>,
    byte_size: usize,
    field_count: usize,
) -> (PreviewSummary, Vec<PreviewBlock>) {
    let mut blocks = Vec::new();
    let title = map
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("JSON Schema");
    let mut meta_rows = vec![vec![
        TableCellValue {
            text: "Title".into(),
            kind: ValueKind::Attribute,
        },
        TableCellValue {
            text: title.into(),
            kind: ValueKind::String,
        },
    ]];
    for key in ["$schema", "$id", "type", "description"] {
        if let Some(v) = map.get(key).and_then(Value::as_str) {
            meta_rows.push(vec![
                TableCellValue {
                    text: key.into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: v.into(),
                    kind: ValueKind::String,
                },
            ]);
        }
    }
    blocks.push(PreviewBlock::Table {
        title: Some("Schema Overview".into()),
        depth: 0,
        headers: vec!["Field".into(), "Value".into()],
        rows: meta_rows,
    });

    if let Some(props) = map.get("properties").and_then(Value::as_object) {
        blocks.push(schema_properties_table(
            "Properties",
            props,
            map.get("required").and_then(Value::as_array),
        ));
    }
    if let Some(defs) = map
        .get("$defs")
        .or_else(|| map.get("definitions"))
        .and_then(Value::as_object)
    {
        for (dname, dval) in defs.iter().take(20) {
            if let Some(props) = dval.get("properties").and_then(Value::as_object) {
                blocks.push(schema_properties_table(
                    &format!("Definition: {dname}"),
                    props,
                    dval.get("required").and_then(Value::as_array),
                ));
            }
        }
    }

    let summary = summarize_blocks(
        byte_size,
        field_count,
        &blocks,
        Some("JSON Schema".into()),
        Vec::new(),
    );
    (summary, blocks)
}

fn schema_properties_table(
    title: &str,
    props: &Map<String, Value>,
    required_arr: Option<&Vec<Value>>,
) -> PreviewBlock {
    let required: BTreeSet<&str> = required_arr
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let rows: Vec<Vec<TableCellValue>> = props
        .iter()
        .map(|(k, v)| {
            let ty = v
                .get("type")
                .map(|t| match t {
                    Value::String(s) => s.clone(),
                    _ => t.to_string(),
                })
                .or_else(|| {
                    v.get("$ref")
                        .and_then(Value::as_str)
                        .map(|r| format!("$ref: {r}"))
                })
                .unwrap_or_else(|| "any".into());
            let req = required.contains(k.as_str());
            let constraints = v
                .get("enum")
                .map(|e| format!("enum: {e}"))
                .or_else(|| v.get("format").and_then(Value::as_str).map(|f| format!("format: {f}")))
                .or_else(|| v.get("default").map(|d| format!("default: {d}")))
                .unwrap_or_default();
            let desc = v
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            vec![
                TableCellValue {
                    text: k.clone(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: ty,
                    kind: ValueKind::Number,
                },
                TableCellValue {
                    text: if req { "required" } else { "optional" }.into(),
                    kind: if req {
                        ValueKind::Boolean
                    } else {
                        ValueKind::Null
                    },
                },
                TableCellValue {
                    text: constraints,
                    kind: ValueKind::String,
                },
                TableCellValue {
                    text: desc,
                    kind: ValueKind::String,
                },
            ]
        })
        .collect();
    PreviewBlock::Table {
        title: Some(format!("{title} ({} fields)", rows.len())),
        depth: 0,
        headers: vec![
            "Property".into(),
            "Type".into(),
            "Required".into(),
            "Constraints".into(),
            "Description".into(),
        ],
        rows,
    }
}

fn build_openapi_preview(
    map: &Map<String, Value>,
    byte_size: usize,
    field_count: usize,
) -> (PreviewSummary, Vec<PreviewBlock>) {
    let mut blocks = Vec::new();
    let info = map.get("info").and_then(Value::as_object);
    let title = info
        .and_then(|i| i.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("OpenAPI Specification");
    let version = info
        .and_then(|i| i.get("version"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let spec_ver = map
        .get("openapi")
        .or_else(|| map.get("swagger"))
        .and_then(Value::as_str)
        .unwrap_or("");
    blocks.push(PreviewBlock::Table {
        title: Some("API Info".into()),
        depth: 0,
        headers: vec!["Field".into(), "Value".into()],
        rows: vec![
            vec![
                TableCellValue {
                    text: "Title".into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: title.into(),
                    kind: ValueKind::String,
                },
            ],
            vec![
                TableCellValue {
                    text: "API / Spec Version".into(),
                    kind: ValueKind::Attribute,
                },
                TableCellValue {
                    text: format!("{version} (OpenAPI {spec_ver})"),
                    kind: ValueKind::String,
                },
            ],
        ],
    });

    if let Some(paths) = map.get("paths").and_then(Value::as_object) {
        let mut ep_rows = Vec::new();
        for (path, methods) in paths {
            let Some(m_obj) = methods.as_object() else { continue };
            for (method, op) in m_obj {
                let m_lower = method.to_ascii_lowercase();
                if !matches!(
                    m_lower.as_str(),
                    "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
                ) {
                    continue;
                }
                let summary = op
                    .get("summary")
                    .or_else(|| op.get("operationId"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let resps: Vec<String> = op
                    .get("responses")
                    .and_then(Value::as_object)
                    .map(|r| r.keys().cloned().collect())
                    .unwrap_or_default();
                ep_rows.push(vec![
                    TableCellValue {
                        text: method.to_ascii_uppercase(),
                        kind: ValueKind::Boolean,
                    },
                    TableCellValue {
                        text: path.clone(),
                        kind: ValueKind::Attribute,
                    },
                    TableCellValue {
                        text: summary,
                        kind: ValueKind::String,
                    },
                    TableCellValue {
                        text: resps.join(", "),
                        kind: ValueKind::Number,
                    },
                ]);
            }
        }
        if !ep_rows.is_empty() {
            blocks.push(PreviewBlock::Table {
                title: Some(format!("Endpoints ({})", ep_rows.len())),
                depth: 0,
                headers: vec![
                    "Method".into(),
                    "Path".into(),
                    "Summary".into(),
                    "Responses".into(),
                ],
                rows: ep_rows,
            });
        }
    }

    if let Some(schemas) = map
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(Value::as_object)
    {
        for (sname, sval) in schemas.iter().take(20) {
            if let Some(props) = sval.get("properties").and_then(Value::as_object) {
                blocks.push(schema_properties_table(
                    &format!("Schema: {sname}"),
                    props,
                    sval.get("required").and_then(Value::as_array),
                ));
            }
        }
    }

    let summary = summarize_blocks(
        byte_size,
        field_count,
        &blocks,
        Some(format!("OpenAPI ({title})")),
        Vec::new(),
    );
    (summary, blocks)
}

#[allow(dead_code)]
fn _keep_json_fields() {
    let _ = json_fields;
}

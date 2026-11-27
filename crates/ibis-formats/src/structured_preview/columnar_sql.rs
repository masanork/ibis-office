// SPDX-License-Identifier: AGPL-3.0-only
//! Columnar (Parquet, Arrow IPC, Feather) and SQL (DDL & queries) structured preview builders.

use ibis_doc_model::columnar_preview::{
    ColumnarCellKind, ColumnarPreviewData, inspect_arrow_ipc, inspect_parquet,
};

use super::visual::{
    self, CalloutTone, PreviewBlock, PreviewSummary, TableCellValue, ValueKind,
};
use super::{Field, MAX_FIELDS};

const MAX_PREVIEW_ROWS: usize = 50;

fn cell(text: impl Into<String>, kind: ValueKind) -> TableCellValue {
    TableCellValue {
        text: text.into(),
        kind,
    }
}

fn push_field(fields: &mut Vec<Field>, name: &str, path: &str, value: &str) {
    if fields.len() < MAX_FIELDS {
        fields.push(Field {
            name: name.to_string(),
            path: path.to_string(),
            value: value.to_string(),
        });
    }
}

pub fn read_parquet(
    bytes: &[u8],
    raw_len: usize,
) -> Result<(Vec<Field>, PreviewSummary, Vec<PreviewBlock>), String> {
    let data = inspect_parquet(bytes, MAX_PREVIEW_ROWS)?;
    Ok(build_columnar_preview(data, raw_len))
}

pub fn read_arrow_ipc(
    bytes: &[u8],
    raw_len: usize,
) -> Result<(Vec<Field>, PreviewSummary, Vec<PreviewBlock>), String> {
    let data = inspect_arrow_ipc(bytes, MAX_PREVIEW_ROWS)?;
    Ok(build_columnar_preview(data, raw_len))
}

fn build_columnar_preview(
    data: ColumnarPreviewData,
    raw_len: usize,
) -> (Vec<Field>, PreviewSummary, Vec<PreviewBlock>) {
    let mut fields = Vec::new();
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();

    push_field(
        &mut fields,
        "total_rows",
        "/metadata/total_rows",
        &data.total_rows.to_string(),
    );
    push_field(
        &mut fields,
        "columns",
        "/metadata/columns",
        &data.schema.len().to_string(),
    );

    let mut meta_rows = vec![
        vec![
            cell("Format", ValueKind::Attribute),
            cell(data.format_label, ValueKind::String),
        ],
        vec![
            cell("Total Rows", ValueKind::Attribute),
            cell(data.total_rows.to_string(), ValueKind::Number),
        ],
        vec![
            cell("Columns", ValueKind::Attribute),
            cell(data.schema.len().to_string(), ValueKind::Number),
        ],
    ];
    if let Some(rg) = data.row_groups {
        meta_rows.push(vec![
            cell("Row Groups / Batches", ValueKind::Attribute),
            cell(rg.to_string(), ValueKind::Number),
        ]);
    }
    if let Some(cb) = &data.created_by {
        meta_rows.push(vec![
            cell("Created By", ValueKind::Attribute),
            cell(cb, ValueKind::String),
        ]);
    }
    blocks.push(PreviewBlock::Table {
        title: Some("Dataset Metadata".into()),
        depth: 0,
        headers: vec!["Property".into(), "Value".into()],
        rows: meta_rows,
    });

    if !data.schema.is_empty() {
        let schema_rows = data
            .schema
            .iter()
            .map(|col| {
                push_field(
                    &mut fields,
                    &col.name,
                    &format!("/schema/{}", col.name),
                    &col.data_type,
                );
                vec![
                    cell(&col.name, ValueKind::Attribute),
                    cell(&col.data_type, ValueKind::String),
                    cell(
                        if col.nullable { "true" } else { "false" },
                        ValueKind::Boolean,
                    ),
                ]
            })
            .collect();
        blocks.push(PreviewBlock::Table {
            title: Some(format!("Schema ({} columns)", data.schema.len())),
            depth: 0,
            headers: vec!["Column".into(), "Type".into(), "Nullable".into()],
            rows: schema_rows,
        });
    }

    if !data.rows.is_empty() {
        let preview_rows: Vec<Vec<TableCellValue>> = data
            .rows
            .iter()
            .enumerate()
            .map(|(r_idx, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c_idx, c)| {
                        let col_name = data
                            .headers
                            .get(c_idx)
                            .map(String::as_str)
                            .unwrap_or("col");
                        push_field(
                            &mut fields,
                            col_name,
                            &format!("/rows/{r_idx}/{col_name}"),
                            &c.text,
                        );
                        let kind = match c.kind {
                            ColumnarCellKind::String => ValueKind::String,
                            ColumnarCellKind::Number => ValueKind::Number,
                            ColumnarCellKind::Boolean => ValueKind::Boolean,
                            ColumnarCellKind::Null => ValueKind::Null,
                        };
                        cell(&c.text, kind)
                    })
                    .collect()
            })
            .collect();

        blocks.push(PreviewBlock::Table {
            title: Some(format!(
                "Head Rows ({} of {})",
                preview_rows.len(),
                data.total_rows
            )),
            depth: 0,
            headers: data.headers,
            rows: preview_rows,
        });
    }

    if data.truncated {
        let msg = format!(
            "Showing the first {} of {} rows.",
            data.rows.len(),
            data.total_rows
        );
        warnings.push(msg.clone());
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Info,
            title: "Rows truncated".into(),
            body: msg,
        });
    }

    let summary = visual::summarize_blocks(
        raw_len,
        fields.len(),
        &blocks,
        Some(data.format_label.into()),
        warnings,
    );
    (fields, summary, blocks)
}

// ============================================================================
// SQL (.sql)
// ============================================================================

pub fn read_sql(
    src: &str,
    raw_len: usize,
) -> Result<(Vec<Field>, PreviewSummary, Vec<PreviewBlock>), String> {
    let stmts = split_sql_statements(src);
    let mut fields = Vec::new();
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();

    let mut ddl_tables = 0usize;
    let mut ddl_views = 0usize;
    let mut queries = 0usize;
    let mut dml = 0usize;
    let mut other = 0usize;

    let mut table_blocks = Vec::new();
    let mut stmt_rows = Vec::new();

    for (idx, stmt) in stmts.iter().enumerate() {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        let upper = strip_leading_sql_comments(trimmed).to_ascii_uppercase();
        let kind = if upper.starts_with("CREATE TABLE")
            || upper.starts_with("CREATE TEMP TABLE")
            || upper.starts_with("CREATE TEMPORARY TABLE")
        {
            ddl_tables += 1;
            if let Some((table_name, cols)) = parse_create_table(trimmed) {
                push_field(
                    &mut fields,
                    &table_name,
                    &format!("/tables/{table_name}"),
                    &format!("{} columns", cols.len()),
                );
                let col_rows = cols
                    .iter()
                    .map(|(cname, ctype, ccons)| {
                        push_field(
                            &mut fields,
                            cname,
                            &format!("/tables/{table_name}/{cname}"),
                            ctype,
                        );
                        vec![
                            cell(cname, ValueKind::Attribute),
                            cell(ctype, ValueKind::String),
                            cell(ccons, ValueKind::String),
                        ]
                    })
                    .collect();
                table_blocks.push(PreviewBlock::Table {
                    title: Some(format!("Table: {table_name} ({} columns)", cols.len())),
                    depth: 0,
                    headers: vec!["Column".into(), "Type".into(), "Constraints".into()],
                    rows: col_rows,
                });
            }
            "CREATE TABLE"
        } else if upper.starts_with("CREATE VIEW") || upper.starts_with("CREATE OR REPLACE VIEW") {
            ddl_views += 1;
            "CREATE VIEW"
        } else if upper.starts_with("SELECT") || upper.starts_with("WITH") {
            queries += 1;
            "QUERY"
        } else if upper.starts_with("INSERT")
            || upper.starts_with("UPDATE")
            || upper.starts_with("DELETE")
            || upper.starts_with("MERGE")
        {
            dml += 1;
            "DML"
        } else {
            other += 1;
            "OTHER"
        };

        let snippet = collapse_ws(trimmed);
        let preview = if snippet.chars().count() > 140 {
            let s: String = snippet.chars().take(137).collect();
            format!("{s}...")
        } else {
            snippet
        };
        push_field(
            &mut fields,
            kind,
            &format!("/statements/{}", idx + 1),
            &preview,
        );
        if stmt_rows.len() < 60 {
            stmt_rows.push(vec![
                cell((idx + 1).to_string(), ValueKind::Number),
                cell(kind, ValueKind::Attribute),
                cell(preview, ValueKind::String),
            ]);
        }
    }

    blocks.push(PreviewBlock::Table {
        title: Some("SQL Script Summary".into()),
        depth: 0,
        headers: vec!["Category".into(), "Count".into()],
        rows: vec![
            vec![
                cell("Total Statements", ValueKind::Attribute),
                cell(stmts.len().to_string(), ValueKind::Number),
            ],
            vec![
                cell("CREATE TABLE", ValueKind::Attribute),
                cell(ddl_tables.to_string(), ValueKind::Number),
            ],
            vec![
                cell("CREATE VIEW", ValueKind::Attribute),
                cell(ddl_views.to_string(), ValueKind::Number),
            ],
            vec![
                cell("SELECT / WITH Queries", ValueKind::Attribute),
                cell(queries.to_string(), ValueKind::Number),
            ],
            vec![
                cell("DML (INSERT/UPDATE/DELETE)", ValueKind::Attribute),
                cell(dml.to_string(), ValueKind::Number),
            ],
            vec![
                cell("Other", ValueKind::Attribute),
                cell(other.to_string(), ValueKind::Number),
            ],
        ],
    });

    blocks.extend(table_blocks);

    if !stmt_rows.is_empty() {
        blocks.push(PreviewBlock::Table {
            title: Some(format!("Statements ({})", stmts.len())),
            depth: 0,
            headers: vec!["#".into(), "Kind".into(), "SQL".into()],
            rows: stmt_rows,
        });
    }

    if stmts.len() > 60 {
        let msg = format!("Showing the first 60 of {} SQL statements.", stmts.len());
        warnings.push(msg.clone());
        blocks.push(PreviewBlock::Callout {
            tone: CalloutTone::Info,
            title: "Statements truncated".into(),
            body: msg,
        });
    }

    let summary = visual::summarize_blocks(
        raw_len,
        fields.len(),
        &blocks,
        Some("SQL".into()),
        warnings,
    );
    Ok((fields, summary, blocks))
}

fn split_sql_statements(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut start = 0;
    let mut out = Vec::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < len {
        if in_line_comment {
            if bytes[i] == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_single {
            if bytes[i] == b'\'' {
                if i + 1 < len && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if bytes[i] == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if bytes[i] == b'\'' {
            in_single = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            in_double = true;
            i += 1;
            continue;
        }
        if bytes[i] == b';' {
            let chunk = src[start..i].trim();
            if !chunk.is_empty() && !strip_leading_sql_comments(chunk).is_empty() {
                out.push(chunk.to_string());
            }
            start = i + 1;
        }
        i += 1;
    }
    let tail = src[start..].trim();
    if !tail.is_empty() && !strip_leading_sql_comments(tail).is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn strip_leading_sql_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("--") {
            if let Some(pos) = rest.find('\n') {
                s = &rest[pos + 1..];
                continue;
            }
            return "";
        }
        if let Some(rest) = s.strip_prefix("/*") {
            if let Some(pos) = rest.find("*/") {
                s = &rest[pos + 2..];
                continue;
            }
            return "";
        }
        return s;
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_create_table(stmt: &str) -> Option<(String, Vec<(String, String, String)>)> {
    let clean = strip_leading_sql_comments(stmt);
    let open_paren = clean.find('(')?;
    let header = clean[..open_paren].trim();
    let table_name = header
        .split_whitespace()
        .last()?
        .trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
        .to_string();

    // Find matching closing paren for the column list
    let after = &clean[open_paren + 1..];
    let mut depth = 1usize;
    let mut end = after.len();
    for (idx, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = idx;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &after[..end];
    let items = split_top_level_commas(body);
    let mut cols = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let upper = item.to_ascii_uppercase();
        if upper.starts_with("PRIMARY KEY")
            || upper.starts_with("FOREIGN KEY")
            || upper.starts_with("UNIQUE")
            || upper.starts_with("CONSTRAINT")
            || upper.starts_with("CHECK")
        {
            continue;
        }
        let mut parts = item.split_whitespace();
        let Some(raw_name) = parts.next() else {
            continue;
        };
        let cname = raw_name
            .trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
            .to_string();
        let rest: Vec<&str> = parts.collect();
        let ctype = rest.first().copied().unwrap_or("").to_string();
        let ccons = if rest.len() > 1 {
            rest[1..].join(" ")
        } else {
            String::new()
        };
        cols.push((cname, ctype, ccons));
    }
    Some((table_name, cols))
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut start = 0usize;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if in_single {
            if bytes[i] == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        match bytes[i] {
            b'\'' => in_single = true,
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start <= s.len() {
        out.push(&s[start..]);
    }
    out
}

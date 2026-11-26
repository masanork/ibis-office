// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! General-purpose JSON extraction and table expansion functions (`SORANE.JSON*`).

use std::collections::HashSet;

use super::{array_value, err, to_bool};
use crate::eval::{Ctx, Value, num};
use crate::formula::Expr;
use crate::value::CellError;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSeg {
    Key(String),
    Index(usize),
}

fn parse_path(raw: &str) -> Result<Vec<PathSeg>, CellError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "$" {
        return Ok(Vec::new());
    }
    let s = trimmed.strip_prefix("$.").or_else(|| trimmed.strip_prefix('$')).unwrap_or(trimmed);
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut segs = Vec::new();

    while i < bytes.len() {
        if bytes[i] == b'.' {
            i += 1;
            if i >= bytes.len() {
                return Err(CellError::Value);
            }
            continue;
        }
        if bytes[i] == b'[' {
            i += 1;
            if i >= bytes.len() {
                return Err(CellError::Value);
            }
            if bytes[i] == b'"' || bytes[i] == b'\'' {
                let quote = bytes[i];
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i + 1 >= bytes.len() || bytes[i + 1] != b']' {
                    return Err(CellError::Value);
                }
                segs.push(PathSeg::Key(s[start..i].to_string()));
                i += 2;
            } else {
                let start = i;
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(CellError::Value);
                }
                let idx: usize = s[start..i].parse().map_err(|_| CellError::Value)?;
                segs.push(PathSeg::Index(idx));
                i += 1;
            }
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b'.' && bytes[i] != b'[' {
                i += 1;
            }
            let key = &s[start..i];
            if key.is_empty() {
                return Err(CellError::Value);
            }
            segs.push(PathSeg::Key(key.to_string()));
        }
    }

    Ok(segs)
}

fn resolve_path<'a>(mut current: &'a serde_json::Value, segs: &[PathSeg]) -> Option<&'a serde_json::Value> {
    for seg in segs {
        current = match seg {
            PathSeg::Key(k) => current.as_object()?.get(k)?,
            PathSeg::Index(idx) => current.as_array()?.get(*idx)?,
        };
    }
    Some(current)
}

fn json_to_cell_value(val: &serde_json::Value, default: &Value) -> Value {
    match val {
        serde_json::Value::Null => default.clone(),
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => n
            .as_f64()
            .filter(|f| f.is_finite())
            .map(num)
            .unwrap_or_else(|| Value::Text(n.to_string().into())),
        serde_json::Value::String(s) => Value::Text(s.as_str().into()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Value::Text(serde_json::to_string(val).unwrap_or_default().into())
        }
    }
}

fn empty_text() -> Value {
    Value::Text("".into())
}

pub(super) fn json_value(ctx: &mut Ctx, args: &[Expr]) -> Value {
    let args: Vec<_> = args.iter().map(|arg| ctx.eval_scalar_or_array(arg)).collect();
    ctx.map_values(args, &mut |_, values| {
        if let Some(error) = values.iter().take(2).find_map(|v| match v {
            Value::Error(e) => Some(*e),
            _ => None,
        }) {
            return err(error);
        }
        let default = match values.get(2) {
            None | Some(Value::Empty) => empty_text(),
            Some(Value::Error(e)) => return err(*e),
            Some(other) => other.clone(),
        };

        let json_str = match &values[0] {
            Value::Empty => return default,
            Value::Text(t) if t.trim().is_empty() => return default,
            Value::Text(t) => t.as_ref(),
            _ => return err(CellError::Value),
        };
        let path_str = match &values[1] {
            Value::Empty => "",
            Value::Text(t) => t.as_ref(),
            _ => return err(CellError::Value),
        };
        let segs = match parse_path(path_str) {
            Ok(segs) => segs,
            Err(e) => return err(e),
        };
        let parsed: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(_) => return err(CellError::Value),
        };
        match resolve_path(&parsed, &segs) {
            Some(found) => json_to_cell_value(found, &default),
            None => default,
        }
    })
}

fn extract_paths(paths_val: &Value) -> Result<Vec<(String, Vec<PathSeg>)>, CellError> {
    match paths_val {
        Value::Error(e) => Err(*e),
        Value::Text(s) => Ok(vec![(s.to_string(), parse_path(s)?)]),
        Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.data.len());
            for item in &arr.data {
                match item {
                    Value::Error(e) => return Err(*e),
                    Value::Text(s) => out.push((s.to_string(), parse_path(s)?)),
                    _ => return Err(CellError::Value),
                }
            }
            if out.is_empty() {
                return Err(CellError::Value);
            }
            Ok(out)
        }
        _ => Err(CellError::Value),
    }
}

pub(super) fn json_table(ctx: &mut Ctx, args: &[Expr]) -> Value {
    let source = ctx.eval_array(&args[0]);
    if let Value::Error(e) = source {
        return err(e);
    }
    let paths_val = ctx.eval_array(&args[1]);
    let paths = match extract_paths(&paths_val) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let include_header = match args.get(2).map(|a| ctx.eval_scalar_or_array(a)) {
        None | Some(Value::Empty) => false,
        Some(Value::Error(e)) => return err(e),
        Some(other) => match to_bool(&other) {
            Ok(b) => b,
            Err(e) => return err(e),
        },
    };

    let cols = paths.len();
    let mut rows_data: Vec<Value> = Vec::new();
    let mut row_count = 0usize;
    let default = empty_text();

    if include_header {
        for (raw, _) in &paths {
            rows_data.push(Value::Text(raw.as_str().into()));
        }
        row_count += 1;
    }

    let push_object_row = |obj: &serde_json::Value, rows_data: &mut Vec<Value>, row_count: &mut usize| {
        for (_, segs) in &paths {
            match resolve_path(obj, segs) {
                Some(found) => rows_data.push(json_to_cell_value(found, &default)),
                None => rows_data.push(default.clone()),
            }
        }
        *row_count += 1;
    };

    match &source {
        Value::Empty => {
            if !include_header {
                rows_data.extend(std::iter::repeat_n(default, cols));
                row_count += 1;
            }
        }
        Value::Text(s) if s.trim().is_empty() => {
            if !include_header {
                rows_data.extend(std::iter::repeat_n(default, cols));
                row_count += 1;
            }
        }
        Value::Text(s) => {
            let parsed: serde_json::Value = match serde_json::from_str(s) {
                Ok(v) => v,
                Err(_) => return err(CellError::Value),
            };
            match &parsed {
                serde_json::Value::Array(items) => {
                    for item in items {
                        push_object_row(item, &mut rows_data, &mut row_count);
                    }
                }
                serde_json::Value::Object(_) => {
                    push_object_row(&parsed, &mut rows_data, &mut row_count);
                }
                _ => return err(CellError::Value),
            }
        }
        Value::Array(arr) => {
            for cell in &arr.data {
                match cell {
                    Value::Error(e) => return err(*e),
                    Value::Empty => {
                        rows_data.extend(std::iter::repeat_n(default.clone(), cols));
                        row_count += 1;
                    }
                    Value::Text(s) if s.trim().is_empty() => {
                        rows_data.extend(std::iter::repeat_n(default.clone(), cols));
                        row_count += 1;
                    }
                    Value::Text(s) => {
                        let parsed: serde_json::Value = match serde_json::from_str(s) {
                            Ok(v) => v,
                            Err(_) => return err(CellError::Value),
                        };
                        push_object_row(&parsed, &mut rows_data, &mut row_count);
                    }
                    _ => return err(CellError::Value),
                }
            }
        }
        _ => return err(CellError::Value),
    }

    array_value(row_count, cols, rows_data)
}

fn collect_keys(
    val: &serde_json::Value,
    prefix: &str,
    depth: usize,
    max_depth: usize,
    seen: &mut HashSet<String>,
    out: &mut Vec<String>,
) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                let is_container = matches!(v, serde_json::Value::Object(m) if !m.is_empty());
                if depth < max_depth && is_container {
                    collect_keys(v, &next, depth + 1, max_depth, seen, out);
                } else if seen.insert(next.clone()) {
                    out.push(next);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_keys(item, prefix, depth, max_depth, seen, out);
            }
        }
        _ => {}
    }
}

pub(super) fn json_keys(ctx: &mut Ctx, args: &[Expr]) -> Value {
    let source = ctx.eval_array(&args[0]);
    if let Value::Error(e) = source {
        return err(e);
    }
    let max_depth = match args.get(1).map(|a| ctx.eval_scalar_or_array(a)) {
        None | Some(Value::Empty) => 2usize,
        Some(Value::Error(e)) => return err(e),
        Some(other) => match super::to_number(&other, ctx.date1904()) {
            Ok(n) if n >= 1.0 && n.is_finite() => n.trunc() as usize,
            Ok(_) => return err(CellError::Value),
            Err(e) => return err(e),
        },
    };

    let mut seen = HashSet::new();
    let mut keys = Vec::new();

    let scan_str = |s: &str, seen: &mut HashSet<String>, keys: &mut Vec<String>| -> Result<(), CellError> {
        if s.trim().is_empty() {
            return Ok(());
        }
        let parsed: serde_json::Value = serde_json::from_str(s).map_err(|_| CellError::Value)?;
        collect_keys(&parsed, "", 1, max_depth, seen, keys);
        Ok(())
    };

    match &source {
        Value::Empty => {}
        Value::Text(s) => {
            if let Err(e) = scan_str(s, &mut seen, &mut keys) {
                return err(e);
            }
        }
        Value::Array(arr) => {
            for cell in &arr.data {
                match cell {
                    Value::Error(e) => return err(*e),
                    Value::Empty => {}
                    Value::Text(s) => {
                        if let Err(e) = scan_str(s, &mut seen, &mut keys) {
                            return err(e);
                        }
                    }
                    _ => return err(CellError::Value),
                }
            }
        }
        _ => return err(CellError::Value),
    }

    if keys.is_empty() {
        return err(CellError::Calc);
    }
    let count = keys.len();
    array_value(count, 1, keys.into_iter().map(|k| Value::Text(k.into())).collect())
}

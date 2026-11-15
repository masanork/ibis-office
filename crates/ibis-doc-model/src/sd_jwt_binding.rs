// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Convention-based binding between worksheet `DefinedName`s / `FormControl`s and JSON claims
//! for SD-JWT (RFC 9901 / SD-JWT VC) generation and OID4VP selective disclosure.
//!
//! Naming conventions (shared by `DefinedName.name` and `FormControl.name`):
//! - `claim.<path>`: always-disclosed plain claim at `<path>` (dot-separated object path).
//! - `sd.<path>`: selectively disclosable claim at `<path>` (`_sd` digest target).
//! - `disclose.<path>`: boolean disclosure toggle for `<path>` (defaults to `true` if absent).

use std::collections::{BTreeMap, BTreeSet};

use crate::command::Command;
use crate::form_control::{ControlKind, FormControl, radio_group, resolve};
use crate::model::{Presentation, ShapeId, ShapeKind, SlideId};
use crate::sheet::{CellRange, CellValue, Sheet};
use crate::sheet_calc::sheet_of_slide;
use crate::sheet_command::{CellEdit, CellInput, SheetCommand};

/// JSON value representation for bound worksheet claims.
#[derive(Clone, Debug)]
pub enum ClaimValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<ClaimValue>),
    Object(BTreeMap<String, ClaimValue>),
}

impl PartialEq for ClaimValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Int(a), Self::Float(b)) | (Self::Float(b), Self::Int(a)) => (*a as f64) == *b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ClaimValue {}

impl ClaimValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(*n as f64),
            Self::Float(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, ClaimValue>> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[ClaimValue]> {
        match self {
            Self::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Parses an RFC 8259 JSON document into a `ClaimValue`.
    pub fn parse_json(input: &str) -> Result<Self, String> {
        let mut parser = JsonParser {
            bytes: input.as_bytes(),
            pos: 0,
        };
        let value = parser.parse_value()?;
        parser.skip_ws();
        if parser.pos < parser.bytes.len() {
            return Err(format!("unexpected trailing characters at byte {}", parser.pos));
        }
        Ok(value)
    }

    /// Compact, deterministic JSON serialization (RFC 8259).
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out, None, 0);
        out
    }

    /// Pretty-printed 2-space indented JSON serialization.
    pub fn to_json_pretty(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out, Some(2), 0);
        out
    }

    fn write_json(&self, out: &mut String, indent: Option<usize>, depth: usize) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Int(n) => out.push_str(&n.to_string()),
            Self::Float(n) => {
                if n.is_finite() {
                    out.push_str(&ibis_calc::number_to_xml(*n));
                } else {
                    out.push_str("null");
                }
            }
            Self::String(s) => write_escaped_json_string(out, s),
            Self::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if let Some(step) = indent {
                        out.push('\n');
                        for _ in 0..(depth + 1) * step {
                            out.push(' ');
                        }
                    }
                    item.write_json(out, indent, depth + 1);
                }
                if let Some(step) = indent {
                    out.push('\n');
                    for _ in 0..depth * step {
                        out.push(' ');
                    }
                }
                out.push(']');
            }
            Self::Object(map) => {
                if map.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if let Some(step) = indent {
                        out.push('\n');
                        for _ in 0..(depth + 1) * step {
                            out.push(' ');
                        }
                    }
                    write_escaped_json_string(out, k);
                    out.push(':');
                    if indent.is_some() {
                        out.push(' ');
                    }
                    v.write_json(out, indent, depth + 1);
                }
                if let Some(step) = indent {
                    out.push('\n');
                    for _ in 0..depth * step {
                        out.push(' ');
                    }
                }
                out.push('}');
            }
        }
    }
}

fn write_escaped_json_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Result<u8, String> {
        let b = self
            .bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| "unexpected end of JSON".to_string())?;
        self.pos += 1;
        Ok(b)
    }

    fn parse_value(&mut self) -> Result<ClaimValue, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'n') => self.parse_literal(b"null", ClaimValue::Null),
            Some(b't') => self.parse_literal(b"true", ClaimValue::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", ClaimValue::Bool(false)),
            Some(b'"') => self.parse_string().map(ClaimValue::String),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(b) => Err(format!("unexpected byte '{}' at {}", b as char, self.pos)),
            None => Err("unexpected end of JSON".to_string()),
        }
    }

    fn parse_literal(&mut self, expected: &[u8], val: ClaimValue) -> Result<ClaimValue, String> {
        if self.bytes[self.pos..].starts_with(expected) {
            self.pos += expected.len();
            Ok(val)
        } else {
            Err(format!("invalid literal at byte {}", self.pos))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.next_byte()? != b'"' {
            return Err("expected '\"'".to_string());
        }
        let mut out = String::new();
        loop {
            let b = self.next_byte()?;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.next_byte()?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\x08'),
                        b'f' => out.push('\x0C'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&code)
                                && self.bytes[self.pos..].starts_with(b"\\u")
                            {
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let cp = 0x10000
                                        + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                    let ch = char::from_u32(cp)
                                        .ok_or_else(|| "invalid surrogate pair".to_string())?;
                                    out.push(ch);
                                    continue;
                                }
                                return Err("invalid low surrogate".to_string());
                            }
                            let ch = char::from_u32(code as u32)
                                .ok_or_else(|| "invalid unicode escape".to_string())?;
                            out.push(ch);
                        }
                        _ => return Err(format!("invalid escape '\\{}'", esc as char)),
                    }
                }
                c if c < 0x20 => return Err("unescaped control character in string".to_string()),
                _ => {
                    // Decode UTF-8 sequence starting at self.pos - 1
                    let start = self.pos - 1;
                    let s = std::str::from_utf8(&self.bytes[start..])
                        .map_err(|_| "invalid UTF-8 in JSON string".to_string())?;
                    let ch = s.chars().next().ok_or_else(|| "unexpected EOF".to_string())?;
                    out.push(ch);
                    self.pos = start + ch.len_utf8();
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        let mut val: u16 = 0;
        for _ in 0..4 {
            let b = self.next_byte()?;
            let digit = match b {
                b'0'..=b'9' => (b - b'0') as u16,
                b'a'..=b'f' => (b - b'a' + 10) as u16,
                b'A'..=b'F' => (b - b'A' + 10) as u16,
                _ => return Err("invalid hex digit in \\u escape".to_string()),
            };
            val = (val << 4) | digit;
        }
        Ok(val)
    }

    fn parse_number(&mut self) -> Result<ClaimValue, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let mut is_float = false;
        let digits_start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(format!("invalid number at byte {start}"));
        }
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            let frac_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(format!("invalid fraction at byte {start}"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == exp_start {
                return Err(format!("invalid exponent at byte {start}"));
            }
        }
        let num_str = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "invalid UTF-8 in number".to_string())?;
        if !is_float {
            if let Ok(i) = num_str.parse::<i64>() {
                return Ok(ClaimValue::Int(i));
            }
        }
        let f = num_str
            .parse::<f64>()
            .map_err(|_| format!("invalid float '{num_str}'"))?;
        Ok(ClaimValue::Float(f))
    }

    fn parse_array(&mut self) -> Result<ClaimValue, String> {
        self.next_byte()?; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(ClaimValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.next_byte()? {
                b',' => continue,
                b']' => return Ok(ClaimValue::Array(items)),
                other => return Err(format!("expected ',' or ']', got '{}'", other as char)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<ClaimValue, String> {
        self.next_byte()?; // '{'
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(ClaimValue::Object(map));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.next_byte()? != b':' {
                return Err("expected ':' after object key".to_string());
            }
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.next_byte()? {
                b',' => continue,
                b'}' => return Ok(ClaimValue::Object(map)),
                other => return Err(format!("expected ',' or '}}', got '{}'", other as char)),
            }
        }
    }
}

/// Where a claim or disclosure toggle was bound from on the worksheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingSource {
    Cell { row: u32, column: u32 },
    Control { shape: ShapeId },
}

/// A single resolved claim (`claim.<path>` or `sd.<path>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundClaim {
    /// Dot-split path segments (e.g. `["address", "locality"]`).
    pub path: Vec<String>,
    /// Canonical dot-joined path (e.g. `"address.locality"`).
    pub path_str: String,
    /// `false` for `claim.<path>` (always in payload), `true` for `sd.<path>` (selective disclosure).
    pub selective: bool,
    /// Resolved JSON value of the claim.
    pub value: ClaimValue,
    /// Whether this claim is currently selected for disclosure (`true` for all `claim.*`,
    /// and resolved from `disclose.<path>` with default `true` for `sd.*`).
    pub disclose: bool,
    /// Worksheet source of this claim.
    pub source: BindingSource,
}

/// A `disclose.<path>` toggle bound to a CheckBox form control or an in-cell checkbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscloseBinding {
    pub path: Vec<String>,
    pub path_str: String,
    pub enabled: bool,
    pub source: BindingSource,
    /// If `source` is a `Control` with a `fmlaLink`, the resolved `(slide, row, col)`.
    pub linked_cell: Option<(SlideId, u32, u32)>,
}

/// All convention-based SD-JWT bindings extracted from a worksheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetClaimBindings {
    pub slide: SlideId,
    pub claims: Vec<BoundClaim>,
    pub disclosures: BTreeMap<String, DiscloseBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrefixKind {
    Claim,
    Selective,
    Disclose,
}

fn parse_binding_name(raw_name: &str) -> Option<(PrefixKind, Vec<String>, String)> {
    let trimmed = raw_name.trim();
    let (kind, rest) = if let Some(r) = strip_ascii_prefix_ignore_case(trimmed, "claim.") {
        (PrefixKind::Claim, r)
    } else if let Some(r) = strip_ascii_prefix_ignore_case(trimmed, "sd.") {
        (PrefixKind::Selective, r)
    } else if let Some(r) = strip_ascii_prefix_ignore_case(trimmed, "disclose.") {
        (PrefixKind::Disclose, r)
    } else {
        return None;
    };
    let segments: Vec<String> = rest.split('.').map(str::trim).map(str::to_string).collect();
    if segments.is_empty() || segments.iter().any(|s| s.is_empty()) {
        return None;
    }
    let path_str = segments.join(".");
    Some((kind, segments, path_str))
}

fn strip_ascii_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn sheet_of(presentation: &Presentation, slide: SlideId) -> Option<&Sheet> {
    presentation.slide(slide).and_then(sheet_of_slide).map(|s| &**s)
}

fn resolve_sheet<'a>(presentation: &'a Presentation, own: SlideId, text: &str) -> Option<(&'a Sheet, CellRange)> {
    use ibis_calc::formula::parser::{Expr, parse};
    let Expr::Ref(r) = parse(text.trim().trim_start_matches('=')).ok()? else { return None };
    let sheet = match &r.sheet {
        None => sheet_of(presentation, own)?,
        Some(s) => {
            let w = presentation.workbook.as_ref()?;
            let ws = w.sheets.iter().find(|ws| ws.name.eq_ignore_ascii_case(&s.first))?;
            ws.slide.and_then(|id| sheet_of(presentation, id)).or(ws.hidden.as_deref())?
        }
    };
    Some((
        sheet,
        CellRange {
            first_row: r.start.row.min(r.end.row),
            first_column: r.start.column.min(r.end.column),
            last_row: r.start.row.max(r.end.row),
            last_column: r.start.column.max(r.end.column),
        },
    ))
}

fn number_of(v: &CellValue) -> Option<f64> {
    match v {
        CellValue::Number(n) => Some(*n),
        CellValue::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

fn control_items(presentation: &Presentation, slide_id: SlideId, c: &FormControl) -> Vec<String> {
    if !c.items.is_empty() {
        return c.items.clone();
    }
    let Some((sheet, range)) = c.range.as_deref().and_then(|r| resolve_sheet(presentation, slide_id, r)) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for row in range.first_row..=range.last_row.min(range.first_row + 255) {
        for col in range.first_column..=range.last_column.min(range.first_column + 255) {
            items.push(sheet.cell(row, col).map(|cell| cell.text.clone()).unwrap_or_default());
        }
    }
    items
}

fn control_selected_index(presentation: &Presentation, slide_id: SlideId, c: &FormControl) -> i64 {
    if let Some((link_sheet, range)) = c.link.as_deref().and_then(|l| resolve_sheet(presentation, slide_id, l)) {
        if let Some(cell) = link_sheet.cell(range.first_row, range.first_column) {
            if let Some(n) = number_of(&cell.value) {
                return (n as i64).max(0);
            }
        }
    }
    c.value.max(0)
}

fn control_checkbox_bool(presentation: &Presentation, slide_id: SlideId, c: &FormControl) -> bool {
    if let Some((link_sheet, range)) = c.link.as_deref().and_then(|l| resolve_sheet(presentation, slide_id, l)) {
        if let Some(cell) = link_sheet.cell(range.first_row, range.first_column) {
            match &cell.value {
                CellValue::Bool(b) => return *b,
                CellValue::Number(n) => return *n != 0.0,
                _ => {}
            }
        }
    }
    c.checked == 1
}

fn resolve_control_value(
    presentation: &Presentation,
    slide: &crate::model::Slide,
    shape_index: usize,
    c: &FormControl,
) -> Option<ClaimValue> {
    match c.kind {
        ControlKind::Drop | ControlKind::List => {
            let idx = control_selected_index(presentation, slide.id, c);
            if idx <= 0 {
                return None;
            }
            let items = control_items(presentation, slide.id, c);
            let text = items.get((idx - 1) as usize)?.trim();
            if text.is_empty() {
                None
            } else {
                Some(ClaimValue::String(text.to_string()))
            }
        }
        ControlKind::CheckBox => Some(ClaimValue::Bool(control_checkbox_bool(presentation, slide.id, c))),
        ControlKind::Spin | ControlKind::Scroll => {
            let val = if let Some((link_sheet, range)) =
                c.link.as_deref().and_then(|l| resolve_sheet(presentation, slide.id, l))
            {
                link_sheet
                    .cell(range.first_row, range.first_column)
                    .and_then(|cell| number_of(&cell.value))
                    .map(|n| (n as i64).clamp(c.min, c.max))
                    .unwrap_or(c.value)
            } else {
                c.value
            };
            Some(ClaimValue::Int(val))
        }
        ControlKind::Radio => {
            let group = radio_group(slide, shape_index);
            if group.is_empty() {
                return None;
            }
            // Determine chosen 1-based position from the first button's linked cell, if any.
            let linked_pos = group
                .first()
                .and_then(|&first_idx| match &slide.shapes[first_idx].kind {
                    ShapeKind::Control(first) => first.link.as_deref(),
                    _ => None,
                })
                .and_then(|l| resolve_sheet(presentation, slide.id, l))
                .and_then(|(sh, r)| sh.cell(r.first_row, r.first_column))
                .and_then(|cell| number_of(&cell.value))
                .map(|n| n as usize);

            for (pos0, &idx) in group.iter().enumerate() {
                let ShapeKind::Control(rc) = &slide.shapes[idx].kind else { continue };
                let chosen = match linked_pos {
                    Some(p) => p == pos0 + 1,
                    None => rc.checked == 1,
                };
                if chosen {
                    let label = rc.text.trim();
                    if !label.is_empty() {
                        return Some(ClaimValue::String(label.to_string()));
                    }
                }
            }
            None
        }
        ControlKind::Button | ControlKind::GroupBox | ControlKind::Label => None,
    }
}

fn resolve_cell_value(
    presentation: &Presentation,
    slide: &crate::model::Slide,
    sheet: &Sheet,
    row: u32,
    column: u32,
    leaf_name: &str,
    date1904: bool,
) -> Option<ClaimValue> {
    // 1. Check if a Drop / List or Radio group on the same sheet links (`fmlaLink`) to this cell.
    for (idx, shape) in slide.shapes.iter().enumerate() {
        let ShapeKind::Control(c) = &shape.kind else { continue };
        let Some((link_slide, range)) = c.link.as_deref().and_then(|l| resolve(presentation, slide.id, l)) else {
            continue;
        };
        if link_slide == slide.id && range.first_row == row && range.first_column == column {
            if matches!(c.kind, ControlKind::Drop | ControlKind::List | ControlKind::Radio) {
                return resolve_control_value(presentation, slide, idx, c);
            }
        }
    }

    let Some(cell) = sheet.cell(row, column) else {
        return None;
    };
    let style = sheet.styles.get(cell.style as usize);
    let is_checkbox_style = style.is_some_and(|st| st.checkbox);
    let is_text_style = style.is_some_and(|st| st.number_format.trim() == "@");
    let is_date_style = style.is_some_and(|st| ibis_calc::numfmt::is_date_format(&st.number_format));

    match &cell.value {
        CellValue::Empty => {
            if is_checkbox_style {
                Some(ClaimValue::Bool(false))
            } else {
                None
            }
        }
        CellValue::Bool(b) => {
            if is_text_style && !cell.text.is_empty() {
                Some(ClaimValue::String(cell.text.clone()))
            } else {
                Some(ClaimValue::Bool(*b))
            }
        }
        CellValue::Number(n) => {
            if is_checkbox_style {
                return Some(ClaimValue::Bool(*n != 0.0));
            }
            if is_text_style {
                let s = if !cell.text.is_empty() {
                    cell.text.clone()
                } else {
                    ibis_calc::number_to_xml(*n)
                };
                return Some(ClaimValue::String(s));
            }
            if matches!(leaf_name, "iat" | "exp" | "nbf") {
                return Some(ClaimValue::Int(n.round() as i64));
            }
            if is_date_style {
                let (y, m, d) = ibis_calc::eval::convert::date_from_serial(n.floor() as i64, date1904);
                return Some(ClaimValue::String(format!("{y:04}-{m:02}-{d:02}")));
            }
            if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                Some(ClaimValue::Int(*n as i64))
            } else {
                Some(ClaimValue::Float(*n))
            }
        }
        CellValue::Text(s) => {
            if s.is_empty() {
                None
            } else {
                Some(ClaimValue::String(s.clone()))
            }
        }
        CellValue::Error(_) => None,
    }
}

/// Resolves a `CellRange` into a `ClaimValue`:
/// - Single cell -> scalar `ClaimValue` (`String`, `Int`, `Float`, `Bool`)
/// - 1D range (single column or single row with >1 cell) -> `ClaimValue::Array` of non-empty values
/// - 2D range (multiple rows and columns) -> `ClaimValue::Array` of `ClaimValue::Object`s using the first row as field headers (e.g. ISO 18013-5 `driving_privileges`).
fn resolve_range_value(
    presentation: &Presentation,
    slide: & crate::model::Slide,
    sheet: &Sheet,
    range: CellRange,
    leaf_name: &str,
    date1904: bool,
) -> Option<ClaimValue> {
    let r0 = range.first_row.min(range.last_row);
    let r1 = range.first_row.max(range.last_row).min(r0 + 255);
    let c0 = range.first_column.min(range.last_column);
    let c1 = range.first_column.max(range.last_column).min(c0 + 63);

    if r0 == r1 && c0 == c1 {
        return resolve_cell_value(presentation, slide, sheet, r0, c0, leaf_name, date1904);
    }

    if r0 == r1 || c0 == c1 {
        // 1D range -> primitive array
        let mut items = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                if let Some(val) = resolve_cell_value(presentation, slide, sheet, r, c, leaf_name, date1904) {
                    items.push(val);
                }
            }
        }
        return (!items.is_empty()).then_some(ClaimValue::Array(items));
    }

    // 2D table range -> first row is header keys, subsequent rows are objects
    let mut headers: Vec<(u32, String)> = Vec::new();
    for c in c0..=c1 {
        if let Some(cell) = sheet.cell(r0, c) {
            let key = match &cell.value {
                CellValue::Text(s) => s.trim().to_string(),
                _ => cell.text.trim().to_string(),
            };
            if !key.is_empty() {
                headers.push((c, key));
            }
        }
    }
    if headers.is_empty() {
        return None;
    }

    let mut rows = Vec::new();
    for r in (r0 + 1)..=r1 {
        let mut obj = BTreeMap::new();
        for (c, key) in &headers {
            if let Some(val) = resolve_cell_value(presentation, slide, sheet, r, *c, key, date1904) {
                obj.insert(key.clone(), val);
            }
        }
        if !obj.is_empty() {
            rows.push(ClaimValue::Object(obj));
        }
    }
    (!rows.is_empty()).then_some(ClaimValue::Array(rows))
}

/// Extracts all `claim.*`, `sd.*`, and `disclose.*` bindings from worksheet `slide_id`.
pub fn extract_sheet_claims(presentation: &Presentation, slide_id: SlideId) -> Option<SheetClaimBindings> {
    let slide = presentation.slide(slide_id)?;
    let sheet = sheet_of(presentation, slide_id)?;
    let date1904 = presentation.workbook.as_ref().is_some_and(|w| w.date1904);

    let mut claim_map: BTreeMap<String, BoundClaim> = BTreeMap::new();
    let mut disclosures: BTreeMap<String, DiscloseBinding> = BTreeMap::new();

    // 1. Scan Workbook DefinedNames (global first, then sheet-local so sheet-local overrides global).
    if let Some(wb) = presentation.workbook.as_ref() {
        let target_sheet_idx = wb.sheets.iter().position(|ws| ws.slide == Some(slide_id));
        for local_pass in [false, true] {
            for dn in &wb.names {
                if dn.hidden {
                    continue;
                }
                match (local_pass, dn.sheet) {
                    (false, None) => {}
                    (true, Some(idx)) if Some(idx) == target_sheet_idx => {}
                    _ => continue,
                }
                let Some((prefix_kind, path, path_str)) = parse_binding_name(&dn.name) else {
                    continue;
                };
                let Some((resolved_slide, range)) = resolve(presentation, slide_id, &dn.formula) else {
                    continue;
                };
                if resolved_slide != slide_id {
                    continue;
                }
                let row = range.first_row;
                let column = range.first_column;
                let source = BindingSource::Cell { row, column };
                let leaf = path.last().map(String::as_str).unwrap_or("");
                match prefix_kind {
                    PrefixKind::Claim | PrefixKind::Selective => {
                        if let Some(value) = resolve_range_value(presentation, slide, sheet, range, leaf, date1904) {
                            claim_map.insert(
                                path_str.clone(),
                                BoundClaim {
                                    path,
                                    path_str,
                                    selective: prefix_kind == PrefixKind::Selective,
                                    value,
                                    disclose: true,
                                    source,
                                },
                            );
                        }
                    }
                    PrefixKind::Disclose => {
                        let enabled = match sheet.cell(row, column).map(|c| &c.value) {
                            Some(CellValue::Bool(b)) => *b,
                            Some(CellValue::Number(n)) => *n != 0.0,
                            _ => false,
                        };
                        disclosures.insert(
                            path_str.clone(),
                            DiscloseBinding {
                                path,
                                path_str,
                                enabled,
                                source,
                                linked_cell: Some((slide_id, row, column)),
                            },
                        );
                    }
                }
            }
        }
    }

    // 2. Scan FormControls on the worksheet.
    for (idx, shape) in slide.shapes.iter().enumerate() {
        let ShapeKind::Control(c) = &shape.kind else { continue };
        let Some((prefix_kind, path, path_str)) = parse_binding_name(&c.name) else {
            continue;
        };
        let source = BindingSource::Control { shape: shape.id };
        match prefix_kind {
            PrefixKind::Claim | PrefixKind::Selective => {
                // For radio groups where multiple buttons share the same `sd.<path>` name,
                // `resolve_control_value` inspects the entire radio group at `idx`.
                if let Some(value) = resolve_control_value(presentation, slide, idx, c) {
                    claim_map.insert(
                        path_str.clone(),
                        BoundClaim {
                            path,
                            path_str,
                            selective: prefix_kind == PrefixKind::Selective,
                            value,
                            disclose: true,
                            source,
                        },
                    );
                }
            }
            PrefixKind::Disclose => {
                if c.kind == ControlKind::CheckBox {
                    let enabled = control_checkbox_bool(presentation, slide_id, c);
                    let linked_cell = c
                        .link
                        .as_deref()
                        .and_then(|l| resolve(presentation, slide_id, l))
                        .map(|(s, r)| (s, r.first_row, r.first_column));
                    disclosures.insert(
                        path_str.clone(),
                        DiscloseBinding {
                            path,
                            path_str,
                            enabled,
                            source,
                            linked_cell,
                        },
                    );
                }
            }
        }
    }

    // 3. Apply `disclose.<path>` states to `sd.<path>` claims (including parent path prefix checks).
    let mut claims: Vec<BoundClaim> = claim_map.into_values().collect();
    for claim in &mut claims {
        if claim.selective {
            claim.disclose = is_path_disclosed(&claim.path, &disclosures);
        } else {
            claim.disclose = true;
        }
    }

    Some(SheetClaimBindings {
        slide: slide_id,
        claims,
        disclosures,
    })
}

fn is_path_disclosed(path: &[String], disclosures: &BTreeMap<String, DiscloseBinding>) -> bool {
    // Check from full path up to top-level ancestor; if any `disclose.<prefix>` is explicitly false,
    // the claim is not disclosed. If none is defined, default is `true`.
    let mut prefix = String::new();
    for (i, seg) in path.iter().enumerate() {
        if i > 0 {
            prefix.push('.');
        }
        prefix.push_str(seg);
        if let Some(d) = disclosures.get(&prefix) {
            if !d.enabled {
                return false;
            }
        }
    }
    true
}

fn insert_nested_path(root: &mut BTreeMap<String, ClaimValue>, path: &[String], value: ClaimValue) {
    let Some((first, rest)) = path.split_first() else { return };
    if rest.is_empty() {
        root.insert(first.clone(), value);
        return;
    }
    let entry = root
        .entry(first.clone())
        .or_insert_with(|| ClaimValue::Object(BTreeMap::new()));
    if !matches!(entry, ClaimValue::Object(_)) {
        *entry = ClaimValue::Object(BTreeMap::new());
    }
    if let ClaimValue::Object(child_map) = entry {
        insert_nested_path(child_map, rest, value);
    }
}

impl SheetClaimBindings {
    /// Builds a plain JSON object containing all bound claims (`claim.*` and `sd.*`) regardless of disclosure toggles.
    pub fn to_plain_claims_json(&self) -> ClaimValue {
        let mut root = BTreeMap::new();
        for claim in &self.claims {
            insert_nested_path(&mut root, &claim.path, claim.value.clone());
        }
        ClaimValue::Object(root)
    }

    /// Builds a JSON object containing always-disclosed `claim.*` claims plus only those `sd.*` claims
    /// whose `disclose` state is currently `true`.
    pub fn to_disclosed_claims_json(&self) -> ClaimValue {
        let mut root = BTreeMap::new();
        for claim in &self.claims {
            if !claim.selective || claim.disclose {
                insert_nested_path(&mut root, &claim.path, claim.value.clone());
            }
        }
        ClaimValue::Object(root)
    }

    /// Looks up a bound claim by its dot-separated path (e.g. `"family_name"` or `"address.locality"`).
    pub fn claim(&self, path: &str) -> Option<&BoundClaim> {
        self.claims.iter().find(|c| c.path_str == path)
    }

    /// Generates document editing commands to synchronize `disclose.<path>` checkboxes on the sheet
    /// with a set of requested claim paths (e.g. from an OID4VP Authorization Request).
    ///
    /// When `exclusive` is `true`, `disclose.<path>` toggles not matching `requested_paths` are turned off;
    /// when `false`, matching toggles are turned on and others are left unchanged.
    pub fn set_disclose_commands(
        &self,
        presentation: &Presentation,
        requested_paths: &[&str],
        exclusive: bool,
    ) -> Vec<Command> {
        let requested: BTreeSet<&str> = requested_paths.iter().copied().collect();
        let Some(slide) = presentation.slide(self.slide) else {
            return Vec::new();
        };
        let mut commands = Vec::new();
        let mut cell_edits_by_slide: BTreeMap<SlideId, Vec<CellEdit>> = BTreeMap::new();

        for d in self.disclosures.values() {
            let matches_request = requested.iter().any(|req| {
                *req == d.path_str
                    || req
                        .strip_prefix(&d.path_str)
                        .is_some_and(|rest| rest.starts_with('.'))
            });
            let target_enabled = if matches_request {
                true
            } else if exclusive {
                false
            } else {
                d.enabled
            };
            if target_enabled == d.enabled {
                continue;
            }
            match d.source {
                BindingSource::Control { shape } => {
                    if let Some(sh) = slide.shape(shape)
                        && let ShapeKind::Control(ctrl) = &sh.kind
                    {
                        let mut next = (**ctrl).clone();
                        next.checked = u8::from(target_enabled);
                        commands.push(Command::Sheet(SheetCommand::EditControl {
                            slide: self.slide,
                            shape,
                            control: next,
                        }));
                    }
                    if let Some((link_slide, row, column)) = d.linked_cell {
                        cell_edits_by_slide.entry(link_slide).or_default().push(CellEdit {
                            row,
                            column,
                            input: CellInput::Value(CellValue::Bool(target_enabled)),
                        });
                    }
                }
                BindingSource::Cell { row, column } => {
                    cell_edits_by_slide.entry(self.slide).or_default().push(CellEdit {
                        row,
                        column,
                        input: CellInput::Value(CellValue::Bool(target_enabled)),
                    });
                }
            }
        }

        for (target_slide, edits) in cell_edits_by_slide {
            if !edits.is_empty() {
                commands.push(Command::Sheet(SheetCommand::SetCells {
                    slide: target_slide,
                    edits,
                }));
            }
        }
        commands
    }

    /// Toggles the disclosure state (`disclose.<path>`) of a single `sd.<path>` claim on the sheet.
    /// If `disclose.<path>` is not yet bound on the sheet and the claim comes from a cell `(row, col)`,
    /// creates an in-cell checkbox at `(row, col + 1)` and binds `disclose.<path>` to it.
    pub fn toggle_claim_disclose_commands(
        &self,
        presentation: &Presentation,
        path_str: &str,
    ) -> Vec<Command> {
        let Some(claim) = self.claim(path_str) else {
            return Vec::new();
        };
        if !claim.selective {
            return Vec::new();
        }
        let target_enabled = !claim.disclose;
        let Some(slide) = presentation.slide(self.slide) else {
            return Vec::new();
        };

        if let Some(d) = self.disclosures.get(path_str) {
            let mut commands = Vec::new();
            match d.source {
                BindingSource::Control { shape } => {
                    if let Some(sh) = slide.shape(shape)
                        && let ShapeKind::Control(ctrl) = &sh.kind
                    {
                        let mut next = (**ctrl).clone();
                        next.checked = u8::from(target_enabled);
                        commands.push(Command::Sheet(SheetCommand::EditControl {
                            slide: self.slide,
                            shape,
                            control: next,
                        }));
                    }
                    if let Some((link_slide, row, column)) = d.linked_cell {
                        commands.push(Command::Sheet(SheetCommand::SetCells {
                            slide: link_slide,
                            edits: vec![CellEdit {
                                row,
                                column,
                                input: CellInput::Value(CellValue::Bool(target_enabled)),
                            }],
                        }));
                    }
                }
                BindingSource::Cell { row, column } => {
                    commands.push(Command::Sheet(SheetCommand::SetCells {
                        slide: self.slide,
                        edits: vec![CellEdit {
                            row,
                            column,
                            input: CellInput::Value(CellValue::Bool(target_enabled)),
                        }],
                    }));
                }
            }
            return commands;
        }

        // If no `disclose.<path>` exists yet and the claim is on a cell, create an in-cell checkbox at `(row, column + 1)`.
        if let BindingSource::Cell { row, column } = claim.source {
            let chk_col = column + 1;
            let sheet_name = slide_sheet_formula_prefix(presentation, self.slide);
            let col_letters = crate::sheet::cell_name(chk_col, 0)
                .trim_end_matches('1')
                .to_string();
            let formula = format!("{sheet_name}${col_letters}${}", row + 1);
            let mut chk_style = crate::sheet::CellStyle::default();
            chk_style.checkbox = true;
            return vec![
                Command::Sheet(SheetCommand::PutCells {
                    slide: self.slide,
                    cells: vec![crate::sheet_command::PutCell {
                        row,
                        column: chk_col,
                        input: CellInput::Value(CellValue::Bool(target_enabled)),
                        style: Some(chk_style),
                    }],
                }),
                Command::Sheet(SheetCommand::SetName {
                    name: format!("disclose.{path_str}"),
                    sheet: None,
                    formula: Some(formula),
                }),
            ];
        }

        Vec::new()
    }
}

/// Built-in credential profile templates for one-click worksheet scaffolding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdJwtTemplateKind {
    /// EUDI PID / JPKI-style Person Identification Data (flat claims + `address.*` 1-level group + `nationalities` 1D array).
    Pid,
    /// ISO/IEC 18013-5 mDL Driving Licence (flat elements + `driving_privileges` 2D object table).
    Iso18013Mdl,
}

pub(crate) fn slide_sheet_formula_prefix(presentation: &Presentation, slide_id: SlideId) -> String {
    let raw_name = presentation
        .workbook
        .as_ref()
        .and_then(|wb| wb.sheets.iter().find(|ws| ws.slide == Some(slide_id)))
        .map(|ws| ws.name.clone())
        .or_else(|| presentation.slide(slide_id).and_then(|s| s.name.clone()))
        .unwrap_or_else(|| "Sheet1".to_string());
    format!("{}!", ibis_calc::formula::quote_sheet(&raw_name))
}

pub(crate) fn abs_cell_ref(col: u32, row: u32) -> String {
    let col_letters = crate::sheet::cell_name(col, 0)
        .trim_end_matches('1')
        .to_string();
    format!("${col_letters}${}", row + 1)
}

pub(crate) fn clear_sd_jwt_bindings_commands(
    presentation: &Presentation,
    slide_id: SlideId,
) -> Vec<Command> {
    let mut cmds = Vec::new();
    if let Some(slide) = presentation.slide(slide_id) {
        for sh in &slide.shapes {
            if let ShapeKind::Control(c) = &sh.kind
                && parse_binding_name(&c.name).is_some()
            {
                cmds.push(Command::RemoveShape {
                    slide: slide_id,
                    shape: sh.id,
                });
            }
        }
    }
    if let Some(wb) = presentation.workbook.as_ref() {
        for dn in &wb.names {
            if parse_binding_name(&dn.name).is_some() {
                cmds.push(Command::Sheet(SheetCommand::SetName {
                    name: dn.name.clone(),
                    sheet: dn.sheet,
                    formula: None,
                }));
            }
        }
    }
    cmds
}

/// Generates undoable worksheet commands that bind `range` on `slide_id` to `claim.<path>` or `sd.<path>`
/// (and when `selective` is `true`, places an in-cell checkbox at `(range.first_row, range.last_column + 1)` bound to `disclose.<path>`).
pub fn bind_range_as_sd_claim_commands(
    presentation: &Presentation,
    slide_id: SlideId,
    range: CellRange,
    raw_path: &str,
    selective: bool,
) -> Option<(String, Vec<Command>)> {
    let cleaned = raw_path
        .trim()
        .trim_start_matches("sd.")
        .trim_start_matches("claim.")
        .trim_start_matches("disclose.")
        .trim();
    let segments: Vec<String> = cleaned
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map( sanitize_claim_segment )
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }
    let path_str = segments.join(".");
    let prefix = if selective { "sd" } else { "claim" };
    let full_name = format!("{prefix}.{path_str}");

    let sheet_prefix = slide_sheet_formula_prefix(presentation, slide_id);
    let r0 = range.first_row.min(range.last_row);
    let r1 = range.first_row.max(range.last_row);
    let c0 = range.first_column.min(range.last_column);
    let c1 = range.first_column.max(range.last_column);
    let area = if r0 == r1 && c0 == c1 {
        abs_cell_ref(c0, r0)
    } else {
        format!("{}:{}", abs_cell_ref(c0, r0), abs_cell_ref(c1, r1))
    };
    let formula = format!("{sheet_prefix}{area}");

    let mut commands = vec![Command::Sheet(SheetCommand::SetName {
        name: full_name.clone(),
        sheet: None,
        formula: Some(formula),
    })];

    if selective {
        let chk_col = c1 + 1;
        let chk_formula = format!("{sheet_prefix}{}", abs_cell_ref(chk_col, r0));
        let mut chk_style = crate::sheet::CellStyle::default();
        chk_style.checkbox = true;
        commands.push(Command::Sheet(SheetCommand::PutCells {
            slide: slide_id,
            cells: vec![crate::sheet_command::PutCell {
                row: r0,
                column: chk_col,
                input: CellInput::Value(CellValue::Bool(true)),
                style: Some(chk_style),
            }],
        }));
        commands.push(Command::Sheet(SheetCommand::SetName {
            name: format!("disclose.{path_str}"),
            sheet: None,
            formula: Some(chk_formula),
        }));
    }

    Some((full_name, commands))
}

fn sanitize_claim_segment(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            out.push(ch);
        } else if ch.is_whitespace() || ch == '-' {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        return String::new();
    }
    let first = trimmed.chars().next().unwrap();
    if first.is_ascii_digit() {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Generates undoable worksheet commands to populate `slide_id` with a complete SD-JWT credential form template
/// (labels in Column A, typed values in Column B, in-cell `disclose.*` checkboxes in Column C, and `DefinedName`s).
pub fn build_sd_jwt_template_commands(
    presentation: &Presentation,
    slide_id: SlideId,
    kind: SdJwtTemplateKind,
) -> Vec<Command> {
    use crate::sheet::CellStyle;
    use crate::sheet_command::PutCell;

    let date1904 = presentation.workbook.as_ref().is_some_and(|w| w.date1904);
    let sheet_prefix = slide_sheet_formula_prefix(presentation, slide_id);

    let mut header_style = CellStyle::default();
    header_style.bold = true;

    let mut date_style = CellStyle::default();
    date_style.number_format = "yyyy-mm-dd".to_string();

    let mut text_style = CellStyle::default();
    text_style.number_format = "@".to_string();

    let mut checkbox_style = CellStyle::default();
    checkbox_style.checkbox = true;

    let mut cells: Vec<PutCell> = Vec::new();
    // Remove previous `claim.*`, `sd.*`, `disclose.*` names and form controls on this sheet so switching templates is clean
    let mut name_cmds = clear_sd_jwt_bindings_commands(presentation, slide_id);

    fn put_text(cells: &mut Vec<crate::sheet_command::PutCell>, r: u32, c: u32, s: &str, style: Option<crate::sheet::CellStyle>) {
        cells.push(crate::sheet_command::PutCell {
            row: r,
            column: c,
            input: CellInput::Value(CellValue::Text(s.to_string())),
            style,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn add_sd_row(
        cells: &mut Vec<crate::sheet_command::PutCell>,
        name_cmds: &mut Vec<Command>,
        sheet_prefix: &str,
        checkbox_style: &crate::sheet::CellStyle,
        r: u32,
        label: &str,
        path: &str,
        val_input: CellInput,
        val_style: Option<crate::sheet::CellStyle>,
        disclose_default: bool,
    ) {
        cells.push(crate::sheet_command::PutCell {
            row: r,
            column: 0,
            input: CellInput::Value(CellValue::Text(label.to_string())),
            style: None,
        });
        cells.push(crate::sheet_command::PutCell {
            row: r,
            column: 1,
            input: val_input,
            style: val_style,
        });
        cells.push(crate::sheet_command::PutCell {
            row: r,
            column: 2,
            input: CellInput::Value(CellValue::Bool(disclose_default)),
            style: Some(checkbox_style.clone()),
        });
        name_cmds.push(Command::Sheet(SheetCommand::SetName {
            name: format!("sd.{path}"),
            sheet: None,
            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, r))),
        }));
        name_cmds.push(Command::Sheet(SheetCommand::SetName {
            name: format!("disclose.{path}"),
            sheet: None,
            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(2, r))),
        }));
    }

    match kind {
        SdJwtTemplateKind::Pid => {
            put_text(&mut cells, 0, 0, "属性 (PID Claim)", Some(header_style.clone()));
            put_text(&mut cells, 0, 1, "値 (Value)", Some(header_style.clone()));
            put_text(&mut cells, 0, 2, "開示 (Disclose)", Some(header_style.clone()));

            // Row 1: claim.vct
            put_text(&mut cells, 1, 0, "クレデンシャル種別 (vct)", None);
            put_text(&mut cells, 1, 1, "urn:eudi:pid:jp:1", None);
            put_text(&mut cells, 1, 2, "常時 (claim.vct)", None);
            name_cmds.push(Command::Sheet(SheetCommand::SetName {
                name: "claim.vct".into(),
                sheet: None,
                formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, 1))),
            }));

            // Row 2: claim.iss
            put_text(&mut cells, 2, 0, "発行者 (iss)", None);
            put_text(&mut cells, 2, 1, "https://issuer.example.go.jp", None);
            put_text(&mut cells, 2, 2, "常時 (claim.iss)", None);
            name_cmds.push(Command::Sheet(SheetCommand::SetName {
                name: "claim.iss".into(),
                sheet: None,
                formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, 2))),
            }));

            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                3,
                "姓 (family_name)",
                "family_name",
                CellInput::Value(CellValue::Text("山田".into())),
                None,
                true,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                4,
                "名 (given_name)",
                "given_name",
                CellInput::Value(CellValue::Text("太郎".into())),
                None,
                true,
            );
            let bday_serial =
                ibis_calc::eval::convert::serial_from_date(1990, 5, 15, date1904) as f64;
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                5,
                "生年月日 (birthdate)",
                "birthdate",
                CellInput::Value(CellValue::Number(bday_serial)),
                Some(date_style.clone()),
                false,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                6,
                "国コード (address.country)",
                "address.country",
                CellInput::Value(CellValue::Text("JP".into())),
                None,
                true,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                7,
                "都道府県 (address.region)",
                "address.region",
                CellInput::Value(CellValue::Text("東京都".into())),
                None,
                true,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                8,
                "市区町村 (address.locality)",
                "address.locality",
                CellInput::Value(CellValue::Text("千代田区".into())),
                None,
                false,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                9,
                "郵便番号 (address.postal_code)",
                "address.postal_code",
                CellInput::Value(CellValue::Text("100-0001".into())),
                Some(text_style.clone()),
                false,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                10,
                "20歳以上 (age_over_20)",
                "age_over_20",
                CellInput::Value(CellValue::Bool(true)),
                Some(checkbox_style.clone()),
                true,
            );

            // Rows 11..=12: 1D array `sd.nationalities` = B12:B13
            put_text(&mut cells, 11, 0, "国籍リスト (nationalities)", None);
            put_text(&mut cells, 11, 1, "JP", None);
            cells.push(PutCell {
                row: 11,
                column: 2,
                input: CellInput::Value(CellValue::Bool(true)),
                style: Some(checkbox_style.clone()),
            });
            put_text(&mut cells, 12, 1, "US", None);
            name_cmds.push(Command::Sheet(SheetCommand::SetName {
                name: "sd.nationalities".into(),
                sheet: None,
                formula: Some(format!(
                    "{sheet_prefix}{}:{}",
                    abs_cell_ref(1, 11),
                    abs_cell_ref(1, 12)
                )),
            }));
            name_cmds.push(Command::Sheet(SheetCommand::SetName {
                name: "disclose.nationalities".into(),
                sheet: None,
                formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(2, 11))),
            }));
        }
        SdJwtTemplateKind::Iso18013Mdl => {
            put_text(&mut cells, 0, 0, "項目 (ISO 18013-5 Element)", Some(header_style.clone()));
            put_text(&mut cells, 0, 1, "値 (Value)", Some(header_style.clone()));
            put_text(&mut cells, 0, 2, "開示 (Disclose)", Some(header_style.clone()));

            put_text(&mut cells, 1, 0, "クレデンシャル種別 (vct)", None);
            put_text(&mut cells, 1, 1, "org.iso.18013.5.1.mDL", None);
            put_text(&mut cells, 1, 2, "常時 (claim.vct)", None);
            name_cmds.push(Command::Sheet(SheetCommand::SetName {
                name: "claim.vct".into(),
                sheet: None,
                formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, 1))),
            }));

            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                2,
                "発行国 (issuing_country)",
                "issuing_country",
                CellInput::Value(CellValue::Text("JP".into())),
                None,
                true,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                3,
                "公安委員会 (issuing_authority)",
                "issuing_authority",
                CellInput::Value(CellValue::Text("東京都公安委員会".into())),
                None,
                true,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                4,
                "免許証番号 (document_number)",
                "document_number",
                CellInput::Value(CellValue::Text("001234567890".into())),
                Some(text_style.clone()),
                false,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                5,
                "姓 (family_name)",
                "family_name",
                CellInput::Value(CellValue::Text("山田".into())),
                None,
                true,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                6,
                "名 (given_name)",
                "given_name",
                CellInput::Value(CellValue::Text("太郎".into())),
                None,
                true,
            );
            let bday = ibis_calc::eval::convert::serial_from_date(1990, 5, 15, date1904) as f64;
            let issue = ibis_calc::eval::convert::serial_from_date(2024, 4, 1, date1904) as f64;
            let expiry = ibis_calc::eval::convert::serial_from_date(2029, 5, 15, date1904) as f64;
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                7,
                "生年月日 (birth_date)",
                "birth_date",
                CellInput::Value(CellValue::Number(bday)),
                Some(date_style.clone()),
                false,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                8,
                "交付日 (issue_date)",
                "issue_date",
                CellInput::Value(CellValue::Number(issue)),
                Some(date_style.clone()),
                true,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                9,
                "有効期限 (expiry_date)",
                "expiry_date",
                CellInput::Value(CellValue::Number(expiry)),
                Some(date_style.clone()),
                true,
            );
            add_sd_row(
                &mut cells,
                &mut name_cmds,
                &sheet_prefix,
                &checkbox_style,
                10,
                "18歳以上 (age_over_18)",
                "age_over_18",
                CellInput::Value(CellValue::Bool(true)),
                Some(checkbox_style.clone()),
                true,
            );

            // Row 12: 2D Table section header + disclose.driving_privileges checkbox
            put_text(
                &mut cells,
                12,
                0,
                "免許種別テーブル (driving_privileges)",
                Some(header_style.clone()),
            );
            cells.push(PutCell {
                row: 12,
                column: 2,
                input: CellInput::Value(CellValue::Bool(true)),
                style: Some(checkbox_style.clone()),
            });

            // Rows 13..=15, Cols 0..=2: 2D table `sd.driving_privileges` = $A$14:$C$16
            put_text(&mut cells, 13, 0, "vehicle_category_code", Some(header_style.clone()));
            put_text(&mut cells, 13, 1, "issue_date", Some(header_style.clone()));
            put_text(&mut cells, 13, 2, "expiry_date", Some(header_style.clone()));

            let issue_b = ibis_calc::eval::convert::serial_from_date(2018, 4, 1, date1904) as f64;
            let issue_a = ibis_calc::eval::convert::serial_from_date(2020, 10, 15, date1904) as f64;
            put_text(&mut cells, 14, 0, "普通", None);
            cells.push(PutCell {
                row: 14,
                column: 1,
                input: CellInput::Value(CellValue::Number(issue_b)),
                style: Some(date_style.clone()),
            });
            cells.push(PutCell {
                row: 14,
                column: 2,
                input: CellInput::Value(CellValue::Number(expiry)),
                style: Some(date_style.clone()),
            });

            put_text(&mut cells, 15, 0, "普通二輪", None);
            cells.push(PutCell {
                row: 15,
                column: 1,
                input: CellInput::Value(CellValue::Number(issue_a)),
                style: Some(date_style.clone()),
            });
            cells.push(PutCell {
                row: 15,
                column: 2,
                input: CellInput::Value(CellValue::Number(expiry)),
                style: Some(date_style.clone()),
            });

            name_cmds.push(Command::Sheet(SheetCommand::SetName {
                name: "sd.driving_privileges".into(),
                sheet: None,
                formula: Some(format!(
                    "{sheet_prefix}{}:{}",
                    abs_cell_ref(0, 13),
                    abs_cell_ref(2, 15)
                )),
            }));
            name_cmds.push(Command::Sheet(SheetCommand::SetName {
                name: "disclose.driving_privileges".into(),
                sheet: None,
                formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(2, 12))),
            }));
        }
    }

    let mut commands = Vec::with_capacity(1 + name_cmds.len());
    commands.push(Command::Sheet(SheetCommand::PutCells {
        slide: slide_id,
        cells,
    }));
    commands.extend(name_cmds);
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::command::Document;
    use crate::geometry::EmuRect;
    use crate::model::{Presentation, Shape, ShapeId, ShapeKind, Slide, SlideId};
    use crate::sheet::{CellKind, CellStyle, DefinedName, Sheet, SheetCell, Workbook, WorkbookSheet};

    fn put_cell(sheet: &mut Sheet, row: u32, column: u32, value: CellValue, text: &str, style: u32) {
        let mut cells = (*sheet.cells).clone();
        cells.push(SheetCell {
            row,
            column,
            kind: CellKind::of(&value),
            value,
            text: text.to_string(),
            formula: None,
            style,
            phonetic: None,
            runs: None,
        });
        cells.sort_by_key(|c| (c.row, c.column));
        sheet.cells = cells.into();
    }

    fn sample_presentation() -> (Presentation, SlideId, ShapeId) {
        let slide_id = SlideId(1);
        let mut sheet = Sheet::default();
        // Style 0: General
        let general_style = CellStyle::default();
        // Style 1: Date format yyyy-mm-dd
        let mut date_style = CellStyle::default();
        date_style.number_format = "yyyy-mm-dd".to_string();
        // Style 2: In-cell Checkbox
        let mut checkbox_style = CellStyle::default();
        checkbox_style.checkbox = true;
        sheet.styles = vec![general_style, date_style, checkbox_style];

        // B1 (row 0, col 1): vct = "https://credentials.example.com/identity_credential"
        put_cell(
            &mut sheet,
            0,
            1,
            CellValue::Text("https://credentials.example.com/identity_credential".into()),
            "https://credentials.example.com/identity_credential",
            0,
        );
        // B2 (row 1, col 1): iss = "https://issuer.example.com"
        put_cell(
            &mut sheet,
            1,
            1,
            CellValue::Text("https://issuer.example.com".into()),
            "https://issuer.example.com",
            0,
        );
        // B3 (row 2, col 1): iat = 1700000000 (even if formatted with date style, iat stays NumericDate int)
        put_cell(&mut sheet, 2, 1, CellValue::Number(1_700_000_000.0), "1700000000", 1);
        // B4 (row 3, col 1): sd.family_name = "山田"
        put_cell(&mut sheet, 3, 1, CellValue::Text("山田".into()), "山田", 0);
        // B5 (row 4, col 1): sd.given_name = "太郎"
        put_cell(&mut sheet, 4, 1, CellValue::Text("太郎".into()), "太郎", 0);
        // B6 (row 5, col 1): sd.birthdate = 1990-05-15 (Excel 1900 serial for 1990-05-15)
        let serial_1990_05_15 = ibis_calc::eval::convert::serial_from_date(1990, 5, 15, false) as f64;
        put_cell(&mut sheet, 5, 1, CellValue::Number(serial_1990_05_15), "1990-05-15", 1);
        // B7 (row 6, col 1): sd.address.country = "JP"
        put_cell(&mut sheet, 6, 1, CellValue::Text("JP".into()), "JP", 0);
        // B8 (row 7, col 1): linked cell for Drop control (index 2 -> "大阪府")
        put_cell(&mut sheet, 7, 1, CellValue::Number(2.0), "2", 0);
        // B9 (row 8, col 1): in-cell checkbox for sd.age_over_18 = true
        put_cell(&mut sheet, 8, 1, CellValue::Bool(true), "TRUE", 2);
        // B10 (row 9, col 1): empty cell for sd.middle_name (should be omitted)
        put_cell(&mut sheet, 9, 1, CellValue::Empty, "", 0);
        // C5 (row 4, col 2): linked cell for disclose.given_name CheckBox control (false)
        put_cell(&mut sheet, 4, 2, CellValue::Bool(false), "FALSE", 0);

        // F1:F3 (rows 0..=2, col 5): Drop-down item list ["東京都", "大阪府", "北海道"]
        put_cell(&mut sheet, 0, 5, CellValue::Text("東京都".into()), "東京都", 0);
        put_cell(&mut sheet, 1, 5, CellValue::Text("大阪府".into()), "大阪府", 0);
        put_cell(&mut sheet, 2, 5, CellValue::Text("北海道".into()), "北海道", 0);

        let mut slide = Slide::new(slide_id);
        slide.shapes.push(Shape::new(
            ShapeId(1),
            EmuRect::new(0, 0, 1_000_000, 1_000_000),
            ShapeKind::Sheet(Arc::new(sheet)),
        ));

        // Drop control linked to $B$8, range $F$1:$F$3, named "sd.address.region"
        let mut drop = FormControl::new(ControlKind::Drop, "sd.address.region");
        drop.link = Some("$B$8".into());
        drop.range = Some("$F$1:$F$3".into());
        drop.value = 2;
        slide.shapes.push(Shape::new(
            ShapeId(10),
            EmuRect::new(0, 0, 500_000, 200_000),
            ShapeKind::Control(Arc::new(drop)),
        ));

        // Radio group for sd.gender: Option 1 ("male", unchecked) and Option 2 ("female", checked)
        let mut radio1 = FormControl::new(ControlKind::Radio, "sd.gender");
        radio1.first_button = true;
        radio1.text = "male".into();
        radio1.checked = 0;
        slide.shapes.push(Shape::new(
            ShapeId(11),
            EmuRect::new(0, 0, 400_000, 200_000),
            ShapeKind::Control(Arc::new(radio1)),
        ));
        let mut radio2 = FormControl::new(ControlKind::Radio, "Radio Button 2");
        radio2.first_button = false;
        radio2.text = "female".into();
        radio2.checked = 1;
        slide.shapes.push(Shape::new(
            ShapeId(12),
            EmuRect::new(0, 0, 400_000, 200_000),
            ShapeKind::Control(Arc::new(radio2)),
        ));

        // CheckBox control for disclose.given_name (unchecked -> false, linked to $C$5)
        let disclose_gn_id = ShapeId(20);
        let mut chk = FormControl::new(ControlKind::CheckBox, "disclose.given_name");
        chk.text = "名を開示".into();
        chk.link = Some("$C$5".into());
        chk.checked = 0;
        slide.shapes.push(Shape::new(
            disclose_gn_id,
            EmuRect::new(0, 0, 400_000, 200_000),
            ShapeKind::Control(Arc::new(chk)),
        ));

        let mut wb = Workbook::default();
        wb.sheets.push(WorkbookSheet {
            name: "Sheet1".into(),
            state: crate::sheet::SheetState::Visible,
            slide: Some(slide_id),
            hidden: None,
            other: false,
        });
        wb.names = vec![
            DefinedName { name: "claim.vct".into(), sheet: None, formula: "Sheet1!$B$1".into(), hidden: false },
            DefinedName { name: "claim.iss".into(), sheet: None, formula: "Sheet1!$B$2".into(), hidden: false },
            DefinedName { name: "claim.iat".into(), sheet: None, formula: "Sheet1!$B$3".into(), hidden: false },
            DefinedName { name: "sd.family_name".into(), sheet: None, formula: "Sheet1!$B$4".into(), hidden: false },
            DefinedName { name: "sd.given_name".into(), sheet: None, formula: "Sheet1!$B$5".into(), hidden: false },
            DefinedName { name: "sd.birthdate".into(), sheet: None, formula: "Sheet1!$B$6".into(), hidden: false },
            DefinedName { name: "sd.address.country".into(), sheet: None, formula: "Sheet1!$B$7".into(), hidden: false },
            // DefinedName on the linked cell $B$8 of the Drop control also resolves to "大阪府"!
            DefinedName { name: "sd.prefecture_from_cell".into(), sheet: None, formula: "Sheet1!$B$8".into(), hidden: false },
            DefinedName { name: "sd.age_over_18".into(), sheet: None, formula: "Sheet1!$B$9".into(), hidden: false },
            DefinedName { name: "sd.middle_name".into(), sheet: None, formula: "Sheet1!$B$10".into(), hidden: false },
        ];

        let mut pres = Presentation::new();
        pres.slides = vec![slide];
        pres.workbook = Some(Arc::new(wb));
        (pres, slide_id, disclose_gn_id)
    }

    #[test]
    fn extracts_claims_nested_objects_dates_controls_and_disclosure_toggles() {
        let (pres, slide_id, _chk_id) = sample_presentation();
        let bindings = extract_sheet_claims(&pres, slide_id).expect("extract bindings");

        // Empty cell (`sd.middle_name`) is omitted
        assert!(bindings.claim("middle_name").is_none());

        // Plain claims (`claim.*`)
        let vct = bindings.claim("vct").unwrap();
        assert!(!vct.selective);
        assert_eq!(vct.value.as_str(), Some("https://credentials.example.com/identity_credential"));
        let iat = bindings.claim("iat").unwrap();
        assert!(!iat.selective);
        assert_eq!(iat.value, ClaimValue::Int(1_700_000_000));

        // Selective claims (`sd.*`)
        let family_name = bindings.claim("family_name").unwrap();
        assert!(family_name.selective);
        assert!(family_name.disclose, "default disclose is true when no disclose.family_name is present");
        assert_eq!(family_name.value.as_str(), Some("山田"));

        let given_name = bindings.claim("given_name").unwrap();
        assert!(given_name.selective);
        assert!(!given_name.disclose, "disclose.given_name is unchecked (false)");
        assert_eq!(given_name.value.as_str(), Some("太郎"));

        // Date-formatted cell is converted to ISO 8601 YYYY-MM-DD
        let birthdate = bindings.claim("birthdate").unwrap();
        assert_eq!(birthdate.value.as_str(), Some("1990-05-15"));

        // Drop control resolved both directly (`sd.address.region`) and via linked cell (`sd.prefecture_from_cell`)
        let region = bindings.claim("address.region").unwrap();
        assert_eq!(region.value.as_str(), Some("大阪府"));
        let pref = bindings.claim("prefecture_from_cell").unwrap();
        assert_eq!(pref.value.as_str(), Some("大阪府"));

        // Radio group resolved to checked button's caption
        let gender = bindings.claim("gender").unwrap();
        assert_eq!(gender.value.as_str(), Some("female"));

        // In-cell checkbox resolved to bool
        let age_over_18 = bindings.claim("age_over_18").unwrap();
        assert_eq!(age_over_18.value, ClaimValue::Bool(true));

        // Disclosed JSON omits `given_name` while plain JSON includes it
        let plain = bindings.to_plain_claims_json();
        let disclosed = bindings.to_disclosed_claims_json();
        assert!(plain.as_object().unwrap().contains_key("given_name"));
        assert!(!disclosed.as_object().unwrap().contains_key("given_name"));

        // Nested `address` object has both `country` and `region`
        let addr = disclosed.as_object().unwrap().get("address").unwrap().as_object().unwrap();
        assert_eq!(addr.get("country").and_then(ClaimValue::as_str), Some("JP"));
        assert_eq!(addr.get("region").and_then(ClaimValue::as_str), Some("大阪府"));
    }

    #[test]
    fn set_disclose_commands_updates_checkbox_control_and_linked_cell() {
        let (pres, slide_id, chk_id) = sample_presentation();
        let bindings = extract_sheet_claims(&pres, slide_id).unwrap();
        assert!(!bindings.claim("given_name").unwrap().disclose);

        // Requesting "given_name" generates commands to check `disclose.given_name` and set $C$5 to TRUE
        let cmds = bindings.set_disclose_commands(&pres, &["given_name"], false);
        assert_eq!(cmds.len(), 2);

        let mut doc = Document::new(pres);
        doc.execute(cmds).unwrap();

        let ShapeKind::Control(c) = &doc.presentation().slide(slide_id).unwrap().shape(chk_id).unwrap().kind else {
            panic!("expected control");
        };
        assert_eq!(c.checked, 1);

        let updated = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
        assert!(updated.claim("given_name").unwrap().disclose);
    }
}

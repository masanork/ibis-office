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
    let is_checkbox_style = sheet
        .styles
        .get(cell.style as usize)
        .is_some_and(|st| st.checkbox);
    let is_date_style = sheet
        .styles
        .get(cell.style as usize)
        .is_some_and(|st| ibis_calc::numfmt::is_date_format(&st.number_format));

    match &cell.value {
        CellValue::Empty => {
            if is_checkbox_style {
                Some(ClaimValue::Bool(false))
            } else {
                None
            }
        }
        CellValue::Bool(b) => Some(ClaimValue::Bool(*b)),
        CellValue::Number(n) => {
            if is_checkbox_style {
                return Some(ClaimValue::Bool(*n != 0.0));
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
                        if let Some(value) = resolve_cell_value(presentation, slide, sheet, row, column, leaf, date1904) {
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

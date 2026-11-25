// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Unified PlantUML diagram-kind dispatcher and UML core converters (Class, State,
//! Usecase, ER) sharing the existing Mermaid layout and SVG renderers without duplicating
//! rendering code.

use std::collections::HashMap;

use super::plantuml::{
    PlantUmlNativeRender, PlantUmlUnsupported, excerpt, is_timing_statement,
    multi_line_unsupported_block, render_plantuml_sequence_native, sanitize_label,
    split_as_outside_quotes, split_colon_outside_quotes, split_trailing_color,
    strip_block_comments, strip_prefix_ci, strip_quotes,
};
use super::{MermaidOutput, render_mermaid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlantUmlKind {
    Sequence,
    Class,
    State,
    Usecase,
    Er,
    Timing,
    UnsupportedEnvelope,
}

/// Render a supported PlantUML diagram (Sequence, Class, State, Usecase, or ER) into SVG
/// using the shared internal renderers, preserving the original source verbatim and
/// reporting unsupported constructs with 1-based line numbers.
pub fn render_plantuml(source: &str) -> PlantUmlNativeRender {
    match detect_plantuml_kind(source) {
        PlantUmlKind::Sequence | PlantUmlKind::Timing | PlantUmlKind::UnsupportedEnvelope => {
            render_plantuml_sequence_native(source)
        }
        PlantUmlKind::Class => render_lowered(source, convert_plantuml_class(source)),
        PlantUmlKind::State => render_lowered(source, convert_plantuml_state(source)),
        PlantUmlKind::Usecase => render_lowered(source, convert_plantuml_usecase(source)),
        PlantUmlKind::Er => render_lowered(source, convert_plantuml_er(source)),
    }
}

fn render_lowered(
    source: &str,
    (mermaid, unsupported): (Option<String>, Vec<PlantUmlUnsupported>),
) -> PlantUmlNativeRender {
    let output = mermaid
        .as_deref()
        .and_then(render_mermaid)
        .filter(|out| matches!(out, MermaidOutput::Svg { .. }));
    PlantUmlNativeRender {
        original: source.to_string(),
        output,
        unsupported,
    }
}

pub(super) fn detect_plantuml_kind(source: &str) -> PlantUmlKind {
    let mut in_comment = false;
    let mut saw_usecase = false;
    let mut saw_er = false;
    let mut saw_state = false;
    let mut saw_class = false;
    let mut saw_sequence = false;
    let mut saw_actor = false;
    let mut saw_box_container = false;

    for raw in source.lines() {
        let cleaned = strip_block_comments(raw, &mut in_comment);
        let t = cleaned.trim();
        if t.is_empty() || t.starts_with('\'') {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("@start") {
            let dir = lower.split_whitespace().next().unwrap_or("");
            if dir != "@startuml" {
                return PlantUmlKind::UnsupportedEnvelope;
            }
            continue;
        }
        if lower.starts_with("@end") {
            break;
        }
        if t.starts_with('!') || t.starts_with('$') {
            continue;
        }
        if is_timing_statement(t, &lower) {
            return PlantUmlKind::Timing;
        }
        if lower.starts_with("usecase ")
            || (t.starts_with('(') && t.contains(')'))
            || (t.starts_with(':') && t[1..].contains(':'))
        {
            saw_usecase = true;
        }
        if contains_crows_foot(t) || (lower.starts_with("entity ") && t.ends_with('{')) {
            saw_er = true;
        }
        if t.starts_with("[*]") || lower.starts_with("state ") {
            saw_state = true;
        }
        if lower.starts_with("class ")
            || lower.starts_with("abstract class ")
            || lower.starts_with("abstract ")
            || lower.starts_with("interface ")
            || lower.starts_with("enum ")
            || lower.starts_with("struct ")
            || lower.starts_with("annotation ")
            || t.contains("<|--")
            || t.contains("--|>")
            || t.contains("*--")
            || t.contains("--*")
            || t.contains("o--")
            || t.contains("--o")
            || t.contains("..|>")
            || t.contains("<|..")
        {
            saw_class = true;
        }
        if lower.starts_with("participant ")
            || lower.starts_with("boundary ")
            || lower.starts_with("control ")
            || lower.starts_with("database ")
            || lower.starts_with("collections ")
            || lower.starts_with("queue ")
            || lower == "autonumber"
            || lower.starts_with("autonumber ")
            || lower == "hide footbox"
            || lower.starts_with("alt ")
            || lower.starts_with("opt ")
            || lower.starts_with("loop ")
            || lower.starts_with("par ")
            || lower.starts_with("activate ")
            || lower.starts_with("deactivate ")
            || lower.starts_with("ref over ")
            || (t.starts_with("==") && t.ends_with("==") && t.len() >= 4)
            || t.starts_with("[->")
            || t.contains("->]")
        {
            saw_sequence = true;
        }
        if lower.starts_with("actor ") {
            saw_actor = true;
        }
        if lower.starts_with("rectangle ") || lower.starts_with("package ") {
            saw_box_container = true;
        }
    }

    if saw_usecase || (saw_actor && saw_box_container && !saw_sequence && !saw_class) {
        return PlantUmlKind::Usecase;
    }
    if saw_er && !saw_sequence {
        return PlantUmlKind::Er;
    }
    if saw_state && !saw_sequence {
        return PlantUmlKind::State;
    }
    if saw_class && !saw_sequence {
        return PlantUmlKind::Class;
    }
    PlantUmlKind::Sequence
}

fn contains_crows_foot(line: &str) -> bool {
    const MARKERS: &[&str] = &[
        "||--", "|o--", "o|--", "}|--", "}o--", "--||", "--|o", "--o|", "--|{", "--o{", "||..",
        "|o..", "o|..", "}|..", "}o..", "..||", "..|o", "..o|", "..|{", "..o{",
    ];
    MARKERS.iter().any(|m| line.contains(m))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UmlSkip {
    MacroProc,
    StyleTag,
    BraceBlock,
    EndKeyword(&'static str),
}

fn handle_common_ignored_or_unsupported(
    line_no: usize,
    trimmed: &str,
    lower: &str,
    skip_until: &mut Option<UmlSkip>,
    unsupported: &mut Vec<PlantUmlUnsupported>,
) -> bool {
    let mut push = |kind: &'static str, reason: &str| {
        unsupported.push(PlantUmlUnsupported {
            line: line_no,
            kind,
            source: excerpt(trimmed, 120),
            reason: reason.to_string(),
        });
    };
    if trimmed.starts_with('!') {
        push(
            "macro",
            "PlantUML preprocessor directives and macros are not expanded.",
        );
        if lower.starts_with("!procedure ")
            || lower.starts_with("!unquoted procedure ")
            || lower.starts_with("!function ")
            || lower.starts_with("!unquoted function ")
        {
            *skip_until = Some(UmlSkip::MacroProc);
        }
        return true;
    }
    if trimmed.starts_with('$') {
        push("macro", "PlantUML macro invocations are not expanded.");
        return true;
    }
    if lower.starts_with("skinparam") {
        push(
            "styling",
            "skinparam styling directives are not applied.",
        );
        if trimmed.ends_with('{') {
            *skip_until = Some(UmlSkip::BraceBlock);
        }
        return true;
    }
    if lower.starts_with("<style>") {
        push("styling", "<style> blocks are not applied.");
        if !lower.contains("</style>") {
            *skip_until = Some(UmlSkip::StyleTag);
        }
        return true;
    }
    if lower.starts_with("hide ") || lower.starts_with("show ") {
        push("styling", "hide/show directives are not applied.");
        return true;
    }
    if lower.starts_with("scale ")
        || lower == "newpage"
        || lower.starts_with("newpage ")
        || lower.starts_with("caption ")
    {
        push("layout", "Layout/pagination directive is not supported.");
        return true;
    }
    if let Some(end_kw) = multi_line_unsupported_block(lower) {
        push("layout", "header/footer/legend blocks are not supported.");
        if !trimmed.contains(' ') || matches!(lower, "legend" | "header" | "footer") {
            *skip_until = Some(UmlSkip::EndKeyword(end_kw));
        }
        return true;
    }
    false
}

fn advance_skip(trimmed: &str, skip_until: &mut Option<UmlSkip>) -> bool {
    let Some(mode) = *skip_until else {
        return false;
    };
    let lower = trimmed.to_ascii_lowercase();
    let done = match mode {
        UmlSkip::MacroProc => {
            lower.starts_with("!endprocedure") || lower.starts_with("!endfunction")
        }
        UmlSkip::StyleTag => lower.contains("</style>"),
        UmlSkip::BraceBlock => trimmed == "}" || trimmed.ends_with('}'),
        UmlSkip::EndKeyword(end_kw) => lower == end_kw,
    };
    if done {
        *skip_until = None;
    }
    true
}

/// Normalize PlantUML directional arrow hints (`-up->`, `-right->`, `-d->`, `.down.>`, etc.)
/// outside double quotes into standard `--` / `..` arrow shafts.
fn normalize_directional_arrows(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_quotes = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    const DIRS: &[&str] = &[
        "up", "down", "left", "right", "u", "d", "l", "r",
    ];
    while i < bytes.len() {
        if bytes[i] == b'"' {
            in_quotes = !in_quotes;
            out.push('"');
            i += 1;
            continue;
        }
        if !in_quotes && matches!(bytes[i], b'-' | b'.') {
            let sep = bytes[i] as char;
            let rest = &line[i + 1..];
            let mut matched = None;
            for &dir in DIRS {
                if rest
                    .get(..dir.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(dir))
                    && rest.as_bytes().get(dir.len()) == Some(&(sep as u8))
                {
                    matched = Some(dir.len());
                    break;
                }
            }
            if let Some(dlen) = matched {
                out.push(sep);
                out.push(sep);
                i += 1 + dlen + 1;
                continue;
            }
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn prepend_frontmatter_title(header: &str, title: Option<&str>, body_lines: &[String]) -> String {
    let mut out = String::new();
    if let Some(t) = title {
        let escaped = t.replace('"', "\\\"");
        out.push_str(&format!("---\ntitle: \"{escaped}\"\n---\n"));
    }
    out.push_str(header);
    out.push('\n');
    for line in body_lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn safe_id(raw: &str) -> String {
    let s = strip_quotes(raw.trim());
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "N".to_string()
    } else {
        trimmed.to_string()
    }
}

fn strip_stereotype(s: &str) -> (&str, Option<&str>) {
    let s = s.trim();
    if s.ends_with(">>")
        && let Some(open) = s.rfind("<<")
    {
        let st = s[open + 2..s.len() - 2].trim();
        return (s[..open].trim(), (!st.is_empty()).then_some(st));
    }
    (s, None)
}

/// Parse `"Label" as Alias` or `Alias` into `(id, optional_display_label)`.
fn parse_uml_entity_head(raw: &str) -> (String, Option<String>) {
    let (clean, _) = split_trailing_color(raw.trim());
    if let Some((lhs, rhs)) = split_as_outside_quotes(clean) {
        let lhs = lhs.trim();
        let rhs = rhs.trim();
        if lhs.starts_with('"') && lhs.ends_with('"') {
            (safe_id(rhs), Some(sanitize_label(strip_quotes(lhs))))
        } else if rhs.starts_with('"') && rhs.ends_with('"') {
            (safe_id(lhs), Some(sanitize_label(strip_quotes(rhs))))
        } else {
            (safe_id(rhs), Some(sanitize_label(lhs)))
        }
    } else if clean.starts_with('"') && clean.ends_with('"') {
        let lbl = sanitize_label(strip_quotes(clean));
        (safe_id(&lbl), Some(lbl))
    } else {
        let raw_clean = clean.trim();
        let id = safe_id(raw_clean);
        if id != raw_clean {
            (id, Some(sanitize_label(raw_clean)))
        } else {
            (id, None)
        }
    }
}

// ============================================================================
// 1. PlantUML Class Diagram -> Mermaid `classDiagram`
// ============================================================================

fn convert_plantuml_class(source: &str) -> (Option<String>, Vec<PlantUmlUnsupported>) {
    let mut unsupported = Vec::new();
    let mut lines_out: Vec<String> = Vec::new();
    let mut title: Option<String> = None;
    let mut in_comment = false;
    let mut skip_until: Option<UmlSkip> = None;
    let mut saw_enduml = false;
    let mut in_class_body = false;
    let mut open_namespaces: usize = 0;
    let mut pending_note: Option<(usize, Option<String>, Vec<String>)> = None;
    let mut has_content = false;

    for (idx, raw) in source.lines().enumerate() {
        let line_no = idx + 1;
        if saw_enduml {
            continue;
        }
        let cleaned = strip_block_comments(raw, &mut in_comment);
        let trimmed = cleaned.trim();
        if advance_skip(trimmed, &mut skip_until) {
            continue;
        }
        if let Some((n_line, target, mut n_lines)) = pending_note.take() {
            let lower = trimmed.to_ascii_lowercase();
            if lower == "end note" || lower == "endnote" {
                let text = sanitize_label(&n_lines.join(" / ")).replace('"', "'");
                if let Some(t) = target {
                    lines_out.push(format!("  note for {t} \"{text}\""));
                } else {
                    lines_out.push(format!("  note \"{text}\""));
                }
                has_content = true;
            } else {
                if !trimmed.is_empty() {
                    n_lines.push(trimmed.to_string());
                }
                pending_note = Some((n_line, target, n_lines));
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('\'') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("@start") {
            continue;
        }
        if lower.starts_with("@end") {
            saw_enduml = true;
            continue;
        }
        if handle_common_ignored_or_unsupported(
            line_no,
            trimmed,
            &lower,
            &mut skip_until,
            &mut unsupported,
        ) {
            continue;
        }
        if in_class_body {
            if trimmed == "}" {
                lines_out.push(format!("{}}}", "  ".repeat(open_namespaces + 1)));
                in_class_body = false;
                continue;
            }
            if trimmed.chars().all(|c| matches!(c, '-' | '.' | '=' | '_')) {
                continue;
            }
            let member = trimmed
                .replace("{static}", "")
                .replace("{abstract}", "")
                .replace("{field}", "")
                .replace("{method}", "");
            let member = member.trim();
            if !member.is_empty() {
                lines_out.push(format!("{}{member}", "  ".repeat(open_namespaces + 2)));
            }
            continue;
        }
        if trimmed == "}" {
            if open_namespaces > 0 {
                open_namespaces -= 1;
                lines_out.push(format!("{}}}", "  ".repeat(open_namespaces + 1)));
            } else {
                unsupported.push(PlantUmlUnsupported {
                    line: line_no,
                    kind: "syntax",
                    source: excerpt(trimmed, 120),
                    reason: "Unexpected closing brace `}`.".to_string(),
                });
            }
            continue;
        }
        if lower == "left to right direction" {
            lines_out.push("  direction LR".to_string());
            continue;
        }
        if lower == "top to bottom direction" {
            lines_out.push("  direction TB".to_string());
            continue;
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "title ") {
            let t = sanitize_label(rest);
            if !t.is_empty() {
                title = Some(t);
            }
            continue;
        }
        // `package "Name" {` / `namespace Name {`
        if let Some(rest) =
            strip_prefix_ci(trimmed, "package ").or_else(|| strip_prefix_ci(trimmed, "namespace "))
        {
            let opens = rest.trim_end().ends_with('{');
            let head = rest.trim_end().trim_end_matches('{').trim();
            let (head_no_st, _) = strip_stereotype(head);
            let (id, label) = parse_uml_entity_head(head_no_st);
            let ind = "  ".repeat(open_namespaces + 1);
            if opens {
                match label {
                    Some(lbl) => lines_out.push(format!("{ind}namespace {id}[\"{lbl}\"] {{")),
                    None => lines_out.push(format!("{ind}namespace {id} {{")),
                }
                open_namespaces += 1;
            }
            has_content = true;
            continue;
        }
        // Class/interface/enum/abstract declarations
        if let Some((default_stereo, rest)) = [
            ("abstract class ", Some("abstract")),
            ("abstract ", Some("abstract")),
            ("interface ", Some("interface")),
            ("enum ", Some("enumeration")),
            ("annotation ", Some("annotation")),
            ("struct ", Some("struct")),
            ("class ", None),
        ]
        .iter()
        .find_map(|&(kw, st)| strip_prefix_ci(trimmed, kw).map(|r| (st, r)))
        {
            let opens = rest.trim_end().ends_with('{');
            let head = rest.trim_end().trim_end_matches('{').trim();
            let (head_clean, had_color) = split_trailing_color(head);
            if had_color {
                unsupported.push(PlantUmlUnsupported {
                    line: line_no,
                    kind: "styling",
                    source: excerpt(trimmed, 120),
                    reason: "Class background color is not preserved.".to_string(),
                });
            }
            let (head_no_st, explicit_st) = strip_stereotype(head_clean);
            let (id, label) = parse_uml_entity_head(head_no_st);
            let stereo = explicit_st.or(default_stereo);
            let ind = "  ".repeat(open_namespaces + 1);
            let decl_name = match label {
                Some(lbl) => format!("{id}[\"{lbl}\"]"),
                None => id.clone(),
            };
            if opens {
                lines_out.push(format!("{ind}class {decl_name} {{"));
                if let Some(st) = stereo {
                    lines_out.push(format!("{ind}  <<{st}>>"));
                }
                in_class_body = true;
            } else {
                lines_out.push(format!("{ind}class {decl_name}"));
                if let Some(st) = stereo {
                    lines_out.push(format!("{ind}<<{st}>> {id}"));
                }
            }
            has_content = true;
            continue;
        }
        // Notes (`note left/right/top/bottom of X : text` or multi-line)
        if let Some(rest) = strip_prefix_ci(trimmed, "note ") {
            let (head, inline) = match split_colon_outside_quotes(rest) {
                Some((h, t)) => (h.trim(), Some(sanitize_label(t))),
                None => (rest.trim(), None),
            };
            let target = [
                "left of ",
                "right of ",
                "top of ",
                "bottom of ",
                "for ",
            ]
            .iter()
            .find_map(|kw| strip_prefix_ci(head, kw))
            .map(|t| safe_id(t.trim()));
            match inline {
                Some(text) => {
                    let clean_text = text.replace('"', "'");
                    if let Some(t) = target {
                        lines_out.push(format!("  note for {t} \"{clean_text}\""));
                    } else {
                        lines_out.push(format!("  note \"{clean_text}\""));
                    }
                    has_content = true;
                }
                None => {
                    pending_note = Some((line_no, target, Vec::new()));
                }
            }
            continue;
        }
        // Class relationships (`A <|-- B`, `Customer "1" --> "*" Order : places`, `A : +field`)
        let norm = normalize_directional_arrows(trimmed);
        if has_class_relation(&norm) {
            lines_out.push(format!("{}{norm}", "  ".repeat(open_namespaces + 1)));
            has_content = true;
            continue;
        }
        if let Some((lhs, rhs)) = split_colon_outside_quotes(trimmed)
            && !lhs.trim().is_empty()
            && !lhs.trim().contains(char::is_whitespace)
            && !rhs.trim().is_empty()
        {
            let id = safe_id(lhs.trim());
            lines_out.push(format!(
                "{}{id} : {}",
                "  ".repeat(open_namespaces + 1),
                rhs.trim()
            ));
            has_content = true;
            continue;
        }
        unsupported.push(PlantUmlUnsupported {
            line: line_no,
            kind: "syntax",
            source: excerpt(trimmed, 120),
            reason: "Unsupported PlantUML class diagram syntax.".to_string(),
        });
    }

    if let Some((n_line, _, _)) = pending_note {
        unsupported.push(PlantUmlUnsupported {
            line: n_line,
            kind: "syntax",
            source: "note".to_string(),
            reason: "Unclosed multi-line note (missing `end note`).".to_string(),
        });
    }
    if in_class_body {
        lines_out.push("  }".to_string());
    }
    while open_namespaces > 0 {
        open_namespaces -= 1;
        lines_out.push(format!("{}}}", "  ".repeat(open_namespaces + 1)));
    }

    if !has_content {
        return (None, unsupported);
    }
    (
        Some(prepend_frontmatter_title(
            "classDiagram",
            title.as_deref(),
            &lines_out,
        )),
        unsupported,
    )
}

fn has_class_relation(line: &str) -> bool {
    const OPS: &[&str] = &[
        "<|--", "--|>", "<|..", "..|>", "*--", "--*", "o--", "--o", "-->", "<--", "..>", "<..",
        "--", "..",
    ];
    let head = split_colon_outside_quotes(line).map_or(line, |(h, _)| h);
    OPS.iter().any(|op| head.contains(op))
}

// ============================================================================
// 2. PlantUML State Diagram -> Mermaid `stateDiagram-v2`
// ============================================================================

fn convert_plantuml_state(source: &str) -> (Option<String>, Vec<PlantUmlUnsupported>) {
    let mut unsupported = Vec::new();
    let mut lines_out: Vec<String> = Vec::new();
    let mut title: Option<String> = None;
    let mut in_comment = false;
    let mut skip_until: Option<UmlSkip> = None;
    let mut saw_enduml = false;
    let mut open_composites: usize = 0;
    let mut in_multiline_note = false;
    let mut has_content = false;

    for (idx, raw) in source.lines().enumerate() {
        let line_no = idx + 1;
        if saw_enduml {
            continue;
        }
        let cleaned = strip_block_comments(raw, &mut in_comment);
        let trimmed = cleaned.trim();
        if advance_skip(trimmed, &mut skip_until) {
            continue;
        }
        if in_multiline_note {
            let lower = trimmed.to_ascii_lowercase();
            if lower == "end note" || lower == "endnote" {
                lines_out.push(format!("{}end note", "  ".repeat(open_composites + 1)));
                in_multiline_note = false;
            } else {
                lines_out.push(format!("{}{trimmed}", "  ".repeat(open_composites + 2)));
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('\'') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("@start") {
            continue;
        }
        if lower.starts_with("@end") {
            saw_enduml = true;
            continue;
        }
        if handle_common_ignored_or_unsupported(
            line_no,
            trimmed,
            &lower,
            &mut skip_until,
            &mut unsupported,
        ) {
            continue;
        }
        if lower == "left to right direction" {
            lines_out.push("  direction LR".to_string());
            continue;
        }
        if lower == "top to bottom direction" {
            lines_out.push("  direction TB".to_string());
            continue;
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "title ") {
            let t = sanitize_label(rest);
            if !t.is_empty() {
                title = Some(t);
            }
            continue;
        }
        if trimmed == "}" {
            if open_composites > 0 {
                open_composites -= 1;
                lines_out.push(format!("{}}}", "  ".repeat(open_composites + 1)));
            } else {
                unsupported.push(PlantUmlUnsupported {
                    line: line_no,
                    kind: "syntax",
                    source: excerpt(trimmed, 120),
                    reason: "Unexpected closing brace `}`.".to_string(),
                });
            }
            continue;
        }
        if trimmed == "--" || trimmed == "||" {
            lines_out.push(format!("{}--", "  ".repeat(open_composites + 1)));
            continue;
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "state ") {
            let opens = rest.trim_end().ends_with('{');
            let body = rest.trim_end().trim_end_matches('{').trim();
            let (body_clean, had_color) = split_trailing_color(body);
            if had_color {
                unsupported.push(PlantUmlUnsupported {
                    line: line_no,
                    kind: "styling",
                    source: excerpt(trimmed, 120),
                    reason: "State background color is not preserved.".to_string(),
                });
            }
            let ind = "  ".repeat(open_composites + 1);
            if opens {
                lines_out.push(format!("{ind}state {body_clean} {{"));
                open_composites += 1;
            } else {
                lines_out.push(format!("{ind}state {body_clean}"));
            }
            has_content = true;
            continue;
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "note ") {
            let ind = "  ".repeat(open_composites + 1);
            if split_colon_outside_quotes(rest).is_some() {
                lines_out.push(format!("{ind}note {rest}"));
            } else {
                lines_out.push(format!("{ind}note {rest}"));
                in_multiline_note = true;
            }
            has_content = true;
            continue;
        }
        // Transitions (`[*] --> Idle`, `A -right-> B : trigger`) and state descriptions (`S : desc`)
        let norm = normalize_directional_arrows(trimmed);
        if norm.contains("-->") || norm.contains("->") {
            let mermaid_line = if !norm.contains("-->") {
                norm.replacen("->", "-->", 1)
            } else {
                norm
            };
            lines_out.push(format!("{}{mermaid_line}", "  ".repeat(open_composites + 1)));
            has_content = true;
            continue;
        }
        if let Some((lhs, rhs)) = split_colon_outside_quotes(trimmed)
            && !lhs.trim().is_empty()
            && !rhs.trim().is_empty()
        {
            lines_out.push(format!(
                "{}{} : {}",
                "  ".repeat(open_composites + 1),
                lhs.trim(),
                sanitize_label(rhs)
            ));
            has_content = true;
            continue;
        }
        unsupported.push(PlantUmlUnsupported {
            line: line_no,
            kind: "syntax",
            source: excerpt(trimmed, 120),
            reason: "Unsupported PlantUML state diagram syntax.".to_string(),
        });
    }

    while open_composites > 0 {
        open_composites -= 1;
        lines_out.push(format!("{}}}", "  ".repeat(open_composites + 1)));
    }
    if !has_content {
        return (None, unsupported);
    }
    (
        Some(prepend_frontmatter_title(
            "stateDiagram-v2",
            title.as_deref(),
            &lines_out,
        )),
        unsupported,
    )
}

// ============================================================================
// 3. PlantUML Usecase Diagram -> Mermaid `usecase-beta`
// ============================================================================

fn convert_plantuml_usecase(source: &str) -> (Option<String>, Vec<PlantUmlUnsupported>) {
    let mut unsupported = Vec::new();
    let mut lines_out: Vec<String> = Vec::new();
    let mut title: Option<String> = None;
    let mut in_comment = false;
    let mut skip_until: Option<UmlSkip> = None;
    let mut saw_enduml = false;
    let mut open_boundaries: usize = 0;
    let mut alias_map: HashMap<String, String> = HashMap::new();
    let mut pending_note: Option<(usize, Option<String>, Vec<String>)> = None;
    let mut has_content = false;

    let resolve_uc_ref =
        |raw: &str, alias_map: &mut HashMap<String, String>, lines: &mut Vec<String>| -> String {
            let s = raw.trim();
            if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
                let lbl = sanitize_label(strip_quotes(inner));
                if let Some(id) = alias_map.get(&lbl) {
                    return id.clone();
                }
                let id = safe_id(&lbl);
                alias_map.insert(lbl.clone(), id.clone());
                lines.push(format!("  {id}(\"{lbl}\")"));
                return id;
            }
            if let Some(inner) = s.strip_prefix(':').and_then(|r| r.strip_suffix(':')) {
                let lbl = sanitize_label(strip_quotes(inner));
                if let Some(id) = alias_map.get(&lbl) {
                    return id.clone();
                }
                let id = safe_id(&lbl);
                alias_map.insert(lbl.clone(), id.clone());
                lines.push(format!("  actor {id}(\"{lbl}\")"));
                return id;
            }
            let unq = strip_quotes(s);
            if let Some(id) = alias_map.get(unq) {
                return id.clone();
            }
            safe_id(unq)
        };

    for (idx, raw) in source.lines().enumerate() {
        let line_no = idx + 1;
        if saw_enduml {
            continue;
        }
        let cleaned = strip_block_comments(raw, &mut in_comment);
        let trimmed = cleaned.trim();
        if advance_skip(trimmed, &mut skip_until) {
            continue;
        }
        if let Some((n_line, target, mut n_lines)) = pending_note.take() {
            let lower = trimmed.to_ascii_lowercase();
            if lower == "end note" || lower == "endnote" {
                let text = sanitize_label(&n_lines.join(" / ")).replace('"', "'");
                if let Some(t) = target {
                    lines_out.push(format!("  note for {t} \"{text}\""));
                    has_content = true;
                }
            } else {
                if !trimmed.is_empty() {
                    n_lines.push(trimmed.to_string());
                }
                pending_note = Some((n_line, target, n_lines));
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('\'') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("@start") {
            continue;
        }
        if lower.starts_with("@end") {
            saw_enduml = true;
            continue;
        }
        if handle_common_ignored_or_unsupported(
            line_no,
            trimmed,
            &lower,
            &mut skip_until,
            &mut unsupported,
        ) {
            continue;
        }
        if lower == "left to right direction" {
            lines_out.push("  direction LR".to_string());
            continue;
        }
        if lower == "top to bottom direction" {
            lines_out.push("  direction TB".to_string());
            continue;
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "title ") {
            let t = sanitize_label(rest);
            if !t.is_empty() {
                title = Some(t);
            }
            continue;
        }
        if trimmed == "}" {
            if open_boundaries > 0 {
                open_boundaries -= 1;
                lines_out.push("  end".to_string());
            } else {
                unsupported.push(PlantUmlUnsupported {
                    line: line_no,
                    kind: "syntax",
                    source: excerpt(trimmed, 120),
                    reason: "Unexpected closing brace `}`.".to_string(),
                });
            }
            continue;
        }
        // `rectangle "System" {` / `package "Pkg" {`
        if let Some((is_pkg, rest)) = strip_prefix_ci(trimmed, "rectangle ")
            .map(|r| (false, r))
            .or_else(|| strip_prefix_ci(trimmed, "package ").map(|r| (true, r)))
        {
            let opens = rest.trim_end().ends_with('{');
            let head = rest.trim_end().trim_end_matches('{').trim();
            let (head_clean, _) = split_trailing_color(head);
            let (id, label) = parse_uml_entity_head(head_clean);
            let title_lbl = label.unwrap_or_else(|| id.clone());
            if opens {
                if is_pkg {
                    lines_out.push(format!(
                        r#"  systemBoundary {id}("{title_lbl}")@{{ "type": "package" }}"#
                    ));
                } else {
                    lines_out.push(format!(r#"  systemBoundary {id}("{title_lbl}")"#));
                }
                open_boundaries += 1;
                has_content = true;
            }
            continue;
        }
        // `actor ...`
        if let Some(rest) = strip_prefix_ci(trimmed, "actor ") {
            let (clean, had_color) = split_trailing_color(rest.trim());
            if had_color {
                unsupported.push(PlantUmlUnsupported {
                    line: line_no,
                    kind: "styling",
                    source: excerpt(trimmed, 120),
                    reason: "Actor color is not preserved.".to_string(),
                });
            }
            let (clean, stereo) = strip_stereotype(clean);
            let normalized = clean.replace(':', "\"");
            let (id, label) = parse_uml_entity_head(&normalized);
            let lbl = label.unwrap_or_else(|| id.clone());
            alias_map.insert(lbl.clone(), id.clone());
            alias_map.insert(id.clone(), id.clone());
            let st_part = stereo.map_or(String::new(), |s| format!(" <<{s}>>"));
            lines_out.push(format!("  actor {id}(\"{lbl}\"){st_part}"));
            has_content = true;
            continue;
        }
        // `usecase ...`
        if let Some(rest) = strip_prefix_ci(trimmed, "usecase ") {
            let (clean, had_color) = split_trailing_color(rest.trim());
            if had_color {
                unsupported.push(PlantUmlUnsupported {
                    line: line_no,
                    kind: "styling",
                    source: excerpt(trimmed, 120),
                    reason: "Usecase color is not preserved.".to_string(),
                });
            }
            let (clean, stereo) = strip_stereotype(clean);
            let normalized = if let Some((lhs, rhs)) = split_as_outside_quotes(clean) {
                let lhs_unparen = lhs
                    .trim()
                    .strip_prefix('(')
                    .and_then(|r| r.strip_suffix(')'))
                    .unwrap_or(lhs.trim());
                format!("\"{}\" as {}", strip_quotes(lhs_unparen), rhs.trim())
            } else {
                let unparen = clean
                    .strip_prefix('(')
                    .and_then(|r| r.strip_suffix(')'))
                    .unwrap_or(clean);
                format!("\"{}\"", strip_quotes(unparen))
            };
            let (id, label) = parse_uml_entity_head(&normalized);
            let lbl = label.unwrap_or_else(|| id.clone());
            alias_map.insert(lbl.clone(), id.clone());
            alias_map.insert(id.clone(), id.clone());
            let st_part = stereo.map_or(String::new(), |s| format!(" <<{s}>>"));
            lines_out.push(format!("  {id}(\"{lbl}\"){st_part}"));
            has_content = true;
            continue;
        }
        // Standalone `(Use Case) as UC` or `:Actor: as A`
        if (trimmed.starts_with('(') || trimmed.starts_with(':'))
            && split_as_outside_quotes(trimmed).is_some()
        {
            let is_actor = trimmed.starts_with(':');
            let (lhs, rhs) = split_as_outside_quotes(trimmed).unwrap();
            let inner = if is_actor {
                lhs.trim()
                    .strip_prefix(':')
                    .and_then(|r| r.strip_suffix(':'))
                    .unwrap_or(lhs.trim())
            } else {
                lhs.trim()
                    .strip_prefix('(')
                    .and_then(|r| r.strip_suffix(')'))
                    .unwrap_or(lhs.trim())
            };
            let lbl = sanitize_label(strip_quotes(inner));
            let id = safe_id(rhs.trim());
            alias_map.insert(lbl.clone(), id.clone());
            alias_map.insert(id.clone(), id.clone());
            if is_actor {
                lines_out.push(format!("  actor {id}(\"{lbl}\")"));
            } else {
                lines_out.push(format!("  {id}(\"{lbl}\")"));
            }
            has_content = true;
            continue;
        }
        // Notes
        if let Some(rest) = strip_prefix_ci(trimmed, "note ") {
            let (head, inline) = match split_colon_outside_quotes(rest) {
                Some((h, t)) => (h.trim(), Some(sanitize_label(t))),
                None => (rest.trim(), None),
            };
            let target = ["left of ", "right of ", "top of ", "bottom of ", "for "]
                .iter()
                .find_map(|kw| strip_prefix_ci(head, kw))
                .map(|t| resolve_uc_ref(t, &mut alias_map, &mut lines_out));
            match inline {
                Some(text) => {
                    if let Some(t) = target {
                        lines_out.push(format!("  note for {t} \"{}\"", text.replace('"', "'")));
                        has_content = true;
                    }
                }
                None => pending_note = Some((line_no, target, Vec::new())),
            }
            continue;
        }
        // Links (`U --> UC1`, `UC1 ..> UC2 : <<include>>`, `U -- (Login)`)
        let norm = normalize_directional_arrows(trimmed);
        if let Some((lhs_raw, op, rhs_raw, label_raw)) = split_usecase_link(&norm) {
            let lhs = resolve_uc_ref(lhs_raw, &mut alias_map, &mut lines_out);
            let rhs = resolve_uc_ref(rhs_raw, &mut alias_map, &mut lines_out);
            let label = label_raw.map(sanitize_label).filter(|l| !l.is_empty());
            let emitted = if op == "<|--" {
                format!("  {rhs} --|> {lhs}")
            } else if op == "--|>" {
                format!("  {lhs} --|> {rhs}")
            } else if let Some(lbl) = label {
                if let Some(st) = lbl.strip_prefix("<<").and_then(|r| r.strip_suffix(">>")) {
                    format!("  {lhs} {op} : {} {rhs}", st.trim())
                } else {
                    let clean_lbl = lbl.replace('"', "'");
                    let lead = if op.contains('.') { ".." } else { "--" };
                    format!("  {lhs} {lead} \"{clean_lbl}\" {op} {rhs}")
                }
            } else {
                format!("  {lhs} {op} {rhs}")
            };
            lines_out.push(emitted);
            has_content = true;
            continue;
        }
        unsupported.push(PlantUmlUnsupported {
            line: line_no,
            kind: "syntax",
            source: excerpt(trimmed, 120),
            reason: "Unsupported PlantUML usecase diagram syntax.".to_string(),
        });
    }

    while open_boundaries > 0 {
        open_boundaries -= 1;
        lines_out.push("  end".to_string());
    }
    if !has_content {
        return (None, unsupported);
    }
    (
        Some(prepend_frontmatter_title(
            "usecase-beta",
            title.as_deref(),
            &lines_out,
        )),
        unsupported,
    )
}

fn split_usecase_link(line: &str) -> Option<(&str, &'static str, &str, Option<&str>)> {
    let (expr, label) = match split_colon_outside_quotes(line) {
        Some((e, l)) => (e.trim(), Some(l.trim())),
        None => (line.trim(), None),
    };
    const OPS: &[(&str, &str)] = &[
        ("<|--", "<|--"),
        ("--|>", "--|>"),
        ("..>", "..>"),
        ("<..", "<.."),
        ("-->", "-->"),
        ("<--", "<--"),
        ("->", "-->"),
        ("<-", "<--"),
        ("..", ".."),
        ("--", "--"),
    ];
    for &(pat, mapped) in OPS {
        if let Some((lhs, rhs)) = split_outside_delims(expr, pat) {
            let lhs = lhs.trim();
            let rhs = rhs.trim();
            if !lhs.is_empty() && !rhs.is_empty() {
                return Some((lhs, mapped, rhs, label));
            }
        }
    }
    None
}

fn split_outside_delims<'a>(s: &'a str, pat: &str) -> Option<(&'a str, &'a str)> {
    let mut in_quotes = false;
    let mut paren_depth = 0usize;
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b'(' if !in_quotes => paren_depth += 1,
            b')' if !in_quotes => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
        if !in_quotes
            && paren_depth == 0
            && s.is_char_boundary(i)
            && s[i..].starts_with(pat)
        {
            return Some((&s[..i], &s[i + pat.len()..]));
        }
    }
    None
}

// ============================================================================
// 4. PlantUML ER Diagram -> Mermaid `erDiagram`
// ============================================================================

fn convert_plantuml_er(source: &str) -> (Option<String>, Vec<PlantUmlUnsupported>) {
    let mut unsupported = Vec::new();
    let mut lines_out: Vec<String> = Vec::new();
    let mut title: Option<String> = None;
    let mut in_comment = false;
    let mut skip_until: Option<UmlSkip> = None;
    let mut saw_enduml = false;
    let mut in_entity = false;
    let mut has_content = false;

    for (idx, raw) in source.lines().enumerate() {
        let line_no = idx + 1;
        if saw_enduml {
            continue;
        }
        let cleaned = strip_block_comments(raw, &mut in_comment);
        let trimmed = cleaned.trim();
        if advance_skip(trimmed, &mut skip_until) {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('\'') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("@start") {
            continue;
        }
        if lower.starts_with("@end") {
            saw_enduml = true;
            continue;
        }
        if handle_common_ignored_or_unsupported(
            line_no,
            trimmed,
            &lower,
            &mut skip_until,
            &mut unsupported,
        ) {
            continue;
        }
        if in_entity {
            if trimmed == "}" {
                lines_out.push("  }".to_string());
                in_entity = false;
                continue;
            }
            if trimmed.chars().all(|c| matches!(c, '-' | '=' | '.')) {
                continue;
            }
            if let Some(attr) = convert_er_attribute(trimmed) {
                lines_out.push(format!("    {attr}"));
            }
            continue;
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "title ") {
            let t = sanitize_label(rest);
            if !t.is_empty() {
                title = Some(t);
            }
            continue;
        }
        if let Some(rest) = strip_prefix_ci(trimmed, "entity ") {
            let opens = rest.trim_end().ends_with('{');
            let head = rest.trim_end().trim_end_matches('{').trim();
            let (head_clean, had_color) = split_trailing_color(head);
            if had_color {
                unsupported.push(PlantUmlUnsupported {
                    line: line_no,
                    kind: "styling",
                    source: excerpt(trimmed, 120),
                    reason: "Entity color is not preserved.".to_string(),
                });
            }
            let (head_no_st, _) = strip_stereotype(head_clean);
            let (id, label) = parse_uml_entity_head(head_no_st);
            let header = match label {
                Some(lbl) => format!("{id}[\"{lbl}\"]"),
                None => id,
            };
            if opens {
                lines_out.push(format!("  {header} {{"));
                in_entity = true;
            } else {
                lines_out.push(format!("  {header} {{}}"));
            }
            has_content = true;
            continue;
        }
        let norm = normalize_directional_arrows(trimmed);
        if contains_crows_foot(&norm) {
            let (expr, label) = match split_colon_outside_quotes(&norm) {
                Some((e, l)) => (e.trim(), sanitize_label(l)),
                None => (norm.trim(), String::new()),
            };
            let lbl = if label.is_empty() {
                "\"\"".to_string()
            } else {
                format!("\"{}\"", label.replace('"', "'"))
            };
            lines_out.push(format!("  {expr} : {lbl}"));
            has_content = true;
            continue;
        }
        unsupported.push(PlantUmlUnsupported {
            line: line_no,
            kind: "syntax",
            source: excerpt(trimmed, 120),
            reason: "Unsupported PlantUML ER diagram syntax.".to_string(),
        });
    }

    if in_entity {
        lines_out.push("  }".to_string());
    }
    if !has_content {
        return (None, unsupported);
    }
    (
        Some(prepend_frontmatter_title(
            "erDiagram",
            title.as_deref(),
            &lines_out,
        )),
        unsupported,
    )
}

/// Convert PlantUML ER attribute syntax (`* id : number <<PK>>`, `user_id : int <<FK>>`,
/// `name : varchar`) into Mermaid `erDiagram` (`number id PK`).
fn convert_er_attribute(line: &str) -> Option<String> {
    let s = line
        .trim_start_matches(['*', '+', '-', '#', '~'])
        .trim();
    if s.is_empty() {
        return None;
    }
    let (clean, stereo) = strip_stereotype(s);
    let key = stereo
        .map(|st| {
            st.split(',')
                .map(str::trim)
                .filter(|k| matches!(k.to_ascii_uppercase().as_str(), "PK" | "FK" | "UK"))
                .map(|k| k.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    let (name, ty) = if let Some((lhs, rhs)) = split_colon_outside_quotes(clean) {
        let n = safe_id(lhs.trim());
        let t = rhs
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("string")
            .to_string();
        (n, t)
    } else {
        let parts: Vec<&str> = clean.split_whitespace().collect();
        match parts.as_slice() {
            [ty, name, ..] => (safe_id(name), (*ty).to_string()),
            [name] => (safe_id(name), "string".to_string()),
            [] => return None,
        }
    };
    if key.is_empty() {
        Some(format!("{ty} {name}"))
    } else {
        Some(format!("{ty} {name} {key}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_plantuml_class_diagram_via_shared_class_renderer() {
        let puml = "\
@startuml
title 注文ドメイン
package \"注文管理\" as Orders {
  interface Payable {
    +pay(amount: int)
  }
  class \"注文\" as Order {
    +id: String
    +total(): int
  }
  class OrderItem {
    -qty: int
  }
}
Order \"1\" *-- \"*\" OrderItem : 明細
Order ..|> Payable
note right of Order : 集約ルート
@enduml
";
        let res = render_plantuml(puml);
        assert_eq!(res.original, puml);
        assert!(res.is_lossless(), "unexpected diagnostics: {:?}", res.unsupported);
        let Some(MermaidOutput::Svg { bytes, .. }) = res.output else {
            panic!("expected SVG for PlantUML class diagram");
        };
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(roxmltree::Document::parse(svg).is_ok(), "invalid SVG");
        assert!(svg.contains("注文ドメイン"), "{svg}");
        assert!(svg.contains("注文管理"), "{svg}");
        assert!(svg.contains("Payable") && svg.contains("OrderItem"), "{svg}");
        assert!(svg.contains("集約ルート"), "{svg}");
    }

    #[test]
    fn renders_plantuml_state_diagram_via_shared_state_renderer() {
        let puml = "\
@startuml
title 審査状態
[*] --> Draft
state \"審査中\" as Review {
  [*] --> Checking
  Checking -right-> Approved : 承認
  --
  [*] --> Logging
}
Draft --> Review : 提出
Review --> [*]
note right of Draft : 編集可能
@enduml
";
        let res = render_plantuml(puml);
        assert!(res.is_lossless(), "unexpected diagnostics: {:?}", res.unsupported);
        let Some(MermaidOutput::Svg { bytes, .. }) = res.output else {
            panic!("expected SVG for PlantUML state diagram");
        };
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(roxmltree::Document::parse(svg).is_ok(), "invalid SVG");
        assert!(svg.contains("審査状態"), "{svg}");
        assert!(svg.contains("審査中") && svg.contains("Checking"), "{svg}");
        assert!(svg.contains("編集可能"), "{svg}");
    }

    #[test]
    fn renders_plantuml_usecase_diagram_via_shared_usecase_renderer() {
        let puml = "\
@startuml
left to right direction
actor \"申請者\" as User
rectangle \"申請システム\" as Sys {
  usecase \"申請書作成\" as UC1
  usecase \"本人確認\" as UC2
}
User --> UC1
UC1 ..> UC2 : <<include>>
note right of UC1 : オンライン入力
@enduml
";
        let res = render_plantuml(puml);
        assert!(res.is_lossless(), "unexpected diagnostics: {:?}", res.unsupported);
        let Some(MermaidOutput::Svg { bytes, .. }) = res.output else {
            panic!("expected SVG for PlantUML usecase diagram");
        };
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(roxmltree::Document::parse(svg).is_ok(), "invalid SVG");
        assert!(svg.contains("申請者"), "{svg}");
        assert!(svg.contains("申請システム"), "{svg}");
        assert!(svg.contains("申請書作成") && svg.contains("本人確認"), "{svg}");
        assert!(svg.contains("オンライン入力"), "{svg}");
    }

    #[test]
    fn renders_plantuml_er_diagram_via_shared_er_renderer() {
        let puml = "\
@startuml
entity \"顧客\" as CUSTOMER {
  * id : number <<PK>>
  --
  + name : varchar
}
entity ORDER {
  * id : number <<PK>>
  customer_id : number <<FK>>
}
CUSTOMER ||--o{ ORDER : places
@enduml
";
        let res = render_plantuml(puml);
        assert!(res.is_lossless(), "unexpected diagnostics: {:?}", res.unsupported);
        let Some(MermaidOutput::Svg { bytes, .. }) = res.output else {
            panic!("expected SVG for PlantUML ER diagram");
        };
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(roxmltree::Document::parse(svg).is_ok(), "invalid SVG");
        assert!(svg.contains("顧客"), "{svg}");
        assert!(svg.contains("ORDER"), "{svg}");
        assert!(svg.contains("PK") && svg.contains("FK"), "{svg}");
        assert!(svg.contains("places"), "{svg}");
    }

    #[test]
    fn unified_render_plantuml_dispatches_sequence_and_rejects_timing() {
        let seq = "@startuml\nhide footbox\nAlice -> Bob : hi\n== part 2 ==\nBob --> Alice : ok\n@enduml\n";
        let res = render_plantuml(seq);
        assert!(res.is_lossless(), "unexpected diagnostics: {:?}", res.unsupported);
        let Some(MermaidOutput::Svg { bytes, .. }) = res.output else {
            panic!("expected SVG for sequence");
        };
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(svg.contains(r#"data-shape="seq-divider""#), "{svg}");

        let timing = "@startuml\nrobust \"Browser\" as B\n@0\nB is Idle\n@enduml\n";
        let timing_res = render_plantuml(timing);
        assert!(timing_res.output.is_none());
        assert!(timing_res.unsupported.iter().any(|u| u.kind == "timing"));
    }

    #[test]
    fn markdown_plantuml_fences_render_svg_and_roundtrip_original_verbatim() {
        let md = "\
# PlantUML 検証

```plantuml
@startuml
hide footbox
Alice -> Bob : 要求
== 後半 ==
Bob --> Alice : 応答
@enduml
```

```puml
@startuml
class Order {
  +id: String
}
class Item
Order *-- Item
@enduml
```

```plantuml
@startuml
robust \"Web\" as W
@0
W is Idle
@enduml
```
";
        let pres = crate::markdown::read_markdown(md.as_bytes()).expect("read markdown");
        // Sequence and Class blocks render as SVG pictures; Timing falls back to code block.
        let svg_count = pres
            .media
            .iter()
            .filter(|m| m.format == ibis_doc_model::MediaFormat::Svg)
            .count();
        assert_eq!(svg_count, 2, "expected 2 rendered PlantUML SVGs");

        let exported =
            crate::markdown::export_presentation_to_markdown(&pres).expect("export markdown");
        assert!(exported.contains("```plantuml\n@startuml\nhide footbox\n"), "{exported}");
        assert!(exported.contains("```puml\n@startuml\nclass Order {\n"), "{exported}");
        assert!(exported.contains("robust \"Web\" as W"), "{exported}");
    }
}

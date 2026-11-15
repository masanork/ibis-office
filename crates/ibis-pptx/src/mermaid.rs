// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust Mermaid diagram parser and SVG / native-chart renderer.
//!
//! Zero external dependencies: converts Mermaid code blocks into either:
//! - Native Office charts (`pie`, `xychart-beta`) via [`MermaidChartSpec`], or
//! - Clean, self-contained SVG vector graphics (`sequenceDiagram`, `flowchart` / `graph`)
//!   embedded as `MediaFormat::Svg` with the original Mermaid source preserved in `PictureLook::alt_text`
//!   for lossless `.md` round-tripping.

use std::collections::HashMap;

use ibis_doc_model::Number;

use crate::save::pptx_new_chart::NewChartKind;

/// Parsed native chart specification from Mermaid `pie` or `xychart-beta`.
#[derive(Debug, Clone, PartialEq)]
pub struct MermaidChartSpec {
    pub kind: NewChartKind,
    pub title: Option<String>,
    pub categories: Vec<String>,
    pub series: Vec<(String, Vec<Option<Number>>)>,
}

/// Result of converting a Mermaid block.
#[derive(Debug, Clone, PartialEq)]
pub enum MermaidOutput {
    /// Native Office chart (`pie`, `xychart-beta`).
    Chart(MermaidChartSpec),
    /// Self-contained SVG vector graphic (`svg_bytes`, `width_px`, `height_px`).
    Svg {
        bytes: Vec<u8>,
        width_px: u32,
        height_px: u32,
    },
}

/// Parse and render a Mermaid diagram into either a native chart or an SVG.
/// Returns `None` if the diagram type is unsupported (caller falls back to a fenced code block).
pub fn render_mermaid(source: &str) -> Option<MermaidOutput> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first_line = trimmed
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("%%"))?;

    if first_line == "pie" || first_line.starts_with("pie ") {
        return parse_pie_chart(trimmed).map(MermaidOutput::Chart);
    }
    if first_line == "xychart-beta" || first_line.starts_with("xychart-beta ") {
        return parse_xy_chart(trimmed).map(MermaidOutput::Chart);
    }
    if first_line == "sequenceDiagram" || first_line.starts_with("sequenceDiagram ") {
        let (bytes, width_px, height_px) = render_sequence_svg(trimmed)?;
        return Some(MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        });
    }
    if first_line.starts_with("flowchart ")
        || first_line == "flowchart"
        || first_line.starts_with("graph ")
        || first_line == "graph"
    {
        let (bytes, width_px, height_px) = render_flowchart_svg(trimmed)?;
        return Some(MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        });
    }
    None
}

// ============================================================================
// 1. Native Chart Parsers (`pie`, `xychart-beta`)
// ============================================================================

fn parse_pie_chart(source: &str) -> Option<MermaidChartSpec> {
    let mut title = None;
    let mut categories = Vec::new();
    let mut values = Vec::new();

    for line in source.lines().skip(1) {
        let l = line.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("title ") {
            let t = unquote(rest.trim());
            if !t.is_empty() {
                title = Some(t);
            }
            continue;
        }
        if let Some((label_part, val_part)) = l.split_once(':') {
            let label = unquote(label_part.trim());
            if let Ok(v) = val_part.trim().parse::<f64>()
                && !label.is_empty()
            {
                categories.push(label);
                values.push(Some(Number(v)));
            }
        }
    }

    if categories.is_empty() {
        return None;
    }
    Some(MermaidChartSpec {
        kind: NewChartKind::Pie,
        title,
        categories,
        series: vec![("Series 1".to_string(), values)],
    })
}

fn parse_xy_chart(source: &str) -> Option<MermaidChartSpec> {
    let mut title = None;
    let mut categories = Vec::new();
    let mut series = Vec::new();
    let mut has_bar = false;
    let mut has_line = false;
    let mut bar_idx = 1usize;
    let mut line_idx = 1usize;

    for line in source.lines().skip(1) {
        let l = line.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("title ") {
            let t = unquote(rest.trim());
            if !t.is_empty() {
                title = Some(t);
            }
        } else if let Some(rest) = l.strip_prefix("x-axis ") {
            if let Some(list) = extract_bracket_list(rest) {
                categories = list.into_iter().map(|s| unquote(&s)).collect();
            }
        } else if let Some(rest) = l.strip_prefix("bar ") {
            if let Some(list) = extract_bracket_list(rest) {
                has_bar = true;
                let vals: Vec<_> = list
                    .into_iter()
                    .map(|s| s.trim().parse::<f64>().ok().map(Number))
                    .collect();
                series.push((format!("Bar {bar_idx}"), vals));
                bar_idx += 1;
            }
        } else if let Some(rest) = l.strip_prefix("line ") {
            if let Some(list) = extract_bracket_list(rest) {
                has_line = true;
                let vals: Vec<_> = list
                    .into_iter()
                    .map(|s| s.trim().parse::<f64>().ok().map(Number))
                    .collect();
                series.push((format!("Line {line_idx}"), vals));
                line_idx += 1;
            }
        }
    }

    if series.is_empty() {
        return None;
    }
    if categories.is_empty() {
        let max_len = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
        categories = (1..=max_len).map(|i| i.to_string()).collect();
    }
    let kind = if has_bar {
        NewChartKind::ClusteredColumn
    } else if has_line {
        NewChartKind::Line
    } else {
        NewChartKind::ClusteredColumn
    };
    Some(MermaidChartSpec {
        kind,
        title,
        categories,
        series,
    })
}

fn extract_bracket_list(s: &str) -> Option<Vec<String>> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end <= start {
        return None;
    }
    Some(
        s[start + 1..end]
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
    )
}

// ============================================================================
// 2. Sequence Diagram -> Pure Rust SVG
// ============================================================================

#[derive(Debug, Clone)]
struct SeqParticipant {
    _id: String,
    label: String,
    is_actor: bool,
}

#[derive(Debug, Clone)]
enum SeqItem {
    Message {
        from: usize,
        to: usize,
        label: String,
        dashed: bool,
        open_arrow: bool,
        cross_end: bool,
    },
    Note {
        from: usize,
        to: usize,
        pos: SeqNotePos,
        text: String,
    },
    BlockStart {
        kind: String,
        label: String,
    },
    BlockElse {
        label: String,
    },
    BlockEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeqNotePos {
    Left,
    Right,
    Over,
}

fn render_sequence_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut participants: Vec<SeqParticipant> = Vec::new();
    let mut part_map: HashMap<String, usize> = HashMap::new();
    let mut items: Vec<SeqItem> = Vec::new();
    let mut autonumber = false;

    let ensure_participant = |id: &str,
                              label: Option<&str>,
                              is_actor: bool,
                              participants: &mut Vec<SeqParticipant>,
                              part_map: &mut HashMap<String, usize>|
     -> usize {
        if let Some(&idx) = part_map.get(id) {
            if let Some(lbl) = label {
                participants[idx].label = lbl.to_string();
            }
            if is_actor {
                participants[idx].is_actor = true;
            }
            return idx;
        }
        let idx = participants.len();
        participants.push(SeqParticipant {
            _id: id.to_string(),
            label: label.unwrap_or(id).to_string(),
            is_actor,
        });
        part_map.insert(id.to_string(), idx);
        idx
    };

    for raw_line in source.lines().skip(1) {
        let l = raw_line.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if l == "autonumber" {
            autonumber = true;
            continue;
        }
        if let Some(rest) = l.strip_prefix("participant ") {
            let (id, lbl) = split_alias(rest);
            ensure_participant(id, lbl, false, &mut participants, &mut part_map);
            continue;
        }
        if let Some(rest) = l.strip_prefix("actor ") {
            let (id, lbl) = split_alias(rest);
            ensure_participant(id, lbl, true, &mut participants, &mut part_map);
            continue;
        }
        if l == "end" {
            items.push(SeqItem::BlockEnd);
            continue;
        }
        if let Some(rest) = l.strip_prefix("else") {
            items.push(SeqItem::BlockElse {
                label: rest.trim().to_string(),
            });
            continue;
        }
        for kw in ["loop ", "alt ", "opt ", "par ", "critical ", "break "] {
            if let Some(rest) = l.strip_prefix(kw) {
                items.push(SeqItem::BlockStart {
                    kind: kw.trim().to_string(),
                    label: rest.trim().to_string(),
                });
                break;
            }
        }
        if matches!(items.last(), Some(SeqItem::BlockStart { .. })) && l.starts_with(|c: char| c.is_ascii_alphabetic())
            && ["loop ", "alt ", "opt ", "par ", "critical ", "break "].iter().any(|kw| l.starts_with(kw))
        {
            continue;
        }
        if let Some(note_item) = parse_seq_note(l, &mut participants, &mut part_map, &ensure_participant) {
            items.push(note_item);
            continue;
        }
        if let Some(msg_item) = parse_seq_message(l, &mut participants, &mut part_map, &ensure_participant) {
            items.push(msg_item);
            continue;
        }
    }

    if participants.is_empty() {
        return None;
    }

    // Layout geometry
    let margin_x = 32.0f32;
    let top_y = 24.0f32;
    let box_h = 38.0f32;
    let col_widths: Vec<f32> = participants
        .iter()
        .map(|p| (estimate_text_width(&p.label, 13.0) + 32.0).max(96.0))
        .collect();

    // Compute horizontal centers ensuring message labels have room.
    let mut col_gaps: Vec<f32> = vec![56.0; participants.len().saturating_sub(1)];
    for item in &items {
        if let SeqItem::Message { from, to, label, .. } = item
            && from != to
        {
            let a = (*from).min(*to);
            let b = (*from).max(*to);
            let needed = estimate_text_width(label, 12.0) + 44.0;
            let span_cols = (b - a) as f32;
            let per_gap = (needed / span_cols - 60.0).max(48.0);
            for g in &mut col_gaps[a..b] {
                if per_gap > *g {
                    *g = per_gap;
                }
            }
        }
    }

    let mut centers = Vec::with_capacity(participants.len());
    let mut cursor_x = margin_x;
    for (idx, w) in col_widths.iter().enumerate() {
        centers.push(cursor_x + w / 2.0);
        cursor_x += w;
        if idx < col_gaps.len() {
            cursor_x += col_gaps[idx];
        }
    }
    let total_w = (cursor_x + margin_x).ceil() as u32;

    let mut y = top_y + box_h + 28.0;
    let row_step = 44.0f32;
    let self_step = 54.0f32;
    let mut item_y = Vec::with_capacity(items.len());
    let mut block_stack: Vec<(usize, f32)> = Vec::new();
    let mut block_boxes: Vec<(String, String, f32, f32, usize)> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        match item {
            SeqItem::Message { from, to, .. } => {
                item_y.push(y);
                y += if from == to { self_step } else { row_step };
            }
            SeqItem::Note { .. } => {
                item_y.push(y);
                y += 42.0;
            }
            SeqItem::BlockStart { kind, label } => {
                item_y.push(y);
                block_stack.push((idx, y - 14.0));
                let _ = (kind, label);
                y += 28.0;
            }
            SeqItem::BlockElse { .. } => {
                item_y.push(y);
                y += 26.0;
            }
            SeqItem::BlockEnd => {
                item_y.push(y);
                if let Some((start_idx, start_y)) = block_stack.pop()
                    && let SeqItem::BlockStart { kind, label } = &items[start_idx]
                {
                    block_boxes.push((kind.clone(), label.clone(), start_y, y + 6.0, block_stack.len()));
                }
                y += 22.0;
            }
        }
    }

    let bottom_box_y = y + 12.0;
    let total_h = (bottom_box_y + box_h + 24.0).ceil() as u32;

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(
        r##"<defs>
  <marker id="seq-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
    <path d="M 0 1 L 10 5 L 0 9 z" fill="#334155"/>
  </marker>
  <marker id="seq-open" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
    <path d="M 1 1 L 9 5 L 1 9" fill="none" stroke="#334155" stroke-width="1.6"/>
  </marker>
</defs>"##,
    );
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));

    // Lifelines
    for &cx in &centers {
        svg.push_str(&format!(
            r##"<line x1="{cx:.1}" y1="{:.1}" x2="{cx:.1}" y2="{bottom_box_y:.1}" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="5,4"/>"##,
            top_y + box_h
        ));
    }

    // Combined fragment frames (`alt`, `loop`, etc.)
    for (kind, label, sy, ey, depth) in &block_boxes {
        let inset = (*depth as f32) * 8.0;
        let bx = 14.0 + inset;
        let bw = (total_w as f32) - 28.0 - inset * 2.0;
        let bh = (ey - sy).max(28.0);
        svg.push_str(&format!(
            r##"<rect x="{bx:.1}" y="{sy:.1}" width="{bw:.1}" height="{bh:.1}" rx="4" fill="#f8fafc" fill-opacity="0.55" stroke="#64748b" stroke-width="1.2"/>"##
        ));
        let tag_w = (estimate_text_width(kind, 11.0) + 18.0).max(46.0);
        svg.push_str(&format!(
            r##"<path d="M {bx:.1} {sy:.1} H {:.1} V {:.1} L {:.1} {:.1} H {bx:.1} Z" fill="#e2e8f0" stroke="#64748b" stroke-width="1"/>"##,
            bx + tag_w,
            sy + 14.0,
            bx + tag_w - 6.0,
            sy + 20.0
        ));
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" font-size="11" font-weight="bold" fill="#1e293b">{}</text>"##,
            bx + 8.0,
            sy + 14.0,
            escape_xml(kind)
        ));
        if !label.is_empty() {
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="{:.1}" font-size="11" fill="#334155">[{}]</text>"##,
                bx + tag_w + 8.0,
                sy + 14.0,
                escape_xml(label)
            ));
        }
    }

    // Messages, notes, else dividers
    let mut msg_num = 0usize;
    for (idx, item) in items.iter().enumerate() {
        let y = item_y[idx];
        match item {
            SeqItem::BlockElse { label } => {
                svg.push_str(&format!(
                    r##"<line x1="18" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="#64748b" stroke-width="1" stroke-dasharray="4,4"/>"##,
                    total_w as f32 - 18.0
                ));
                if !label.is_empty() {
                    svg.push_str(&format!(
                        r##"<text x="28" y="{:.1}" font-size="11" fill="#475569">[{}]</text>"##,
                        y + 14.0,
                        escape_xml(label)
                    ));
                }
            }
            SeqItem::Note { from, to, pos, text } => {
                let nw = (estimate_text_width(text, 11.5) + 22.0).max(80.0);
                let nh = 28.0f32;
                let nx = match pos {
                    SeqNotePos::Left => centers[*from] - nw - 12.0,
                    SeqNotePos::Right => centers[*from] + 12.0,
                    SeqNotePos::Over => (centers[*from] + centers[*to]) / 2.0 - nw / 2.0,
                }
                .max(8.0);
                svg.push_str(&format!(
                    r##"<rect x="{nx:.1}" y="{:.1}" width="{nw:.1}" height="{nh:.1}" rx="3" fill="#fef9c3" stroke="#ca8a04" stroke-width="1.1"/>"##,
                    y - 14.0
                ));
                svg.push_str(&format!(
                    r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="11.5" fill="#713f12">{}</text>"##,
                    nx + nw / 2.0,
                    y + 4.0,
                    escape_xml(text)
                ));
            }
            SeqItem::Message {
                from,
                to,
                label,
                dashed,
                open_arrow,
                cross_end,
            } => {
                msg_num += 1;
                let display_label = if autonumber {
                    format!("{msg_num}. {label}")
                } else {
                    label.clone()
                };
                let x1 = centers[*from];
                let x2 = centers[*to];
                let dash_attr = if *dashed { r##" stroke-dasharray="6,4""## } else { "" };
                let marker_attr = if *cross_end {
                    ""
                } else if *open_arrow {
                    r##" marker-end="url(#seq-open)""##
                } else {
                    r##" marker-end="url(#seq-arrow)""##
                };

                if from == to {
                    let loop_w = 38.0;
                    let loop_h = 24.0;
                    svg.push_str(&format!(
                        r##"<path d="M {x1:.1} {y:.1} H {:.1} V {:.1} H {x1:.1}" fill="none" stroke="#334155" stroke-width="1.4"{dash_attr}{marker_attr}/>"##,
                        x1 + loop_w,
                        y + loop_h
                    ));
                    svg.push_str(&format!(
                        r##"<text x="{:.1}" y="{:.1}" font-size="12" fill="#0f172a">{}</text>"##,
                        x1 + loop_w + 6.0,
                        y + loop_h / 2.0 + 4.0,
                        escape_xml(&display_label)
                    ));
                } else {
                    svg.push_str(&format!(
                        r##"<line x1="{x1:.1}" y1="{y:.1}" x2="{x2:.1}" y2="{y:.1}" stroke="#334155" stroke-width="1.4"{dash_attr}{marker_attr}/>"##
                    ));
                    if *cross_end {
                        svg.push_str(&format!(
                            r##"<path d="M {:.1} {:.1} L {:.1} {:.1} M {:.1} {:.1} L {:.1} {:.1}" stroke="#dc2626" stroke-width="1.6"/>"##,
                            x2 - 5.0, y - 5.0, x2 + 5.0, y + 5.0, x2 + 5.0, y - 5.0, x2 - 5.0, y + 5.0
                        ));
                    }
                    let mid_x = (x1 + x2) / 2.0;
                    let tw = estimate_text_width(&display_label, 12.0) + 8.0;
                    svg.push_str(&format!(
                        r##"<rect x="{:.1}" y="{:.1}" width="{tw:.1}" height="16" fill="#ffffff" fill-opacity="0.88"/>"##,
                        mid_x - tw / 2.0,
                        y - 19.0
                    ));
                    svg.push_str(&format!(
                        r##"<text x="{mid_x:.1}" y="{:.1}" text-anchor="middle" font-size="12" fill="#0f172a">{}</text>"##,
                        y - 7.0,
                        escape_xml(&display_label)
                    ));
                }
            }
            SeqItem::BlockStart { .. } | SeqItem::BlockEnd => {}
        }
    }

    // Participant boxes (top & bottom)
    for (idx, p) in participants.iter().enumerate() {
        let cx = centers[idx];
        let w = col_widths[idx];
        for by in [top_y, bottom_box_y] {
            let bx = cx - w / 2.0;
            let fill = if p.is_actor { "#eff6ff" } else { "#f1f5f9" };
            let stroke = if p.is_actor { "#2563eb" } else { "#475569" };
            svg.push_str(&format!(
                r##"<rect x="{bx:.1}" y="{by:.1}" width="{w:.1}" height="{box_h:.1}" rx="6" fill="{fill}" stroke="{stroke}" stroke-width="1.4"/>"##
            ));
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="13" font-weight="600" fill="#0f172a">{}</text>"##,
                by + box_h / 2.0 + 4.5,
                escape_xml(&p.label)
            ));
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

fn split_alias(rest: &str) -> (&str, Option<&str>) {
    if let Some((id, lbl)) = rest.split_once(" as ") {
        (id.trim(), Some(lbl.trim()))
    } else {
        (rest.trim(), None)
    }
}

fn parse_seq_note(
    l: &str,
    participants: &mut Vec<SeqParticipant>,
    part_map: &mut HashMap<String, usize>,
    ensure: &impl Fn(&str, Option<&str>, bool, &mut Vec<SeqParticipant>, &mut HashMap<String, usize>) -> usize,
) -> Option<SeqItem> {
    let rest = l.strip_prefix("Note ")?.trim();
    let (target_part, text) = rest.split_once(':')?;
    let text = text.trim().to_string();
    let target_part = target_part.trim();
    let (pos, who) = if let Some(w) = target_part.strip_prefix("right of ") {
        (SeqNotePos::Right, w.trim())
    } else if let Some(w) = target_part.strip_prefix("left of ") {
        (SeqNotePos::Left, w.trim())
    } else if let Some(w) = target_part.strip_prefix("over ") {
        (SeqNotePos::Over, w.trim())
    } else {
        return None;
    };
    if let Some((a, b)) = who.split_once(',') {
        let from = ensure(a.trim(), None, false, participants, part_map);
        let to = ensure(b.trim(), None, false, participants, part_map);
        Some(SeqItem::Note { from, to, pos, text })
    } else {
        let idx = ensure(who, None, false, participants, part_map);
        Some(SeqItem::Note {
            from: idx,
            to: idx,
            pos,
            text,
        })
    }
}

fn parse_seq_message(
    l: &str,
    participants: &mut Vec<SeqParticipant>,
    part_map: &mut HashMap<String, usize>,
    ensure: &impl Fn(&str, Option<&str>, bool, &mut Vec<SeqParticipant>, &mut HashMap<String, usize>) -> usize,
) -> Option<SeqItem> {
    let (lhs, label) = l.split_once(':')?;
    let arrows = [
        ("-->>", true, false, false),
        ("->>", false, false, false),
        ("-->", true, true, false),
        ("->", false, true, false),
        ("--x", true, false, true),
        ("-x", false, false, true),
        ("--)", true, true, false),
        ("-)", false, true, false),
    ];
    for (arrow, dashed, open_arrow, cross_end) in arrows {
        if let Some((from_raw, to_raw)) = lhs.split_once(arrow) {
            let from_id = from_raw.trim().trim_end_matches(['+', '-']);
            let to_id = to_raw.trim().trim_start_matches(['+', '-']).trim_end_matches(['+', '-']);
            if from_id.is_empty() || to_id.is_empty() {
                return None;
            }
            let from = ensure(from_id, None, false, participants, part_map);
            let to = ensure(to_id, None, false, participants, part_map);
            return Some(SeqItem::Message {
                from,
                to,
                label: label.trim().to_string(),
                dashed,
                open_arrow,
                cross_end,
            });
        }
    }
    None
}

// ============================================================================
// 3. Flowchart / Graph -> Pure Rust SVG
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowDir {
    Td,
    Bt,
    Lr,
    Rl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeShape {
    Rect,
    Round,
    Stadium,
    Diamond,
    Circle,
    Subroutine,
}

#[derive(Debug, Clone)]
struct FlowNode {
    _id: String,
    label: String,
    shape: NodeShape,
    subgraph: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeStyle {
    Solid,
    Dashed,
    Thick,
}

#[derive(Debug, Clone)]
struct FlowEdge {
    from: usize,
    to: usize,
    label: Option<String>,
    style: EdgeStyle,
    arrow: bool,
}

#[derive(Debug, Clone)]
struct SubgraphInfo {
    title: String,
}

fn render_flowchart_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut lines = source.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let header = lines.next()?;
    let dir_token = header
        .split_whitespace()
        .nth(1)
        .unwrap_or("TD")
        .to_ascii_uppercase();
    let dir = match dir_token.as_str() {
        "LR" => FlowDir::Lr,
        "RL" => FlowDir::Rl,
        "BT" => FlowDir::Bt,
        _ => FlowDir::Td,
    };

    let mut nodes: Vec<FlowNode> = Vec::new();
    let mut node_map: HashMap<String, usize> = HashMap::new();
    let mut edges: Vec<FlowEdge> = Vec::new();
    let mut subgraphs: Vec<SubgraphInfo> = Vec::new();
    let mut sg_stack: Vec<usize> = Vec::new();

    for line in lines {
        let l = line.trim_end_matches(';').trim();
        if l.is_empty() || l.starts_with("classDef ") || l.starts_with("class ") || l.starts_with("style ") || l.starts_with("linkStyle ") {
            continue;
        }
        if l == "end" {
            sg_stack.pop();
            continue;
        }
        if let Some(rest) = l.strip_prefix("subgraph ") {
            let title = parse_subgraph_title(rest.trim());
            let sg_idx = subgraphs.len();
            subgraphs.push(SubgraphInfo { title });
            sg_stack.push(sg_idx);
            continue;
        }
        if l.starts_with("direction ") {
            continue;
        }
        parse_flowchart_statement(
            l,
            sg_stack.last().copied(),
            &mut nodes,
            &mut node_map,
            &mut edges,
        );
    }

    if nodes.is_empty() {
        return None;
    }

    // Assign layers via longest-path DFS (ignoring back-edges).
    let n = nodes.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &edges {
        if e.from != e.to {
            adj[e.from].push(e.to);
        }
    }
    let mut in_deg = vec![0usize; n];
    for list in &adj {
        for &v in list {
            in_deg[v] += 1;
        }
    }
    let mut layer = vec![0usize; n];
    let mut state = vec![0u8; n]; // 0 = unvisited, 1 = visiting, 2 = done
    let mut topo = Vec::with_capacity(n);
    fn dfs(
        u: usize,
        adj: &[Vec<usize>],
        state: &mut [u8],
        topo: &mut Vec<usize>,
    ) {
        state[u] = 1;
        for &v in &adj[u] {
            if state[v] == 0 {
                dfs(v, adj, state, topo);
            }
        }
        state[u] = 2;
        topo.push(u);
    }
    for i in 0..n {
        if in_deg[i] == 0 && state[i] == 0 {
            dfs(i, &adj, &mut state, &mut topo);
        }
    }
    for i in 0..n {
        if state[i] == 0 {
            dfs(i, &adj, &mut state, &mut topo);
        }
    }
    topo.reverse();
    let mut rank_pos = vec![0usize; n];
    for (pos, &u) in topo.iter().enumerate() {
        rank_pos[u] = pos;
    }
    for &u in &topo {
        for &v in &adj[u] {
            if rank_pos[u] < rank_pos[v] {
                layer[v] = layer[v].max(layer[u] + 1);
            }
        }
    }

    let max_layer = layer.iter().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_layer + 1];
    for (idx, &l) in layer.iter().enumerate() {
        layers[l].push(idx);
    }

    // Simple barycenter ordering pass to reduce edge crossings.
    let mut rev_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &edges {
        if e.from != e.to {
            rev_adj[e.to].push(e.from);
        }
    }
    for _ in 0..2 {
        for l in 1..=max_layer {
            let prev_pos: HashMap<usize, usize> = layers[l - 1]
                .iter()
                .enumerate()
                .map(|(i, &u)| (u, i))
                .collect();
            layers[l].sort_by(|&a, &b| {
                let bary = |u: usize| -> f32 {
                    let mut sum = 0.0;
                    let mut cnt = 0.0;
                    for &p in &rev_adj[u] {
                        if let Some(&pos) = prev_pos.get(&p) {
                            sum += pos as f32;
                            cnt += 1.0;
                        }
                    }
                    if cnt > 0.0 { sum / cnt } else { f32::MAX }
                };
                bary(a).partial_cmp(&bary(b)).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    // Node dimensions
    let node_sizes: Vec<(f32, f32)> = nodes
        .iter()
        .map(|node| {
            let tw = estimate_text_width(&node.label, 13.0);
            match node.shape {
                NodeShape::Diamond => ((tw + 44.0).max(88.0), 54.0),
                NodeShape::Circle => {
                    let d = (tw + 24.0).max(54.0);
                    (d, d)
                }
                _ => ((tw + 32.0).max(84.0), 40.0),
            }
        })
        .collect();

    let horizontal = matches!(dir, FlowDir::Lr | FlowDir::Rl);
    let reversed = matches!(dir, FlowDir::Bt | FlowDir::Rl);
    let margin = if subgraphs.is_empty() { 32.0f32 } else { 44.0f32 };
    let layer_gap = 64.0f32;
    let sibling_gap = 34.0f32;

    // Calculate center coordinates for each node.
    let mut layer_primary_sizes: Vec<f32> = Vec::with_capacity(layers.len());
    let mut layer_cross_sizes: Vec<f32> = Vec::with_capacity(layers.len());
    for l_nodes in &layers {
        let mut p_max = 40.0f32;
        let mut c_sum = 0.0f32;
        for (i, &u) in l_nodes.iter().enumerate() {
            let (nw, nh) = node_sizes[u];
            let (p, c) = if horizontal { (nw, nh) } else { (nh, nw) };
            p_max = p_max.max(p);
            c_sum += c;
            if i + 1 < l_nodes.len() {
                c_sum += sibling_gap;
            }
        }
        layer_primary_sizes.push(p_max);
        layer_cross_sizes.push(c_sum);
    }

    let max_cross = layer_cross_sizes.iter().copied().fold(0.0f32, f32::max);
    let total_primary: f32 = layer_primary_sizes.iter().sum::<f32>()
        + layer_gap * (layers.len().saturating_sub(1) as f32);

    let mut centers = vec![(0.0f32, 0.0f32); n];
    let mut p_cursor = margin;
    let layer_iter: Vec<usize> = if reversed {
        (0..layers.len()).rev().collect()
    } else {
        (0..layers.len()).collect()
    };
    for l in layer_iter {
        let p_size = layer_primary_sizes[l];
        let c_total = layer_cross_sizes[l];
        let mut c_cursor = margin + (max_cross - c_total) / 2.0;
        for &u in &layers[l] {
            let (nw, nh) = node_sizes[u];
            let (p_mid, c_span) = (p_cursor + p_size / 2.0, if horizontal { nh } else { nw });
            let c_mid = c_cursor + c_span / 2.0;
            centers[u] = if horizontal {
                (p_mid, c_mid)
            } else {
                (c_mid, p_mid)
            };
            c_cursor += c_span + sibling_gap;
        }
        p_cursor += p_size + layer_gap;
    }

    let (total_w, total_h) = if horizontal {
        ((total_primary + margin * 2.0).ceil() as u32, (max_cross + margin * 2.0).ceil() as u32)
    } else {
        ((max_cross + margin * 2.0).ceil() as u32, (total_primary + margin * 2.0).ceil() as u32)
    };

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(
        r##"<defs>
  <marker id="flow-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
    <path d="M 0 1 L 10 5 L 0 9 z" fill="#475569"/>
  </marker>
</defs>"##,
    );
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));

    // Subgraph bounding boxes
    for (sg_idx, sg) in subgraphs.iter().enumerate() {
        let members: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| (node.subgraph == Some(sg_idx)).then_some(i))
            .collect();
        if members.is_empty() {
            continue;
        }
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for u in members {
            let (cx, cy) = centers[u];
            let (w, h) = node_sizes[u];
            min_x = min_x.min(cx - w / 2.0);
            min_y = min_y.min(cy - h / 2.0);
            max_x = max_x.max(cx + w / 2.0);
            max_y = max_y.max(cy + h / 2.0);
        }
        let sx = (min_x - 14.0).max(4.0);
        let sy = (min_y - 24.0).max(4.0);
        let sw = (max_x + 14.0) - sx;
        let sh = (max_y + 12.0) - sy;
        svg.push_str(&format!(
            r##"<rect x="{sx:.1}" y="{sy:.1}" width="{sw:.1}" height="{sh:.1}" rx="8" fill="#f8fafc" stroke="#cbd5e1" stroke-width="1.2" stroke-dasharray="5,3"/>"##
        ));
        if !sg.title.is_empty() {
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="{:.1}" font-size="11.5" font-weight="600" fill="#475569">{}</text>"##,
                sx + 10.0,
                sy + 15.0,
                escape_xml(&sg.title)
            ));
        }
    }

    // Edges
    for e in &edges {
        let (x1, y1) = centers[e.from];
        let (x2, y2) = centers[e.to];
        let (w1, h1) = node_sizes[e.from];
        let (w2, h2) = node_sizes[e.to];
        let (sx, sy) = edge_anchor(x1, y1, w1, h1, x2, y2);
        let (ex, ey) = edge_anchor(x2, y2, w2, h2, x1, y1);
        let stroke_w = match e.style {
            EdgeStyle::Thick => "2.6",
            _ => "1.4",
        };
        let dash = match e.style {
            EdgeStyle::Dashed => r##" stroke-dasharray="5,4""##,
            _ => "",
        };
        let marker = if e.arrow {
            r##" marker-end="url(#flow-arrow)""##
        } else {
            ""
        };
        let path_d = if horizontal {
            let mx = (sx + ex) / 2.0;
            format!("M {sx:.1} {sy:.1} C {mx:.1} {sy:.1}, {mx:.1} {ey:.1}, {ex:.1} {ey:.1}")
        } else {
            let my = (sy + ey) / 2.0;
            format!("M {sx:.1} {sy:.1} C {sx:.1} {my:.1}, {ex:.1} {my:.1}, {ex:.1} {ey:.1}")
        };
        svg.push_str(&format!(
            r##"<path d="{path_d}" fill="none" stroke="#475569" stroke-width="{stroke_w}"{dash}{marker}/>"##
        ));
        if let Some(lbl) = &e.label {
            let lx = (sx + ex) / 2.0;
            let ly = (sy + ey) / 2.0;
            let lw = estimate_text_width(lbl, 11.0) + 10.0;
            svg.push_str(&format!(
                r##"<rect x="{:.1}" y="{:.1}" width="{lw:.1}" height="18" rx="3" fill="#ffffff" stroke="#e2e8f0" stroke-width="0.8"/>"##,
                lx - lw / 2.0,
                ly - 9.0
            ));
            svg.push_str(&format!(
                r##"<text x="{lx:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#334155">{}</text>"##,
                ly + 3.5,
                escape_xml(lbl)
            ));
        }
    }

    // Nodes
    for (idx, node) in nodes.iter().enumerate() {
        let (cx, cy) = centers[idx];
        let (w, h) = node_sizes[idx];
        let x = cx - w / 2.0;
        let y = cy - h / 2.0;
        match node.shape {
            NodeShape::Rect => {
                svg.push_str(&format!(
                    r##"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="3" fill="#eff6ff" stroke="#2563eb" stroke-width="1.4"/>"##
                ));
            }
            NodeShape::Round => {
                svg.push_str(&format!(
                    r##"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="10" fill="#eff6ff" stroke="#2563eb" stroke-width="1.4"/>"##
                ));
            }
            NodeShape::Stadium => {
                svg.push_str(&format!(
                    r##"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="{:.1}" fill="#f0fdf4" stroke="#16a34a" stroke-width="1.4"/>"##,
                    h / 2.0
                ));
            }
            NodeShape::Subroutine => {
                svg.push_str(&format!(
                    r##"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="3" fill="#eff6ff" stroke="#2563eb" stroke-width="1.4"/>"##
                ));
                svg.push_str(&format!(
                    r##"<line x1="{:.1}" y1="{y:.1}" x2="{:.1}" y2="{:.1}" stroke="#2563eb" stroke-width="1.2"/>"##,
                    x + 7.0, x + 7.0, y + h
                ));
                svg.push_str(&format!(
                    r##"<line x1="{:.1}" y1="{y:.1}" x2="{:.1}" y2="{:.1}" stroke="#2563eb" stroke-width="1.2"/>"##,
                    x + w - 7.0, x + w - 7.0, y + h
                ));
            }
            NodeShape::Diamond => {
                svg.push_str(&format!(
                    r##"<polygon points="{cx:.1},{y:.1} {:.1},{cy:.1} {cx:.1},{:.1} {x:.1},{cy:.1}" fill="#fefce8" stroke="#ca8a04" stroke-width="1.4"/>"##,
                    x + w, y + h
                ));
            }
            NodeShape::Circle => {
                svg.push_str(&format!(
                    r##"<ellipse cx="{cx:.1}" cy="{cy:.1}" rx="{:.1}" ry="{:.1}" fill="#f5f3ff" stroke="#7c3aed" stroke-width="1.4"/>"##,
                    w / 2.0, h / 2.0
                ));
            }
        }
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="13" font-weight="500" fill="#0f172a">{}</text>"##,
            cy + 4.5,
            escape_xml(&node.label)
        ));
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

fn edge_anchor(cx: f32, cy: f32, w: f32, h: f32, tx: f32, ty: f32) -> (f32, f32) {
    let dx = tx - cx;
    let dy = ty - cy;
    if dx.abs() < 1e-3 && dy.abs() < 1e-3 {
        return (cx, cy);
    }
    let rx = w / 2.0;
    let ry = h / 2.0;
    let sx = if dx.abs() > 1e-3 { rx / dx.abs() } else { f32::MAX };
    let sy = if dy.abs() > 1e-3 { ry / dy.abs() } else { f32::MAX };
    let scale = sx.min(sy);
    (cx + dx * scale, cy + dy * scale)
}

fn parse_subgraph_title(rest: &str) -> String {
    if let Some(open) = rest.find('[')
        && let Some(close) = rest.rfind(']')
        && close > open
    {
        return unquote(rest[open + 1..close].trim());
    }
    unquote(rest)
}

fn parse_flowchart_statement(
    stmt: &str,
    subgraph: Option<usize>,
    nodes: &mut Vec<FlowNode>,
    node_map: &mut HashMap<String, usize>,
    edges: &mut Vec<FlowEdge>,
) {
    // Split statement by edges while keeping track of edge tokens.
    let mut parts: Vec<(&str, Option<(EdgeStyle, bool, Option<String>)>)> = Vec::new();
    let mut cursor = 0usize;
    let bytes = stmt.as_bytes();
    while cursor < bytes.len() {
        // Skip bracketed/quoted node labels so `--` inside `[a-b]` isn't treated as an edge.
        if matches!(bytes[cursor], b'[' | b'(' | b'{') {
            let close = match bytes[cursor] {
                b'[' => b']',
                b'(' => b')',
                _ => b'}',
            };
            if let Some(rel_end) = stmt[cursor + 1..].find(close as char) {
                cursor += 1 + rel_end + 1;
                continue;
            }
        }
        if let Some((edge_len, style, arrow, label, next_start)) = match_flowchart_edge(&stmt[cursor..]) {
            let lhs = stmt[..cursor].trim();
            let _ = edge_len;
            parts.push((lhs, Some((style, arrow, label.clone()))));
            // Continue parsing the remainder recursively by slicing `stmt`
            let rest = &stmt[cursor + next_start..];
            let mut prev_group = ensure_node_group(lhs, subgraph, nodes, node_map);
            let mut rest_str = rest;
            let mut pending = (style, arrow, label);
            loop {
                if let Some((next_lhs, next_edge, next_rest)) = split_next_flowchart_edge(rest_str) {
                    let cur_group = ensure_node_group(next_lhs, subgraph, nodes, node_map);
                    for &u in &prev_group {
                        for &v in &cur_group {
                            edges.push(FlowEdge {
                                from: u,
                                to: v,
                                label: pending.2.clone(),
                                style: pending.0,
                                arrow: pending.1,
                            });
                        }
                    }
                    prev_group = cur_group;
                    pending = next_edge;
                    rest_str = next_rest;
                } else {
                    let cur_group = ensure_node_group(rest_str, subgraph, nodes, node_map);
                    for &u in &prev_group {
                        for &v in &cur_group {
                            edges.push(FlowEdge {
                                from: u,
                                to: v,
                                label: pending.2.clone(),
                                style: pending.0,
                                arrow: pending.1,
                            });
                        }
                    }
                    break;
                }
            }
            return;
        }
        let ch_len = stmt[cursor..].chars().next().map_or(1, char::len_utf8);
        cursor += ch_len;
    }

    if parts.is_empty() {
        let _ = ensure_node_group(stmt, subgraph, nodes, node_map);
    }
}

fn split_next_flowchart_edge(s: &str) -> Option<(&str, (EdgeStyle, bool, Option<String>), &str)> {
    let bytes = s.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if matches!(bytes[cursor], b'[' | b'(' | b'{') {
            let close = match bytes[cursor] {
                b'[' => b']',
                b'(' => b')',
                _ => b'}',
            };
            if let Some(rel_end) = s[cursor + 1..].find(close as char) {
                cursor += 1 + rel_end + 1;
                continue;
            }
        }
        if let Some((_len, style, arrow, label, next_start)) = match_flowchart_edge(&s[cursor..]) {
            return Some((s[..cursor].trim(), (style, arrow, label), &s[cursor + next_start..]));
        }
        let ch_len = s[cursor..].chars().next().map_or(1, char::len_utf8);
        cursor += ch_len;
    }
    None
}

fn match_flowchart_edge(s: &str) -> Option<(usize, EdgeStyle, bool, Option<String>, usize)> {
    // Check `-- label -->`, `-. label .->`, `== label ==>`
    for (prefix, suffix, style, arrow) in [
        ("-- ", " -->", EdgeStyle::Solid, true),
        ("-- ", " ---", EdgeStyle::Solid, false),
        ("-. ", " .->", EdgeStyle::Dashed, true),
        ("== ", " ==>", EdgeStyle::Thick, true),
    ] {
        if let Some(after) = s.strip_prefix(prefix)
            && let Some(end_idx) = after.find(suffix)
        {
            let lbl = after[..end_idx].trim();
            let consumed = prefix.len() + end_idx + suffix.len();
            return Some((
                consumed,
                style,
                arrow,
                (!lbl.is_empty()).then(|| unquote(lbl)),
                consumed,
            ));
        }
    }

    // Standard arrows with optional `|label|`
    for (tok, style, arrow) in [
        ("-.->", EdgeStyle::Dashed, true),
        ("-.-", EdgeStyle::Dashed, false),
        ("==>", EdgeStyle::Thick, true),
        ("===", EdgeStyle::Thick, false),
        ("-->", EdgeStyle::Solid, true),
        ("---", EdgeStyle::Solid, false),
    ] {
        if let Some(after) = s.strip_prefix(tok) {
            let after_ws = after.trim_start();
            let ws_len = after.len() - after_ws.len();
            if let Some(after_pipe) = after_ws.strip_prefix('|')
                && let Some(close_pipe) = after_pipe.find('|')
            {
                let lbl = after_pipe[..close_pipe].trim();
                let consumed = tok.len() + ws_len + 1 + close_pipe + 1;
                return Some((
                    consumed,
                    style,
                    arrow,
                    (!lbl.is_empty()).then(|| unquote(lbl)),
                    consumed,
                ));
            }
            return Some((tok.len(), style, arrow, None, tok.len()));
        }
    }
    None
}

fn ensure_node_group(
    expr: &str,
    subgraph: Option<usize>,
    nodes: &mut Vec<FlowNode>,
    node_map: &mut HashMap<String, usize>,
) -> Vec<usize> {
    let mut ids = Vec::new();
    for part in expr.split('&') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let (id, label_opt, shape_opt) = parse_node_token(p);
        if id.is_empty() {
            continue;
        }
        if let Some(&idx) = node_map.get(id) {
            if let Some(lbl) = label_opt {
                nodes[idx].label = lbl;
            }
            if let Some(sh) = shape_opt {
                nodes[idx].shape = sh;
            }
            if nodes[idx].subgraph.is_none() {
                nodes[idx].subgraph = subgraph;
            }
            ids.push(idx);
        } else {
            let idx = nodes.len();
            nodes.push(FlowNode {
                _id: id.to_string(),
                label: label_opt.unwrap_or_else(|| id.to_string()),
                shape: shape_opt.unwrap_or(NodeShape::Rect),
                subgraph,
            });
            node_map.insert(id.to_string(), idx);
            ids.push(idx);
        }
    }
    ids
}

fn parse_node_token(token: &str) -> (&str, Option<String>, Option<NodeShape>) {
    let t = token.trim();
    for (open, close, shape) in [
        ("([", "])", NodeShape::Stadium),
        ("[[", "]]", NodeShape::Subroutine),
        ("((", "))", NodeShape::Circle),
        ("[", "]", NodeShape::Rect),
        ("(", ")", NodeShape::Round),
        ("{", "}", NodeShape::Diamond),
    ] {
        if let Some(open_idx) = t.find(open)
            && t.ends_with(close)
            && t.len() >= open_idx + open.len() + close.len()
        {
            let id = t[..open_idx].trim();
            let inner = &t[open_idx + open.len()..t.len() - close.len()];
            return (id, Some(unquote(inner.trim())), Some(shape));
        }
    }
    (t, None, None)
}

// ============================================================================
// Helpers
// ============================================================================

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\''))) {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn estimate_text_width(s: &str, font_size: f32) -> f32 {
    let mut units = 0.0f32;
    for ch in s.chars() {
        if ch.is_ascii() {
            units += 0.56;
        } else {
            units += 1.0;
        }
    }
    units * font_size
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pie_and_xychart_to_native_chart_specs() {
        let pie = "pie showData\n  title ブラウザシェア\n  \"Chrome\" : 65\n  \"Safari\" : 20\n  \"Other\" : 15\n";
        let Some(MermaidOutput::Chart(spec)) = render_mermaid(pie) else {
            panic!("expected native chart for pie");
        };
        assert_eq!(spec.kind, NewChartKind::Pie);
        assert_eq!(spec.title.as_deref(), Some("ブラウザシェア"));
        assert_eq!(spec.categories, vec!["Chrome", "Safari", "Other"]);
        assert_eq!(spec.series[0].1.len(), 3);

        let xy = "xychart-beta\n  title \"月次売上\"\n  x-axis [Jan, Feb, Mar]\n  bar [100, 140, 180]\n  line [90, 120, 170]\n";
        let Some(MermaidOutput::Chart(xy_spec)) = render_mermaid(xy) else {
            panic!("expected native chart for xychart-beta");
        };
        assert_eq!(xy_spec.kind, NewChartKind::ClusteredColumn);
        assert_eq!(xy_spec.title.as_deref(), Some("月次売上"));
        assert_eq!(xy_spec.series.len(), 2);
    }

    #[test]
    fn renders_sequence_diagram_svg() {
        let seq = "\
sequenceDiagram
  autonumber
  actor U as ユーザー
  participant API as バックエンドAPI
  U->>API: リクエスト送信
  alt 成功時
    API-->>U: 200 OK
  else エラー時
    API--xU: 500 Error
  end
  Note over U,API: 完了
";
        let Some(MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        }) = render_mermaid(seq)
        else {
            panic!("expected svg for sequenceDiagram");
        };
        assert!(width_px > 200 && height_px > 200);
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(svg.starts_with("<svg "));
        assert!(svg.contains("ユーザー"));
        assert!(svg.contains("バックエンドAPI"));
        assert!(svg.contains("1. リクエスト送信"));
        assert!(svg.contains("200 OK"));
        assert!(svg.contains("alt"));
    }

    #[test]
    fn renders_flowchart_svg_with_shapes_subgraphs_and_cycles() {
        let flow = "\
flowchart LR
  subgraph Input
    A([開始]) --> B[データ検証]
  end
  B -->|OK| C{分岐}
  C -->|Yes| D((完了))
  C -->|Retry| B
";
        let Some(MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        }) = render_mermaid(flow)
        else {
            panic!("expected svg for flowchart");
        };
        assert!(width_px > 250 && height_px > 100);
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(svg.contains("開始"));
        assert!(svg.contains("データ検証"));
        assert!(svg.contains("分岐"));
        assert!(svg.contains("完了"));
        assert!(svg.contains("Input"));
        assert!(svg.contains("OK"));
    }
}

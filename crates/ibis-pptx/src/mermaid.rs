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
    /// One drawing kind per series. Empty means every series uses [`Self::kind`].
    pub series_kinds: Vec<NewChartKind>,
    /// `pie showData`: the slices show their values.
    pub show_data: bool,
    /// `xychart` axis titles (`x-axis "Month" [...]`, `y-axis "Revenue" 0 --> 100`).
    pub x_axis_title: Option<String>,
    pub y_axis_title: Option<String>,
    /// `xychart` value-axis range (`y-axis 4000 --> 11000`); `None` is automatic.
    pub y_range: Option<(f64, f64)>,
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

fn strip_mermaid_frontmatter(source: &str) -> &str {
    let mut s = source.trim();
    if let Some(after_open) = s.strip_prefix("---") {
        if after_open.starts_with('\n') || after_open.starts_with("\r\n") {
            let mut offset = 3;
            for line in after_open.split_inclusive('\n') {
                offset += line.len();
                if line.trim() == "---" {
                    s = s[offset..].trim_start();
                    break;
                }
            }
        }
    }
    // Advance past any leading blank or `%%` directive/comment lines so line 0 is the diagram header.
    loop {
        let trimmed_start = s.trim_start();
        if trimmed_start.starts_with("%%") {
            if let Some(nl) = trimmed_start.find('\n') {
                s = &trimmed_start[nl + 1..];
                continue;
            }
            return "";
        }
        return trimmed_start;
    }
}

/// The top-level `title:` of a YAML frontmatter block (`---` … `---`), unquoted.
/// Nested keys (such as `config:` children) are ignored.
fn mermaid_frontmatter_title(source: &str) -> Option<String> {
    let after_open = source.trim_start().strip_prefix("---")?;
    if !(after_open.starts_with('\n') || after_open.starts_with("\r\n")) {
        return None;
    }
    for line in after_open.lines().skip(1) {
        if line.trim() == "---" {
            break;
        }
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        if let Some(rest) = line.strip_prefix("title:") {
            let title = unquote(rest.trim());
            return (!title.is_empty()).then_some(title);
        }
    }
    None
}

/// Diagram families this renderer understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagramKind {
    Pie,
    XyChart,
    Sequence,
    Flowchart,
    State,
    Class,
    Er,
    Gantt,
    Mindmap,
    GitGraph,
    Journey,
    Timeline,
    Quadrant,
}

/// Header keywords, including the aliases current mermaid still accepts
/// (`graph`, `flowchart-elk`, `classDiagram-v2`, `stateDiagram-v2`, `xychart-beta`).
/// Matching is on the first word, so `gitGraph:`, `graph LR;` and
/// `xychart horizontal` are recognised too.
const DIAGRAM_HEADERS: &[(&str, DiagramKind)] = &[
    ("pie", DiagramKind::Pie),
    ("xychart", DiagramKind::XyChart),
    ("xychart-beta", DiagramKind::XyChart),
    ("sequenceDiagram", DiagramKind::Sequence),
    ("flowchart", DiagramKind::Flowchart),
    ("flowchart-v2", DiagramKind::Flowchart),
    ("flowchart-elk", DiagramKind::Flowchart),
    ("graph", DiagramKind::Flowchart),
    ("stateDiagram", DiagramKind::State),
    ("stateDiagram-v2", DiagramKind::State),
    ("classDiagram", DiagramKind::Class),
    ("classDiagram-v2", DiagramKind::Class),
    ("erDiagram", DiagramKind::Er),
    ("gantt", DiagramKind::Gantt),
    ("mindmap", DiagramKind::Mindmap),
    ("gitGraph", DiagramKind::GitGraph),
    ("journey", DiagramKind::Journey),
    ("timeline", DiagramKind::Timeline),
    ("quadrantChart", DiagramKind::Quadrant),
];

fn diagram_kind(header: &str) -> Option<DiagramKind> {
    let header = header.trim();
    let word_end = header
        .find(|c: char| c.is_whitespace() || c == ':' || c == ';')
        .unwrap_or(header.len());
    let word = &header[..word_end];
    DIAGRAM_HEADERS.iter().find(|(name, _)| *name == word).map(|(_, kind)| *kind)
}

/// Parse and render a Mermaid diagram into either a native chart or an SVG.
/// Returns `None` if the diagram type is unsupported (caller falls back to a fenced code block).
pub fn render_mermaid(source: &str) -> Option<MermaidOutput> {
    let trimmed = strip_mermaid_frontmatter(source);
    if trimmed.is_empty() {
        return None;
    }
    let first_line = trimmed.lines().next()?.trim();
    let kind = diagram_kind(first_line)?;
    // A `title` statement in the body wins over the frontmatter title, so the title is drawn once.
    let body_title = trimmed
        .lines()
        .skip(1)
        .any(|line| line.trim_start().starts_with("title ") || line.trim() == "title");
    let front_title = mermaid_frontmatter_title(source).filter(|_| !body_title);

    let svg = match kind {
        DiagramKind::Pie => {
            return parse_pie_chart(trimmed).map(|mut spec| {
                spec.title = spec.title.or(front_title);
                MermaidOutput::Chart(spec)
            });
        }
        DiagramKind::XyChart => {
            return parse_xy_chart(trimmed).map(|mut spec| {
                spec.title = spec.title.or(front_title);
                MermaidOutput::Chart(spec)
            });
        }
        DiagramKind::Sequence => render_sequence_svg(trimmed),
        DiagramKind::Flowchart => render_flowchart_svg(trimmed),
        DiagramKind::State => render_state_diagram_svg(trimmed),
        DiagramKind::Class => render_class_diagram_svg(trimmed),
        DiagramKind::Er => render_er_diagram_svg(trimmed),
        DiagramKind::Gantt => render_gantt_svg(trimmed),
        DiagramKind::Mindmap => render_mindmap_svg(trimmed),
        DiagramKind::GitGraph => render_git_graph_svg(trimmed),
        DiagramKind::Journey => render_journey_svg(trimmed),
        DiagramKind::Timeline => render_timeline_svg(trimmed),
        DiagramKind::Quadrant => render_quadrant_chart_svg(trimmed),
    };
    let (mut bytes, mut width_px, mut height_px) = svg?;
    if let Some(title) = front_title {
        (bytes, width_px, height_px) = add_svg_title(bytes, width_px, height_px, &title);
    }
    Some(MermaidOutput::Svg {
        bytes,
        width_px,
        height_px,
    })
}

/// Put a title band above one of this module's SVGs: the drawing moves down
/// (and is centred when the title is wider than it).
fn add_svg_title(bytes: Vec<u8>, width: u32, height: u32, title: &str) -> (Vec<u8>, u32, u32) {
    let Ok(svg) = std::str::from_utf8(&bytes) else {
        return (bytes, width, height);
    };
    let (Some(open_end), Some(close)) = (svg.find('>'), svg.rfind("</svg>")) else {
        return (bytes, width, height);
    };
    if !svg.starts_with("<svg ") || close <= open_end {
        return (bytes, width, height);
    }
    let band = 40u32;
    let title_w = (estimate_text_width(title, 16.0) + 32.0).ceil() as u32;
    let new_w = width.max(title_w);
    let new_h = height + band;
    let dx = (new_w - width) as f32 / 2.0;
    let mut out = String::with_capacity(svg.len() + 400);
    out.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{new_w}" height="{new_h}" viewBox="0 0 {new_w} {new_h}" font-family="sans-serif">"##
    ));
    out.push_str(&format!(r##"<rect width="{new_w}" height="{new_h}" fill="#ffffff"/>"##));
    out.push_str(&format!(
        r##"<text x="{:.1}" y="27" text-anchor="middle" font-size="16" font-weight="bold" fill="#0f172a" data-role="diagram-title">{}</text>"##,
        new_w as f32 / 2.0,
        escape_xml(title)
    ));
    out.push_str(&format!(r##"<g transform="translate({dx:.1},{band})">"##));
    out.push_str(&svg[open_end + 1..close]);
    out.push_str("</g></svg>");
    (out.into_bytes(), new_w, new_h)
}

// ============================================================================
// 1. Native Chart Parsers (`pie`, `xychart-beta`)
// ============================================================================

fn parse_pie_chart(source: &str) -> Option<MermaidChartSpec> {
    let show_data = source
        .lines()
        .next()
        .is_some_and(|line| line.split_whitespace().any(|token| token == "showData"));
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
        series_kinds: vec![NewChartKind::Pie],
        show_data,
        x_axis_title: None,
        y_axis_title: None,
        y_range: None,
    })
}

fn parse_xy_chart(source: &str) -> Option<MermaidChartSpec> {
    let mut title = None;
    let mut categories = Vec::new();
    let mut series = Vec::new();
    let mut series_kinds = Vec::new();
    let mut has_bar = false;
    let mut has_line = false;
    let mut bar_idx = 1usize;
    let mut line_idx = 1usize;
    let mut x_axis_title = None;
    let mut y_axis_title = None;
    let mut y_range = None;

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
                let title = unquote(rest[..rest.find('[').unwrap_or(0)].trim());
                x_axis_title = (!title.is_empty()).then_some(title);
            } else {
                (x_axis_title, _) = parse_xy_axis(rest);
            }
        } else if let Some(rest) = l.strip_prefix("y-axis ") {
            (y_axis_title, y_range) = parse_xy_axis(rest);
        } else if let Some(rest) = l.strip_prefix("bar ") {
            if let Some(list) = extract_bracket_list(rest) {
                has_bar = true;
                let vals: Vec<_> = list
                    .into_iter()
                    .map(|s| s.trim().parse::<f64>().ok().map(Number))
                    .collect();
                let name = xy_series_name(rest, &format!("Bar {bar_idx}"));
                series.push((name, vals));
                series_kinds.push(NewChartKind::ClusteredColumn);
                bar_idx += 1;
            }
        } else if let Some(rest) = l.strip_prefix("line ") {
            if let Some(list) = extract_bracket_list(rest) {
                has_line = true;
                let vals: Vec<_> = list
                    .into_iter()
                    .map(|s| s.trim().parse::<f64>().ok().map(Number))
                    .collect();
                let name = xy_series_name(rest, &format!("Line {line_idx}"));
                series.push((name, vals));
                series_kinds.push(NewChartKind::Line);
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
        series_kinds,
        show_data: false,
        x_axis_title,
        y_axis_title,
        y_range,
    })
}

/// `"title" min --> max`, `title`, or `min --> max` of an `xychart` axis line.
fn parse_xy_axis(rest: &str) -> (Option<String>, Option<(f64, f64)>) {
    let rest = rest.trim();
    let (head, range) = match rest.split_once("-->") {
        Some((left, right)) => {
            let left = left.trim();
            let (title_part, min_token) = match left.rfind(char::is_whitespace) {
                Some(at) => (&left[..at], left[at..].trim()),
                None => ("", left),
            };
            match (min_token.parse::<f64>(), right.trim().parse::<f64>()) {
                (Ok(min), Ok(max)) if min.is_finite() && max.is_finite() && max > min => (title_part, Some((min, max))),
                _ => (rest, None),
            }
        }
        None => (rest, None),
    };
    let title = unquote(head.trim());
    ((!title.is_empty()).then_some(title), range)
}

fn xy_series_name(rest: &str, fallback: &str) -> String {
    let Some(list_at) = rest.find('[') else {
        return fallback.to_string();
    };
    let name = unquote(rest[..list_at].trim());
    if name.is_empty() { fallback.to_string() } else { name }
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

struct SeqBox {
    title: String,
    members: Vec<usize>,
}

enum SeqOpen {
    Fragment,
    Box,
    /// `rect rgb(...)` background highlight.
    Rect,
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
        activate_from: bool,
        deactivate_from: bool,
        activate_to: bool,
        deactivate_to: bool,
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
    Activate(usize),
    Deactivate(usize),
    /// `rect <color>` opens a highlighted background band.
    RectStart {
        color: String,
    },
    RectEnd,
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
    let mut boxes: Vec<SeqBox> = Vec::new();
    let mut open: Vec<SeqOpen> = Vec::new();
    let mut title: Option<String> = None;
    // `create` / `destroy` wait for the next message that involves the participant.
    let mut pending_create: Vec<usize> = Vec::new();
    let mut pending_destroy: Vec<usize> = Vec::new();
    let mut created_at: HashMap<usize, usize> = HashMap::new();
    let mut destroyed_at: HashMap<usize, usize> = HashMap::new();

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
        if l == "autonumber" || l.starts_with("autonumber ") {
            autonumber = true;
            continue;
        }
        if let Some(rest) = l.strip_prefix("title ").or_else(|| l.strip_prefix("title:")) {
            let t = unquote(rest.trim());
            title = (!t.is_empty()).then_some(t);
            continue;
        }
        if l.starts_with("accTitle") || l.starts_with("accDescr") || l.starts_with("link ") || l.starts_with("links ") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("create ") {
            let rest = rest.trim();
            let (is_actor, decl) = match (rest.strip_prefix("actor "), rest.strip_prefix("participant ")) {
                (Some(decl), _) => (true, decl),
                (None, Some(decl)) => (false, decl),
                (None, None) => (false, rest),
            };
            let (id, lbl) = split_alias(decl);
            let idx = ensure_participant(id, lbl, is_actor, &mut participants, &mut part_map);
            note_seq_box(open.last(), &mut boxes, idx);
            pending_create.push(idx);
            continue;
        }
        if let Some(rest) = l.strip_prefix("destroy ") {
            let id = rest.trim();
            let idx = ensure_participant(id, None, false, &mut participants, &mut part_map);
            pending_destroy.push(idx);
            continue;
        }
        if let Some(rest) = l.strip_prefix("rect ") {
            items.push(SeqItem::RectStart { color: rest.trim().to_string() });
            open.push(SeqOpen::Rect);
            continue;
        }
        if let Some(rest) = l.strip_prefix("and ").or_else(|| l.strip_prefix("option ")).or(if l == "and" { Some("") } else { None })
            && matches!(open.last(), Some(SeqOpen::Fragment))
        {
            items.push(SeqItem::BlockElse { label: rest.trim().to_string() });
            continue;
        }
        if let Some(rest) = l.strip_prefix("participant ") {
            let (id, lbl) = split_alias(rest);
            let idx = ensure_participant(id, lbl, false, &mut participants, &mut part_map);
            note_seq_box(open.last(), &mut boxes, idx);
            continue;
        }
        if let Some(rest) = l.strip_prefix("actor ") {
            let (id, lbl) = split_alias(rest);
            let idx = ensure_participant(id, lbl, true, &mut participants, &mut part_map);
            note_seq_box(open.last(), &mut boxes, idx);
            continue;
        }
        if l == "end" {
            match open.pop() {
                Some(SeqOpen::Box) => {}
                Some(SeqOpen::Rect) => items.push(SeqItem::RectEnd),
                Some(SeqOpen::Fragment) | None => items.push(SeqItem::BlockEnd),
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("box ") {
            boxes.push(SeqBox { title: seq_box_title(rest), members: Vec::new() });
            open.push(SeqOpen::Box);
            continue;
        }
        if let Some(rest) = l.strip_prefix("activate ") {
            let id = rest.split_whitespace().next().unwrap_or("");
            let idx = ensure_participant(id, None, false, &mut participants, &mut part_map);
            note_seq_box(open.last(), &mut boxes, idx);
            items.push(SeqItem::Activate(idx));
            continue;
        }
        if let Some(rest) = l.strip_prefix("deactivate ") {
            let id = rest.split_whitespace().next().unwrap_or("");
            let idx = ensure_participant(id, None, false, &mut participants, &mut part_map);
            note_seq_box(open.last(), &mut boxes, idx);
            items.push(SeqItem::Deactivate(idx));
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
                open.push(SeqOpen::Fragment);
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
            if let SeqItem::Message { from, to, .. } = &msg_item {
                note_seq_box(open.last(), &mut boxes, *from);
                note_seq_box(open.last(), &mut boxes, *to);
                for who in [*to, *from] {
                    if let Some(pos) = pending_create.iter().position(|p| *p == who) {
                        pending_create.remove(pos);
                        created_at.insert(who, items.len());
                        break;
                    }
                }
                for who in [*from, *to] {
                    if let Some(pos) = pending_destroy.iter().position(|p| *p == who) {
                        pending_destroy.remove(pos);
                        destroyed_at.insert(who, items.len());
                    }
                }
            }
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
    let mut rect_stack: Vec<(usize, f32)> = Vec::new();
    // (color, first item, last item, top, bottom, depth)
    let mut rect_boxes: Vec<(String, usize, usize, f32, f32, usize)> = Vec::new();
    let creating: Vec<usize> = created_at.values().copied().collect();

    for (idx, item) in items.iter().enumerate() {
        match item {
            SeqItem::Message { from, to, .. } => {
                // A message that creates its participant draws the participant's box on the arrow.
                let creates = creating.contains(&idx);
                if creates {
                    y += box_h / 2.0;
                }
                item_y.push(y);
                y += if from == to { self_step } else { row_step };
                if creates {
                    y += box_h / 2.0;
                }
            }
            SeqItem::RectStart { .. } => {
                item_y.push(y);
                rect_stack.push((idx, y - 22.0));
                y += 4.0;
            }
            SeqItem::RectEnd => {
                item_y.push(y);
                if let Some((start_idx, start_y)) = rect_stack.pop()
                    && let SeqItem::RectStart { color } = &items[start_idx]
                {
                    rect_boxes.push((color.clone(), start_idx, idx, start_y, y - 4.0, rect_stack.len()));
                }
                y += 24.0;
            }
            SeqItem::Activate(_) | SeqItem::Deactivate(_) => {
                item_y.push(y);
                y += 8.0;
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

    for seq_box in &boxes {
        if seq_box.members.is_empty() {
            continue;
        }
        let min_i = *seq_box.members.iter().min().unwrap_or(&0);
        let max_i = *seq_box.members.iter().max().unwrap_or(&0);
        let x = centers[min_i] - col_widths[min_i] / 2.0 - 10.0;
        let right = centers[max_i] + col_widths[max_i] / 2.0 + 10.0;
        let y = (top_y - 22.0).max(4.0);
        let height = bottom_box_y + box_h - y + 8.0;
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{height:.1}" rx="8" fill="#f8fafc" stroke="#94a3b8" stroke-width="1.2" data-shape="seq-box"/>"##,
            right - x
        ));
        if !seq_box.title.is_empty() {
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="{:.1}" font-size="11" font-weight="600" fill="#475569">{}</text>"##,
                x + 8.0,
                y + 14.0,
                escape_xml(&seq_box.title)
            ));
        }
    }

    // `rect` highlights: behind everything else, across the participants they involve.
    // Outer bands first, so a nested band stays visible on top of its parent.
    rect_boxes.sort_by_key(|band| band.5);
    for (color, first, last, sy, ey, depth) in &rect_boxes {
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        for item in &items[*first..=*last] {
            let (a, b) = match item {
                SeqItem::Message { from, to, .. } | SeqItem::Note { from, to, .. } => (*from, *to),
                SeqItem::Activate(who) | SeqItem::Deactivate(who) => (*who, *who),
                _ => continue,
            };
            lo = lo.min(a.min(b));
            hi = hi.max(a.max(b));
        }
        let inset = *depth as f32 * 6.0;
        let (x, right) = if lo <= hi {
            (centers[lo] - col_widths[lo] / 2.0 - 6.0 + inset, centers[hi] + col_widths[hi] / 2.0 + 6.0 - inset)
        } else {
            (10.0 + inset, total_w as f32 - 10.0 - inset)
        };
        let (fill, opacity) = seq_rect_paint(color);
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{sy:.1}" width="{:.1}" height="{:.1}" fill="{fill}" fill-opacity="{opacity:.2}" data-shape="seq-rect"/>"##,
            (right - x).max(4.0),
            (ey - sy).max(8.0)
        ));
    }

    // Lifelines: a created participant's starts at its box, a destroyed one's ends at its cross.
    for (idx, &cx) in centers.iter().enumerate() {
        let start = created_at.get(&idx).map_or(top_y + box_h, |&item| item_y[item] + box_h / 2.0);
        let end = destroyed_at.get(&idx).map_or(bottom_box_y, |&item| item_y[item]);
        svg.push_str(&format!(
            r##"<line x1="{cx:.1}" y1="{start:.1}" x2="{cx:.1}" y2="{end:.1}" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="5,4"/>"##
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
                activate_from: _,
                deactivate_from: _,
                activate_to: _,
                deactivate_to: _,
            } => {
                msg_num += 1;
                let display_label = if autonumber {
                    format!("{msg_num}. {label}")
                } else {
                    label.clone()
                };
                let x1 = centers[*from];
                let mut x2 = centers[*to];
                // The creating message ends at the new participant's box.
                if created_at.get(to) == Some(&idx) && from != to {
                    x2 += if x2 > x1 { -col_widths[*to] / 2.0 } else { col_widths[*to] / 2.0 };
                }
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
            SeqItem::BlockStart { .. }
            | SeqItem::BlockEnd
            | SeqItem::Activate(_)
            | SeqItem::Deactivate(_)
            | SeqItem::RectStart { .. }
            | SeqItem::RectEnd => {}
        }
    }

    // Activation bars sit on the lifeline from `+` / `activate` until `-` / `deactivate`.
    for (who, y1, y2, depth) in seq_activations(&items, &item_y, bottom_box_y) {
        let width = 10.0f32;
        let x = centers[who] - width / 2.0 + depth as f32 * 5.0;
        let top = y1.min(y2);
        let height = (y1 - y2).abs().max(12.0);
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{top:.1}" width="{width:.1}" height="{height:.1}" rx="2" fill="#bfdbfe" stroke="#2563eb" stroke-width="1.1" data-shape="activation"/>"##
        ));
    }

    // Participant boxes (top & bottom). A created participant's top box sits on its creating
    // message; a destroyed one has a cross where its lifeline ends and no bottom box.
    for (idx, p) in participants.iter().enumerate() {
        let cx = centers[idx];
        let w = col_widths[idx];
        let top = created_at.get(&idx).map_or(top_y, |&item| item_y[item] - box_h / 2.0);
        let mut rows = vec![top];
        if let Some(&item) = destroyed_at.get(&idx) {
            let dy = item_y[item];
            svg.push_str(&format!(
                r##"<path d="M {:.1} {:.1} L {:.1} {:.1} M {:.1} {:.1} L {:.1} {:.1}" stroke="#dc2626" stroke-width="2" data-shape="seq-destroy"/>"##,
                cx - 8.0, dy - 8.0, cx + 8.0, dy + 8.0, cx + 8.0, dy - 8.0, cx - 8.0, dy + 8.0
            ));
        } else {
            rows.push(bottom_box_y);
        }
        for by in rows {
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
    match title {
        Some(title) => Some(add_svg_title(svg.into_bytes(), total_w, total_h, &title)),
        None => Some((svg.into_bytes(), total_w, total_h)),
    }
}

/// `rect` colour: `rgb(r, g, b)`, `rgba(r, g, b, a)`, `#hex` or a CSS name, as an SVG fill and opacity.
/// Anything else falls back to mermaid's default pale highlight.
fn seq_rect_paint(color: &str) -> (String, f32) {
    let color = color.trim();
    let nums = |inner: &str| -> Vec<f32> { inner.split(',').filter_map(|n| n.trim().parse::<f32>().ok()).collect() };
    if let Some(inner) = color.strip_prefix("rgba(").and_then(|r| r.strip_suffix(')')) {
        let v = nums(inner);
        if v.len() == 4 {
            return (format!("rgb({},{},{})", v[0].clamp(0.0, 255.0), v[1].clamp(0.0, 255.0), v[2].clamp(0.0, 255.0)), v[3].clamp(0.0, 1.0));
        }
    }
    if let Some(inner) = color.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let v = nums(inner);
        if v.len() == 3 {
            return (format!("rgb({},{},{})", v[0].clamp(0.0, 255.0), v[1].clamp(0.0, 255.0), v[2].clamp(0.0, 255.0)), 1.0);
        }
    }
    let hex = color.strip_prefix('#').is_some_and(|h| matches!(h.len(), 3 | 6) && h.chars().all(|c| c.is_ascii_hexdigit()));
    if hex || (!color.is_empty() && color.len() <= 20 && color.chars().all(|c| c.is_ascii_alphabetic())) {
        return (color.to_string(), 1.0);
    }
    ("rgb(200,200,255)".to_string(), 0.35)
}

fn note_seq_box(open: Option<&SeqOpen>, boxes: &mut [SeqBox], idx: usize) {
    if !matches!(open, Some(SeqOpen::Box)) {
        return;
    }
    if let Some(seq_box) = boxes.last_mut()
        && !seq_box.members.contains(&idx)
    {
        seq_box.members.push(idx);
    }
}

fn seq_box_title(rest: &str) -> String {
    let rest = rest.trim();
    if let Some(after) = rest.strip_prefix("rgb(").or_else(|| rest.strip_prefix("rgba("))
        && let Some(end) = after.find(')')
    {
        return after[end + 1..].trim().to_string();
    }
    rest.split_once(char::is_whitespace).map(|(_, title)| title.trim().to_string()).filter(|title| !title.is_empty()).unwrap_or_else(|| rest.to_string())
}

fn seq_actor_token(raw: &str) -> (&str, bool, bool) {
    let token = raw.trim();
    let activate = token.starts_with('+') || token.ends_with('+');
    let deactivate = token.starts_with('-') || token.ends_with('-');
    (token.trim_matches(['+', '-']).trim(), activate, deactivate)
}

fn seq_activations(items: &[SeqItem], item_y: &[f32], bottom: f32) -> Vec<(usize, f32, f32, usize)> {
    let mut open: Vec<(usize, f32, usize)> = Vec::new();
    let mut bars = Vec::new();
    let mut depth = vec![0usize; 64];
    let depth_of = |depth: &mut Vec<usize>, who: usize| -> usize {
        if who >= depth.len() {
            depth.resize(who + 1, 0);
        }
        depth[who]
    };
    for (idx, item) in items.iter().enumerate() {
        let y = item_y.get(idx).copied().unwrap_or(bottom);
        let mut turn_on = Vec::new();
        let mut turn_off = Vec::new();
        match item {
            SeqItem::Activate(who) => turn_on.push(*who),
            SeqItem::Deactivate(who) => turn_off.push(*who),
            SeqItem::Message {
                from,
                to,
                activate_from,
                deactivate_from,
                activate_to,
                deactivate_to,
                ..
            } => {
                if *deactivate_from {
                    turn_off.push(*from);
                }
                if *deactivate_to {
                    turn_off.push(*to);
                }
                if *activate_from {
                    turn_on.push(*from);
                }
                if *activate_to {
                    turn_on.push(*to);
                }
            }
            _ => {}
        }
        for who in turn_off {
            if let Some(pos) = open.iter().rposition(|(id, _, _)| *id == who) {
                let (_, start, level) = open.remove(pos);
                bars.push((who, start, y, level));
                if who < depth.len() && depth[who] > 0 {
                    depth[who] -= 1;
                }
            }
        }
        for who in turn_on {
            let level = depth_of(&mut depth, who);
            open.push((who, y, level));
            depth[who] += 1;
        }
    }
    for (who, start, level) in open {
        bars.push((who, start, bottom, level));
    }
    bars
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
    let rest = l.strip_prefix("Note ").or_else(|| l.strip_prefix("note "))?.trim();
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
            let (from_id, activate_from, deactivate_from) = seq_actor_token(from_raw);
            let (to_id, activate_to, deactivate_to) = seq_actor_token(to_raw);
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
                activate_from,
                deactivate_from,
                activate_to,
                deactivate_to,
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
    Cylinder,
    Hexagon,
    Trapezoid,
    InvTrapezoid,
    Parallelogram,
    Asymmetric,
}

#[derive(Debug, Clone)]
struct FlowNode {
    _id: String,
    label: String,
    shape: NodeShape,
    subgraph: Option<usize>,
    /// `class id name` / `id:::name`, in order.
    classes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeStyle {
    Solid,
    Dashed,
    Thick,
    /// `~~~`: takes part in the layout but is not drawn.
    Invisible,
}

/// What an edge ends in: `-->` (arrow), `--o` (circle), `--x` (cross) or nothing (`---`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeEnd {
    None,
    Arrow,
    Circle,
    Cross,
}

/// One parsed link token between two node groups.
#[derive(Debug, Clone, PartialEq)]
struct EdgeTok {
    style: EdgeStyle,
    start: EdgeEnd,
    end: EdgeEnd,
    label: Option<String>,
}

#[derive(Debug, Clone)]
struct FlowEdge {
    from: usize,
    to: usize,
    label: Option<String>,
    style: EdgeStyle,
    start: EdgeEnd,
    end: EdgeEnd,
}

/// The subset of CSS properties `classDef`, `class`, `style` and `linkStyle` apply.
#[derive(Debug, Clone, Default, PartialEq)]
struct FlowStyle {
    fill: Option<String>,
    stroke: Option<String>,
    stroke_width: Option<String>,
    dasharray: Option<String>,
    color: Option<String>,
}

impl FlowStyle {
    fn merge(&mut self, other: &FlowStyle) {
        if other.fill.is_some() {
            self.fill.clone_from(&other.fill);
        }
        if other.stroke.is_some() {
            self.stroke.clone_from(&other.stroke);
        }
        if other.stroke_width.is_some() {
            self.stroke_width.clone_from(&other.stroke_width);
        }
        if other.dasharray.is_some() {
            self.dasharray.clone_from(&other.dasharray);
        }
        if other.color.is_some() {
            self.color.clone_from(&other.color);
        }
    }

    fn is_empty(&self) -> bool {
        *self == FlowStyle::default()
    }
}

/// `fill:#f9f,stroke:#333,stroke-width:4px` (commas inside `rgb(...)` stay). Unknown keys and
/// values with characters outside a conservative CSS set are dropped, so nothing reaches the SVG unescaped.
fn parse_flow_style(props: &str) -> FlowStyle {
    let mut out = FlowStyle::default();
    let mut depth = 0i32;
    let mut start = 0usize;
    let props = props.trim().trim_end_matches(';');
    let mut pieces = Vec::new();
    for (idx, ch) in props.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                pieces.push(&props[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    pieces.push(&props[start..]);
    for piece in pieces {
        let Some((key, value)) = piece.split_once(':') else { continue };
        let value = value.trim().trim_end_matches("!important").trim();
        if value.is_empty()
            || !value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '#' | '.' | '%' | '(' | ')' | ',' | ' ' | '-'))
        {
            continue;
        }
        let value = value.to_string();
        match key.trim() {
            "fill" | "background" | "background-color" => out.fill = Some(value),
            "stroke" => out.stroke = Some(value),
            "stroke-width" => out.stroke_width = Some(value.trim_end_matches("px").trim().to_string()),
            "stroke-dasharray" => out.dasharray = Some(value),
            "color" => out.color = Some(value),
            _ => {}
        }
    }
    out
}

/// Rewrite the paint attributes of a fragment drawn by this module: every `fill` that is not
/// `none`, every `stroke` and `stroke-width`, and a dash pattern on every element.
fn restyle_svg_fragment(fragment: &str, style: &FlowStyle) -> String {
    fn replace_attr(input: &str, name: &str, value: &str, skip_none: bool) -> String {
        let needle = format!(" {name}=\"");
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(at) = rest.find(&needle) {
            let value_start = at + needle.len();
            let Some(len) = rest[value_start..].find('"') else { break };
            let old = &rest[value_start..value_start + len];
            out.push_str(&rest[..value_start]);
            if skip_none && old == "none" {
                out.push_str(old);
            } else {
                out.push_str(&escape_xml(value));
            }
            rest = &rest[value_start + len..];
        }
        out.push_str(rest);
        out
    }
    let mut out = fragment.to_string();
    if let Some(fill) = &style.fill {
        out = replace_attr(&out, "fill", fill, true);
    }
    if let Some(stroke) = &style.stroke {
        out = replace_attr(&out, "stroke", stroke, true);
    }
    if let Some(width) = &style.stroke_width {
        out = replace_attr(&out, "stroke-width", width, false);
    }
    if let Some(dash) = &style.dasharray {
        out = out.replace("/>", &format!(r#" stroke-dasharray="{}"/>"#, escape_xml(dash)));
    }
    out
}

#[derive(Debug, Clone)]
struct SubgraphInfo {
    title: String,
    dir: Option<FlowDir>,
}

fn render_flowchart_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut lines = source.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let header = lines.next()?;
    let dir_token = header
        .split_whitespace()
        .nth(1)
        .unwrap_or("TD")
        .trim_end_matches(';')
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
    let mut class_defs: HashMap<String, FlowStyle> = HashMap::new();
    let mut class_assign: Vec<(String, String)> = Vec::new();
    let mut node_styles: Vec<(String, FlowStyle)> = Vec::new();
    let mut link_styles: Vec<(Option<usize>, FlowStyle)> = Vec::new();

    for line in lines {
        for stmt in split_flow_statements(line) {
            let l = stmt.trim();
            if l.is_empty() || l.starts_with("click ") || l.starts_with("accTitle") || l.starts_with("accDescr") {
                continue;
            }
            if let Some(rest) = l.strip_prefix("classDef ") {
                let rest = rest.trim();
                let (names, props) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let style = parse_flow_style(props);
                for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    class_defs.entry(name.to_string()).or_default().merge(&style);
                }
                continue;
            }
            if let Some(rest) = l.strip_prefix("class ") {
                let rest = rest.trim();
                if let Some((ids, class)) = rest.rsplit_once(char::is_whitespace) {
                    for id in ids.split(',').map(str::trim).filter(|id| !id.is_empty()) {
                        class_assign.push((id.to_string(), class.trim().to_string()));
                    }
                }
                continue;
            }
            if let Some(rest) = l.strip_prefix("style ") {
                let rest = rest.trim();
                if let Some((id, props)) = rest.split_once(char::is_whitespace) {
                    node_styles.push((id.to_string(), parse_flow_style(props)));
                }
                continue;
            }
            if let Some(rest) = l.strip_prefix("linkStyle ") {
                let rest = rest.trim();
                if let Some((which, props)) = rest.split_once(char::is_whitespace) {
                    let style = parse_flow_style(props);
                    if which == "default" {
                        link_styles.push((None, style));
                    } else {
                        for index in which.split(',').filter_map(|n| n.trim().parse::<usize>().ok()) {
                            link_styles.push((Some(index), style.clone()));
                        }
                    }
                }
                continue;
            }
            if l == "end" {
                sg_stack.pop();
                continue;
            }
            if let Some(rest) = l.strip_prefix("subgraph ") {
                let title = parse_subgraph_title(rest.trim());
                let sg_idx = subgraphs.len();
                subgraphs.push(SubgraphInfo { title, dir: None });
                sg_stack.push(sg_idx);
                continue;
            }
            if let Some(rest) = l.strip_prefix("direction ") {
                if let Some(&sg) = sg_stack.last() {
                    subgraphs[sg].dir = Some(parse_flow_dir(rest));
                }
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
    }

    if nodes.is_empty() {
        return None;
    }

    for (id, class) in class_assign {
        if let Some(&idx) = node_map.get(&id) {
            nodes[idx].classes.push(class);
        }
    }
    let mut node_style: Vec<FlowStyle> = nodes
        .iter()
        .map(|node| {
            let mut style = class_defs.get("default").cloned().unwrap_or_default();
            for class in &node.classes {
                if let Some(def) = class_defs.get(class) {
                    style.merge(def);
                }
            }
            style
        })
        .collect();
    for (id, style) in &node_styles {
        if let Some(&idx) = node_map.get(id) {
            node_style[idx].merge(style);
        }
    }
    let edge_style: Vec<FlowStyle> = (0..edges.len())
        .map(|index| {
            // `linkStyle default` is the base; numbered statements override it.
            let mut style = FlowStyle::default();
            for (_, s) in link_styles.iter().filter(|(which, _)| which.is_none()) {
                style.merge(s);
            }
            for (_, s) in link_styles.iter().filter(|(which, _)| *which == Some(index)) {
                style.merge(s);
            }
            style
        })
        .collect();

    // Node dimensions. Long labels wrap inside a width cap and the box grows taller.
    let node_sizes: Vec<(f32, f32)> = nodes
        .iter()
        .map(|node| flow_node_size(node.shape, &node.label))
        .collect();

    let (centers, total_w, total_h) = layout_flowchart(&nodes, &edges, &node_sizes, &subgraphs, dir);

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(
        r##"<defs>
  <marker id="flow-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
    <path d="M 0 1 L 10 5 L 0 9 z" fill="#475569"/>
  </marker>
  <marker id="flow-circle" viewBox="0 0 10 10" refX="9.5" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
    <circle cx="5" cy="5" r="4" fill="#ffffff" stroke="#475569" stroke-width="1.5"/>
  </marker>
  <marker id="flow-cross" viewBox="0 0 10 10" refX="10" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse">
    <line x1="1" y1="1" x2="9" y2="9" stroke="#475569" stroke-width="1.8"/>
    <line x1="9" y1="1" x2="1" y2="9" stroke="#475569" stroke-width="1.8"/>
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
        // Constant padding. The title band stays above the children.
        let frame_pad = 16.0f32;
        let top_inset = if sg.title.is_empty() { frame_pad } else { frame_pad + 22.0 };
        let sx = min_x - frame_pad;
        let sy = min_y - top_inset;
        let sw = (max_x + frame_pad) - sx;
        let sh = (max_y + frame_pad) - sy;
        svg.push_str(&format!(
            r##"<rect x="{sx:.1}" y="{sy:.1}" width="{sw:.1}" height="{sh:.1}" rx="8" fill="#f8fafc" stroke="#cbd5e1" stroke-width="1.2" stroke-dasharray="5,3"/>"##
        ));
        if !sg.title.is_empty() {
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="{:.1}" font-size="11.5" font-weight="600" fill="#475569">{}</text>"##,
                sx + 10.0,
                sy + 16.0,
                escape_xml(&sg.title)
            ));
        }
    }

    // Edges follow the diagram direction. A subgraph's own direction is layout-only.
    let horizontal = matches!(dir, FlowDir::Lr | FlowDir::Rl);
    for (edge_idx, e) in edges.iter().enumerate() {
        if e.style == EdgeStyle::Invisible {
            continue;
        }
        let route = route_layered_edge(
            e.from,
            e.to,
            &centers,
            &node_sizes,
            horizontal,
            true,
            (total_w as f32, total_h as f32),
        );
        let linked = &edge_style[edge_idx];
        let stroke_w = linked.stroke_width.clone().unwrap_or_else(|| {
            match e.style {
                EdgeStyle::Thick => "2.6",
                _ => "1.4",
            }
            .to_string()
        });
        let dash = match (&linked.dasharray, e.style) {
            (Some(dash), _) => format!(r##" stroke-dasharray="{}""##, escape_xml(dash)),
            (None, EdgeStyle::Dashed) => r##" stroke-dasharray="5,4""##.to_string(),
            _ => String::new(),
        };
        let marker_id = |end: EdgeEnd| match end {
            EdgeEnd::Arrow => Some("flow-arrow"),
            EdgeEnd::Circle => Some("flow-circle"),
            EdgeEnd::Cross => Some("flow-cross"),
            EdgeEnd::None => None,
        };
        let mut extra = dash;
        if let Some(id) = marker_id(e.start) {
            extra.push_str(&format!(r##" marker-start="url(#{id})""##));
        }
        if let Some(id) = marker_id(e.end) {
            extra.push_str(&format!(r##" marker-end="url(#{id})""##));
        }
        let stroke = linked.stroke.as_deref().map(escape_xml).unwrap_or_else(|| "#475569".to_string());
        push_routed_edge(&mut svg, &route, &stroke, &escape_xml(&stroke_w), &extra);
        if let Some(lbl) = &e.label {
            let (lx, ly) = route.label_at;
            let lines = label_lines(lbl, 140.0, 11.0);
            let lw = lines.iter().map(|line| estimate_text_width(line, 11.0)).fold(0.0f32, f32::max) + 10.0;
            let lh = lines.len() as f32 * 14.0 + 6.0;
            svg.push_str(&format!(
                r##"<rect x="{:.1}" y="{:.1}" width="{lw:.1}" height="{lh:.1}" rx="3" fill="#ffffff" stroke="#e2e8f0" stroke-width="0.8"/>"##,
                lx - lw / 2.0,
                ly - lh / 2.0
            ));
            push_centered_label(&mut svg, lx, ly, lbl, 11.0, 140.0, "#334155", "400");
        }
    }

    // Nodes
    for (idx, node) in nodes.iter().enumerate() {
        let (cx, cy) = centers[idx];
        let (w, h) = node_sizes[idx];
        let x = cx - w / 2.0;
        let y = cy - h / 2.0;
        let shape_at = svg.len();
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
                    r##"<ellipse cx="{cx:.1}" cy="{cy:.1}" rx="{:.1}" ry="{:.1}" fill="#f5f3ff" stroke="#7c3aed" stroke-width="1.4" data-shape="circle"/>"##,
                    w / 2.0, h / 2.0
                ));
            }
            NodeShape::Cylinder => push_flow_cylinder(&mut svg, x, y, w, h),
            NodeShape::Hexagon => push_flow_polygon(&mut svg, &hexagon_points(x, y, w, h), "#f5f3ff", "#7c3aed", "hexagon"),
            NodeShape::Trapezoid => push_flow_polygon(&mut svg, &trapezoid_points(x, y, w, h, true), "#eff6ff", "#2563eb", "trapezoid"),
            NodeShape::InvTrapezoid => push_flow_polygon(&mut svg, &trapezoid_points(x, y, w, h, false), "#eff6ff", "#2563eb", "inv-trapezoid"),
            NodeShape::Parallelogram => push_flow_polygon(&mut svg, &parallelogram_points(x, y, w, h), "#eff6ff", "#2563eb", "parallelogram"),
            NodeShape::Asymmetric => push_flow_polygon(&mut svg, &asymmetric_points(x, y, w, h), "#eff6ff", "#2563eb", "asymmetric"),
        }
        let style = &node_style[idx];
        if !style.is_empty() {
            let shape_svg = svg.split_off(shape_at);
            svg.push_str(&restyle_svg_fragment(&shape_svg, style));
        }
        let label_width = match node.shape {
            NodeShape::Diamond | NodeShape::Hexagon | NodeShape::Asymmetric => w * 0.62,
            NodeShape::Circle => w * 0.72,
            _ => w - 16.0,
        };
        let text_fill = style.color.as_deref().map(escape_xml).unwrap_or_else(|| "#0f172a".to_string());
        // Size and draw share the 168px cap. A wider box must not reflow the lines.
        push_centered_label(&mut svg, cx, cy, &node.label, 13.0, label_width.clamp(24.0, 168.0), &text_fill, "500");
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

#[derive(Clone, Copy, Debug)]
struct RectF {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn node_rect(cx: f32, cy: f32, w: f32, h: f32) -> RectF {
    RectF { x: cx - w / 2.0, y: cy - h / 2.0, w, h }
}

fn pad_rect(rect: RectF, pad: f32) -> RectF {
    RectF {
        x: rect.x - pad,
        y: rect.y - pad,
        w: (rect.w + pad * 2.0).max(0.0),
        h: (rect.h + pad * 2.0).max(0.0),
    }
}

fn point_in_rect(px: f32, py: f32, rect: &RectF) -> bool {
    px > rect.x + 0.15 && py > rect.y + 0.15 && px < rect.x + rect.w - 0.15 && py < rect.y + rect.h - 0.15
}

fn orient(ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> f32 {
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}

fn on_segment(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> bool {
    px >= ax.min(bx) - 0.05 && px <= ax.max(bx) + 0.05 && py >= ay.min(by) - 0.05 && py <= ay.max(by) + 0.05
}

fn segments_cross(x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32, x4: f32, y4: f32) -> bool {
    let o1 = orient(x1, y1, x2, y2, x3, y3);
    let o2 = orient(x1, y1, x2, y2, x4, y4);
    let o3 = orient(x3, y3, x4, y4, x1, y1);
    let o4 = orient(x3, y3, x4, y4, x2, y2);
    if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
        return true;
    }
    o1.abs() <= 0.05 && on_segment(x1, y1, x2, y2, x3, y3)
        || o2.abs() <= 0.05 && on_segment(x1, y1, x2, y2, x4, y4)
        || o3.abs() <= 0.05 && on_segment(x3, y3, x4, y4, x1, y1)
        || o4.abs() <= 0.05 && on_segment(x3, y3, x4, y4, x2, y2)
}

fn segment_hits_rect(x1: f32, y1: f32, x2: f32, y2: f32, rect: &RectF) -> bool {
    if (x1 - x2).abs() < 0.4 && (y1 - y2).abs() < 0.4 {
        return point_in_rect(x1, y1, rect);
    }
    if point_in_rect(x1, y1, rect) || point_in_rect(x2, y2, rect) {
        return true;
    }
    let left = rect.x;
    let right = rect.x + rect.w;
    let top = rect.y;
    let bottom = rect.y + rect.h;
    segments_cross(x1, y1, x2, y2, left, top, right, top)
        || segments_cross(x1, y1, x2, y2, right, top, right, bottom)
        || segments_cross(x1, y1, x2, y2, right, bottom, left, bottom)
        || segments_cross(x1, y1, x2, y2, left, bottom, left, top)
}

fn segment_hits_any(x1: f32, y1: f32, x2: f32, y2: f32, obstacles: &[RectF]) -> bool {
    obstacles.iter().any(|rect| segment_hits_rect(x1, y1, x2, y2, rect))
}

fn cubic_point(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1)
}

fn curve_hits(sx: f32, sy: f32, ex: f32, ey: f32, horizontal: bool, obstacles: &[RectF]) -> bool {
    let (p1, p2) = if horizontal {
        let mx = (sx + ex) / 2.0;
        ((mx, sy), (mx, ey))
    } else {
        let my = (sy + ey) / 2.0;
        ((sx, my), (ex, my))
    };
    let mut prev = (sx, sy);
    for step in 1..=24 {
        let p = cubic_point((sx, sy), p1, p2, (ex, ey), step as f32 / 24.0);
        if segment_hits_any(prev.0, prev.1, p.0, p.1, obstacles) {
            return true;
        }
        prev = p;
    }
    false
}

fn curve_d(sx: f32, sy: f32, ex: f32, ey: f32, horizontal: bool) -> String {
    if horizontal {
        let mx = (sx + ex) / 2.0;
        format!("M {sx:.1} {sy:.1} C {mx:.1} {sy:.1}, {mx:.1} {ey:.1}, {ex:.1} {ey:.1}")
    } else {
        let my = (sy + ey) / 2.0;
        format!("M {sx:.1} {sy:.1} C {sx:.1} {my:.1}, {ex:.1} {my:.1}, {ex:.1} {ey:.1}")
    }
}

fn unit_dir(x1: f32, y1: f32, x2: f32, y2: f32) -> (f32, f32) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-3 { (0.0, 0.0) } else { (dx / len, dy / len) }
}

fn point_in_canvas(px: f32, py: f32, canvas: (f32, f32)) -> bool {
    px >= -0.05 && py >= -0.05 && px <= canvas.0 + 0.05 && py <= canvas.1 + 0.05
}

/// Free lane coordinates inside `[lo, hi]`, inset from obstacle intervals.
fn lane_coords(lo: f32, hi: f32, blocked: &[(f32, f32)]) -> Vec<f32> {
    if hi - lo < 8.0 {
        return Vec::new();
    }
    let mut spans: Vec<(f32, f32)> = blocked.iter().copied().filter(|(a, b)| b - a > 0.5).collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (a, b) in spans {
        if let Some((_, end)) = merged.last_mut()
            && a <= *end
        {
            *end = end.max(b);
            continue;
        }
        merged.push((a, b));
    }
    let mut gaps = Vec::new();
    let mut cursor = lo;
    for (a, b) in merged {
        let gap_end = a.min(hi);
        if gap_end - cursor >= 8.0 {
            gaps.push((cursor, gap_end));
        }
        cursor = cursor.max(b);
        if cursor >= hi {
            break;
        }
    }
    if hi - cursor >= 8.0 {
        gaps.push((cursor, hi));
    }
    let mut lanes = Vec::new();
    for (a, b) in gaps {
        let inset_a = a + 6.0;
        let inset_b = b - 6.0;
        if inset_b >= inset_a {
            lanes.push((inset_a + inset_b) / 2.0);
            if inset_b - inset_a > 12.0 {
                lanes.push(inset_a);
                lanes.push(inset_b);
            }
        }
    }
    lanes
}

fn exit_points(anchor: (f32, f32), center: (f32, f32), own: RectF, obstacles: &[RectF], canvas: (f32, f32)) -> Vec<(f32, f32)> {
    let own = pad_rect(own, -0.5);
    let dx = anchor.0 - center.0;
    let dy = anchor.1 - center.1;
    let mut dirs = Vec::new();
    if dx.abs() >= dy.abs() && dx.abs() > 1e-3 {
        dirs.push((dx.signum(), 0.0));
        dirs.push((0.0, 1.0));
        dirs.push((0.0, -1.0));
    } else if dy.abs() > 1e-3 {
        dirs.push((0.0, dy.signum()));
        dirs.push((1.0, 0.0));
        dirs.push((-1.0, 0.0));
    }
    let mut out = Vec::new();
    for (ux, uy) in dirs {
        for len in [14.0_f32, 8.0] {
            let p = (anchor.0 + ux * len, anchor.1 + uy * len);
            if !point_in_canvas(p.0, p.1, canvas) {
                continue;
            }
            if point_in_rect(p.0, p.1, &own) || segment_hits_rect(anchor.0, anchor.1, p.0, p.1, &own) {
                continue;
            }
            if obstacles.iter().any(|rect| point_in_rect(p.0, p.1, rect) || segment_hits_rect(anchor.0, anchor.1, p.0, p.1, rect)) {
                continue;
            }
            out.push(p);
            break;
        }
    }
    if out.is_empty() {
        out.push(anchor);
    }
    out
}

fn simplify_points(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut cleaned = Vec::new();
    for &p in points {
        if cleaned.last().is_some_and(|q: &(f32, f32)| (q.0 - p.0).abs() < 0.4 && (q.1 - p.1).abs() < 0.4) {
            continue;
        }
        cleaned.push(p);
    }
    if cleaned.len() < 3 {
        return cleaned;
    }
    let mut slim = vec![cleaned[0]];
    for i in 1..cleaned.len() - 1 {
        let a = *slim.last().unwrap();
        let b = cleaned[i];
        let c = cleaned[i + 1];
        let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if cross.abs() > 0.5 {
            slim.push(b);
        }
    }
    slim.push(*cleaned.last().unwrap());
    slim
}

fn polyline_clear(points: &[(f32, f32)], obstacles: &[RectF], canvas: (f32, f32)) -> bool {
    if points.iter().any(|p| !point_in_canvas(p.0, p.1, canvas)) {
        return false;
    }
    points.windows(2).all(|w| !segment_hits_any(w[0].0, w[0].1, w[1].0, w[1].1, obstacles))
}

fn label_on_polyline(points: &[(f32, f32)], obstacles: &[RectF]) -> (f32, f32) {
    let mut best_len = -1.0f32;
    let mut best = points[0];
    for w in points.windows(2) {
        let dx = w[1].0 - w[0].0;
        let dy = w[1].1 - w[0].1;
        let len = (dx * dx + dy * dy).sqrt();
        if len <= best_len {
            continue;
        }
        let mid = ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0);
        if obstacles.iter().any(|rect| point_in_rect(mid.0, mid.1, rect)) {
            continue;
        }
        best_len = len;
        best = mid;
    }
    best
}

fn path_from_points(points: &[(f32, f32)]) -> String {
    let mut d = format!("M {:.1} {:.1}", points[0].0, points[0].1);
    for p in &points[1..] {
        d.push_str(&format!(" L {:.1} {:.1}", p.0, p.1));
    }
    d
}

fn first_dir(points: &[(f32, f32)]) -> (f32, f32) {
    for w in points.windows(2) {
        let d = unit_dir(w[0].0, w[0].1, w[1].0, w[1].1);
        if d.0 != 0.0 || d.1 != 0.0 {
            return d;
        }
    }
    (0.0, 0.0)
}

fn last_dir(points: &[(f32, f32)]) -> (f32, f32) {
    for i in (1..points.len()).rev() {
        let d = unit_dir(points[i - 1].0, points[i - 1].1, points[i].0, points[i].1);
        if d.0 != 0.0 || d.1 != 0.0 {
            return d;
        }
    }
    (0.0, 0.0)
}

#[derive(Debug)]
enum EdgeRouteKind {
    Line,
    Path { d: String, detour: bool },
}

#[derive(Debug)]
struct EdgeRoute {
    kind: EdgeRouteKind,
    label_at: (f32, f32),
    start: (f32, f32),
    end: (f32, f32),
    leave_dir: (f32, f32),
    arrive_dir: (f32, f32),
}

fn straight_route(start: (f32, f32), end: (f32, f32)) -> EdgeRoute {
    let dir = unit_dir(start.0, start.1, end.0, end.1);
    EdgeRoute {
        kind: EdgeRouteKind::Line,
        label_at: ((start.0 + end.0) / 2.0, (start.1 + end.1) / 2.0),
        start,
        end,
        leave_dir: dir,
        arrive_dir: dir,
    }
}

fn path_route(d: String, detour: bool, start: (f32, f32), end: (f32, f32), label_at: (f32, f32), points: &[(f32, f32)]) -> EdgeRoute {
    let (leave_dir, arrive_dir) = if points.len() >= 2 {
        (first_dir(points), last_dir(points))
    } else {
        let dir = unit_dir(start.0, start.1, end.0, end.1);
        (dir, dir)
    };
    EdgeRoute { kind: EdgeRouteKind::Path { d, detour }, label_at, start, end, leave_dir, arrive_dir }
}

/// Border-to-border route. A curve or line that enters another node becomes an
/// orthogonal polyline. When no lane stays outside the nodes, the original curve remains.
fn route_layered_edge(
    from: usize,
    to: usize,
    centers: &[(f32, f32)],
    sizes: &[(f32, f32)],
    horizontal: bool,
    curved: bool,
    canvas: (f32, f32),
) -> EdgeRoute {
    let (x1, y1) = centers[from];
    let (x2, y2) = centers[to];
    let (w1, h1) = sizes[from];
    let (w2, h2) = sizes[to];
    let start = edge_anchor(x1, y1, w1, h1, x2, y2);
    let end = edge_anchor(x2, y2, w2, h2, x1, y1);
    let fallback = |start: (f32, f32), end: (f32, f32)| -> EdgeRoute {
        if curved {
            let d = curve_d(start.0, start.1, end.0, end.1, horizontal);
            let dir = unit_dir(start.0, start.1, end.0, end.1);
            EdgeRoute {
                kind: EdgeRouteKind::Path { d, detour: false },
                label_at: ((start.0 + end.0) / 2.0, (start.1 + end.1) / 2.0),
                start,
                end,
                leave_dir: dir,
                arrive_dir: dir,
            }
        } else {
            straight_route(start, end)
        }
    };
    if from == to {
        return fallback(start, end);
    }
    let mut blockers = Vec::new();
    for idx in 0..centers.len() {
        if idx == from || idx == to {
            continue;
        }
        blockers.push(node_rect(centers[idx].0, centers[idx].1, sizes[idx].0, sizes[idx].1));
    }
    let hits = if curved {
        curve_hits(start.0, start.1, end.0, end.1, horizontal, &blockers)
    } else {
        segment_hits_any(start.0, start.1, end.0, end.1, &blockers)
    };
    if !hits {
        return fallback(start, end);
    }

    let source = node_rect(x1, y1, w1, h1);
    let target = node_rect(x2, y2, w2, h2);
    let mut avoid: Vec<RectF> = blockers.iter().copied().map(|rect| pad_rect(rect, 4.0)).collect();
    let shrunk_source = pad_rect(source, -0.5);
    let shrunk_target = pad_rect(target, -0.5);
    avoid.push(shrunk_source);
    avoid.push(shrunk_target);
    let exits_s = exit_points(start, (x1, y1), source, &blockers, canvas);
    let exits_t = exit_points(end, (x2, y2), target, &blockers, canvas);
    let pad = 8.0;
    let y_span = (start.1.min(end.1) - 14.0, start.1.max(end.1) + 14.0);
    let x_span = (start.0.min(end.0) - 14.0, start.0.max(end.0) + 14.0);
    let blocked_x: Vec<(f32, f32)> = avoid
        .iter()
        .filter(|rect| rect.y < y_span.1 && rect.y + rect.h > y_span.0)
        .map(|rect| (rect.x, rect.x + rect.w))
        .collect();
    let blocked_y: Vec<(f32, f32)> = avoid
        .iter()
        .filter(|rect| rect.x < x_span.1 && rect.x + rect.w > x_span.0)
        .map(|rect| (rect.y, rect.y + rect.h))
        .collect();
    // `vertical` lanes are constant-x. Flow-direction gaps are tried first.
    let axes = if horizontal { [false, true] } else { [true, false] };
    for vertical_lane in axes {
        let mut lanes = if vertical_lane {
            lane_coords(pad, canvas.0 - pad, &blocked_x)
        } else {
            lane_coords(pad, canvas.1 - pad, &blocked_y)
        };
        let ideal = if vertical_lane { (start.0 + end.0) / 2.0 } else { (start.1 + end.1) / 2.0 };
        lanes.sort_by(|a, b| {
            (a - ideal)
                .abs()
                .partial_cmp(&(b - ideal).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        });
        lanes.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        for lane in lanes {
            for &s in &exits_s {
                for &t in &exits_t {
                    let raw = if vertical_lane {
                        vec![start, s, (lane, s.1), (lane, t.1), t, end]
                    } else {
                        vec![start, s, (s.0, lane), (t.0, lane), t, end]
                    };
                    let points = simplify_points(&raw);
                    if points.len() < 2 || !polyline_clear(&points, &avoid, canvas) {
                        continue;
                    }
                    let label_at = label_on_polyline(&points, &avoid);
                    return path_route(path_from_points(&points), true, start, end, label_at, &points);
                }
            }
        }
    }
    fallback(start, end)
}

fn push_routed_edge(svg: &mut String, route: &EdgeRoute, stroke: &str, width: &str, extra: &str) {
    match &route.kind {
        EdgeRouteKind::Line => {
            let (x1, y1) = route.start;
            let (x2, y2) = route.end;
            svg.push_str(&format!(
                r##"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="{stroke}" stroke-width="{width}"{extra}/>"##
            ));
        }
        EdgeRouteKind::Path { d, detour } => {
            let flag = if *detour { r#" data-route="detour""# } else { "" };
            svg.push_str(&format!(
                r##"<path d="{d}" fill="none" stroke="{stroke}" stroke-width="{width}"{extra}{flag}/>"##
            ));
        }
    }
}

#[cfg(test)]
fn sample_path(d: &str) -> Vec<(f32, f32)> {
    let mut samples = Vec::new();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut cmd = ' ';
    let mut number = String::new();
    let mut nums = Vec::new();
    let flush_num = |number: &mut String, nums: &mut Vec<f32>| {
        if !number.is_empty() {
            if let Ok(value) = number.parse::<f32>() {
                nums.push(value);
            }
            number.clear();
        }
    };
    let apply = |cmd: char, nums: &mut Vec<f32>, cx: &mut f32, cy: &mut f32, samples: &mut Vec<(f32, f32)>| {
        match cmd {
            'M' => {
                if nums.len() >= 2 {
                    *cx = nums[0];
                    *cy = nums[1];
                    samples.push((*cx, *cy));
                }
                let mut i = 2;
                while i + 1 < nums.len() {
                    let next = (nums[i], nums[i + 1]);
                    let dist = ((next.0 - *cx).powi(2) + (next.1 - *cy).powi(2)).sqrt();
                    let steps = ((dist / 4.0).ceil() as i32).max(1);
                    for step in 1..=steps {
                        let t = step as f32 / steps as f32;
                        samples.push((*cx + (next.0 - *cx) * t, *cy + (next.1 - *cy) * t));
                    }
                    *cx = next.0;
                    *cy = next.1;
                    i += 2;
                }
            }
            'L' => {
                let mut i = 0;
                while i + 1 < nums.len() {
                    let next = (nums[i], nums[i + 1]);
                    let dist = ((next.0 - *cx).powi(2) + (next.1 - *cy).powi(2)).sqrt();
                    let steps = ((dist / 4.0).ceil() as i32).max(1);
                    for step in 1..=steps {
                        let t = step as f32 / steps as f32;
                        samples.push((*cx + (next.0 - *cx) * t, *cy + (next.1 - *cy) * t));
                    }
                    *cx = next.0;
                    *cy = next.1;
                    i += 2;
                }
            }
            'C' => {
                let mut i = 0;
                while i + 5 < nums.len() {
                    let p0 = (*cx, *cy);
                    let p1 = (nums[i], nums[i + 1]);
                    let p2 = (nums[i + 2], nums[i + 3]);
                    let p3 = (nums[i + 4], nums[i + 5]);
                    for step in 1..=16 {
                        samples.push(cubic_point(p0, p1, p2, p3, step as f32 / 16.0));
                    }
                    *cx = p3.0;
                    *cy = p3.1;
                    i += 6;
                }
            }
            _ => {}
        }
        nums.clear();
    };
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() {
            flush_num(&mut number, &mut nums);
            apply(cmd, &mut nums, &mut cx, &mut cy, &mut samples);
            cmd = ch;
        } else if ch == ',' || ch.is_whitespace() {
            flush_num(&mut number, &mut nums);
        } else {
            number.push(ch);
        }
    }
    flush_num(&mut number, &mut nums);
    apply(cmd, &mut nums, &mut cx, &mut cy, &mut samples);
    samples
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
    let Some((lhs, mut pending, mut rest)) = split_next_flowchart_edge(stmt) else {
        let _ = ensure_node_group(stmt, subgraph, nodes, node_map);
        return;
    };
    let mut prev_group = ensure_node_group(lhs, subgraph, nodes, node_map);
    loop {
        let next = split_next_flowchart_edge(rest);
        let node_expr = next.as_ref().map_or(rest, |(next_lhs, _, _)| next_lhs);
        let cur_group = ensure_node_group(node_expr, subgraph, nodes, node_map);
        for &u in &prev_group {
            for &v in &cur_group {
                edges.push(FlowEdge {
                    from: u,
                    to: v,
                    label: pending.label.clone(),
                    style: pending.style,
                    start: pending.start,
                    end: pending.end,
                });
            }
        }
        prev_group = cur_group;
        match next {
            Some((_, tok, next_rest)) => {
                pending = tok;
                rest = next_rest;
            }
            None => break,
        }
    }
}

/// The text before the first link of `s` (skipping bracketed and quoted labels), the link, and the text after it.
fn split_next_flowchart_edge(s: &str) -> Option<(&str, EdgeTok, &str)> {
    let bytes = s.as_bytes();
    let mut cursor = 0usize;
    let mut quote = false;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"' {
            quote = !quote;
            cursor += 1;
            continue;
        }
        if !quote && matches!(bytes[cursor], b'[' | b'(' | b'{') {
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
        let word_start = cursor == 0 || bytes[cursor - 1].is_ascii_whitespace();
        if !quote && let Some((tok, consumed)) = match_flowchart_edge(&s[cursor..], word_start) {
            let mut lhs = s[..cursor].trim();
            // `A e1@--> B`: an edge id is not part of the node.
            if let Some(before_id) = lhs.strip_suffix('@') {
                lhs = before_id.rsplit_once(char::is_whitespace).map_or(before_id, |(node, _)| node).trim();
            }
            return Some((lhs, tok, &s[cursor + consumed..]));
        }
        cursor += s[cursor..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

fn edge_end_marker(byte: Option<&u8>) -> Option<EdgeEnd> {
    match byte {
        Some(b'>') => Some(EdgeEnd::Arrow),
        Some(b'o') => Some(EdgeEnd::Circle),
        Some(b'x') => Some(EdgeEnd::Cross),
        _ => None,
    }
}

/// The link body at the start of `s` (after any start marker): its style, end marker and length.
/// Solid `--+[-xo>]`, thick `==+[=xo>]`, dotted `-.+-[xo>]?` (or `.+-[xo>]?` closing a
/// dotted text label), invisible `~~~+`.
fn match_edge_body(s: &str) -> Option<(EdgeStyle, EdgeEnd, usize)> {
    let b = s.as_bytes();
    let run = |c: u8, from: usize| b[from..].iter().take_while(|&&x| x == c).count();
    let dotted = |from: usize| -> Option<(EdgeStyle, EdgeEnd, usize)> {
        let dots = run(b'.', from);
        if dots == 0 || b.get(from + dots) != Some(&b'-') {
            return None;
        }
        let at = from + dots + 1;
        match edge_end_marker(b.get(at)) {
            Some(end) => Some((EdgeStyle::Dashed, end, at + 1)),
            None => Some((EdgeStyle::Dashed, EdgeEnd::None, at)),
        }
    };
    match b.first()? {
        b'~' => {
            let n = run(b'~', 0);
            (n >= 3).then_some((EdgeStyle::Invisible, EdgeEnd::None, n))
        }
        b'-' if b.get(1) == Some(&b'.') => dotted(1),
        b'.' => dotted(0),
        &c @ (b'-' | b'=') => {
            let n = run(c, 0);
            let style = if c == b'-' { EdgeStyle::Solid } else { EdgeStyle::Thick };
            if n >= 2
                && let Some(end) = edge_end_marker(b.get(n))
            {
                return Some((style, end, n + 1));
            }
            (n >= 3).then_some((style, EdgeEnd::None, n))
        }
        _ => None,
    }
}

/// A link at the start of `s`: `-->`, `---`, `-.->`, `==>`, `~~~`, `<-->`, `o--o`, `x--x`,
/// `--o`, `--x`, longer forms, `-- text -->` style labels and `-->|text|` labels.
/// `word_start`: `s` begins a word, so a leading `o` / `x` may be a start marker.
fn match_flowchart_edge(s: &str, word_start: bool) -> Option<(EdgeTok, usize)> {
    let b = s.as_bytes();
    let marker_then_link = || matches!(b.get(1), Some(b'-' | b'=')) && matches!(b.get(2), Some(b'-' | b'=' | b'.'));
    let (start, i) = match b.first()? {
        b'<' if matches!(b.get(1), Some(b'-' | b'=')) => (EdgeEnd::Arrow, 1),
        b'o' if word_start && marker_then_link() => (EdgeEnd::Circle, 1),
        b'x' if word_start && marker_then_link() => (EdgeEnd::Cross, 1),
        b'-' | b'=' | b'~' => (EdgeEnd::None, 0),
        _ => return None,
    };
    let body = &s[i..];

    // Text labels inside the link: `-- text -->`, `-. text .->`, `== text ==>`.
    for (open, closer, style) in [("--", "--", EdgeStyle::Solid), ("-.", ".-", EdgeStyle::Dashed), ("==", "==", EdgeStyle::Thick)] {
        let Some(after) = body.strip_prefix(open) else { continue };
        if !after.starts_with(char::is_whitespace) {
            continue;
        }
        let mut search = 0usize;
        while let Some(rel) = after[search..].find(closer) {
            let at = search + rel;
            if let Some((close_style, end, len)) = match_edge_body(&after[at..])
                && close_style == style
            {
                let text = after[..at].trim();
                return Some((
                    EdgeTok {
                        style,
                        start,
                        end,
                        label: (!text.is_empty()).then(|| unquote(text)),
                    },
                    i + open.len() + at + len,
                ));
            }
            search = at + closer.len();
        }
    }

    let (style, end, len) = match_edge_body(body)?;
    if style == EdgeStyle::Invisible && start != EdgeEnd::None {
        return None;
    }
    let mut consumed = i + len;
    let after = &s[consumed..];
    let after_ws = after.trim_start();
    let mut label = None;
    if let Some(after_pipe) = after_ws.strip_prefix('|')
        && let Some(close_pipe) = after_pipe.find('|')
    {
        let text = after_pipe[..close_pipe].trim();
        label = (!text.is_empty()).then(|| unquote(text));
        consumed += (after.len() - after_ws.len()) + 1 + close_pipe + 1;
    }
    Some((EdgeTok { style, start, end, label }, consumed))
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
        let (p, class) = split_node_class(p);
        let (id, label_opt, shape_opt) = parse_node_token(p);
        if id.is_empty() {
            continue;
        }
        if let Some(&idx) = node_map.get(id) {
            if let Some(class) = class {
                nodes[idx].classes.push(class.to_string());
            }
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
                classes: class.map(|c| vec![c.to_string()]).unwrap_or_default(),
            });
            node_map.insert(id.to_string(), idx);
            ids.push(idx);
        }
    }
    ids
}

/// `A[Label]:::name` → (`A[Label]`, `name`). A `:::` inside the label is left alone.
fn split_node_class(token: &str) -> (&str, Option<&str>) {
    if let Some(at) = token.rfind(":::") {
        let class = token[at + 3..].trim();
        if !class.is_empty() && !class.contains([']', ')', '}', '"']) {
            return (token[..at].trim(), Some(class));
        }
    }
    (token, None)
}

fn parse_node_token(token: &str) -> (&str, Option<String>, Option<NodeShape>) {
    let t = token.trim();
    // The shape starts at the first delimiter. A `>` inside `<br/>` is not asymmetric.
    let Some(open_at) = first_shape_delim(t) else {
        return (t, None, None);
    };
    let tail = &t[open_at..];
    for (open, close, shape) in [
        ("[(", ")]", NodeShape::Cylinder),
        ("([", "])", NodeShape::Stadium),
        ("[[", "]]", NodeShape::Subroutine),
        ("((", "))", NodeShape::Circle),
        ("{{", "}}", NodeShape::Hexagon),
        ("[/", "/]", NodeShape::Trapezoid),
        ("[/", "\\]", NodeShape::Parallelogram),
        ("[\\", "\\]", NodeShape::InvTrapezoid),
        ("[\\", "/]", NodeShape::Parallelogram),
        (">", "]", NodeShape::Asymmetric),
        ("[", "]", NodeShape::Rect),
        ("(", ")", NodeShape::Round),
        ("{", "}", NodeShape::Diamond),
    ] {
        if tail.starts_with(open) && t.ends_with(close) && t.len() >= open_at + open.len() + close.len() {
            let id = t[..open_at].trim();
            let inner = &t[open_at + open.len()..t.len() - close.len()];
            return (id, Some(unquote(inner.trim())), Some(shape));
        }
    }
    (t, None, None)
}

fn first_shape_delim(token: &str) -> Option<usize> {
    let bytes = token.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'<' {
            if let Some(rel) = token[index + 1..].find('>') {
                index += 1 + rel + 1;
                continue;
            }
        }
        if matches!(bytes[index], b'[' | b'(' | b'{' | b'>') {
            if bytes[index] == b'>' && index > 0 && bytes[index - 1] == b'-' {
                index += 1;
                continue;
            }
            return Some(index);
        }
        index += token[index..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

fn parse_flow_dir(token: &str) -> FlowDir {
    match token.trim().to_ascii_uppercase().as_str() {
        "LR" => FlowDir::Lr,
        "RL" => FlowDir::Rl,
        "BT" => FlowDir::Bt,
        _ => FlowDir::Td,
    }
}

/// Split a flowchart line on `;` that is outside brackets and quotes.
fn split_flow_statements(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (idx, ch) in line.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                let stmt = line[start..idx].trim();
                if !stmt.is_empty() {
                    out.push(stmt);
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    let rest = line[start..].trim().trim_end_matches(';').trim();
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

fn flow_node_size(shape: NodeShape, label: &str) -> (f32, f32) {
    let max_text = 168.0f32;
    let lines = label_lines(label, max_text, 13.0);
    let tw = lines.iter().map(|line| estimate_text_width(line, 13.0)).fold(0.0f32, f32::max);
    let text_h = lines.len() as f32 * 16.0;
    match shape {
        NodeShape::Diamond | NodeShape::Hexagon => ((tw + 56.0).max(96.0), (text_h + 28.0).max(54.0)),
        NodeShape::Circle => {
            let d = (tw + 28.0).max(text_h + 24.0).max(54.0);
            (d, d)
        }
        NodeShape::Asymmetric => ((tw + 48.0).max(96.0), (text_h + 20.0).max(40.0)),
        _ => ((tw + 36.0).max(84.0), (text_h + 18.0).max(40.0)),
    }
}

fn layout_flowchart(
    nodes: &[FlowNode],
    edges: &[FlowEdge],
    node_sizes: &[(f32, f32)],
    subgraphs: &[SubgraphInfo],
    dir: FlowDir,
) -> (Vec<(f32, f32)>, u32, u32) {
    let horizontal = matches!(dir, FlowDir::Lr | FlowDir::Rl);
    let reversed = matches!(dir, FlowDir::Bt | FlowDir::Rl);
    let margin = if subgraphs.is_empty() { 32.0f32 } else { 44.0f32 };
    let directed: Vec<usize> = subgraphs.iter().enumerate().filter_map(|(idx, sg)| sg.dir.map(|_| idx)).collect();
    if directed.is_empty() {
        let edge_pairs: Vec<(usize, usize)> = edges.iter().map(|edge| (edge.from, edge.to)).collect();
        return compute_layered_layout(nodes.len(), &edge_pairs, node_sizes, horizontal, reversed, margin, 64.0, 34.0);
    }

    struct LocalLayout {
        members: Vec<usize>,
        centers: Vec<(f32, f32)>,
        w: f32,
        h: f32,
    }
    let mut locals: HashMap<usize, LocalLayout> = HashMap::new();
    for &sg in &directed {
        let members: Vec<usize> = nodes.iter().enumerate().filter_map(|(idx, node)| (node.subgraph == Some(sg)).then_some(idx)).collect();
        if members.is_empty() {
            continue;
        }
        let index_of: HashMap<usize, usize> = members.iter().enumerate().map(|(idx, &node)| (node, idx)).collect();
        let local_edges: Vec<(usize, usize)> = edges
            .iter()
            .filter_map(|edge| Some((*index_of.get(&edge.from)?, *index_of.get(&edge.to)?)))
            .collect();
        let sizes: Vec<(f32, f32)> = members.iter().map(|&node| node_sizes[node]).collect();
        let sg_dir = subgraphs[sg].dir.unwrap_or(dir);
        let (local_centers, w, h) = compute_layered_layout(
            members.len(),
            &local_edges,
            &sizes,
            matches!(sg_dir, FlowDir::Lr | FlowDir::Rl),
            matches!(sg_dir, FlowDir::Bt | FlowDir::Rl),
            28.0,
            56.0,
            28.0,
        );
        locals.insert(sg, LocalLayout { members, centers: local_centers, w: w as f32, h: h as f32 });
    }

    enum Slot {
        Node(usize),
        Group(usize),
    }
    let mut slots: Vec<Slot> = Vec::new();
    let mut slot_of_node: HashMap<usize, usize> = HashMap::new();
    let mut slot_of_group: HashMap<usize, usize> = HashMap::new();
    for (idx, node) in nodes.iter().enumerate() {
        if let Some(sg) = node.subgraph
            && locals.contains_key(&sg)
        {
            if let Some(&slot) = slot_of_group.get(&sg) {
                slot_of_node.insert(idx, slot);
            } else {
                let slot = slots.len();
                slots.push(Slot::Group(sg));
                slot_of_group.insert(sg, slot);
                slot_of_node.insert(idx, slot);
            }
        } else {
            let slot = slots.len();
            slots.push(Slot::Node(idx));
            slot_of_node.insert(idx, slot);
        }
    }
    let mut outer_edges = Vec::new();
    for edge in edges {
        if let (Some(&from), Some(&to)) = (slot_of_node.get(&edge.from), slot_of_node.get(&edge.to))
            && from != to
        {
            outer_edges.push((from, to));
        }
    }
    let outer_sizes: Vec<(f32, f32)> = slots
        .iter()
        .map(|slot| match slot {
            Slot::Node(idx) => node_sizes[*idx],
            Slot::Group(sg) => {
                let local = &locals[sg];
                (local.w, local.h)
            }
        })
        .collect();
    let (outer_centers, total_w, total_h) = compute_layered_layout(slots.len(), &outer_edges, &outer_sizes, horizontal, reversed, margin, 64.0, 34.0);
    let mut centers = vec![(0.0f32, 0.0f32); nodes.len()];
    for (idx, slot) in slots.iter().enumerate() {
        match slot {
            Slot::Node(node) => centers[*node] = outer_centers[idx],
            Slot::Group(sg) => {
                let local = &locals[sg];
                let (sx, sy) = outer_centers[idx];
                let ox = sx - local.w / 2.0;
                let oy = sy - local.h / 2.0;
                for (member_idx, &node) in local.members.iter().enumerate() {
                    let (lx, ly) = local.centers[member_idx];
                    centers[node] = (ox + lx, oy + ly);
                }
            }
        }
    }
    (centers, total_w, total_h)
}

fn push_flow_polygon(svg: &mut String, points: &[(f32, f32)], fill: &str, stroke: &str, shape: &str) {
    let body: String = points.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect::<Vec<_>>().join(" ");
    svg.push_str(&format!(
        r##"<polygon points="{body}" fill="{fill}" stroke="{stroke}" stroke-width="1.4" data-shape="{shape}"/>"##
    ));
}

fn push_flow_cylinder(svg: &mut String, x: f32, y: f32, w: f32, h: f32) {
    let ry = (h * 0.18).clamp(6.0, 14.0);
    let body_y = y + ry;
    let body_h = (h - ry * 2.0).max(8.0);
    svg.push_str(&format!(
        r##"<rect x="{x:.1}" y="{body_y:.1}" width="{w:.1}" height="{body_h:.1}" fill="#ecfeff" stroke="#0891b2" stroke-width="1.4" data-shape="cylinder"/>"##
    ));
    svg.push_str(&format!(
        r##"<ellipse cx="{:.1}" cy="{:.1}" rx="{:.1}" ry="{ry:.1}" fill="#ecfeff" stroke="#0891b2" stroke-width="1.4"/>"##,
        x + w / 2.0,
        y + h - ry,
        w / 2.0
    ));
    svg.push_str(&format!(
        r##"<ellipse cx="{:.1}" cy="{:.1}" rx="{:.1}" ry="{ry:.1}" fill="#cffafe" stroke="#0891b2" stroke-width="1.4"/>"##,
        x + w / 2.0,
        y + ry,
        w / 2.0
    ));
}

fn hexagon_points(x: f32, y: f32, w: f32, h: f32) -> Vec<(f32, f32)> {
    let inset = w * 0.18;
    vec![(x + inset, y), (x + w - inset, y), (x + w, y + h / 2.0), (x + w - inset, y + h), (x + inset, y + h), (x, y + h / 2.0)]
}

fn trapezoid_points(x: f32, y: f32, w: f32, h: f32, narrow_top: bool) -> Vec<(f32, f32)> {
    let inset = w * 0.16;
    if narrow_top {
        vec![(x + inset, y), (x + w - inset, y), (x + w, y + h), (x, y + h)]
    } else {
        vec![(x, y), (x + w, y), (x + w - inset, y + h), (x + inset, y + h)]
    }
}

fn parallelogram_points(x: f32, y: f32, w: f32, h: f32) -> Vec<(f32, f32)> {
    let skew = w * 0.16;
    vec![(x + skew, y), (x + w, y), (x + w - skew, y + h), (x, y + h)]
}

fn asymmetric_points(x: f32, y: f32, w: f32, h: f32) -> Vec<(f32, f32)> {
    let inset = w * 0.16;
    vec![(x, y), (x + w - inset, y), (x + w, y + h / 2.0), (x + w - inset, y + h), (x, y + h)]
}

// ============================================================================
// Shared Layered Layout (used by flowchart, stateDiagram, classDiagram, erDiagram)
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn compute_layered_layout(
    n: usize,
    edges: &[(usize, usize)],
    node_sizes: &[(f32, f32)],
    horizontal: bool,
    reversed: bool,
    margin: f32,
    layer_gap: f32,
    sibling_gap: f32,
) -> (Vec<(f32, f32)>, u32, u32) {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(from, to) in edges {
        if from != to {
            adj[from].push(to);
        }
    }
    let mut in_deg = vec![0usize; n];
    for list in &adj {
        for &v in list {
            in_deg[v] += 1;
        }
    }
    let mut layer = vec![0usize; n];
    let mut state = vec![0u8; n];
    let mut topo = Vec::with_capacity(n);
    fn dfs(u: usize, adj: &[Vec<usize>], state: &mut [u8], topo: &mut Vec<usize>) {
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

    let mut rev_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(from, to) in edges {
        if from != to {
            rev_adj[to].push(from);
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
                bary(a)
                    .partial_cmp(&bary(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    let mut layer_primary_sizes: Vec<f32> = Vec::with_capacity(layers.len());
    let mut layer_cross_sizes: Vec<f32> = Vec::with_capacity(layers.len());
    for l_nodes in &layers {
        let mut p_max = 32.0f32;
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
        (
            (total_primary + margin * 2.0).ceil() as u32,
            (max_cross + margin * 2.0).ceil() as u32,
        )
    } else {
        (
            (max_cross + margin * 2.0).ceil() as u32,
            (total_primary + margin * 2.0).ceil() as u32,
        )
    };
    (centers, total_w.max(120), total_h.max(80))
}

// ============================================================================
// State Diagram (stateDiagram / stateDiagram-v2)
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum StateKind {
    Start,
    End,
    Normal,
    Choice,
    Fork,
    Join,
}

struct StateNode {
    label: String,
    description: Option<String>,
    kind: StateKind,
    composite: Option<String>,
}

struct StateEdge {
    from: usize,
    to: usize,
    label: Option<String>,
}

fn state_stereotype(rest: &str) -> Option<StateKind> {
    if rest.contains("<<choice>>") {
        Some(StateKind::Choice)
    } else if rest.contains("<<fork>>") {
        Some(StateKind::Fork)
    } else if rest.contains("<<join>>") {
        Some(StateKind::Join)
    } else {
        None
    }
}

/// `[*]` is one start or end node per composite state. The top-level pair stays `__start__` / `__end__`.
fn state_endpoint(comp: Option<&str>, end: bool, nodes: &mut Vec<StateNode>, map: &mut HashMap<String, usize>) -> usize {
    let key = match (comp, end) {
        (Some(name), false) => format!("{name}\0[*]start"),
        (Some(name), true) => format!("{name}\0[*]end"),
        (None, false) => "__start__".to_string(),
        (None, true) => "__end__".to_string(),
    };
    if let Some(&idx) = map.get(&key) {
        return idx;
    }
    let idx = nodes.len();
    nodes.push(StateNode {
        label: String::new(),
        description: None,
        kind: if end { StateKind::End } else { StateKind::Start },
        composite: comp.map(ToString::to_string),
    });
    map.insert(key, idx);
    idx
}

fn render_state_diagram_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut horizontal = false;
    let mut reversed = false;
    let mut nodes: Vec<StateNode> = Vec::new();
    let mut node_map: HashMap<String, usize> = HashMap::new();
    let mut edges: Vec<StateEdge> = Vec::new();
    let mut composite_stack: Vec<String> = Vec::new();

    let ensure_state = |raw_id: &str, nodes: &mut Vec<StateNode>, map: &mut HashMap<String, usize>, comp: Option<&str>| -> usize {
        let id = raw_id.trim();
        if let Some(&idx) = map.get(id) {
            if nodes[idx].composite.is_none() {
                nodes[idx].composite = comp.map(ToString::to_string);
            }
            return idx;
        }
        let idx = nodes.len();
        nodes.push(StateNode {
            label: id.to_string(),
            description: None,
            kind: StateKind::Normal,
            composite: comp.map(ToString::to_string),
        });
        map.insert(id.to_string(), idx);
        idx
    };

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if l == "}" {
            composite_stack.pop();
            continue;
        }
        if let Some(rest) = l.strip_prefix("direction ") {
            match rest.trim().to_ascii_uppercase().as_str() {
                "LR" => {
                    horizontal = true;
                    reversed = false;
                }
                "RL" => {
                    horizontal = true;
                    reversed = true;
                }
                "BT" => {
                    horizontal = false;
                    reversed = true;
                }
                _ => {
                    horizontal = false;
                    reversed = false;
                }
            }
            continue;
        }
        if l.starts_with("note ") || l.starts_with("end note") {
            continue;
        }
        let cur_comp = composite_stack.last().map(String::as_str);
        if let Some(rest) = l.strip_prefix("state ") {
            if let Some(kind) = state_stereotype(rest) {
                let head = rest.split("<<").next().unwrap_or("").trim();
                let id = if let Some((_, alias)) = head.split_once(" as ") {
                    unquote(alias.trim())
                } else {
                    unquote(head.split_whitespace().next().unwrap_or(""))
                };
                if !id.is_empty() {
                    let idx = ensure_state(&id, &mut nodes, &mut node_map, cur_comp);
                    nodes[idx].kind = kind;
                }
                continue;
            }
            if let Some(before_brace) = rest.strip_suffix('{') {
                let name = unquote(before_brace.trim());
                composite_stack.push(name);
                continue;
            }
            if let Some((disp, alias)) = rest.split_once(" as ") {
                let idx = ensure_state(alias.trim(), &mut nodes, &mut node_map, cur_comp);
                nodes[idx].label = unquote(disp.trim());
                continue;
            }
        }
        if let Some((lhs_raw, rhs_all)) = l.split_once("-->") {
            let lhs = lhs_raw.trim();
            let (rhs_raw, label) = if let Some((r, lbl)) = rhs_all.split_once(':') {
                (r.trim(), Some(lbl.trim().to_string()).filter(|s| !s.is_empty()))
            } else {
                (rhs_all.trim(), None)
            };
            let from = if lhs == "[*]" {
                state_endpoint(cur_comp, false, &mut nodes, &mut node_map)
            } else {
                ensure_state(lhs, &mut nodes, &mut node_map, cur_comp)
            };
            let to = if rhs_raw == "[*]" {
                state_endpoint(cur_comp, true, &mut nodes, &mut node_map)
            } else {
                ensure_state(rhs_raw, &mut nodes, &mut node_map, cur_comp)
            };
            edges.push(StateEdge { from, to, label });
            continue;
        }
        if let Some((state_id, desc)) = l.split_once(':') {
            let sid = state_id.trim();
            if !sid.is_empty() && !sid.contains(' ') {
                let idx = ensure_state(sid, &mut nodes, &mut node_map, cur_comp);
                nodes[idx].description = Some(desc.trim().to_string());
            }
        }
    }

    if nodes.is_empty() {
        return None;
    }

    let node_sizes: Vec<(f32, f32)> = nodes
        .iter()
        .map(|node| match node.kind {
            StateKind::Start | StateKind::End => (26.0, 26.0),
            StateKind::Choice => (54.0, 54.0),
            StateKind::Fork | StateKind::Join => {
                if horizontal { (14.0, 48.0) } else { (48.0, 14.0) }
            }
            StateKind::Normal => {
                let title_w = estimate_text_width(&node.label, 13.0);
                let desc_w = node
                    .description
                    .as_deref()
                    .map(|d| estimate_text_width(d, 11.0))
                    .unwrap_or(0.0);
                let w = (title_w.max(desc_w) + 32.0).max(88.0);
                let h = if node.description.is_some() { 54.0 } else { 38.0 };
                (w, h)
            }
        })
        .collect();

    let has_comp = nodes.iter().any(|n| n.composite.is_some());
    let margin = if has_comp { 44.0 } else { 32.0 };
    let edge_pairs: Vec<(usize, usize)> = edges.iter().map(|e| (e.from, e.to)).collect();
    let (centers, total_w, total_h) = compute_layered_layout(
        nodes.len(),
        &edge_pairs,
        &node_sizes,
        horizontal,
        reversed,
        margin,
        62.0,
        36.0,
    );

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(
        r##"<defs>
  <marker id="st-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto">
    <path d="M 0 1 L 10 5 L 0 9 z" fill="#475569"/>
  </marker>
</defs>"##,
    );
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));

    // Composite state containers
    let mut comp_groups: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut comp_order: Vec<&str> = Vec::new();
    for (idx, node) in nodes.iter().enumerate() {
        if let Some(c) = node.composite.as_deref() {
            if !comp_groups.contains_key(c) {
                comp_order.push(c);
            }
            comp_groups.entry(c).or_default().push(idx);
        }
    }
    for comp in comp_order {
        let idxs = &comp_groups[comp];
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for &i in idxs {
            let (cx, cy) = centers[i];
            let (nw, nh) = node_sizes[i];
            min_x = min_x.min(cx - nw / 2.0);
            min_y = min_y.min(cy - nh / 2.0);
            max_x = max_x.max(cx + nw / 2.0);
            max_y = max_y.max(cy + nh / 2.0);
        }
        let sx = min_x - 14.0;
        let sy = min_y - 32.0;
        let sw = (max_x - min_x) + 28.0;
        let sh = (max_y - min_y) + 46.0;
        svg.push_str(&format!(
            r##"<rect x="{sx:.1}" y="{sy:.1}" width="{sw:.1}" height="{sh:.1}" rx="10" fill="#f8fafc" stroke="#94a3b8" stroke-width="1.2"/>"##
        ));
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" font-size="11.5" font-weight="bold" fill="#334155">{}</text>"##,
            sx + 10.0,
            sy + 18.0,
            escape_xml(comp)
        ));
    }

    // Transitions
    for e in &edges {
        let (x1, y1) = centers[e.from];
        let (w1, h1) = node_sizes[e.from];
        if e.from == e.to {
            let lx = x1 + w1 / 2.0;
            let ty = y1 - h1 / 4.0;
            let by = y1 + h1 / 4.0;
            svg.push_str(&format!(
                r##"<path d="M {lx:.1} {ty:.1} C {:.1} {:.1}, {:.1} {:.1}, {lx:.1} {by:.1}" fill="none" stroke="#475569" stroke-width="1.4" marker-end="url(#st-arrow)"/>"##,
                lx + 32.0,
                ty - 14.0,
                lx + 32.0,
                by + 14.0
            ));
            if let Some(lbl) = &e.label {
                svg.push_str(&format!(
                    r##"<text x="{:.1}" y="{:.1}" font-size="11" fill="#334155">{}</text>"##,
                    lx + 36.0,
                    y1 + 4.0,
                    escape_xml(lbl)
                ));
            }
            continue;
        }
        let route = route_layered_edge(
            e.from,
            e.to,
            &centers,
            &node_sizes,
            horizontal,
            false,
            (total_w as f32, total_h as f32),
        );
        push_routed_edge(&mut svg, &route, "#475569", "1.4", r#" marker-end="url(#st-arrow)""#);
        if let Some(lbl) = &e.label {
            let (mx, my) = route.label_at;
            let lw = estimate_text_width(lbl, 11.0) + 10.0;
            svg.push_str(&format!(
                r##"<rect x="{:.1}" y="{:.1}" width="{lw:.1}" height="18" rx="3" fill="#ffffff" stroke="#e2e8f0" stroke-width="0.8"/>"##,
                mx - lw / 2.0,
                my - 9.0
            ));
            svg.push_str(&format!(
                r##"<text x="{mx:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#334155">{}</text>"##,
                my + 4.0,
                escape_xml(lbl)
            ));
        }
    }

    // State nodes
    for (idx, node) in nodes.iter().enumerate() {
        let (cx, cy) = centers[idx];
        let (nw, nh) = node_sizes[idx];
        match node.kind {
            StateKind::Start => {
                svg.push_str(&format!(
                    r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="10" fill="#1e293b"/>"##
                ));
            }
            StateKind::End => {
                svg.push_str(&format!(
                    r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="11" fill="#ffffff" stroke="#1e293b" stroke-width="2"/>"##
                ));
                svg.push_str(&format!(
                    r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="6" fill="#1e293b"/>"##
                ));
            }
            StateKind::Choice => {
                let hw = nw / 2.0;
                let hh = nh / 2.0;
                svg.push_str(&format!(
                    r##"<polygon points="{:.1},{cy:.1} {cx:.1},{:.1} {:.1},{cy:.1} {cx:.1},{:.1}" fill="#fff7ed" stroke="#ea580c" stroke-width="1.4" data-shape="choice"/>"##,
                    cx - hw,
                    cy - hh,
                    cx + hw,
                    cy + hh
                ));
            }
            StateKind::Fork | StateKind::Join => {
                let x = cx - nw / 2.0;
                let y = cy - nh / 2.0;
                let shape = if node.kind == StateKind::Fork { "fork" } else { "join" };
                svg.push_str(&format!(
                    r##"<rect x="{x:.1}" y="{y:.1}" width="{nw:.1}" height="{nh:.1}" fill="#1e293b" data-shape="{shape}"/>"##
                ));
            }
            StateKind::Normal => {
                let x = cx - nw / 2.0;
                let y = cy - nh / 2.0;
                svg.push_str(&format!(
                    r##"<rect x="{x:.1}" y="{y:.1}" width="{nw:.1}" height="{nh:.1}" rx="10" fill="#eff6ff" stroke="#2563eb" stroke-width="1.4"/>"##
                ));
                if let Some(desc) = &node.description {
                    svg.push_str(&format!(
                        r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="12.5" font-weight="bold" fill="#0f172a">{}</text>"##,
                        y + 18.0,
                        escape_xml(&node.label)
                    ));
                    svg.push_str(&format!(
                        r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="#bfdbfe" stroke-width="1"/>"##,
                        x + 6.0,
                        y + 25.0,
                        x + nw - 6.0,
                        y + 25.0
                    ));
                    svg.push_str(&format!(
                        r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#475569">{}</text>"##,
                        y + 42.0,
                        escape_xml(desc)
                    ));
                } else {
                    svg.push_str(&format!(
                        r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="13" fill="#0f172a">{}</text>"##,
                        cy + 4.5,
                        escape_xml(&node.label)
                    ));
                }
            }
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

// ============================================================================
// Class Diagram (classDiagram)
// ============================================================================

struct ClassNode {
    name: String,
    stereotype: Option<String>,
    fields: Vec<String>,
    methods: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClassRelKind {
    Inheritance, // <|-- / --|>
    Composition, // *-- / --*
    Aggregation, // o-- / --o
    Association, // --> / <--
    Dependency,  // ..> / <..
    Realization, // ..|> / <|..
    Link,        // -- / ..
}

struct ClassEdge {
    from: usize,
    to: usize,
    kind: ClassRelKind,
    dashed: bool,
    label: Option<String>,
}

fn render_class_diagram_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut horizontal = false;
    let mut reversed = false;
    let mut classes: Vec<ClassNode> = Vec::new();
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut edges: Vec<ClassEdge> = Vec::new();
    let mut current_class: Option<usize> = None;

    let mut ensure_class = |name: &str, classes: &mut Vec<ClassNode>, map: &mut HashMap<String, usize>| -> usize {
        let key = name.trim();
        if let Some(&idx) = map.get(key) {
            return idx;
        }
        let idx = classes.len();
        classes.push(ClassNode {
            name: key.to_string(),
            stereotype: None,
            fields: Vec::new(),
            methods: Vec::new(),
        });
        map.insert(key.to_string(), idx);
        idx
    };

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if let Some(cidx) = current_class {
            if l == "}" {
                current_class = None;
                continue;
            }
            if l.starts_with("<<") && l.ends_with(">>") {
                classes[cidx].stereotype = Some(l.to_string());
            } else if l.contains('(') && l.contains(')') {
                classes[cidx].methods.push(l.to_string());
            } else {
                classes[cidx].fields.push(l.to_string());
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("direction ") {
            match rest.trim().to_ascii_uppercase().as_str() {
                "LR" => {
                    horizontal = true;
                    reversed = false;
                }
                "RL" => {
                    horizontal = true;
                    reversed = true;
                }
                "BT" => {
                    horizontal = false;
                    reversed = true;
                }
                _ => {
                    horizontal = false;
                    reversed = false;
                }
            }
            continue;
        }
        if l.starts_with("<<") {
            if let Some(end) = l.find(">>") {
                let stereo = &l[..=end + 1];
                let cname = l[end + 2..].trim();
                if !cname.is_empty() {
                    let idx = ensure_class(cname, &mut classes, &mut map);
                    classes[idx].stereotype = Some(stereo.to_string());
                    continue;
                }
            }
        }
        if let Some(rest) = l.strip_prefix("class ") {
            if let Some(before_brace) = rest.strip_suffix('{') {
                let cname = before_brace.trim();
                let idx = ensure_class(cname, &mut classes, &mut map);
                current_class = Some(idx);
                continue;
            }
            let cname = rest.trim();
            if !cname.is_empty() {
                ensure_class(cname, &mut classes, &mut map);
                continue;
            }
        }
        if let Some((lhs_part, label_opt)) = split_class_relation(l, &mut classes, &mut map, &mut edges, &mut ensure_class) {
            let _ = (lhs_part, label_opt);
            continue;
        }
        if let Some((cname, member)) = l.split_once(':') {
            let cn = cname.trim();
            let mb = member.trim();
            if !cn.is_empty() && !cn.contains(' ') && !mb.is_empty() {
                let idx = ensure_class(cn, &mut classes, &mut map);
                if mb.contains('(') && mb.contains(')') {
                    classes[idx].methods.push(mb.to_string());
                } else {
                    classes[idx].fields.push(mb.to_string());
                }
            }
        }
    }

    if classes.is_empty() {
        return None;
    }

    let node_sizes: Vec<(f32, f32)> = classes
        .iter()
        .map(|c| {
            let mut max_w = estimate_text_width(&c.name, 13.0) + 28.0;
            if let Some(st) = &c.stereotype {
                max_w = max_w.max(estimate_text_width(st, 10.5) + 24.0);
            }
            for f in &c.fields {
                max_w = max_w.max(estimate_text_width(f, 11.5) + 24.0);
            }
            for m in &c.methods {
                max_w = max_w.max(estimate_text_width(m, 11.5) + 24.0);
            }
            let header_h = if c.stereotype.is_some() { 40.0 } else { 28.0 };
            let fields_h = if c.fields.is_empty() {
                0.0
            } else {
                c.fields.len() as f32 * 18.0 + 10.0
            };
            let methods_h = if c.methods.is_empty() {
                0.0
            } else {
                c.methods.len() as f32 * 18.0 + 10.0
            };
            (max_w.max(110.0), (header_h + fields_h + methods_h).max(36.0))
        })
        .collect();

    let edge_pairs: Vec<(usize, usize)> = edges.iter().map(|e| (e.from, e.to)).collect();
    let (centers, total_w, total_h) = compute_layered_layout(
        classes.len(),
        &edge_pairs,
        &node_sizes,
        horizontal,
        reversed,
        34.0,
        70.0,
        40.0,
    );

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(
        r##"<defs>
  <marker id="cls-inherit" viewBox="0 0 12 12" refX="11" refY="6" markerWidth="10" markerHeight="10" orient="auto">
    <path d="M 1 1 L 11 6 L 1 11 z" fill="#ffffff" stroke="#334155" stroke-width="1.4"/>
  </marker>
  <marker id="cls-comp" viewBox="0 0 14 10" refX="13" refY="5" markerWidth="11" markerHeight="9" orient="auto">
    <path d="M 1 5 L 7 1 L 13 5 L 7 9 z" fill="#334155"/>
  </marker>
  <marker id="cls-agg" viewBox="0 0 14 10" refX="13" refY="5" markerWidth="11" markerHeight="9" orient="auto">
    <path d="M 1 5 L 7 1 L 13 5 L 7 9 z" fill="#ffffff" stroke="#334155" stroke-width="1.3"/>
  </marker>
  <marker id="cls-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto">
    <path d="M 1 1 L 9 5 L 1 9" fill="none" stroke="#334155" stroke-width="1.5"/>
  </marker>
</defs>"##,
    );
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));

    // Relationships
    for e in &edges {
        let route = route_layered_edge(
            e.from,
            e.to,
            &centers,
            &node_sizes,
            horizontal,
            false,
            (total_w as f32, total_h as f32),
        );
        let dash = if e.dashed { r#" stroke-dasharray="5,4""# } else { "" };
        let marker = match e.kind {
            ClassRelKind::Inheritance | ClassRelKind::Realization => r#" marker-end="url(#cls-inherit)""#,
            ClassRelKind::Composition => r#" marker-end="url(#cls-comp)""#,
            ClassRelKind::Aggregation => r#" marker-end="url(#cls-agg)""#,
            ClassRelKind::Association | ClassRelKind::Dependency => r#" marker-end="url(#cls-arrow)""#,
            ClassRelKind::Link => "",
        };
        let extra = format!("{dash}{marker}");
        push_routed_edge(&mut svg, &route, "#334155", "1.4", &extra);
        if let Some(lbl) = &e.label {
            let (mx, my) = route.label_at;
            let lw = estimate_text_width(lbl, 11.0) + 10.0;
            svg.push_str(&format!(
                r##"<rect x="{:.1}" y="{:.1}" width="{lw:.1}" height="18" rx="3" fill="#ffffff" stroke="#e2e8f0" stroke-width="0.8"/>"##,
                mx - lw / 2.0,
                my - 9.0
            ));
            svg.push_str(&format!(
                r##"<text x="{mx:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#334155">{}</text>"##,
                my + 4.0,
                escape_xml(lbl)
            ));
        }
    }

    // Class boxes
    for (idx, c) in classes.iter().enumerate() {
        let (cx, cy) = centers[idx];
        let (nw, nh) = node_sizes[idx];
        let x = cx - nw / 2.0;
        let y = cy - nh / 2.0;
        let header_h = if c.stereotype.is_some() { 40.0 } else { 28.0 };
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{y:.1}" width="{nw:.1}" height="{nh:.1}" rx="4" fill="#ffffff" stroke="#334155" stroke-width="1.4"/>"##
        ));
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{y:.1}" width="{nw:.1}" height="{header_h:.1}" rx="4" fill="#f1f5f9" stroke="#334155" stroke-width="1.4"/>"##
        ));
        if let Some(st) = &c.stereotype {
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="10.5" fill="#475569">{}</text>"##,
                y + 15.0,
                escape_xml(st)
            ));
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="13" font-weight="bold" fill="#0f172a">{}</text>"##,
                y + 32.0,
                escape_xml(&c.name)
            ));
        } else {
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="13" font-weight="bold" fill="#0f172a">{}</text>"##,
                y + 19.0,
                escape_xml(&c.name)
            ));
        }

        let mut cur_y = y + header_h;
        if !c.fields.is_empty() {
            for (i, f) in c.fields.iter().enumerate() {
                svg.push_str(&format!(
                    r##"<text x="{:.1}" y="{:.1}" font-size="11.5" fill="#1e293b">{}</text>"##,
                    x + 10.0,
                    cur_y + 16.0 + i as f32 * 18.0,
                    escape_xml(f)
                ));
            }
            cur_y += c.fields.len() as f32 * 18.0 + 10.0;
        }
        if !c.methods.is_empty() {
            if !c.fields.is_empty() {
                svg.push_str(&format!(
                    r##"<line x1="{x:.1}" y1="{cur_y:.1}" x2="{:.1}" y2="{cur_y:.1}" stroke="#cbd5e1" stroke-width="1"/>"##,
                    x + nw
                ));
            }
            for (i, m) in c.methods.iter().enumerate() {
                svg.push_str(&format!(
                    r##"<text x="{:.1}" y="{:.1}" font-size="11.5" fill="#1e293b">{}</text>"##,
                    x + 10.0,
                    cur_y + 16.0 + i as f32 * 18.0,
                    escape_xml(m)
                ));
            }
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

fn split_class_relation<'a>(
    line: &'a str,
    classes: &mut Vec<ClassNode>,
    map: &mut HashMap<String, usize>,
    edges: &mut Vec<ClassEdge>,
    ensure_class: &mut impl FnMut(&str, &mut Vec<ClassNode>, &mut HashMap<String, usize>) -> usize,
) -> Option<(&'a str, Option<String>)> {
    // Operators ordered longest-first so `<|..` matches before `..`.
    // Tuple: (token, target_is_lhs, kind, dashed)
    const OPS: &[(&str, bool, ClassRelKind, bool)] = &[
        ("<|--", true, ClassRelKind::Inheritance, false),
        ("--|>", false, ClassRelKind::Inheritance, false),
        ("<|..", true, ClassRelKind::Realization, true),
        ("..|>", false, ClassRelKind::Realization, true),
        ("*--", true, ClassRelKind::Composition, false),
        ("--*", false, ClassRelKind::Composition, false),
        ("o--", true, ClassRelKind::Aggregation, false),
        ("--o", false, ClassRelKind::Aggregation, false),
        ("<--", true, ClassRelKind::Association, false),
        ("-->", false, ClassRelKind::Association, false),
        ("<..", true, ClassRelKind::Dependency, true),
        ("..>", false, ClassRelKind::Dependency, true),
        ("--", false, ClassRelKind::Link, false),
        ("..", false, ClassRelKind::Link, true),
    ];

    let (body, label) = if let Some((b, lbl)) = line.split_once(':') {
        (b.trim(), Some(lbl.trim().to_string()).filter(|s| !s.is_empty()))
    } else {
        (line.trim(), None)
    };

    for &(op, target_is_lhs, kind, dashed) in OPS {
        if let Some(pos) = body.find(op) {
            let lhs_raw = body[..pos].trim();
            let rhs_raw = body[pos + op.len()..].trim();
            // Strip optional quoted cardinalities (`Foo "1" --> "*" Bar`)
            let lhs_name = lhs_raw.split_whitespace().next().unwrap_or("");
            let rhs_name = rhs_raw.split_whitespace().last().unwrap_or("");
            if lhs_name.is_empty() || rhs_name.is_empty() {
                return None;
            }
            let lhs_idx = ensure_class(lhs_name, classes, map);
            let rhs_idx = ensure_class(rhs_name, classes, map);
            let (from, to) = if target_is_lhs {
                (rhs_idx, lhs_idx)
            } else {
                (lhs_idx, rhs_idx)
            };
            edges.push(ClassEdge {
                from,
                to,
                kind,
                dashed,
                label: label.clone(),
            });
            return Some((lhs_name, label));
        }
    }
    None
}

// ============================================================================
// ER Diagram (erDiagram)
// ============================================================================

struct ErAttr {
    ty: String,
    name: String,
    key: String,
}

struct ErEntity {
    name: String,
    attrs: Vec<ErAttr>,
}

struct ErEdge {
    from: usize,
    to: usize,
    left_card: String,
    right_card: String,
    dashed: bool,
    label: String,
}

fn render_er_diagram_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut entities: Vec<ErEntity> = Vec::new();
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut edges: Vec<ErEdge> = Vec::new();
    let mut current_entity: Option<usize> = None;

    let ensure_entity = |name: &str, entities: &mut Vec<ErEntity>, map: &mut HashMap<String, usize>| -> usize {
        let key = unquote(name.trim());
        if let Some(&idx) = map.get(&key) {
            return idx;
        }
        let idx = entities.len();
        entities.push(ErEntity {
            name: key.clone(),
            attrs: Vec::new(),
        });
        map.insert(key, idx);
        idx
    };

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if let Some(eidx) = current_entity {
            if l == "}" {
                current_entity = None;
                continue;
            }
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 2 {
                let ty = parts[0].to_string();
                let name = parts[1].to_string();
                let key = parts
                    .get(2)
                    .filter(|s| !s.starts_with('"'))
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                entities[eidx].attrs.push(ErAttr { ty, name, key });
            }
            continue;
        }
        if let Some(before_brace) = l.strip_suffix('{') {
            let ename = before_brace.trim();
            if !ename.is_empty() && !ename.contains("--") && !ename.contains("..") {
                let idx = ensure_entity(ename, &mut entities, &mut map);
                current_entity = Some(idx);
                continue;
            }
        }
        if let Some((rel_part, label_raw)) = l.split_once(':') {
            if let Some((lhs, left_card, dashed, right_card, rhs)) = parse_er_relation(rel_part.trim()) {
                let from = ensure_entity(lhs, &mut entities, &mut map);
                let to = ensure_entity(rhs, &mut entities, &mut map);
                edges.push(ErEdge {
                    from,
                    to,
                    left_card: format_er_card(left_card),
                    right_card: format_er_card(right_card),
                    dashed,
                    label: unquote(label_raw.trim()),
                });
            }
        }
    }

    if entities.is_empty() {
        return None;
    }

    let node_sizes: Vec<(f32, f32)> = entities
        .iter()
        .map(|e| {
            let mut max_w = estimate_text_width(&e.name, 13.0) + 32.0;
            for a in &e.attrs {
                let row = if a.key.is_empty() {
                    format!("{}  {}", a.ty, a.name)
                } else {
                    format!("{}  {}  {}", a.ty, a.name, a.key)
                };
                max_w = max_w.max(estimate_text_width(&row, 11.5) + 28.0);
            }
            let h = 30.0
                + if e.attrs.is_empty() {
                    8.0
                } else {
                    e.attrs.len() as f32 * 20.0 + 10.0
                };
            (max_w.max(120.0), h)
        })
        .collect();

    let edge_pairs: Vec<(usize, usize)> = edges.iter().map(|e| (e.from, e.to)).collect();
    let (centers, total_w, total_h) = compute_layered_layout(
        entities.len(),
        &edge_pairs,
        &node_sizes,
        true,
        false,
        36.0,
        84.0,
        40.0,
    );

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));

    // Edges
    for e in &edges {
        let route = route_layered_edge(
            e.from,
            e.to,
            &centers,
            &node_sizes,
            true,
            false,
            (total_w as f32, total_h as f32),
        );
        let dash = if e.dashed { r#" stroke-dasharray="5,4""# } else { "" };
        push_routed_edge(&mut svg, &route, "#475569", "1.5", dash);
        let (sx, sy) = route.start;
        let (ex, ey) = route.end;
        let (ux, uy) = route.leave_dir;
        let (vx, vy) = route.arrive_dir;
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="10.5" font-weight="bold" fill="#0f766e">{}</text>"##,
            sx + ux * 16.0,
            sy + uy * 16.0 - 5.0,
            escape_xml(&e.left_card)
        ));
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="10.5" font-weight="bold" fill="#0f766e">{}</text>"##,
            ex - vx * 16.0,
            ey - vy * 16.0 - 5.0,
            escape_xml(&e.right_card)
        ));
        if !e.label.is_empty() {
            let (mx, my) = route.label_at;
            let lw = estimate_text_width(&e.label, 11.0) + 10.0;
            svg.push_str(&format!(
                r##"<rect x="{:.1}" y="{:.1}" width="{lw:.1}" height="18" rx="3" fill="#ffffff" stroke="#e2e8f0" stroke-width="0.8"/>"##,
                mx - lw / 2.0,
                my - 9.0
            ));
            svg.push_str(&format!(
                r##"<text x="{mx:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#334155">{}</text>"##,
                my + 4.0,
                escape_xml(&e.label)
            ));
        }
    }

    // Entity boxes
    for (idx, ent) in entities.iter().enumerate() {
        let (cx, cy) = centers[idx];
        let (nw, nh) = node_sizes[idx];
        let x = cx - nw / 2.0;
        let y = cy - nh / 2.0;
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{y:.1}" width="{nw:.1}" height="{nh:.1}" rx="4" fill="#ffffff" stroke="#0f766e" stroke-width="1.4"/>"##
        ));
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{y:.1}" width="{nw:.1}" height="28" rx="4" fill="#f0fdfa" stroke="#0f766e" stroke-width="1.4"/>"##
        ));
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="13" font-weight="bold" fill="#134e4a">{}</text>"##,
            y + 19.0,
            escape_xml(&ent.name)
        ));
        for (i, a) in ent.attrs.iter().enumerate() {
            let ay = y + 46.0 + i as f32 * 20.0;
            let row = if a.key.is_empty() {
                format!("{}  {}", a.ty, a.name)
            } else {
                format!("{}  {}  [{}]", a.ty, a.name, a.key)
            };
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="{ay:.1}" font-size="11.5" fill="#1e293b">{}</text>"##,
                x + 10.0,
                escape_xml(&row)
            ));
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

fn parse_er_relation(s: &str) -> Option<(&str, &str, bool, &str, &str)> {
    let (sep, dashed) = if let Some(idx) = s.find("--") {
        (idx, false)
    } else if let Some(idx) = s.find("..") {
        (idx, true)
    } else {
        return None;
    };
    let lhs_all = s[..sep].trim();
    let rhs_all = s[sep + 2..].trim();
    let lhs_split = lhs_all.rfind(' ')?;
    let rhs_split = rhs_all.find(' ')?;
    let lhs = lhs_all[..lhs_split].trim();
    let left_card = lhs_all[lhs_split + 1..].trim();
    let right_card = rhs_all[..rhs_split].trim();
    let rhs = rhs_all[rhs_split + 1..].trim();
    if lhs.is_empty() || rhs.is_empty() {
        return None;
    }
    Some((lhs, left_card, dashed, right_card, rhs))
}

fn format_er_card(card: &str) -> String {
    match card {
        "||" => "1".to_string(),
        "o|" | "|o" => "0..1".to_string(),
        "}|" | "|{" => "1..*".to_string(),
        "}o" | "o{" => "0..*".to_string(),
        other => other.to_string(),
    }
}

// ============================================================================
// Gantt Chart (gantt)
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum GanttStatus {
    Normal,
    Done,
    Active,
    Crit,
    Milestone,
}

struct GanttTask {
    section: String,
    name: String,
    status: GanttStatus,
    start_day: i64,
    end_day: i64,
}

fn render_gantt_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let (title, tasks, exclude_weekends) = parse_gantt_chart(source)?;

    let min_day = tasks.iter().map(|t| t.start_day).min().unwrap_or(0);
    let max_day = tasks.iter().map(|t| t.end_day).max().unwrap_or(min_day + 1).max(min_day + 1);
    let total_days = (max_day - min_day) as f32;

    let label_col_w = tasks
        .iter()
        .map(|t| estimate_text_width(&t.name, 12.0))
        .fold(120.0f32, f32::max)
        .min(240.0)
        + 24.0;
    let chart_w = 480.0f32;
    let right_pad = 24.0f32;
    let top_pad = if title.is_some() { 52.0f32 } else { 28.0f32 };
    let row_h = 30.0f32;
    let axis_h = 28.0f32;
    let total_w = (label_col_w + chart_w + right_pad).ceil() as u32;
    let total_h = (top_pad + tasks.len() as f32 * row_h + axis_h + 12.0).ceil() as u32;

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));
    if let Some(t) = &title {
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="28" text-anchor="middle" font-size="15" font-weight="bold" fill="#0f172a">{}</text>"##,
            total_w as f32 / 2.0,
            escape_xml(t)
        ));
    }

    // Vertical grid lines & date ticks (up to 6 ticks)
    let ticks = 5usize;
    let bottom_y = top_pad + tasks.len() as f32 * row_h;
    for i in 0..=ticks {
        let frac = i as f32 / ticks as f32;
        let gx = label_col_w + frac * chart_w;
        svg.push_str(&format!(
            r##"<line x1="{gx:.1}" y1="{top_pad:.1}" x2="{gx:.1}" y2="{bottom_y:.1}" stroke="#e2e8f0" stroke-width="1"/>"##
        ));
        let day = min_day + ((total_days * frac).round() as i64);
        let (y, m, d) = civil_from_days(day);
        svg.push_str(&format!(
            r##"<text x="{gx:.1}" y="{:.1}" text-anchor="middle" font-size="10.5" fill="#64748b">{y:04}-{m:02}-{d:02}</text>"##,
            bottom_y + 18.0
        ));
    }

    // Rows
    let mut prev_section: Option<&str> = None;
    for (idx, task) in tasks.iter().enumerate() {
        let ry = top_pad + idx as f32 * row_h;
        let bg = if idx % 2 == 0 { "#f8fafc" } else { "#ffffff" };
        svg.push_str(&format!(
            r##"<rect x="8" y="{ry:.1}" width="{:.1}" height="{row_h:.1}" fill="{bg}" fill-opacity="0.7"/>"##,
            total_w as f32 - 16.0
        ));
        let label = if prev_section != Some(task.section.as_str()) {
            prev_section = Some(task.section.as_str());
            format!("[{}] {}", task.section, task.name)
        } else {
            task.name.clone()
        };
        svg.push_str(&format!(
            r##"<text x="14" y="{:.1}" font-size="12" fill="#1e293b">{}</text>"##,
            ry + 19.0,
            escape_xml(&label)
        ));
    }

    // `excludes weekends` shades the axis. It does not change a bar's calendar length.
    if exclude_weekends && total_days > 0.0 {
        let mut day = min_day;
        while day < max_day {
            let weekday = (day + 4).rem_euclid(7);
            if weekday == 0 || weekday == 6 {
                let x = label_col_w + ((day - min_day) as f32 / total_days) * chart_w;
                let w = chart_w / total_days;
                let band_h = bottom_y - top_pad;
                svg.push_str(&format!(
                    r##"<rect x="{x:.1}" y="{top_pad:.1}" width="{w:.1}" height="{band_h:.1}" fill="#e2e8f0" fill-opacity="0.9" data-nonworking="weekend"/>"##
                ));
            }
            day += 1;
        }
    }

    for (idx, task) in tasks.iter().enumerate() {
        let ry = top_pad + idx as f32 * row_h;
        let x1 = label_col_w + ((task.start_day - min_day) as f32 / total_days) * chart_w;
        let x2 = label_col_w + ((task.end_day - min_day) as f32 / total_days) * chart_w;
        let bw = (x2 - x1).max(6.0);
        let start_iso = gantt_iso(task.start_day);
        let (fill, stroke) = match task.status {
            GanttStatus::Done => ("#cbd5e1", "#64748b"),
            GanttStatus::Active => ("#93c5fd", "#2563eb"),
            GanttStatus::Crit => ("#fca5a5", "#dc2626"),
            GanttStatus::Milestone => ("#fde047", "#ca8a04"),
            GanttStatus::Normal => ("#a5b4fc", "#4f46e5"),
        };
        if task.status == GanttStatus::Milestone {
            let cy = ry + row_h / 2.0;
            svg.push_str(&format!(
                r##"<polygon points="{x1:.1},{:.1} {:.1},{cy:.1} {x1:.1},{:.1} {:.1},{cy:.1}" fill="{fill}" stroke="{stroke}" stroke-width="1.3" data-start="{start_iso}"/>"##,
                cy - 8.0,
                x1 + 8.0,
                cy + 8.0,
                x1 - 8.0
            ));
        } else {
            svg.push_str(&format!(
                r##"<rect x="{x1:.1}" y="{:.1}" width="{bw:.1}" height="18" rx="4" fill="{fill}" stroke="{stroke}" stroke-width="1.2" data-start="{start_iso}"/>"##,
                ry + 6.0
            ));
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

enum GanttAnchor {
    Date(i64),
    After(Vec<String>),
    Follow,
}

struct GanttPending {
    section: String,
    name: String,
    status: GanttStatus,
    id: Option<String>,
    anchor: GanttAnchor,
    span_days: i64,
    end_date: Option<i64>,
}

fn parse_gantt_chart(source: &str) -> Option<(Option<String>, Vec<GanttTask>, bool)> {
    let mut title = None;
    let mut current_section = String::from("Tasks");
    let mut pending = Vec::new();
    let mut date_format = String::new();
    let mut exclude_weekends = false;
    let mut absolute_ends = Vec::new();

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("title ") {
            title = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = l.strip_prefix("dateFormat ") {
            date_format = rest.trim().to_string();
            continue;
        }
        if l.starts_with("axisFormat ") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("excludes ") {
            if rest.split_whitespace().any(|word| word.eq_ignore_ascii_case("weekends")) {
                exclude_weekends = true;
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("section ") {
            current_section = rest.trim().to_string();
            continue;
        }
        let Some((name_part, meta_part)) = l.split_once(':') else {
            continue;
        };
        let task_name = name_part.trim().to_string();
        if task_name.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = meta_part.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if tokens.is_empty() {
            continue;
        }
        let mut status = GanttStatus::Normal;
        let mut idx = 0;
        while idx < tokens.len() {
            match tokens[idx] {
                "done" => {
                    status = GanttStatus::Done;
                    idx += 1;
                }
                "active" => {
                    status = GanttStatus::Active;
                    idx += 1;
                }
                "crit" => {
                    status = GanttStatus::Crit;
                    idx += 1;
                }
                "milestone" => {
                    status = GanttStatus::Milestone;
                    idx += 1;
                }
                _ => break,
            }
        }
        let rem = &tokens[idx..];
        if rem.is_empty() {
            continue;
        }
        let is_date = |tok: &str| parse_gantt_date_formatted(tok, &date_format).is_some();
        let (task_id, start_tok, dur_tok) = match rem.len() {
            1 => (None, None, rem[0]),
            2 => {
                if is_date(rem[0]) || rem[0].starts_with("after ") {
                    (None, Some(rem[0]), rem[1])
                } else {
                    (Some(rem[0].to_string()), None, rem[1])
                }
            }
            _ => (Some(rem[0].to_string()), Some(rem[1]), rem[2]),
        };
        let anchor = if let Some(st) = start_tok {
            if let Some(rest) = st.strip_prefix("after ") {
                let ids: Vec<String> = rest.split_whitespace().filter(|id| !id.is_empty()).map(str::to_string).collect();
                if ids.is_empty() { GanttAnchor::Follow } else { GanttAnchor::After(ids) }
            } else if let Some(day) = parse_gantt_date_formatted(st, &date_format) {
                GanttAnchor::Date(day)
            } else {
                GanttAnchor::Follow
            }
        } else {
            GanttAnchor::Follow
        };
        let end_date = parse_gantt_date_formatted(dur_tok, &date_format);
        let span_days = if end_date.is_some() { 1 } else { parse_gantt_duration_days(dur_tok) };
        if let GanttAnchor::Date(start) = anchor {
            let end = if let Some(ed) = end_date {
                ed.max(start + 1)
            } else if status == GanttStatus::Milestone {
                start + 1
            } else {
                start + span_days.max(1)
            };
            absolute_ends.push(end);
        }
        pending.push(GanttPending {
            section: current_section.clone(),
            name: task_name,
            status,
            id: task_id,
            anchor,
            span_days,
            end_date,
        });
    }

    if pending.is_empty() {
        return None;
    }
    // A chart with no dated task still starts at 2026-01-01. A dated chart
    // never pulls an unresolved `after` back to that origin.
    let origin = days_from_civil(2026, 1, 1);
    let absolute_end = absolute_ends.into_iter().max();
    let mut id_end: HashMap<String, i64> = HashMap::new();
    let mut last_end = None;
    let mut tasks = Vec::new();
    for task in pending {
        let start_day = match &task.anchor {
            GanttAnchor::Date(day) => *day,
            GanttAnchor::Follow => last_end.unwrap_or(origin),
            GanttAnchor::After(ids) => {
                let known: Vec<i64> = ids.iter().filter_map(|id| id_end.get(id).copied()).collect();
                if !known.is_empty() && known.len() == ids.len() {
                    known.into_iter().max().unwrap_or(origin)
                } else if let Some(end) = absolute_end {
                    end
                } else {
                    origin
                }
            }
        };
        let end_day = if let Some(ed) = task.end_date {
            ed.max(start_day + 1)
        } else if task.status == GanttStatus::Milestone {
            start_day + 1
        } else {
            start_day + task.span_days.max(1)
        };
        if let Some(id) = task.id.clone() {
            id_end.insert(id, end_day);
        }
        last_end = Some(end_day);
        tasks.push(GanttTask {
            section: task.section,
            name: task.name,
            status: task.status,
            start_day,
            end_day,
        });
    }
    Some((title, tasks, exclude_weekends))
}

fn gantt_iso(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Read a date with Mermaid `dateFormat` tokens `YYYY`, `MM` and `DD`.
fn parse_gantt_date_formatted(text: &str, format: &str) -> Option<i64> {
    let format = format.trim();
    if format.is_empty() {
        return parse_gantt_date(text);
    }
    let text = text.trim().as_bytes();
    let format = format.as_bytes();
    let mut ti = 0usize;
    let mut fi = 0usize;
    let mut year = None;
    let mut month = None;
    let mut day = None;
    while fi < format.len() {
        if format[fi..].len() >= 4 && format[fi..fi + 4].eq_ignore_ascii_case(b"YYYY") {
            year = Some(take_gantt_digits(text, &mut ti, 4)?);
            fi += 4;
        } else if format[fi..].len() >= 2 && format[fi..fi + 2].eq_ignore_ascii_case(b"MM") {
            month = Some(take_gantt_digits(text, &mut ti, 2)? as u32);
            fi += 2;
        } else if format[fi..].len() >= 2 && format[fi..fi + 2].eq_ignore_ascii_case(b"DD") {
            day = Some(take_gantt_digits(text, &mut ti, 2)? as u32);
            fi += 2;
        } else {
            fi += 1;
            if ti < text.len() && !text[ti].is_ascii_digit() {
                ti += 1;
            }
        }
    }
    let (y, m, d) = (year?, month?, day?);
    if !(1..=12).contains(&m) || d == 0 || d > 31 {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

fn take_gantt_digits(text: &[u8], ti: &mut usize, n: usize) -> Option<i64> {
    if *ti + n > text.len() {
        return None;
    }
    let slice = &text[*ti..*ti + n];
    if slice.iter().any(|b| !b.is_ascii_digit()) {
        return None;
    }
    let value = std::str::from_utf8(slice).ok()?.parse().ok()?;
    *ti += n;
    Some(value)
}

fn parse_gantt_date(s: &str) -> Option<i64> {
    let t = s.trim();
    let sep = if t.contains('-') { '-' } else if t.contains('/') { '/' } else { return None };
    let parts: Vec<&str> = t.split(sep).collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    Some(days_from_civil(y, m, d))
}

fn parse_gantt_duration_days(s: &str) -> i64 {
    let t = s.trim().to_ascii_lowercase();
    if let Some(n) = t.strip_suffix('w').and_then(|v| v.parse::<i64>().ok()) {
        return n * 7;
    }
    if let Some(n) = t.strip_suffix('d').and_then(|v| v.parse::<i64>().ok()) {
        return n;
    }
    if let Some(n) = t.strip_suffix('h').and_then(|v| v.parse::<i64>().ok()) {
        return ((n + 23) / 24).max(1);
    }
    1
}

/// Howard Hinnant's civil-from-days / days-from-civil algorithms (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m as u32, d as u32)
}

// ============================================================================
// Chart SVG Fallback (`pie`, `xychart-beta` when native chart isn't available, e.g. clipboard paste)
// ============================================================================

/// Render a parsed `MermaidChartSpec` (`pie` or `xychart-beta`) to a standalone SVG.
///
/// Used as a fallback when a native Office chart cannot be instantiated (e.g., clipboard fragment paste).
pub fn render_chart_spec_svg(spec: &MermaidChartSpec) -> Option<(Vec<u8>, u32, u32)> {
    const PALETTE: &[&str] = &[
        "#3b82f6", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6", "#ec4899", "#06b6d4", "#84cc16",
    ];
    if spec.series.is_empty() || spec.categories.is_empty() {
        return None;
    }

    if spec.kind == NewChartKind::Pie {
        let vals: Vec<f64> = spec.series[0]
            .1
            .iter()
            .map(|v| v.map(|n| n.0.max(0.0)).unwrap_or(0.0))
            .collect();
        let total: f64 = vals.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let width_px = 460u32;
        let legend_rows = spec.categories.len().max(1) as u32;
        let height_px = (legend_rows * 24 + 80).max(250);
        let cx = 130.0f64;
        let cy = (height_px as f64) / 2.0 + if spec.title.is_some() { 12.0 } else { 0.0 };
        let r = 86.0f64;

        let mut svg = String::new();
        svg.push_str(&format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width_px}" height="{height_px}" viewBox="0 0 {width_px} {height_px}" font-family="sans-serif">"##
        ));
        svg.push_str(&format!(
            r##"<rect width="{width_px}" height="{height_px}" fill="#ffffff"/>"##
        ));
        if let Some(t) = &spec.title {
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="26" text-anchor="middle" font-size="14" font-weight="bold" fill="#0f172a">{}</text>"##,
                width_px as f32 / 2.0,
                escape_xml(t)
            ));
        }

        let mut angle = -std::f64::consts::FRAC_PI_2;
        for (i, &val) in vals.iter().enumerate() {
            let frac = val / total;
            if frac <= 0.0 {
                continue;
            }
            let color = PALETTE[i % PALETTE.len()];
            if frac >= 0.9999 {
                svg.push_str(&format!(
                    r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}" fill="{color}" stroke="#ffffff" stroke-width="1.5"/>"##
                ));
            } else {
                let next_angle = angle + frac * 2.0 * std::f64::consts::PI;
                let x1 = cx + r * angle.cos();
                let y1 = cy + r * angle.sin();
                let x2 = cx + r * next_angle.cos();
                let y2 = cy + r * next_angle.sin();
                let large_arc = if frac > 0.5 { 1 } else { 0 };
                svg.push_str(&format!(
                    r##"<path d="M {cx:.1} {cy:.1} L {x1:.1} {y1:.1} A {r:.1} {r:.1} 0 {large_arc} 1 {x2:.1} {y2:.1} Z" fill="{color}" stroke="#ffffff" stroke-width="1.5"/>"##
                ));
                angle = next_angle;
            }
        }

        // Legend
        let lx = 248.0f32;
        let ly_start = (cy as f32) - (spec.categories.len() as f32 * 22.0) / 2.0;
        for (i, cat) in spec.categories.iter().enumerate() {
            let color = PALETTE[i % PALETTE.len()];
            let ly = ly_start + i as f32 * 22.0;
            let pct = (vals.get(i).copied().unwrap_or(0.0) / total) * 100.0;
            svg.push_str(&format!(
                r##"<rect x="{lx:.1}" y="{:.1}" width="12" height="12" rx="2" fill="{color}"/>"##,
                ly
            ));
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="{:.1}" font-size="11.5" fill="#1e293b">{} ({pct:.1}%)</text>"##,
                lx + 18.0,
                ly + 10.5,
                escape_xml(cat)
            ));
        }

        svg.push_str("</svg>");
        return Some((svg.into_bytes(), width_px, height_px));
    }

    // XY chart (bar / line)
    let mut max_val = 1.0f64;
    for (_, vals) in &spec.series {
        for v in vals.iter().flatten() {
            max_val = max_val.max(v.0);
        }
    }
    // `y-axis min --> max` fixes the value range; otherwise it runs from 0 to the largest value.
    let (y_lo, y_hi) = spec.y_range.unwrap_or((0.0, max_val));
    let y_span = (y_hi - y_lo).max(f64::EPSILON);
    let y_frac = |v: f64| ((v - y_lo) / y_span).clamp(0.0, 1.0) as f32;
    let width_px = 520u32;
    let height_px = if spec.x_axis_title.is_some() { 300u32 } else { 280u32 };
    let pad_l = if spec.y_axis_title.is_some() { 70.0f32 } else { 52.0f32 };
    let pad_r = 24.0f32;
    let pad_t = if spec.title.is_some() { 44.0f32 } else { 24.0f32 };
    let pad_b = if spec.x_axis_title.is_some() { 62.0f32 } else { 42.0f32 };
    let plot_w = width_px as f32 - pad_l - pad_r;
    let plot_h = height_px as f32 - pad_t - pad_b;

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width_px}" height="{height_px}" viewBox="0 0 {width_px} {height_px}" font-family="sans-serif">"##
    ));
    svg.push_str(&format!(
        r##"<rect width="{width_px}" height="{height_px}" fill="#ffffff"/>"##
    ));
    if let Some(t) = &spec.title {
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="24" text-anchor="middle" font-size="14" font-weight="bold" fill="#0f172a">{}</text>"##,
            width_px as f32 / 2.0,
            escape_xml(t)
        ));
    }
    // Grid & axes
    for i in 0..=4 {
        let frac = i as f32 / 4.0;
        let gy = pad_t + plot_h * (1.0 - frac);
        svg.push_str(&format!(
            r##"<line x1="{pad_l:.1}" y1="{gy:.1}" x2="{:.1}" y2="{gy:.1}" stroke="#e2e8f0" stroke-width="1"/>"##,
            pad_l + plot_w
        ));
        let tick_val = y_lo + y_span * frac as f64;
        let tick = if y_span < 8.0 { format!("{tick_val:.1}") } else { format!("{tick_val:.0}") };
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="end" font-size="10" fill="#64748b">{tick}</text>"##,
            pad_l - 6.0,
            gy + 3.5
        ));
    }
    if let Some(t) = &spec.y_axis_title {
        let ty = pad_t + plot_h / 2.0;
        svg.push_str(&format!(
            r##"<text x="16" y="{ty:.1}" text-anchor="middle" font-size="11" fill="#334155" transform="rotate(-90 16 {ty:.1})" data-role="y-axis-title">{}</text>"##,
            escape_xml(t)
        ));
    }
    if let Some(t) = &spec.x_axis_title {
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#334155" data-role="x-axis-title">{}</text>"##,
            pad_l + plot_w / 2.0,
            height_px as f32 - 12.0,
            escape_xml(t)
        ));
    }
    svg.push_str(&format!(
        r##"<line x1="{pad_l:.1}" y1="{pad_t:.1}" x2="{pad_l:.1}" y2="{:.1}" stroke="#64748b" stroke-width="1.2"/>"##,
        pad_t + plot_h
    ));
    svg.push_str(&format!(
        r##"<line x1="{pad_l:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="#64748b" stroke-width="1.2"/>"##,
        pad_t + plot_h,
        pad_l + plot_w,
        pad_t + plot_h
    ));

    let cat_count = spec.categories.len().max(1);
    let slot_w = plot_w / cat_count as f32;
    for (ci, cat) in spec.categories.iter().enumerate() {
        let cx = pad_l + (ci as f32 + 0.5) * slot_w;
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#334155">{}</text>"##,
            pad_t + plot_h + 18.0,
            escape_xml(cat)
        ));
    }

    for (si, (sname, vals)) in spec.series.iter().enumerate() {
        let color = PALETTE[si % PALETTE.len()];
        let is_line = spec.series_kinds.get(si) == Some(&NewChartKind::Line)
            || (spec.series_kinds.is_empty() && (spec.kind == NewChartKind::Line || sname.starts_with("Line")));
        if is_line {
            let mut pts = Vec::new();
            for (ci, v_opt) in vals.iter().enumerate() {
                if let Some(v) = v_opt {
                    let px = pad_l + (ci as f32 + 0.5) * slot_w;
                    let py = pad_t + plot_h * (1.0 - y_frac(v.0));
                    pts.push((px, py));
                }
            }
            if pts.len() >= 2 {
                let poly: String = pts
                    .iter()
                    .map(|(x, y)| format!("{x:.1},{y:.1}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                svg.push_str(&format!(
                    r##"<polyline points="{poly}" fill="none" stroke="{color}" stroke-width="2.2"/>"##
                ));
            }
            for (px, py) in pts {
                svg.push_str(&format!(
                    r##"<circle cx="{px:.1}" cy="{py:.1}" r="3.5" fill="{color}" stroke="#ffffff" stroke-width="1"/>"##
                ));
            }
        } else {
            let bar_w = (slot_w * 0.54).min(40.0);
            for (ci, v_opt) in vals.iter().enumerate() {
                if let Some(v) = v_opt {
                    let bh = (plot_h * y_frac(v.0)).max(2.0);
                    let bx = pad_l + (ci as f32 + 0.5) * slot_w - bar_w / 2.0;
                    let by = pad_t + plot_h - bh;
                    svg.push_str(&format!(
                        r##"<rect x="{bx:.1}" y="{by:.1}" width="{bar_w:.1}" height="{bh:.1}" rx="2" fill="{color}" fill-opacity="0.88"/>"##
                    ));
                }
            }
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), width_px, height_px))
}

// ============================================================================
// Mindmap (mindmap)
// ============================================================================

struct MindNode {
    label: String,
    depth: usize,
    children: Vec<usize>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn render_mindmap_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut nodes: Vec<MindNode> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (indent, node_idx)

    for raw in source.lines().skip(1) {
        if raw.trim().is_empty() || raw.trim_start().starts_with("%%") {
            continue;
        }
        let indent = raw.chars().take_while(|c| c.is_whitespace()).count();
        let label = clean_mindmap_label(raw.trim());
        if label.is_empty() {
            continue;
        }
        while let Some(&(top_indent, _)) = stack.last() {
            if top_indent >= indent {
                stack.pop();
            } else {
                break;
            }
        }
        let depth = stack.len();
        let w = (estimate_text_width(&label, 12.5) + 26.0).max(68.0);
        let h = if depth == 0 { 38.0 } else { 30.0 };
        let idx = nodes.len();
        nodes.push(MindNode {
            label,
            depth,
            children: Vec::new(),
            x: 0.0,
            y: 0.0,
            w,
            h,
        });
        if let Some(&(_, parent_idx)) = stack.last() {
            nodes[parent_idx].children.push(idx);
        }
        stack.push((indent, idx));
    }

    if nodes.is_empty() {
        return None;
    }

    // Assign Y by leaf slots, X by depth.
    fn subtree_leaves(idx: usize, nodes: &[MindNode]) -> usize {
        if nodes[idx].children.is_empty() {
            1
        } else {
            nodes[idx]
                .children
                .iter()
                .map(|&c| subtree_leaves(c, nodes))
                .sum()
        }
    }

    let mut max_depth = 0usize;
    for n in &nodes {
        max_depth = max_depth.max(n.depth);
    }
    let mut depth_widths = vec![80.0f32; max_depth + 1];
    for n in &nodes {
        depth_widths[n.depth] = depth_widths[n.depth].max(n.w);
    }
    let mut depth_x = vec![28.0f32; max_depth + 1];
    for d in 1..=max_depth {
        depth_x[d] = depth_x[d - 1] + depth_widths[d - 1] + 44.0;
    }

    let leaf_h = 44.0f32;
    let margin_y = 24.0f32;
    fn layout_node(
        idx: usize,
        y_start: f32,
        leaf_h: f32,
        depth_x: &[f32],
        nodes: &mut [MindNode],
    ) -> f32 {
        let children = nodes[idx].children.clone();
        nodes[idx].x = depth_x[nodes[idx].depth];
        if children.is_empty() {
            nodes[idx].y = y_start + leaf_h / 2.0;
            return y_start + leaf_h;
        }
        let mut cursor = y_start;
        let mut first_cy = 0.0f32;
        let mut last_cy = 0.0f32;
        for (i, &c) in children.iter().enumerate() {
            cursor = layout_node(c, cursor, leaf_h, depth_x, nodes);
            let cy = nodes[c].y;
            if i == 0 {
                first_cy = cy;
            }
            last_cy = cy;
        }
        nodes[idx].y = (first_cy + last_cy) / 2.0;
        cursor
    }

    let mut total_y = margin_y;
    for i in 0..nodes.len() {
        if nodes[i].depth == 0 {
            let _ = subtree_leaves(i, &nodes);
            total_y = layout_node(i, total_y, leaf_h, &depth_x, &mut nodes);
        }
    }

    let total_w = (depth_x[max_depth] + depth_widths[max_depth] + 28.0).ceil() as u32;
    let total_h = (total_y + margin_y).ceil() as u32;

    const BRANCH_COLORS: &[&str] = &["#2563eb", "#059669", "#d97706", "#7c3aed", "#db2777", "#0891b2"];
    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));

    // Curved edges
    for (p_idx, p) in nodes.iter().enumerate() {
        for (ci, &c_idx) in p.children.iter().enumerate() {
            let c = &nodes[c_idx];
            let sx = p.x + p.w;
            let sy = p.y;
            let ex = c.x;
            let ey = c.y;
            let mx = (sx + ex) / 2.0;
            let color = BRANCH_COLORS[(p_idx + ci) % BRANCH_COLORS.len()];
            svg.push_str(&format!(
                r##"<path d="M {sx:.1} {sy:.1} C {mx:.1} {sy:.1}, {mx:.1} {ey:.1}, {ex:.1} {ey:.1}" fill="none" stroke="{color}" stroke-width="1.8"/>"##
            ));
        }
    }

    // Nodes
    for (idx, n) in nodes.iter().enumerate() {
        let ny = n.y - n.h / 2.0;
        let (fill, stroke, text_fill, rx) = if n.depth == 0 {
            ("#1e293b", "#0f172a", "#ffffff", 18.0)
        } else {
            let c = BRANCH_COLORS[idx % BRANCH_COLORS.len()];
            ("#f8fafc", c, "#0f172a", 8.0)
        };
        svg.push_str(&format!(
            r##"<rect x="{:.1}" y="{ny:.1}" width="{:.1}" height="{:.1}" rx="{rx:.1}" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"##,
            n.x, n.w, n.h
        ));
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="12.5" font-weight="{}" fill="{text_fill}">{}</text>"##,
            n.x + n.w / 2.0,
            n.y + 4.5,
            if n.depth == 0 { "bold" } else { "normal" },
            escape_xml(&n.label)
        ));
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

fn clean_mindmap_label(s: &str) -> String {
    let mut t = s.trim();
    for (open, close) in [
        ("((", "))"),
        ("{{", "}}"),
        ("(", ")"),
        ("[", "]"),
    ] {
        if let Some(pos) = t.find(open) {
            if let Some(end) = t.rfind(close) {
                if end > pos + open.len() {
                    t = t[pos + open.len()..end].trim();
                    break;
                }
            }
        }
    }
    unquote(t)
}

// ============================================================================
// Git Graph (gitGraph)
// ============================================================================

struct GitCommit {
    branch: usize,
    id: String,
    tag: Option<String>,
    parent: Option<usize>,
    merge_parent: Option<usize>,
}

fn render_git_graph_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut branches: Vec<String> = vec!["main".to_string()];
    let mut branch_head: Vec<Option<usize>> = vec![None];
    let mut current_branch = 0usize;
    let mut commits: Vec<GitCommit> = Vec::new();
    let mut auto_idx = 1usize;

    let ensure_branch = |name: &str, branches: &mut Vec<String>, branch_head: &mut Vec<Option<usize>>, head: Option<usize>| -> usize {
        if let Some(pos) = branches.iter().position(|b| b == name) {
            return pos;
        }
        let idx = branches.len();
        branches.push(name.to_string());
        branch_head.push(head);
        idx
    };

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if l.starts_with("commit") {
            let id = extract_git_attr(l, "id:").unwrap_or_else(|| {
                let s = format!("c{auto_idx}");
                auto_idx += 1;
                s
            });
            let tag = extract_git_attr(l, "tag:");
            let parent = branch_head[current_branch];
            let idx = commits.len();
            commits.push(GitCommit {
                branch: current_branch,
                id,
                tag,
                parent,
                merge_parent: None,
            });
            branch_head[current_branch] = Some(idx);
        } else if let Some(rest) = l.strip_prefix("branch ") {
            let bname = unquote(rest.split_whitespace().next().unwrap_or(""));
            if !bname.is_empty() {
                let head = branch_head[current_branch];
                current_branch = ensure_branch(&bname, &mut branches, &mut branch_head, head);
            }
        } else if let Some(rest) = l.strip_prefix("checkout ").or_else(|| l.strip_prefix("switch ")) {
            let bname = unquote(rest.split_whitespace().next().unwrap_or(""));
            if let Some(pos) = branches.iter().position(|b| b == &bname) {
                current_branch = pos;
            }
        } else if let Some(rest) = l.strip_prefix("merge ") {
            let bname = unquote(rest.split_whitespace().next().unwrap_or(""));
            if let Some(src_branch) = branches.iter().position(|b| b == &bname) {
                let parent = branch_head[current_branch];
                let merge_parent = branch_head[src_branch];
                let id = extract_git_attr(l, "id:").unwrap_or_else(|| format!("merge {bname}"));
                let tag = extract_git_attr(l, "tag:");
                let idx = commits.len();
                commits.push(GitCommit {
                    branch: current_branch,
                    id,
                    tag,
                    parent,
                    merge_parent,
                });
                branch_head[current_branch] = Some(idx);
            }
        }
    }

    if commits.is_empty() {
        return None;
    }

    const COLORS: &[&str] = &["#2563eb", "#10b981", "#f59e0b", "#8b5cf6", "#ef4444", "#06b6d4"];
    let label_col = branches
        .iter()
        .map(|b| estimate_text_width(b, 12.0))
        .fold(64.0f32, f32::max)
        + 28.0;
    let step_x = 64.0f32;
    let lane_h = 46.0f32;
    let top_pad = 28.0f32;
    let bottom_pad = 36.0f32;
    let total_w = (label_col + commits.len() as f32 * step_x + 36.0).ceil() as u32;
    let total_h = (top_pad + branches.len() as f32 * lane_h + bottom_pad).ceil() as u32;

    let commit_pos = |idx: usize| -> (f32, f32) {
        let x = label_col + (idx as f32 + 0.5) * step_x;
        let y = top_pad + (commits[idx].branch as f32 + 0.5) * lane_h;
        (x, y)
    };

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));

    // Branch guide lines & labels
    for (bi, bname) in branches.iter().enumerate() {
        let ly = top_pad + (bi as f32 + 0.5) * lane_h;
        let color = COLORS[bi % COLORS.len()];
        svg.push_str(&format!(
            r##"<line x1="{label_col:.1}" y1="{ly:.1}" x2="{:.1}" y2="{ly:.1}" stroke="#e2e8f0" stroke-width="1.5" stroke-dasharray="4,4"/>"##,
            total_w as f32 - 18.0
        ));
        svg.push_str(&format!(
            r##"<text x="14" y="{:.1}" font-size="12" font-weight="bold" fill="{color}">{}</text>"##,
            ly + 4.0,
            escape_xml(bname)
        ));
    }

    // Commit links
    for (idx, c) in commits.iter().enumerate() {
        let (x2, y2) = commit_pos(idx);
        if let Some(p) = c.parent {
            let (x1, y1) = commit_pos(p);
            let color = COLORS[c.branch % COLORS.len()];
            let mx = (x1 + x2) / 2.0;
            svg.push_str(&format!(
                r##"<path d="M {x1:.1} {y1:.1} C {mx:.1} {y1:.1}, {mx:.1} {y2:.1}, {x2:.1} {y2:.1}" fill="none" stroke="{color}" stroke-width="2.2"/>"##
            ));
        }
        if let Some(mp) = c.merge_parent {
            let (x1, y1) = commit_pos(mp);
            let color = COLORS[commits[mp].branch % COLORS.len()];
            let mx = (x1 + x2) / 2.0;
            svg.push_str(&format!(
                r##"<path d="M {x1:.1} {y1:.1} C {mx:.1} {y1:.1}, {mx:.1} {y2:.1}, {x2:.1} {y2:.1}" fill="none" stroke="{color}" stroke-width="2" stroke-dasharray="4,3"/>"##
            ));
        }
    }

    // Commits & tags
    for (idx, c) in commits.iter().enumerate() {
        let (cx, cy) = commit_pos(idx);
        let color = COLORS[c.branch % COLORS.len()];
        svg.push_str(&format!(
            r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="7.5" fill="{color}" stroke="#ffffff" stroke-width="2"/>"##
        ));
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="10.5" fill="#334155">{}</text>"##,
            cy + 20.0,
            escape_xml(&c.id)
        ));
        if let Some(tag) = &c.tag {
            let tw = estimate_text_width(tag, 10.0) + 12.0;
            svg.push_str(&format!(
                r##"<rect x="{:.1}" y="{:.1}" width="{tw:.1}" height="16" rx="4" fill="#fef9c3" stroke="#ca8a04" stroke-width="1"/>"##,
                cx - tw / 2.0,
                cy - 27.0
            ));
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="10" font-weight="bold" fill="#854d0e">{}</text>"##,
                cy - 15.5,
                escape_xml(tag)
            ));
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

fn extract_git_attr(line: &str, key: &str) -> Option<String> {
    let pos = line.find(key)?;
    let rest = line[pos + key.len()..].trim_start();
    if let Some(after_q) = rest.strip_prefix('"') {
        let end = after_q.find('"')?;
        Some(after_q[..end].to_string())
    } else {
        rest.split_whitespace().next().map(ToString::to_string)
    }
}

// ============================================================================
// User Journey (`journey`)
// ============================================================================

struct JourneyStep {
    section: String,
    task: String,
    score: f32, // 1..=5
    actors: Vec<String>,
}

fn render_journey_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut title: Option<String> = None;
    let mut section = String::from("Journey");
    let mut steps: Vec<JourneyStep> = Vec::new();

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("title ") {
            title = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = l.strip_prefix("section ") {
            section = rest.trim().to_string();
            continue;
        }
        let parts: Vec<&str> = l.split(':').map(str::trim).collect();
        if parts.len() >= 2 {
            let task = parts[0].to_string();
            let score = parts[1].parse::<f32>().unwrap_or(3.0).clamp(1.0, 5.0);
            let actors = if parts.len() >= 3 {
                parts[2]
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .collect()
            } else {
                Vec::new()
            };
            steps.push(JourneyStep {
                section: section.clone(),
                task,
                score,
                actors,
            });
        }
    }

    if steps.is_empty() {
        return None;
    }

    let col_w = 112.0f32;
    let pad_l = 32.0f32;
    let pad_r = 32.0f32;
    let top_y = if title.is_some() { 52.0f32 } else { 24.0f32 };
    let chart_h = 150.0f32;
    let total_w = (pad_l + pad_r + steps.len() as f32 * col_w).ceil() as u32;
    let total_h = (top_y + chart_h + 76.0).ceil() as u32;

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));
    if let Some(t) = &title {
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="28" text-anchor="middle" font-size="15" font-weight="bold" fill="#0f172a">{}</text>"##,
            total_w as f32 / 2.0,
            escape_xml(t)
        ));
    }

    // Score guide lines (1..5)
    for s in 1..=5 {
        let gy = top_y + chart_h * (1.0 - (s as f32 - 1.0) / 4.0);
        svg.push_str(&format!(
            r##"<line x1="{pad_l:.1}" y1="{gy:.1}" x2="{:.1}" y2="{gy:.1}" stroke="#e2e8f0" stroke-width="1"/>"##,
            total_w as f32 - pad_r
        ));
    }

    // Polyline connecting scores
    let mut pts: Vec<(f32, f32)> = Vec::with_capacity(steps.len());
    for (i, st) in steps.iter().enumerate() {
        let cx = pad_l + (i as f32 + 0.5) * col_w;
        let cy = top_y + chart_h * (1.0 - (st.score - 1.0) / 4.0);
        pts.push((cx, cy));
    }
    if pts.len() >= 2 {
        let poly: String = pts
            .iter()
            .map(|(x, y)| format!("{x:.1},{y:.1}"))
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(
            r##"<polyline points="{poly}" fill="none" stroke="#3b82f6" stroke-width="2.2"/>"##
        ));
    }

    let mut prev_sec: Option<&str> = None;
    for (i, st) in steps.iter().enumerate() {
        let (cx, cy) = pts[i];
        let color = if st.score >= 4.0 {
            "#10b981"
        } else if st.score >= 3.0 {
            "#f59e0b"
        } else {
            "#ef4444"
        };
        if prev_sec != Some(st.section.as_str()) {
            prev_sec = Some(st.section.as_str());
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="11" font-weight="bold" fill="#475569">{}</text>"##,
                top_y + chart_h + 22.0,
                escape_xml(&st.section)
            ));
        }
        svg.push_str(&format!(
            r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="11" fill="{color}" stroke="#ffffff" stroke-width="2"/>"##
        ));
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="11" font-weight="bold" fill="#ffffff">{:.0}</text>"##,
            cy + 4.0,
            st.score
        ));
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="11.5" fill="#0f172a">{}</text>"##,
            top_y + chart_h + 40.0,
            escape_xml(&st.task)
        ));
        if !st.actors.is_empty() {
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="10" fill="#64748b">{}</text>"##,
                top_y + chart_h + 56.0,
                escape_xml(&st.actors.join(", "))
            ));
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

// ============================================================================
// Timeline (`timeline`)
// ============================================================================

struct TimelineEntry {
    period: String,
    events: Vec<String>,
}

fn render_timeline_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut title: Option<String> = None;
    let mut entries: Vec<TimelineEntry> = Vec::new();

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("%%") || l.starts_with("section ") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("title ") {
            title = Some(rest.trim().to_string());
            continue;
        }
        if let Some((period, rest)) = l.split_once(':') {
            let p = period.trim();
            let evs: Vec<String> = rest
                .split(':')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect();
            if p.is_empty() && !entries.is_empty() {
                if let Some(last) = entries.last_mut() {
                    last.events.extend(evs);
                }
            } else if !p.is_empty() {
                entries.push(TimelineEntry {
                    period: p.to_string(),
                    events: evs,
                });
            }
        }
    }

    if entries.is_empty() {
        return None;
    }

    let col_w = 136.0f32;
    let pad_x = 28.0f32;
    let top_y = if title.is_some() { 54.0f32 } else { 26.0f32 };
    let max_events = entries.iter().map(|e| e.events.len()).max().unwrap_or(1).max(1);
    let total_w = (pad_x * 2.0 + entries.len() as f32 * col_w).ceil() as u32;
    let total_h = (top_y + 68.0 + max_events as f32 * 38.0 + 20.0).ceil() as u32;
    let axis_y = top_y + 34.0;

    const ACCENTS: &[&str] = &["#2563eb", "#059669", "#d97706", "#7c3aed", "#db2777"];
    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));
    if let Some(t) = &title {
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="28" text-anchor="middle" font-size="15" font-weight="bold" fill="#0f172a">{}</text>"##,
            total_w as f32 / 2.0,
            escape_xml(t)
        ));
    }

    svg.push_str(&format!(
        r##"<line x1="{pad_x:.1}" y1="{axis_y:.1}" x2="{:.1}" y2="{axis_y:.1}" stroke="#94a3b8" stroke-width="2.5"/>"##,
        total_w as f32 - pad_x
    ));

    for (i, entry) in entries.iter().enumerate() {
        let cx = pad_x + (i as f32 + 0.5) * col_w;
        let color = ACCENTS[i % ACCENTS.len()];
        svg.push_str(&format!(
            r##"<circle cx="{cx:.1}" cy="{axis_y:.1}" r="7" fill="{color}" stroke="#ffffff" stroke-width="2"/>"##
        ));
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="12.5" font-weight="bold" fill="#0f172a">{}</text>"##,
            axis_y - 14.0,
            escape_xml(&entry.period)
        ));
        for (ei, ev) in entry.events.iter().enumerate() {
            let ey = axis_y + 20.0 + ei as f32 * 38.0;
            let bw = col_w - 16.0;
            svg.push_str(&format!(
                r##"<rect x="{:.1}" y="{ey:.1}" width="{bw:.1}" height="28" rx="6" fill="#f8fafc" stroke="{color}" stroke-width="1.3"/>"##,
                cx - bw / 2.0
            ));
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="11.5" fill="#1e293b">{}</text>"##,
                ey + 18.0,
                escape_xml(ev)
            ));
        }
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

// ============================================================================
// Quadrant Chart (`quadrantChart`)
// ============================================================================

struct QuadrantPoint {
    label: String,
    x: f32, // 0.0..=1.0
    y: f32, // 0.0..=1.0
}

fn render_quadrant_chart_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut title: Option<String> = None;
    let mut x_left = String::from("Low");
    let mut x_right = String::from("High");
    let mut y_bottom = String::from("Low");
    let mut y_top = String::from("High");
    let mut q_labels = [
        String::from("Q1"),
        String::from("Q2"),
        String::from("Q3"),
        String::from("Q4"),
    ];
    let mut points: Vec<QuadrantPoint> = Vec::new();

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("title ") {
            title = Some(rest.trim().to_string());
        } else if let Some(rest) = l.strip_prefix("x-axis ") {
            if let Some((a, b)) = rest.split_once("-->") {
                x_left = a.trim().to_string();
                x_right = b.trim().to_string();
            } else {
                x_left = rest.trim().to_string();
            }
        } else if let Some(rest) = l.strip_prefix("y-axis ") {
            if let Some((a, b)) = rest.split_once("-->") {
                y_bottom = a.trim().to_string();
                y_top = b.trim().to_string();
            } else {
                y_bottom = rest.trim().to_string();
            }
        } else if let Some(rest) = l.strip_prefix("quadrant-1 ") {
            q_labels[0] = rest.trim().to_string();
        } else if let Some(rest) = l.strip_prefix("quadrant-2 ") {
            q_labels[1] = rest.trim().to_string();
        } else if let Some(rest) = l.strip_prefix("quadrant-3 ") {
            q_labels[2] = rest.trim().to_string();
        } else if let Some(rest) = l.strip_prefix("quadrant-4 ") {
            q_labels[3] = rest.trim().to_string();
        } else if let Some((lbl, coords)) = l.split_once(':') {
            if let Some(vals) = extract_bracket_list(coords) {
                if vals.len() >= 2 {
                    if let (Ok(x), Ok(y)) = (vals[0].trim().parse::<f32>(), vals[1].trim().parse::<f32>()) {
                        points.push(QuadrantPoint {
                            label: lbl.trim().to_string(),
                            x: x.clamp(0.0, 1.0),
                            y: y.clamp(0.0, 1.0),
                        });
                    }
                }
            }
        }
    }

    if points.is_empty() {
        return None;
    }

    let size = 340.0f32;
    let pad_l = 56.0f32;
    let pad_r = 36.0f32;
    let pad_t = if title.is_some() { 52.0f32 } else { 28.0f32 };
    let pad_b = 46.0f32;
    let total_w = (pad_l + size + pad_r).ceil() as u32;
    let total_h = (pad_t + size + pad_b).ceil() as u32;
    let half = size / 2.0;

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="sans-serif">"##
    ));
    svg.push_str(&format!(
        r##"<rect width="{total_w}" height="{total_h}" fill="#ffffff"/>"##
    ));
    if let Some(t) = &title {
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="26" text-anchor="middle" font-size="15" font-weight="bold" fill="#0f172a">{}</text>"##,
            total_w as f32 / 2.0,
            escape_xml(t)
        ));
    }

    // 4 quadrants: Q2 (top-left), Q1 (top-right), Q3 (bottom-left), Q4 (bottom-right)
    let quads = [
        (pad_l + half, pad_t, "#eff6ff", &q_labels[0]),
        (pad_l, pad_t, "#f0fdf4", &q_labels[1]),
        (pad_l, pad_t + half, "#fefce8", &q_labels[2]),
        (pad_l + half, pad_t + half, "#fdf2f8", &q_labels[3]),
    ];
    for (qx, qy, bg, qlbl) in quads {
        svg.push_str(&format!(
            r##"<rect x="{qx:.1}" y="{qy:.1}" width="{half:.1}" height="{half:.1}" fill="{bg}" stroke="#cbd5e1" stroke-width="1"/>"##
        ));
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="11" font-weight="bold" fill="#64748b">{}</text>"##,
            qx + half / 2.0,
            qy + 18.0,
            escape_xml(qlbl)
        ));
    }

    // Axis labels
    svg.push_str(&format!(
        r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#334155">{}</text>"##,
        pad_l + half * 0.5,
        pad_t + size + 22.0,
        escape_xml(&x_left)
    ));
    svg.push_str(&format!(
        r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#334155">{}</text>"##,
        pad_l + half * 1.5,
        pad_t + size + 22.0,
        escape_xml(&x_right)
    ));
    svg.push_str(&format!(
        r##"<text x="{:.1}" y="{:.1}" text-anchor="end" font-size="10.5" fill="#334155">{}</text>"##,
        pad_l - 6.0,
        pad_t + 14.0,
        escape_xml(&y_top)
    ));
    svg.push_str(&format!(
        r##"<text x="{:.1}" y="{:.1}" text-anchor="end" font-size="10.5" fill="#334155">{}</text>"##,
        pad_l - 6.0,
        pad_t + size - 6.0,
        escape_xml(&y_bottom)
    ));

    for pt in &points {
        let px = pad_l + pt.x * size;
        let py = pad_t + (1.0 - pt.y) * size;
        svg.push_str(&format!(
            r##"<circle cx="{px:.1}" cy="{py:.1}" r="5.5" fill="#2563eb" stroke="#ffffff" stroke-width="1.5"/>"##
        ));
        svg.push_str(&format!(
            r##"<text x="{px:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#0f172a">{}</text>"##,
            py - 9.0,
            escape_xml(&pt.label)
        ));
    }

    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
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

fn push_centered_label(svg: &mut String, cx: f32, cy: f32, text: &str, font_size: f32, max_width: f32, fill: &str, weight: &str) {
    let lines = label_lines(text, max_width, font_size);
    let line_h = font_size * 1.25;
    let block = lines.len() as f32 * line_h;
    let mut y = cy - block / 2.0 + font_size * 0.8;
    for line in &lines {
        svg.push_str(&format!(
            r##"<text x="{cx:.1}" y="{y:.1}" text-anchor="middle" font-size="{font_size}" font-weight="{weight}" fill="{fill}">{}</text>"##,
            escape_xml(line)
        ));
        y += line_h;
    }
}

/// `<br>` becomes a line break. A line wider than `max_width` wraps, using
/// [`estimate_text_width`] so Japanese characters take a full em.
fn label_lines(text: &str, max_width: f32, font_size: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for piece in break_label(text).split('\n') {
        if piece.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            continue;
        }
        lines.extend(wrap_piece(piece, max_width.max(8.0), font_size));
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn break_label(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let raw = text.as_bytes();
    let folded = lower.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut idx = 0usize;
    while idx < raw.len() {
        if folded[idx..].starts_with(b"<br")
            && let Some(rel) = folded[idx + 3..].iter().position(|ch| *ch == b'>')
        {
            let inside = &folded[idx + 3..idx + 3 + rel];
            if inside.iter().all(|ch| ch.is_ascii_whitespace() || *ch == b'/') {
                out.push('\n');
                idx += 3 + rel + 1;
                continue;
            }
        }
        let ch = text[idx..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        idx += ch.len_utf8();
    }
    out
}

fn wrap_piece(text: &str, max_width: f32, font_size: f32) -> Vec<String> {
    if estimate_text_width(text, font_size) <= max_width {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut width = 0.0f32;
    for token in split_wrap_tokens(text) {
        let token_width = estimate_text_width(&token, font_size);
        if current.is_empty() || width + token_width <= max_width {
            if token_width <= max_width || current.is_empty() && !token.chars().any(|ch| !ch.is_ascii() || ch.is_whitespace()) {
                if token_width <= max_width {
                    current.push_str(&token);
                    width += token_width;
                    continue;
                }
            }
        } else {
            lines.push(std::mem::take(&mut current));
            width = 0.0;
        }
        if estimate_text_width(&token, font_size) <= max_width {
            current = token;
            width = token_width;
            continue;
        }
        for ch in token.chars() {
            let ch_width = estimate_text_width(&ch.to_string(), font_size);
            if !current.is_empty() && width + ch_width > max_width {
                lines.push(std::mem::take(&mut current));
                width = 0.0;
            }
            current.push(ch);
            width += ch_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() { vec![String::new()] } else { lines }
}

fn split_wrap_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut ascii_word = false;
    for ch in text.chars() {
        let word = ch.is_ascii() && !ch.is_ascii_whitespace();
        if current.is_empty() {
            current.push(ch);
            ascii_word = word;
            continue;
        }
        if ch.is_whitespace() || (ascii_word && word) {
            current.push(ch);
            continue;
        }
        tokens.push(std::mem::take(&mut current));
        current.push(ch);
        ascii_word = word;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
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
    fn unsupported_diagram_returns_none() {
        assert!(render_mermaid("C4Context\n  Person(user, \"User\")\n").is_none());
        assert!(render_mermaid("").is_none());
        assert!(render_mermaid("requirementDiagram\n  requirement req1 {\n    id: 1\n    text: hello\n  }\n").is_none());
    }

    #[test]
    fn parses_pie_and_xychart_to_native_chart_specs() {
        let pie = "pie showData\n  title ブラウザシェア\n  \"Chrome\" : 65\n  \"Safari\" : 20\n  \"Other\" : 15\n";
        let Some(MermaidOutput::Chart(spec)) = render_mermaid(pie) else {
            panic!("expected native chart for pie");
        };
        assert_eq!(spec.kind, NewChartKind::Pie);
        assert!(spec.show_data);
        assert_eq!(spec.title.as_deref(), Some("ブラウザシェア"));
        assert_eq!(spec.categories, vec!["Chrome", "Safari", "Other"]);
        assert_eq!(spec.series[0].1.len(), 3);
        assert_eq!(spec.series_kinds, vec![NewChartKind::Pie]);

        let xy = "xychart-beta\n  title \"月次売上\"\n  x-axis [Jan, Feb, Mar]\n  bar [100, 140, 180]\n  line [90, 120, 170]\n";
        let Some(MermaidOutput::Chart(xy_spec)) = render_mermaid(xy) else {
            panic!("expected native chart for xychart-beta");
        };
        assert_eq!(xy_spec.kind, NewChartKind::ClusteredColumn);
        assert_eq!(xy_spec.title.as_deref(), Some("月次売上"));
        assert_eq!(xy_spec.series.len(), 2);
        assert_eq!(xy_spec.series_kinds, vec![NewChartKind::ClusteredColumn, NewChartKind::Line]);
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

    #[test]
    fn renders_state_diagram_svg() {
        let src = "\
stateDiagram-v2
  [*] --> Idle
  Idle --> Running : start
  state Active {
    Running --> Paused : pause
    Paused --> Running : resume
  }
  Running --> [*] : finish
";
        let Some(MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        }) = render_mermaid(src)
        else {
            panic!("expected svg for stateDiagram-v2");
        };
        assert!(width_px > 120 && height_px > 120);
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(svg.contains("Idle"));
        assert!(svg.contains("Running"));
        assert!(svg.contains("Active"));
        assert!(svg.contains("start"));
    }

    #[test]
    fn renders_class_diagram_svg() {
        let src = "\
classDiagram
  class Animal {
    <<abstract>>
    +String name
    +speak() void
  }
  class Dog {
    +fetch() void
  }
  Animal <|-- Dog : inherits
";
        let Some(MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        }) = render_mermaid(src)
        else {
            panic!("expected svg for classDiagram");
        };
        assert!(width_px > 120 && height_px > 120);
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(svg.contains("Animal"));
        assert!(svg.contains("Dog"));
        assert!(svg.contains("speak() void"));
        assert!(svg.contains("inherits"));
    }

    #[test]
    fn renders_er_diagram_svg() {
        let src = "\
erDiagram
  CUSTOMER {
    int id PK
    string name
  }
  ORDER {
    int id PK
    int customer_id FK
  }
  CUSTOMER ||--o{ ORDER : places
";
        let Some(MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        }) = render_mermaid(src)
        else {
            panic!("expected svg for erDiagram");
        };
        assert!(width_px > 200 && height_px > 100);
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(svg.contains("CUSTOMER"));
        assert!(svg.contains("ORDER"));
        assert!(svg.contains("places"));
        assert!(svg.contains("0..*"));
    }

    #[test]
    fn renders_gantt_svg() {
        let src = "\
gantt
  title 開発スケジュール
  dateFormat YYYY-MM-DD
  section 設計
  要件定義 :done, req, 2026-01-01, 5d
  基本設計 :active, des, after req, 7d
  section 実装
  プロトタイプ :crit, impl, after des, 10d
  リリース :milestone, rel, after impl, 1d
";
        let Some(MermaidOutput::Svg {
            bytes,
            width_px,
            height_px,
        }) = render_mermaid(src)
        else {
            panic!("expected svg for gantt");
        };
        assert!(width_px > 400 && height_px > 150);
        let svg = std::str::from_utf8(&bytes).unwrap();
        assert!(svg.contains("開発スケジュール"));
        assert!(svg.contains("要件定義"));
        assert!(svg.contains("基本設計"));
        assert!(svg.contains("2026-01-01"));
    }

    #[test]
    fn renders_mindmap_and_gitgraph_and_chart_svg_fallback() {
        let mm = "\
mindmap
  root((ibis-office))
    Markdown
      Mermaid
      Math
    Formats
      DOCX
      PPTX
";
        let Some(MermaidOutput::Svg { bytes, width_px, height_px }) = render_mermaid(mm) else {
            panic!("expected svg for mindmap");
        };
        assert!(width_px > 200 && height_px > 120);
        let mm_svg = std::str::from_utf8(&bytes).unwrap();
        assert!(mm_svg.contains("ibis-office"));
        assert!(mm_svg.contains("Mermaid"));

        let git = "\
gitGraph
  commit id: \"init\"
  branch feat
  checkout feat
  commit id: \"wip\"
  checkout main
  merge feat tag: \"v1.0\"
";
        let Some(MermaidOutput::Svg { bytes: gbytes, width_px: gw, height_px: gh }) = render_mermaid(git) else {
            panic!("expected svg for gitGraph");
        };
        assert!(gw > 250 && gh > 80);
        let git_svg = std::str::from_utf8(&gbytes).unwrap();
        assert!(git_svg.contains("main"));
        assert!(git_svg.contains("feat"));
        assert!(git_svg.contains("v1.0"));

        let pie = "pie\n  title Share\n  \"A\" : 60\n  \"B\" : 40\n";
        let Some(MermaidOutput::Chart(spec)) = render_mermaid(pie) else {
            panic!("expected chart spec");
        };
        let (svg_bytes, pw, ph) = render_chart_spec_svg(&spec).expect("svg fallback");
        assert!(pw > 300 && ph > 200);
        let pie_svg = std::str::from_utf8(&svg_bytes).unwrap();
        assert!(pie_svg.contains("Share"));
        assert!(pie_svg.contains("60.0%"));
    }

    #[test]
    fn renders_journey_timeline_quadrant_and_frontmatter() {
        let journey = "\
---
title: User Onboarding
---
journey
  title My Day
  section Morning
    Coffee: 5: Me
    Commute: 2: Me, Train
";
        let Some(MermaidOutput::Svg { bytes: jbytes, width_px: jw, height_px: jh }) = render_mermaid(journey) else {
            panic!("expected svg for journey with YAML frontmatter");
        };
        assert!(jw > 200 && jh > 180);
        let jsvg = std::str::from_utf8(&jbytes).unwrap();
        assert!(jsvg.contains("Coffee"));
        assert!(jsvg.contains("Commute"));

        let tl = "\
timeline
  title History
  2024 : Alpha
  2025 : Beta : GA
";
        let Some(MermaidOutput::Svg { bytes: tbytes, .. }) = render_mermaid(tl) else {
            panic!("expected svg for timeline");
        };
        let tsvg = std::str::from_utf8(&tbytes).unwrap();
        assert!(tsvg.contains("2024"));
        assert!(tsvg.contains("Alpha"));
        assert!(tsvg.contains("GA"));

        let qc = "\
quadrantChart
  title Priority Matrix
  x-axis Low Effort --> High Effort
  y-axis Low Impact --> High Impact
  quadrant-1 Strategic
  quadrant-2 Quick Wins
  Task A: [0.2, 0.8]
  Task B: [0.75, 0.3]
";
        let Some(MermaidOutput::Svg { bytes: qbytes, .. }) = render_mermaid(qc) else {
            panic!("expected svg for quadrantChart");
        };
        let qsvg = std::str::from_utf8(&qbytes).unwrap();
        assert!(qsvg.contains("Priority Matrix"));
        assert!(qsvg.contains("Quick Wins"));
        assert!(qsvg.contains("Task A"));
    }

    fn svg_of(source: &str) -> String {
        let Some(MermaidOutput::Svg { bytes, .. }) = render_mermaid(source) else {
            panic!("expected svg:\n{source}");
        };
        String::from_utf8(bytes).expect("svg utf-8")
    }

    fn text_x(svg: &str, label: &str) -> f32 {
        let needle = format!(">{label}<");
        let at = svg.find(&needle).unwrap_or_else(|| panic!("missing {label}"));
        let head = &svg[..at];
        let x_at = head.rfind("x=\"").unwrap_or_else(|| panic!("no x for {label}"));
        let rest = &head[x_at + 3..];
        let end = rest.find('"').unwrap();
        rest[..end].parse().unwrap()
    }

    #[test]
    fn flowchart_labels_shapes_statements_and_subgraph_direction() {
        let labeled = "\
flowchart TD
  A[\"上<br/>下\"]
  B[日本語の長い見出しを折り返して枠を広げる確認]
";
        let svg = svg_of(labeled);
        assert!(svg.contains(">上<") && svg.contains(">下<"));
        assert!(!svg.contains("<br") && !svg.contains("&lt;br"));
        let long = "日本語の長い見出しを折り返して枠を広げる確認";
        let lines = label_lines(long, 168.0, 13.0);
        assert!(lines.len() >= 2, "{lines:?}");
        for line in &lines {
            assert!(svg.contains(line), "missing wrapped line {line}");
        }
        assert!(!svg.contains(long));

        let shapes = "\
flowchart LR
  A[(筒)] --> B{{六角}}
  C[/台形/] --> D[\\逆台形\\]
  E[/平行四辺形\\] --> F>旗]
";
        let svg = svg_of(shapes);
        for shape in ["cylinder", "hexagon", "trapezoid", "inv-trapezoid", "parallelogram", "asymmetric"] {
            assert!(svg.contains(&format!("data-shape=\"{shape}\"")), "missing {shape}:\n{svg}");
        }
        for label in ["筒", "六角", "台形", "逆台形", "平行四辺形", "旗"] {
            assert!(svg.contains(label), "missing {label}");
        }

        let split = "flowchart LR\n  A[A]-->B[B]; C[C]-->D[D]\n";
        let svg = svg_of(split);
        for label in ["A", "B", "C", "D"] {
            assert!(svg.contains(&format!(">{label}<")));
        }
        assert!(svg.matches("marker-end=\"url(#flow-arrow)\"").count() >= 2);

        let directed = "\
flowchart TD
  subgraph side
    direction LR
    L[左] --> R[右]
  end
";
        let svg = svg_of(directed);
        assert!(text_x(&svg, "左") < text_x(&svg, "右"), "subgraph LR should place 左 left of 右:\n{svg}");
    }

    #[test]
    fn sequence_activation_and_box() {
        let src = "\
sequenceDiagram
  box rgb(200,220,240) 利用者
    actor U as ユーザー
    participant API as API
  end
  U->>+API: 依頼
  API-->>-U: 応答
";
        let svg = svg_of(src);
        assert!(svg.contains("data-shape=\"activation\""), "{svg}");
        assert!(svg.contains("data-shape=\"seq-box\""), "{svg}");
        assert!(svg.contains("利用者"));
        assert!(svg.contains("依頼"));
    }

    #[test]
    fn xychart_alias_names_and_mixed_series_kinds() {
        let src = "\
xychart
  title Mix
  x-axis [Jan, Feb]
  bar \"売上\" [10, 20]
  line \"目標\" [8, 18]
  bar \"再\" [1, 2]
";
        let Some(MermaidOutput::Chart(spec)) = render_mermaid(src) else {
            panic!("expected chart");
        };
        assert_eq!(spec.kind, NewChartKind::ClusteredColumn);
        assert_eq!(spec.series.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), vec!["売上", "目標", "再"]);
        assert_eq!(
            spec.series_kinds,
            vec![NewChartKind::ClusteredColumn, NewChartKind::Line, NewChartKind::ClusteredColumn]
        );
    }

    #[test]
    fn gantt_date_format_weekends_and_dependencies() {
        let formatted = "\
gantt
  dateFormat DD.MM.YYYY
  section S
  開始 :a, 15.04.2026, 2d
";
        let svg = svg_of(formatted);
        assert!(svg.contains("data-start=\"2026-04-15\""), "{svg}");

        let weekends = "\
gantt
  dateFormat YYYY-MM-DD
  excludes weekends
  section S
  作業 :a, 2026-01-02, 5d
";
        let svg = svg_of(weekends);
        assert!(svg.contains("data-start=\"2026-01-02\""), "{svg}");
        assert!(svg.contains("data-nonworking=\"weekend\""), "{svg}");
        assert!(svg.contains("width=\"480.0\" height=\"18\""), "weekend shading must not change the 5-day bar:\n{svg}");

        let after = "\
gantt
  dateFormat YYYY-MM-DD
  section S
  甲 :a, 2026-03-01, 2d
  乙 :b, 2026-03-01, 5d
  丙 :c, after a b, 2d
";
        let svg = svg_of(after);
        assert!(svg.contains("data-start=\"2026-03-06\""), "after a b starts at the later end:\n{svg}");

        let unresolved = "\
gantt
  dateFormat YYYY-MM-DD
  section S
  既知 :done, k, 2026-03-01, 3d
  不明 :after nope, 2d
";
        let svg = svg_of(unresolved);
        assert!(svg.contains("不明"));
        assert!(svg.contains("data-start=\"2026-03-04\""), "{svg}");
        assert!(!svg.contains("2026-01-01"), "a dated chart must not snap an unresolved after to the origin:\n{svg}");

        let only = "\
gantt
  section S
  のみ :after missing, 3d
";
        let svg = svg_of(only);
        assert!(svg.contains("のみ"));
        assert!(svg.contains("data-start=\"2026-01-01\""), "with no dated task the origin stays 2026-01-01:\n{svg}");
    }

    #[test]
    fn state_composite_endpoints_choice_and_bars() {
        let src = "\
stateDiagram-v2
  state One {
    [*] --> A
  }
  state Two {
    [*] --> B
  }
  state pick <<choice>>
  A --> pick
  state split <<fork>>
  pick --> split
  state joined <<join>>
  split --> joined
";
        let svg = svg_of(src);
        assert_eq!(svg.matches("r=\"10\" fill=\"#1e293b\"").count(), 2, "each composite keeps its own start:\n{svg}");
        assert!(svg.contains("data-shape=\"choice\""), "{svg}");
        assert!(svg.contains("data-shape=\"fork\""), "{svg}");
        assert!(svg.contains("data-shape=\"join\""), "{svg}");
    }

    fn text_y(svg: &str, label: &str) -> f32 {
        let needle = format!(">{label}<");
        let at = svg.find(&needle).unwrap_or_else(|| panic!("missing {label}"));
        let head = &svg[..at];
        let y_at = head.rfind("y=\"").unwrap_or_else(|| panic!("no y for {label}"));
        let rest = &head[y_at + 3..];
        let end = rest.find('"').unwrap();
        rest[..end].parse().unwrap()
    }

    fn attr_f32(tag: &str, name: &str) -> Option<f32> {
        let key = format!("{name}=\"");
        let at = tag.find(&key)?;
        let rest = &tag[at + key.len()..];
        let end = rest.find('"')?;
        rest[..end].parse().ok()
    }

    fn painted_rects(svg: &str, fill: &str) -> Vec<(f32, f32, f32, f32)> {
        let mut out = Vec::new();
        let mut rest = svg;
        let needle = format!("fill=\"{fill}\"");
        while let Some(at) = rest.find("<rect ") {
            let end = rest[at..].find("/>").map(|i| at + i).unwrap_or(rest.len());
            let tag = &rest[at..end];
            if tag.contains(&needle)
                && let (Some(x), Some(y), Some(w), Some(h)) = (attr_f32(tag, "x"), attr_f32(tag, "y"), attr_f32(tag, "width"), attr_f32(tag, "height"))
            {
                out.push((x, y, w, h));
            }
            rest = &rest[end..];
        }
        out
    }

    fn view_box(svg: &str) -> (f32, f32) {
        let at = svg.find("viewBox=\"").expect("viewBox");
        let rest = &svg[at + 9..];
        let end = rest.find('"').unwrap();
        let parts: Vec<f32> = rest[..end].split_whitespace().map(|part| part.parse().unwrap()).collect();
        (parts[2], parts[3])
    }

    fn rect_holds(rect: (f32, f32, f32, f32), px: f32, py: f32) -> bool {
        px >= rect.0 && py >= rect.1 && px <= rect.0 + rect.2 && py <= rect.1 + rect.3
    }

    fn rects_overlap(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        a.0 < b.0 + b.2 - 0.5 && a.0 + a.2 > b.0 + 0.5 && a.1 < b.1 + b.3 - 0.5 && a.1 + a.3 > b.1 + 0.5
    }

    fn fill_none_paths(svg: &str) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        let mut rest = svg;
        while let Some(at) = rest.find("<path ") {
            let end = rest[at..].find("/>").map(|i| at + i).unwrap_or(rest.len());
            let tag = &rest[at..end];
            if tag.contains("fill=\"none\"")
                && let Some(d_at) = tag.find("d=\"")
            {
                let d_rest = &tag[d_at + 3..];
                let d_end = d_rest.find('"').unwrap();
                out.push((d_rest[..d_end].to_string(), tag.contains("data-route=\"detour\"")));
            }
            rest = &rest[end..];
        }
        out
    }

    fn inside_inset(px: f32, py: f32, rect: (f32, f32, f32, f32)) -> bool {
        px > rect.0 + 1.0 && py > rect.1 + 1.0 && px < rect.0 + rect.2 - 1.0 && py < rect.1 + rect.3 - 1.0
    }

    #[test]
    fn layered_layout_sibling_gap_uses_each_node_width() {
        let sizes = [(200.0f32, 40.0), (80.0, 40.0)];
        let (centers, total_w, total_h) = compute_layered_layout(2, &[], &sizes, false, false, 32.0, 64.0, 34.0);
        let (x0, y0) = centers[0];
        let (x1, y1) = centers[1];
        assert!(x0 < x1, "insertion order keeps the wider node on the left");
        assert!((y0 - y1).abs() < 0.01, "same layer shares a center line");
        let gap = (x1 - 40.0) - (x0 + 100.0);
        assert!((gap - 34.0).abs() < 0.01, "gap {gap} should be the sibling gap");
        assert!(x0 - 100.0 >= 0.0 && x1 + 40.0 <= total_w as f32 + 0.01);
        assert!(y0 - 20.0 >= 0.0 && y0 + 20.0 <= total_h as f32 + 0.01);
    }

    #[test]
    fn flowchart_siblings_keep_wrapped_label_gaps() {
        let long = "日本語の長い見出しを折り返して枠を広げる確認";
        let src = format!("flowchart TD\n  T[親] --> A[短い]\n  T --> B[{long}]\n");
        let svg = svg_of(&src);
        let (vw, vh) = view_box(&svg);
        let rects = painted_rects(&svg, "#eff6ff");
        assert!(rects.len() >= 3, "{svg}");
        for rect in &rects {
            assert!(rect.0 >= -0.5 && rect.1 >= -0.5 && rect.0 + rect.2 <= vw + 0.5 && rect.1 + rect.3 <= vh + 0.5, "node outside viewBox {rect:?} {vw}x{vh}");
        }
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                assert!(!rects_overlap(rects[i], rects[j]), "nodes overlap: {:?} {:?}", rects[i], rects[j]);
            }
        }
        let short = rects.iter().copied().find(|rect| rect_holds(*rect, text_x(&svg, "短い"), text_y(&svg, "短い"))).expect("short node");
        let first = label_lines(long, 168.0, 13.0).into_iter().next().unwrap();
        let wide = rects.iter().copied().find(|rect| rect_holds(*rect, text_x(&svg, &first), text_y(&svg, &first))).expect("wide node");
        assert!(wide.2 > short.2 + 50.0, "wrapped label should widen its box: {wide:?} vs {short:?}");
        let gap = if short.0 + short.2 <= wide.0 {
            wide.0 - (short.0 + short.2)
        } else {
            short.0 - (wide.0 + wide.2)
        };
        assert!((gap - 34.0).abs() < 0.6, "sibling gap {gap}");
        assert!(svg.contains(&first));
    }

    #[test]
    fn edges_detour_around_intermediate_nodes() {
        let sizes = [(84.0f32, 40.0), (114.0, 40.0), (84.0, 40.0)];
        let pairs = [(0usize, 1usize), (0, 2), (1, 2)];
        let (centers, total_w, total_h) = compute_layered_layout(3, &pairs, &sizes, false, false, 32.0, 64.0, 34.0);
        let canvas = (total_w as f32, total_h as f32);
        let skip = route_layered_edge(0, 2, &centers, &sizes, false, true, canvas);
        let EdgeRouteKind::Path { d, detour: true } = &skip.kind else {
            panic!("skip edge should detour: {skip:?}");
        };
        let mid = node_rect(centers[1].0, centers[1].1, sizes[1].0, sizes[1].1);
        for (px, py) in sample_path(d) {
            assert!(!point_in_rect(px, py, &pad_rect(mid, -1.0)), "detour sample {px},{py} enters the middle node");
            assert!(px >= -0.5 && py >= -0.5 && px <= canvas.0 + 0.5 && py <= canvas.1 + 0.5);
        }
        assert!(!point_in_rect(skip.label_at.0, skip.label_at.1, &mid), "label sits inside the obstacle");
        let adjacent = route_layered_edge(0, 1, &centers, &sizes, false, true, canvas);
        assert!(matches!(adjacent.kind, EdgeRouteKind::Path { detour: false, .. }), "a clear edge keeps its curve: {adjacent:?}");

        let blocked = route_layered_edge(
            0,
            2,
            &[(40.0, 12.0), (40.0, 50.0), (40.0, 88.0)],
            &[(24.0, 16.0), (200.0, 200.0), (24.0, 16.0)],
            false,
            true,
            (80.0, 100.0),
        );
        let EdgeRouteKind::Path { d: blocked_d, detour: false } = &blocked.kind else {
            panic!("an unavoidable crossing keeps the curve: {blocked:?}");
        };
        assert!(blocked_d.contains('C'), "{blocked_d}");

        let svg = svg_of("\
flowchart TD
  Top[上] --> Mid[中間障害物]
  Top -->|経由| Bottom[下]
  Mid --> Bottom
");
        let (vw, vh) = view_box(&svg);
        for label in ["上", "中間障害物", "下", "経由"] {
            assert!(svg.contains(&format!(">{label}<")), "{label} missing:\n{svg}");
        }
        let rects = painted_rects(&svg, "#eff6ff");
        assert_eq!(rects.len(), 3, "{svg}");
        for rect in &rects {
            assert!(rect.0 >= -0.5 && rect.1 >= -0.5 && rect.0 + rect.2 <= vw + 0.5 && rect.1 + rect.3 <= vh + 0.5, "{rect:?} outside {vw}x{vh}");
        }
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                assert!(!rects_overlap(rects[i], rects[j]));
            }
        }
        let mid_rect = rects.iter().copied().find(|rect| rect_holds(*rect, text_x(&svg, "中間障害物"), text_y(&svg, "中間障害物"))).expect("middle");
        let paths = fill_none_paths(&svg);
        let detours: Vec<_> = paths.iter().filter(|(_, detour)| *detour).collect();
        assert_eq!(detours.len(), 1, "only the skip edge detours:\n{svg}");
        let samples = sample_path(&detours[0].0);
        assert!(samples.iter().all(|(px, py)| !inside_inset(*px, *py, mid_rect)), "path enters 中間障害物:\n{svg}");
        let (lx, ly) = (text_x(&svg, "経由"), text_y(&svg, "経由"));
        assert!(!rect_holds(mid_rect, lx, ly), "edge label is inside the middle node");
        assert!(samples.iter().any(|(px, py)| (px - lx).hypot(py - ly) < 24.0), "label should sit on the detour");
        assert!(paths.iter().filter(|(_, detour)| !detour).all(|(d, _)| d.contains('C')));

        let plain = svg_of("flowchart TD\n  A[上] --> B[下]\n");
        let plain_paths = fill_none_paths(&plain);
        assert_eq!(plain_paths.len(), 1);
        assert!(!plain_paths[0].1 && plain_paths[0].0.contains('C'));

        let state = svg_of("\
stateDiagram-v2
  A --> B
  A --> C
  B --> C
");
        assert!(state.contains("data-route=\"detour\""), "{state}");
        for label in ["A", "B", "C"] {
            assert!(state.contains(&format!(">{label}<")));
        }
        let class = svg_of("\
classDiagram
  class A
  class B
  class C
  A --> C
  A --> B
  B --> C
");
        assert!(class.contains("data-route=\"detour\""), "{class}");
        assert!(class.contains(">A<") && class.contains(">B<") && class.contains(">C<"));
        let er = svg_of("\
erDiagram
  A ||--|| C : skip
  A ||--|| B : near
  B ||--|| C : next
");
        assert!(er.contains("data-route=\"detour\""), "{er}");
        assert!(er.contains("skip") && er.contains(">A<") && er.contains(">B<") && er.contains(">C<"));
    }

    #[test]
    fn subgraph_frame_pads_title_away_from_children() {
        let svg = svg_of("\
flowchart TD
  subgraph 外側
    A[子ノード]
  end
");
        let (vw, vh) = view_box(&svg);
        let frames = painted_rects(&svg, "#f8fafc");
        let nodes = painted_rects(&svg, "#eff6ff");
        assert_eq!(frames.len(), 1, "{svg}");
        assert_eq!(nodes.len(), 1, "{svg}");
        let frame = frames[0];
        let node = nodes[0];
        assert!((node.0 - frame.0 - 16.0).abs() < 0.2, "left pad {frame:?} {node:?}");
        assert!(((frame.0 + frame.2) - (node.0 + node.2) - 16.0).abs() < 0.2, "right pad");
        assert!(((frame.1 + frame.3) - (node.1 + node.3) - 16.0).abs() < 0.2, "bottom pad");
        assert!((node.1 - frame.1 - 38.0).abs() < 0.2, "title band {frame:?} {node:?}");
        let title_y = text_y(&svg, "外側");
        assert!((title_y - (frame.1 + 16.0)).abs() < 0.2, "title baseline {title_y}");
        assert!(node.1 - title_y >= 16.0, "title overlaps the child");
        assert!(frame.0 >= -0.5 && frame.1 >= -0.5 && frame.0 + frame.2 <= vw + 0.5 && frame.1 + frame.3 <= vh + 0.5, "frame outside viewBox");
        assert!(node.0 >= -0.5 && node.1 >= -0.5 && node.0 + node.2 <= vw + 0.5 && node.1 + node.3 <= vh + 0.5);
        assert!(svg.contains(">子ノード<"));
    }

    // ---- P0 compatibility additions --------------------------------------------------------

    #[test]
    fn header_aliases_reach_the_same_renderers() {
        for (alias, canonical) in [
            ("classDiagram-v2\n  class Animal\n  Animal <|-- Duck\n", "classDiagram\n  class Animal\n  Animal <|-- Duck\n"),
            ("stateDiagram-v2\n  [*] --> Still\n", "stateDiagram\n  [*] --> Still\n"),
            ("graph LR\n  A --> B\n", "flowchart LR\n  A --> B\n"),
            ("flowchart-elk LR\n  A --> B\n", "flowchart LR\n  A --> B\n"),
            ("flowchart-v2 LR\n  A --> B\n", "flowchart LR\n  A --> B\n"),
            ("gitGraph:\n  commit\n  branch dev\n  commit\n", "gitGraph\n  commit\n  branch dev\n  commit\n"),
        ] {
            assert_eq!(render_mermaid(alias), render_mermaid(canonical), "{alias}");
            assert!(render_mermaid(alias).is_some(), "{alias}");
        }
        // `graph LR;` keeps its direction: the two nodes sit side by side.
        let svg = svg_of("graph LR;\n  A-->B;\n");
        assert!((text_y(&svg, "A") - text_y(&svg, "B")).abs() < 1.0, "{svg}");
        assert!(text_x(&svg, "B") > text_x(&svg, "A") + 40.0);
        // Only whole words count as headers.
        assert!(render_mermaid("graphql\n  A --> B\n").is_none());
        assert!(render_mermaid("classDiagramX\n  class A\n").is_none());
        assert!(render_mermaid("sankey-beta\n  A,B,10\n").is_none());
        assert_eq!(diagram_kind("xychart horizontal"), Some(DiagramKind::XyChart));
        assert_eq!(diagram_kind("sequenceDiagram"), Some(DiagramKind::Sequence));
    }

    #[test]
    fn frontmatter_title_is_drawn_above_svg_diagrams() {
        let plain = svg_of("flowchart LR\n  A --> B\n");
        let titled_src = "---\ntitle: \"受注の流れ\"\nconfig:\n  title: nested is ignored\n---\nflowchart LR\n  A --> B\n";
        let Some(MermaidOutput::Svg { bytes, width_px, height_px }) = render_mermaid(titled_src) else {
            panic!("svg");
        };
        let titled = String::from_utf8(bytes).unwrap();
        assert!(titled.contains(r#"data-role="diagram-title">受注の流れ</text>"#), "{titled}");
        assert!(!titled.contains("nested is ignored"));
        let (pw, ph) = view_box(&plain);
        assert_eq!(width_px as f32, pw.max(width_px as f32));
        assert_eq!(height_px as f32, ph + 40.0);
        assert!(titled.contains(r#"<g transform="translate("#));
        assert!(roxmltree::Document::parse(&titled).is_ok());
        // A long title widens the canvas and centres the drawing.
        let long = svg_of("---\ntitle: A very long diagram title that is wider than the single node below\n---\nflowchart LR\n  A\n");
        let (lw, _) = view_box(&long);
        assert!(lw > view_box(&svg_of("flowchart LR\n  A\n")).0);
        // Charts take it as their title unless the body has one.
        let Some(MermaidOutput::Chart(pie)) = render_mermaid("---\ntitle: Pets\n---\npie\n  \"Dogs\" : 3\n  \"Cats\" : 2\n") else {
            panic!("pie");
        };
        assert_eq!(pie.title.as_deref(), Some("Pets"));
        let Some(MermaidOutput::Chart(pie)) = render_mermaid("---\ntitle: Pets\n---\npie title Body\n  title Body\n  \"Dogs\" : 3\n") else {
            panic!("pie");
        };
        assert_eq!(pie.title.as_deref(), Some("Body"));
        // A diagram with its own title statement is not titled twice.
        let seq = svg_of("---\ntitle: Front\n---\nsequenceDiagram\n  title Body title\n  A->>B: hi\n");
        assert!(seq.contains(">Body title<"));
        assert!(!seq.contains(">Front<"));
        assert_eq!(mermaid_frontmatter_title("---\ntitle: 'Quoted'\n---\ngraph\n"), Some("Quoted".to_string()));
        assert_eq!(mermaid_frontmatter_title("graph\n  A\n"), None);
    }

    #[test]
    fn flowchart_bidirectional_circle_cross_and_invisible_edges() {
        let tok = |s: &str| match_flowchart_edge(s, true).map(|(t, n)| (t.style, t.start, t.end, t.label, n));
        assert_eq!(tok("<--> B"), Some((EdgeStyle::Solid, EdgeEnd::Arrow, EdgeEnd::Arrow, None, 4)));
        assert_eq!(tok("o--o B"), Some((EdgeStyle::Solid, EdgeEnd::Circle, EdgeEnd::Circle, None, 4)));
        assert_eq!(tok("x--x B"), Some((EdgeStyle::Solid, EdgeEnd::Cross, EdgeEnd::Cross, None, 4)));
        assert_eq!(tok("--o B"), Some((EdgeStyle::Solid, EdgeEnd::None, EdgeEnd::Circle, None, 3)));
        assert_eq!(tok("--x B"), Some((EdgeStyle::Solid, EdgeEnd::None, EdgeEnd::Cross, None, 3)));
        assert_eq!(tok("<-.-> B"), Some((EdgeStyle::Dashed, EdgeEnd::Arrow, EdgeEnd::Arrow, None, 5)));
        assert_eq!(tok("<==> B"), Some((EdgeStyle::Thick, EdgeEnd::Arrow, EdgeEnd::Arrow, None, 4)));
        assert_eq!(tok("----> B"), Some((EdgeStyle::Solid, EdgeEnd::None, EdgeEnd::Arrow, None, 5)));
        assert_eq!(tok("~~~ B"), Some((EdgeStyle::Invisible, EdgeEnd::None, EdgeEnd::None, None, 3)));
        assert_eq!(tok("<-- text --> B"), Some((EdgeStyle::Solid, EdgeEnd::Arrow, EdgeEnd::Arrow, Some("text".into()), 12)));
        assert_eq!(tok("-. 点線 .-> B").map(|t| (t.0, t.2, t.3)), Some((EdgeStyle::Dashed, EdgeEnd::Arrow, Some("点線".into()))));
        assert_eq!(tok("-->|yes| B").map(|t| t.3), Some(Some("yes".into())));
        assert_eq!(tok("-- B"), None);
        // `o` / `x` start a link only at the start of a word: `Bo--x` is not a circle start.
        assert!(match_flowchart_edge("o--o B", false).is_none());

        let svg = svg_of("flowchart LR\n  A <--> B\n  B o--o C\n  C x--x D\n  D --o E\n  E ~~~ F\n");
        for label in ["A", "B", "C", "D", "E", "F"] {
            assert!(svg.contains(&format!(">{label}<")), "{label} missing:\n{svg}");
        }
        assert!(!svg.contains(">A <<") && !svg.contains("&lt;"), "no stray node text: {svg}");
        assert_eq!(svg.matches(r##"marker-start="url(#flow-arrow)""##).count(), 1);
        assert_eq!(svg.matches(r##"marker-start="url(#flow-circle)""##).count(), 1);
        assert_eq!(svg.matches(r##"marker-start="url(#flow-cross)""##).count(), 1);
        assert_eq!(svg.matches(r##"marker-end="url(#flow-circle)""##).count(), 2);
        assert_eq!(svg.matches(r##"marker-end="url(#flow-cross)""##).count(), 1);
        // The `~~~` link places F after E but draws nothing.
        let body = &svg[svg.find("</defs>").expect("defs")..];
        assert_eq!(fill_none_paths(body).len() + body.matches("<line x1").count(), 4, "{svg}");
        assert!(text_x(&svg, "F") > text_x(&svg, "E"));
        // Edge ids are not node text.
        let ids = svg_of("flowchart LR\n  A e1@--> B\n");
        assert!(ids.contains(">A<") && !ids.contains("e1"), "{ids}");
    }

    #[test]
    fn flowchart_class_def_class_style_and_link_style() {
        let svg = svg_of(
            "flowchart LR
  A:::warn --> B
  B --> C
  C --> D
  classDef warn fill:#fee2e2,stroke:#dc2626,stroke-width:3px
  classDef ok fill:#dcfce7
  class B,C ok
  style D fill:#f9f,stroke:#333,color:#ffffff,stroke-dasharray: 5 5
  linkStyle 1 stroke:#ff3,stroke-width:4px
  linkStyle default stroke:#0000ff
",
        );
        assert!(svg.contains(r##"fill="#fee2e2" stroke="#dc2626" stroke-width="3""##), "{svg}");
        assert_eq!(painted_rects(&svg, "#dcfce7").len(), 2, "class B,C ok:\n{svg}");
        assert_eq!(painted_rects(&svg, "#f9f").len(), 1);
        assert!(svg.contains(r##"stroke-dasharray="5 5""##));
        assert!(svg.contains(r##"fill="#ffffff">D</text>"##), "style color is the label colour:\n{svg}");
        // linkStyle: the second edge is yellow and wide, the others take the default.
        assert_eq!(svg.matches(r##"stroke="#ff3" stroke-width="4""##).count(), 1, "{svg}");
        assert_eq!(svg.matches(r##"stroke="#0000ff""##).count(), 2);
        // `classDef default` applies to every node without being named.
        let def = svg_of("flowchart LR\n  A --> B\n  classDef default fill:#abcdef\n");
        assert_eq!(painted_rects(&def, "#abcdef").len(), 2);
        // Values that could break out of the attribute are dropped.
        let evil = svg_of("flowchart LR\n  A\n  style A fill:red\"/><script>,stroke:#123456\n");
        assert!(!evil.contains("<script>") && evil.contains(r##"stroke="#123456""##), "{evil}");
        assert!(roxmltree::Document::parse(&evil).is_ok());
        assert_eq!(split_node_class("A[x]:::hot"), ("A[x]", Some("hot")));
        assert_eq!(split_node_class("A[a:::b]"), ("A[a:::b]", None));
        assert_eq!(parse_flow_style("fill:rgb(1, 2, 3),stroke-width:2px").fill.as_deref(), Some("rgb(1, 2, 3)"));
    }

    #[test]
    fn sequence_rect_create_destroy_title_and_dividers() {
        let svg = svg_of(
            "sequenceDiagram
  title 予約の流れ
  participant Alice
  rect rgba(0, 0, 255, 0.1)
    Alice->>Bob: Hello
    rect rgb(200, 150, 255)
      Bob-->>Alice: Hi
    end
  end
  create participant Carl
  Alice->>Carl: Hi Carl
  par Alice to Bob
    Alice->>Bob: one
  and Alice to Carl
    Alice->>Carl: two
  end
  destroy Carl
  Alice-xCarl: bye
  note over Alice: done
",
        );
        assert!(svg.contains(r#"data-role="diagram-title">予約の流れ</text>"#), "{svg}");
        assert_eq!(svg.matches(r#"data-shape="seq-rect""#).count(), 2);
        assert!(svg.contains(r#"fill="rgb(0,0,255)" fill-opacity="0.10""#), "{svg}");
        assert!(svg.contains(r#"fill="rgb(200,150,255)" fill-opacity="1.00""#));
        // Nested band drawn after (on top of) its parent.
        assert!(svg.find("rgb(0,0,255)").unwrap() < svg.find("rgb(200,150,255)").unwrap());
        // `rect ... end` does not close a fragment: one `par` frame, with an `and` divider.
        assert!(svg.contains(">par<"), "{svg}");
        assert!(svg.contains(">[Alice to Carl]<"));
        // Carl's top box sits on the creating message, below Alice's; Carl has no bottom box.
        let alice_boxes = svg.matches(">Alice</text>").count();
        let carl_boxes = svg.matches(">Carl</text>").count();
        assert_eq!(alice_boxes, 2);
        assert_eq!(carl_boxes, 1, "{svg}");
        assert!(text_y(&svg, "Carl") > text_y(&svg, "Alice") + 60.0);
        assert!(svg.contains(r#"data-shape="seq-destroy""#));
        assert!(svg.contains(">done<"), "lower-case note");
        assert!(roxmltree::Document::parse(&svg).is_ok());

        assert_eq!(seq_rect_paint("rgb(1,2,3)"), ("rgb(1,2,3)".to_string(), 1.0));
        assert_eq!(seq_rect_paint("#abc"), ("#abc".to_string(), 1.0));
        assert_eq!(seq_rect_paint("aqua"), ("aqua".to_string(), 1.0));
        assert_eq!(seq_rect_paint("url(javascript:x)").0, "rgb(200,200,255)");
    }

    #[test]
    fn xychart_axis_titles_and_value_range() {
        let src = "xychart
  title \"Sales Revenue\"
  x-axis \"Month\" [jan, feb, mar]
  y-axis \"Revenue (in $)\" 4000 --> 11000
  bar [5000, 6000, 7500]
";
        let Some(MermaidOutput::Chart(spec)) = render_mermaid(src) else {
            panic!("chart");
        };
        assert_eq!(spec.x_axis_title.as_deref(), Some("Month"));
        assert_eq!(spec.y_axis_title.as_deref(), Some("Revenue (in $)"));
        assert_eq!(spec.y_range, Some((4000.0, 11000.0)));
        assert_eq!(spec.categories, vec!["jan", "feb", "mar"]);
        let (bytes, _, _) = render_chart_spec_svg(&spec).expect("svg");
        let svg = String::from_utf8(bytes).unwrap();
        assert!(svg.contains(r#"data-role="y-axis-title">Revenue (in $)</text>"#), "{svg}");
        assert!(svg.contains(r#"data-role="x-axis-title">Month</text>"#));
        assert!(svg.contains(">4000<") && svg.contains(">11000<"), "{svg}");

        assert_eq!(parse_xy_axis("\"Revenue\""), (Some("Revenue".into()), None));
        assert_eq!(parse_xy_axis("0 --> 100"), (None, Some((0.0, 100.0))));
        assert_eq!(parse_xy_axis("Score 1.5 --> 2.5"), (Some("Score".into()), Some((1.5, 2.5))));
        assert_eq!(parse_xy_axis("\"Low --> High\""), (Some("Low --> High".into()), None));
        assert_eq!(parse_xy_axis("10 --> 5"), (Some("10 --> 5".into()), None));
    }
}

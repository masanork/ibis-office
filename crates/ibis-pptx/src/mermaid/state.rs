// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! `stateDiagram` / `stateDiagram-v2`.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StateKind {
    Start,
    End,
    Normal,
    Choice,
    Fork,
    Join,
}

pub(super) struct StateNode {
    pub(super) label: String,
    pub(super) description: Option<String>,
    pub(super) kind: StateKind,
    pub(super) composite: Option<String>,
    /// The concurrent region (`--`) of the composite the state is in; 0 without regions.
    pub(super) region: usize,
    /// `class` / `:::` names.
    pub(super) classes: Vec<String>,
}

/// `note left of X : text` or a `note right of X` ... `end note` block.
pub(super) struct StateNote {
    pub(super) target: usize,
    pub(super) left: bool,
    pub(super) text: String,
}

/// `Moving:::movement` as (`Moving`, `Some("movement")`).
pub(super) fn split_state_class(id: &str) -> (&str, Option<&str>) {
    match id.trim().split_once(":::") {
        Some((id, class)) => (id.trim(), Some(class.trim()).filter(|c| !c.is_empty())),
        None => (id.trim(), None),
    }
}

/// Split at the first `:` that is not part of `:::`.
pub(super) fn split_lone_colon(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    (0..b.len())
        .find(|&i| b[i] == b':' && (i == 0 || b[i - 1] != b':') && b.get(i + 1) != Some(&b':'))
        .map(|i| (&s[..i], &s[i + 1..]))
}

/// Moves the concurrent regions of each composite apart along the cross axis (x for TB, y for
/// LR), in source order, and everything beyond them along with them. Returns how much the
/// drawing grew.
pub(super) fn separate_state_regions(nodes: &[StateNode], sizes: &[(f32, f32)], centers: &mut [(f32, f32)], horizontal: bool) -> f32 {
    const GAP: f32 = 36.0;
    let mut grown = 0.0f32;
    let mut comps: Vec<&str> = Vec::new();
    for n in nodes {
        if n.region > 0
            && let Some(c) = n.composite.as_deref()
            && !comps.contains(&c)
        {
            comps.push(c);
        }
    }
    let cross = |i: usize, centers: &[(f32, f32)]| {
        let (c, half) = if horizontal { (centers[i].1, sizes[i].1 / 2.0) } else { (centers[i].0, sizes[i].0 / 2.0) };
        (c - half, c + half)
    };
    for comp in comps {
        let members: Vec<usize> = (0..nodes.len()).filter(|&i| nodes[i].composite.as_deref() == Some(comp)).collect();
        let regions = members.iter().map(|&i| nodes[i].region).max().unwrap_or(0);
        let span = |idxs: &[usize], centers: &[(f32, f32)]| {
            idxs.iter().map(|&i| cross(i, centers)).fold((f32::MAX, f32::MIN), |a, b| (a.0.min(b.0), a.1.max(b.1)))
        };
        let (lo, hi) = span(&members, centers);
        let mut cursor = lo;
        for region in 0..=regions {
            let idxs: Vec<usize> = members.iter().copied().filter(|&i| nodes[i].region == region).collect();
            if idxs.is_empty() {
                continue;
            }
            let (r_lo, r_hi) = span(&idxs, centers);
            let d = cursor - r_lo;
            for &i in &idxs {
                if horizontal { centers[i].1 += d } else { centers[i].0 += d }
            }
            cursor = r_hi + d + GAP;
        }
        let growth = (cursor - GAP - hi).max(0.0);
        if growth > 0.0 {
            // Whatever lay beyond the composite moves on by as much.
            for i in 0..nodes.len() {
                if nodes[i].composite.as_deref() != Some(comp) && cross(i, centers).0 >= hi {
                    if horizontal { centers[i].1 += growth } else { centers[i].0 += growth }
                }
            }
            grown += growth;
        }
    }
    grown
}

pub(super) struct StateEdge {
    pub(super) from: usize,
    pub(super) to: usize,
    pub(super) label: Option<String>,
}

pub(super) fn state_stereotype(rest: &str) -> Option<StateKind> {
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
pub(super) fn state_endpoint(comp: Option<&str>, region: usize, end: bool, nodes: &mut Vec<StateNode>, map: &mut HashMap<String, usize>) -> usize {
    // Each concurrent region of a composite has its own start and end.
    let key = match (comp, end) {
        (Some(name), false) => format!("{name}\0{region}[*]start"),
        (Some(name), true) => format!("{name}\0{region}[*]end"),
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
        region,
        classes: Vec::new(),
    });
    map.insert(key, idx);
    idx
}

pub(super) fn render_state_diagram_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
    let mut horizontal = false;
    let mut reversed = false;
    let mut nodes: Vec<StateNode> = Vec::new();
    let mut node_map: HashMap<String, usize> = HashMap::new();
    let mut edges: Vec<StateEdge> = Vec::new();
    // (composite name, current concurrent region)
    let mut composite_stack: Vec<(String, usize)> = Vec::new();
    let mut class_defs: HashMap<String, FlowStyle> = HashMap::new();
    let mut notes: Vec<StateNote> = Vec::new();
    // An open `note left|right of X` block: (target, left, lines).
    let mut open_note: Option<(usize, bool, Vec<String>)> = None;

    let ensure_state = |raw_id: &str, nodes: &mut Vec<StateNode>, map: &mut HashMap<String, usize>, comp: Option<(&str, usize)>| -> usize {
        let (id, class) = split_state_class(raw_id);
        let idx = match map.get(id) {
            Some(&idx) => {
                if nodes[idx].composite.is_none()
                    && let Some((name, region)) = comp
                {
                    nodes[idx].composite = Some(name.to_string());
                    nodes[idx].region = region;
                }
                idx
            }
            None => {
                let idx = nodes.len();
                nodes.push(StateNode {
                    label: id.to_string(),
                    description: None,
                    kind: StateKind::Normal,
                    composite: comp.map(|(name, _)| name.to_string()),
                    region: comp.map_or(0, |(_, region)| region),
                    classes: Vec::new(),
                });
                map.insert(id.to_string(), idx);
                idx
            }
        };
        if let Some(class) = class {
            nodes[idx].classes.push(class.to_string());
        }
        idx
    };

    for raw in source.lines().skip(1) {
        let l = raw.trim();
        if let Some((target, left, lines)) = &mut open_note {
            if l == "end note" {
                notes.push(StateNote { target: *target, left: *left, text: lines.join("\n") });
                open_note = None;
            } else if !l.is_empty() {
                lines.push(l.to_string());
            }
            continue;
        }
        if l.is_empty() || l.starts_with("%%") {
            continue;
        }
        if l == "--" {
            // A concurrent region starts inside the current composite.
            if let Some((_, region)) = composite_stack.last_mut() {
                *region += 1;
            }
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
        let cur_comp = composite_stack.last().map(|(name, region)| (name.as_str(), *region));
        if let Some(rest) = l.strip_prefix("note ") {
            let rest = rest.trim();
            let side = if rest.starts_with("left of ") {
                Some(true)
            } else if rest.starts_with("right of ") {
                Some(false)
            } else {
                None
            };
            if let Some(left) = side {
                let rest = rest.split_once(" of ").map_or("", |(_, r)| r);
                match split_lone_colon(rest) {
                    Some((id, text)) => {
                        let target = ensure_state(id, &mut nodes, &mut node_map, cur_comp);
                        notes.push(StateNote { target, left, text: text.trim().to_string() });
                    }
                    None => {
                        let target = ensure_state(rest, &mut nodes, &mut node_map, cur_comp);
                        open_note = Some((target, left, Vec::new()));
                    }
                }
            }
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
            if let Some((ids, class)) = rest.trim().rsplit_once(char::is_whitespace) {
                for id in ids.split(',').map(str::trim).filter(|id| !id.is_empty()) {
                    let idx = ensure_state(id, &mut nodes, &mut node_map, cur_comp);
                    nodes[idx].classes.push(class.trim().to_string());
                }
            }
            continue;
        }
        if l.starts_with("accTitle") || l.starts_with("accDescr") {
            continue;
        }
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
                composite_stack.push((name, 0));
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
            let (rhs_raw, label) = if let Some((r, lbl)) = split_lone_colon(rhs_all) {
                (r.trim(), Some(lbl.trim().to_string()).filter(|s| !s.is_empty()))
            } else {
                (rhs_all.trim(), None)
            };
            let (comp_name, region) = (cur_comp.map(|(name, _)| name), cur_comp.map_or(0, |(_, r)| r));
            let from = if lhs == "[*]" {
                state_endpoint(comp_name, region, false, &mut nodes, &mut node_map)
            } else {
                ensure_state(lhs, &mut nodes, &mut node_map, cur_comp)
            };
            let to = if rhs_raw == "[*]" {
                state_endpoint(comp_name, region, true, &mut nodes, &mut node_map)
            } else {
                ensure_state(rhs_raw, &mut nodes, &mut node_map, cur_comp)
            };
            edges.push(StateEdge { from, to, label });
            continue;
        }
        if let Some((state_id, desc)) = split_lone_colon(l) {
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
    let (mut centers, mut total_w, mut total_h) = compute_layered_layout(
        nodes.len(),
        &edge_pairs,
        &node_sizes,
        horizontal,
        reversed,
        margin,
        62.0,
        36.0,
    );

    // Concurrent regions side by side (stacked in LR), each in its own band.
    let grown = separate_state_regions(&nodes, &node_sizes, &mut centers, horizontal);
    if horizontal {
        total_h += grown.ceil() as u32;
    } else {
        total_w += grown.ceil() as u32;
    }

    // `classDef` styles: `default` first, then each class in order.
    let node_style: Vec<FlowStyle> = nodes
        .iter()
        .map(|node| {
            let mut style = class_defs.get("default").cloned().unwrap_or_default();
            for name in &node.classes {
                if let Some(def) = class_defs.get(name) {
                    style.merge(def);
                }
            }
            style
        })
        .collect();

    // Notes sit beside their state, outside the layout; the canvas grows to hold them.
    const NOTE_FONT: f32 = 11.0;
    let note_boxes: Vec<(f32, f32, f32, f32, Vec<String>)> = notes
        .iter()
        .map(|note| {
            let lines = label_lines(&note.text, 220.0, NOTE_FONT);
            let w = lines.iter().map(|l| estimate_text_width(l, NOTE_FONT)).fold(0.0f32, f32::max) + 16.0;
            let h = lines.len() as f32 * 14.0 + 12.0;
            let (cx, cy) = centers[note.target];
            let (nw, _) = node_sizes[note.target];
            let x = if note.left { cx - nw / 2.0 - 24.0 - w } else { cx + nw / 2.0 + 24.0 };
            (x, cy - h / 2.0, w, h, lines)
        })
        .collect();
    let left_room = note_boxes.iter().map(|b| b.0).fold(f32::MAX, f32::min);
    let shift_x = if left_room < 8.0 { (8.0 - left_room).ceil() } else { 0.0 };
    let top_room = note_boxes.iter().map(|b| b.1).fold(f32::MAX, f32::min);
    let shift_y = if top_room < 8.0 { (8.0 - top_room).ceil() } else { 0.0 };
    let right_edge = note_boxes.iter().map(|b| b.0 + b.2 + 8.0).fold(0.0f32, f32::max);
    let total_w = (total_w as f32 + shift_x).max(right_edge + shift_x).ceil() as u32;
    let total_h = note_boxes.iter().map(|b| b.1 + b.3 + 8.0).fold(total_h as f32, f32::max) + shift_y;
    let total_h = total_h.ceil() as u32;

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
    if shift_x > 0.0 || shift_y > 0.0 {
        svg.push_str(&format!(r#"<g transform="translate({shift_x:.1} {shift_y:.1})">"#));
    }

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
        // Concurrent regions (`--`) are divided by dashed lines.
        let regions = idxs.iter().map(|&i| nodes[i].region).max().unwrap_or(0);
        let bounds = |region: usize| {
            idxs.iter().filter(|&&i| nodes[i].region == region).fold(None, |acc: Option<(f32, f32, f32, f32)>, &i| {
                let (cx, cy) = centers[i];
                let (nw, nh) = node_sizes[i];
                let b = (cx - nw / 2.0, cy - nh / 2.0, cx + nw / 2.0, cy + nh / 2.0);
                Some(acc.map_or(b, |a| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))))
            })
        };
        for region in 1..=regions {
            let (Some(a), Some(b)) = (bounds(region - 1), bounds(region)) else { continue };
            let (x1, y1, x2, y2) = if a.2 <= b.0 || b.2 <= a.0 {
                let x = if a.2 <= b.0 { (a.2 + b.0) / 2.0 } else { (b.2 + a.0) / 2.0 };
                (x, sy + 24.0, x, sy + sh - 4.0)
            } else if a.3 <= b.1 || b.3 <= a.1 {
                let y = if a.3 <= b.1 { (a.3 + b.1) / 2.0 } else { (b.3 + a.1) / 2.0 };
                (sx + 4.0, y, sx + sw - 4.0, y)
            } else {
                continue;
            };
            svg.push_str(&format!(
                r##"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="6,4" data-role="region-divider"/>"##
            ));
        }
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
                let style = &node_style[idx];
                let rect = format!(
                    r##"<rect x="{x:.1}" y="{y:.1}" width="{nw:.1}" height="{nh:.1}" rx="10" fill="#eff6ff" stroke="#2563eb" stroke-width="1.4"/>"##
                );
                svg.push_str(&if style.is_empty() { rect } else { restyle_svg_fragment(&rect, style) });
                let ink = style.color.as_deref().map(escape_attr).unwrap_or_else(|| "#0f172a".to_string());
                let font = format!(
                    "{}{}",
                    style.font_style.as_deref().map(|v| format!(r#" font-style="{}""#, escape_attr(v))).unwrap_or_default(),
                    style.font_weight.as_deref().map(|v| format!(r#" font-weight="{}""#, escape_attr(v))).unwrap_or_default()
                );
                if let Some(desc) = &node.description {
                    svg.push_str(&format!(
                        r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="12.5" font-weight="bold" fill="{ink}"{}>{}</text>"##,
                        y + 18.0,
                        style.font_style.as_deref().map(|v| format!(r#" font-style="{}""#, escape_attr(v))).unwrap_or_default(),
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
                        r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="{}"{font}>{}</text>"##,
                        y + 42.0,
                        style.color.as_deref().map(escape_attr).unwrap_or_else(|| "#475569".to_string()),
                        escape_xml(desc)
                    ));
                } else {
                    svg.push_str(&format!(
                        r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="13" fill="{ink}"{font}>{}</text>"##,
                        cy + 4.5,
                        escape_xml(&node.label)
                    ));
                }
            }
        }
    }

    for (note, (x, y, w, h, lines)) in notes.iter().zip(&note_boxes) {
        let (cx, cy) = centers[note.target];
        let (nw, _) = node_sizes[note.target];
        let (from, to) = if note.left { (cx - nw / 2.0, x + w) } else { (cx + nw / 2.0, *x) };
        svg.push_str(&format!(
            r##"<line x1="{from:.1}" y1="{cy:.1}" x2="{to:.1}" y2="{cy:.1}" stroke="#ca8a04" stroke-width="1" stroke-dasharray="3,3"/>"##
        ));
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" fill="#fef9c3" stroke="#ca8a04" stroke-width="1" data-role="note"/>"##
        ));
        for (k, line) in lines.iter().enumerate() {
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="{:.1}" font-size="{NOTE_FONT}" fill="#422006">{}</text>"##,
                x + 8.0,
                y + 18.0 + k as f32 * 14.0,
                escape_xml(line)
            ));
        }
    }

    if shift_x > 0.0 || shift_y > 0.0 {
        svg.push_str("</g>");
    }
    svg.push_str("</svg>");
    Some((svg.into_bytes(), total_w, total_h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::test_support::*;

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

    #[test]
    fn state_class_defs_and_inline_classes() {
        let src = "stateDiagram
   classDef notMoving fill:white
   classDef movement font-style:italic;
   classDef badBadEvent fill:#f00,color:white,font-weight:bold,stroke-width:2px,stroke:yellow
   [*] --> Still:::notMoving
   Still --> Moving:::movement
   Moving --> Crash:::movement
   Crash:::badBadEvent --> [*]
   class Still movement
";
        let svg = svg_of(src);
        assert!(!svg.contains(":::") && !svg.contains(">::"), "{svg}");
        let crash = tag_with(&svg, ">Crash<")[0];
        assert!(crash.contains(r#"fill="white""#) && crash.contains(r#"font-weight="bold""#) && crash.contains(r#"font-style="italic""#), "{crash}");
        assert!(svg.contains(r##"fill="#f00" stroke="yellow" stroke-width="2""##), "{svg}");
        assert!(tag_with(&svg, ">Still<")[0].contains(r#"font-style="italic""#), "class X name");
        assert_eq!(split_lone_colon("A:::c : label"), Some(("A:::c ", " label")));
        assert_eq!(split_lone_colon("A:::c"), None);
    }

    #[test]
    fn state_notes_beside_their_state() {
        let src = "stateDiagram-v2
    State1: The state with a note
    note right of State1
        Important information! You can write
        notes.
    end note
    State1 --> State2
    note left of State2 : This is the note to the left.
";
        let svg = svg_of(src);
        let notes = tag_with(&svg, r#"data-role="note""#);
        assert_eq!(notes.len(), 2, "{svg}");
        assert!(svg.contains(">notes.</text>") && svg.contains(">This is the note to the left.</text>"));
        assert!(!svg.contains("end note") && !svg.contains(">note "), "{svg}");
        // The left note fits: the drawing moves right to make room.
        assert!(svg.contains("<g transform=\"translate("), "{svg}");
        let (w, _) = view_box(&svg);
        for n in notes {
            let x = attr_f32(n, "x").unwrap();
            assert!(x + attr_f32(n, "width").unwrap() <= w + 0.5, "{n}");
        }
    }

    #[test]
    fn state_concurrent_regions() {
        let src = "stateDiagram-v2
    [*] --> Active
    state Active {
        [*] --> NumLockOff
        NumLockOff --> NumLockOn : EvNumLockPressed
        --
        [*] --> CapsLockOff
        CapsLockOff --> CapsLockOn : EvCapsLockPressed
        --
        [*] --> ScrollLockOff
        ScrollLockOff --> ScrollLockOn : EvScrollLockPressed
    }
";
        let svg = svg_of(src);
        assert_eq!(svg.matches(r#"data-role="region-divider""#).count(), 2, "{svg}");
        // Each region starts from its own [*], plus the top-level one.
        assert_eq!(svg.matches("r=\"10\" fill=\"#1e293b\"").count(), 4, "{svg}");
        assert!(!svg.contains(">--<"));
    }
}

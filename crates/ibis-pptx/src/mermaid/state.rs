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
pub(super) fn state_endpoint(comp: Option<&str>, end: bool, nodes: &mut Vec<StateNode>, map: &mut HashMap<String, usize>) -> usize {
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

pub(super) fn render_state_diagram_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
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
}

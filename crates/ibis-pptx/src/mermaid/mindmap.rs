// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! `mindmap`.

use super::*;

pub(super) struct MindNode {
    pub(super) label: String,
    pub(super) depth: usize,
    pub(super) children: Vec<usize>,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
}

pub(super) fn render_mindmap_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
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

pub(super) fn clean_mindmap_label(s: &str) -> String {
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

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure radial 1-hop Local Graph layout for the Right Dock (`RightPanelKind::Graph`).

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::TAU;

const MAX_NEIGHBORS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDirection {
    Incoming,
    Outgoing,
    Bidirectional,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub path: String,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub is_center: bool,
    pub direction: Option<LinkDirection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphEdge {
    pub from_x: f32,
    pub from_y: f32,
    pub to_x: f32,
    pub to_y: f32,
    pub direction: LinkDirection,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalGraphLayout {
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub truncated: usize,
}

fn short_note_label(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(".md")
        .or_else(|| base.strip_suffix(".markdown"))
        .unwrap_or(base)
        .to_owned()
}

/// Compute deterministic radial coordinates for the active note (`center_path`) and its
/// 1-hop incoming and outgoing Vault neighbors inside a `width x height` box.
pub fn compute_local_graph_layout(
    center_path: &str,
    incoming: &[String],
    outgoing: &[String],
    width: f32,
    height: f32,
) -> LocalGraphLayout {
    let w = width.max(180.0);
    let h = height.max(220.0);
    let cx = w * 0.5;
    let cy = h * 0.5;

    let incoming_set: BTreeSet<&str> = incoming
        .iter()
        .map(String::as_str)
        .filter(|p| *p != center_path)
        .collect();
    let outgoing_set: BTreeSet<&str> = outgoing
        .iter()
        .map(String::as_str)
        .filter(|p| *p != center_path)
        .collect();

    let mut merged: BTreeMap<&str, LinkDirection> = BTreeMap::new();
    for &p in &incoming_set {
        merged.insert(p, LinkDirection::Incoming);
    }
    for &p in &outgoing_set {
        merged
            .entry(p)
            .and_modify(|d| *d = LinkDirection::Bidirectional)
            .or_insert(LinkDirection::Outgoing);
    }

    let total_neighbors = merged.len();
    let shown: Vec<(&str, LinkDirection)> = merged.into_iter().take(MAX_NEIGHBORS).collect();
    let truncated = total_neighbors.saturating_sub(shown.len());

    let mut nodes = Vec::with_capacity(shown.len() + 1);
    let mut edges = Vec::with_capacity(shown.len());

    nodes.push(GraphNode {
        path: center_path.to_owned(),
        label: short_note_label(center_path),
        x: cx,
        y: cy,
        is_center: true,
        direction: None,
    });

    let rx = (w * 0.5 - 48.0).max(40.0);
    let ry = (h * 0.5 - 32.0).max(52.0);
    let count = shown.len();

    for (idx, (path, direction)) in shown.into_iter().enumerate() {
        let angle = -std::f32::consts::FRAC_PI_2 + (idx as f32 / count.max(1) as f32) * TAU;
        let nx = cx + rx * angle.cos();
        let ny = cy + ry * angle.sin();
        edges.push(GraphEdge {
            from_x: cx,
            from_y: cy,
            to_x: nx,
            to_y: ny,
            direction,
        });
        nodes.push(GraphNode {
            path: path.to_owned(),
            label: short_note_label(path),
            x: nx,
            y: ny,
            is_center: false,
            direction: Some(direction),
        });
    }

    LocalGraphLayout {
        width: w,
        height: h,
        nodes,
        edges,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radial_graph_merges_bidirectional_neighbors_and_excludes_self() {
        let incoming = vec![
            "index.md".into(),
            "both.md".into(),
            "center.md".into(),
        ];
        let outgoing = vec![
            "both.md".into(),
            "docs/child.md".into(),
            "center.md".into(),
        ];
        let layout = compute_local_graph_layout("center.md", &incoming, &outgoing, 260.0, 280.0);

        assert_eq!(layout.nodes.len(), 4);
        assert_eq!(layout.edges.len(), 3);
        assert_eq!(layout.truncated, 0);
        assert!(layout.nodes[0].is_center);
        assert_eq!(layout.nodes[0].label, "center");

        let by_path: BTreeMap<&str, &GraphNode> = layout
            .nodes
            .iter()
            .skip(1)
            .map(|n| (n.path.as_str(), n))
            .collect();
        assert_eq!(by_path["both.md"].direction, Some(LinkDirection::Bidirectional));
        assert_eq!(by_path["index.md"].direction, Some(LinkDirection::Incoming));
        assert_eq!(by_path["docs/child.md"].direction, Some(LinkDirection::Outgoing));
        assert_eq!(by_path["docs/child.md"].label, "child");
    }
}

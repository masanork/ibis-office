// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! `quadrantChart`.

use super::*;

pub(super) struct QuadrantPoint {
    pub(super) label: String,
    pub(super) x: f32, // 0.0..=1.0
    pub(super) y: f32, // 0.0..=1.0
}

pub(super) fn render_quadrant_chart_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
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

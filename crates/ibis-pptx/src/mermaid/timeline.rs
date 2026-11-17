// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! `timeline`.

use super::*;

pub(super) struct TimelineEntry {
    pub(super) period: String,
    pub(super) events: Vec<String>,
}

pub(super) fn render_timeline_svg(source: &str, config: &MermaidConfig) -> Option<(Vec<u8>, u32, u32)> {
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
    let multicolor = !config.flag("timeline.disableMulticolor", false);
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
        // `themeVariables.cScaleN` recolours period N; `timeline.disableMulticolor` uses one colour.
        let n = if multicolor { i } else { 0 };
        let scale = config.color_var(&format!("cScale{}", n % 12));
        let color = scale.clone().unwrap_or_else(|| ACCENTS[n % ACCENTS.len()].to_string());
        let (box_fill, text_fill) = match config.color_var(&format!("cScaleLabel{}", n % 12)) {
            Some(label) => (color.clone(), label),
            None => ("#f8fafc".to_string(), "#1e293b".to_string()),
        };
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
                r##"<rect x="{:.1}" y="{ey:.1}" width="{bw:.1}" height="28" rx="6" fill="{box_fill}" stroke="{color}" stroke-width="1.3"/>"##,
                cx - bw / 2.0
            ));
            svg.push_str(&format!(
                r##"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="11.5" fill="{text_fill}">{}</text>"##,
                ey + 18.0,
                escape_xml(ev)
            ));
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
    fn timeline_period_colours_and_class_empty_members_from_config() {
        let body = "timeline\n  2001 : A\n  2002 : B\n";
        let svg = svg_of(&format!("---\nconfig:\n  themeVariables:\n    cScale0: '#ff0000'\n    cScaleLabel0: '#ffffff'\n---\n{body}"));
        assert!(svg.contains(r##"fill="#ff0000""##) && svg.contains(r##"fill="#ffffff">A<"##), "{svg}");
        let mono = svg_of(&format!("---\nconfig:\n  timeline:\n    disableMulticolor: true\n---\n{body}"));
        assert!(!mono.contains("#059669"), "second period uses the first colour");
        assert!(svg_of(body).contains("#059669"));

        let height = |src: &str| match render_mermaid(src) {
            Some(MermaidOutput::Svg { height_px, .. }) => height_px,
            _ => panic!("svg"),
        };
        let class = "classDiagram\n  class Duck\n";
        let hidden = height(&format!("---\nconfig:\n  class:\n    hideEmptyMembersBox: true\n---\n{class}"));
        assert!(hidden < height(class), "{hidden}");
    }
}

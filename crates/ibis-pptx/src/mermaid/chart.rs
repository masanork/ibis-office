// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! `pie` / `xychart-beta`: native chart specs and their SVG fallback.

use super::*;

pub(super) fn parse_pie_chart(source: &str, config: &MermaidConfig) -> Option<MermaidChartSpec> {
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
    let style = pie_style(config, categories.len());
    // `pie.donutHole` turns the pie into a doughnut (native `c:doughnutChart`).
    let kind = if style.donut_hole.is_some() { NewChartKind::Doughnut } else { NewChartKind::Pie };
    Some(MermaidChartSpec {
        kind,
        title,
        categories,
        series: vec![("Series 1".to_string(), values)],
        series_kinds: vec![kind],
        show_data,
        x_axis_title: None,
        y_axis_title: None,
        y_range: None,
        point_labels: Vec::new(),
        style,
    })
}

/// `config.pie.*` and the `pie1`..`pie12` / `pieOuterStrokeWidth` theme variables.
pub(super) fn pie_style(config: &MermaidConfig, slices: usize) -> MermaidChartStyle {
    let number = |key: &str| config.option(key).and_then(|v| unquote(v).trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    let custom: Vec<Option<String>> =
        (1..=slices.min(12)).map(|i| config.vars.get(&format!("pie{i}")).and_then(|v| hex_color(v))).collect();
    let palette = if custom.iter().any(Option::is_some) {
        custom
            .into_iter()
            .enumerate()
            .map(|(i, c)| c.unwrap_or_else(|| CHART_PALETTE[i % CHART_PALETTE.len()].to_string()))
            .collect()
    } else {
        Vec::new()
    };
    MermaidChartStyle {
        palette,
        text_position: number("pie.textPosition").map(|v| v.clamp(0.0, 1.0)),
        donut_hole: number("pie.donutHole").map(|v| v.clamp(0.0, 0.9)).filter(|v| *v > 0.0),
        highlight: config
            .option("pie.highlightSlice")
            .map(unquote)
            .filter(|label| !label.is_empty() && label != "hover"),
        outer_stroke_width: config.vars.get("pieOuterStrokeWidth").and_then(|v| px_number(v)),
        ..MermaidChartStyle::default()
    }
}

/// `config.xyChart.*` and `themeVariables.xyChart.*`.
pub(super) fn xy_style(config: &MermaidConfig) -> MermaidChartStyle {
    let size = |key: &str| {
        config
            .option(key)
            .and_then(|v| unquote(v).trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && (100.0..=4000.0).contains(v))
            .map(|v| v.round() as u32)
    };
    let palette = config
        .vars
        .get("xyChart.plotColorPalette")
        .map(|list| list.split(',').filter_map(hex_color).collect())
        .unwrap_or_default();
    let size_px = match (size("xyChart.width"), size("xyChart.height")) {
        (None, None) => None,
        (w, h) => Some((w.unwrap_or(XY_WIDTH), h.unwrap_or(XY_HEIGHT))),
    };
    MermaidChartStyle {
        palette,
        data_labels: config.flag("xyChart.showDataLabel", false),
        data_labels_outside: config.flag("xyChart.showDataLabelOutsideBar", false),
        size_px,
        title_color: config.vars.get("xyChart.titleColor").and_then(|v| hex_color(v)),
        ..MermaidChartStyle::default()
    }
}

/// One `bar` / `line` value: `12`, or `540 "PaLM"` with a point label (Mermaid 11.16).
pub(super) fn parse_xy_value(token: &str) -> (Option<Number>, Option<String>) {
    let token = token.trim();
    let (number, label) = match token.find(|c: char| c.is_whitespace() || c == '"') {
        Some(at) => (&token[..at], Some(unquote(token[at..].trim()))),
        None => (token, None),
    };
    (number.parse::<f64>().ok().map(Number), label.filter(|l| !l.is_empty()))
}

pub(super) fn parse_xy_chart(source: &str, config: &MermaidConfig) -> Option<MermaidChartSpec> {
    let mut title = None;
    let mut categories = Vec::new();
    let mut series = Vec::new();
    let mut series_kinds = Vec::new();
    let mut point_labels: Vec<Vec<Option<String>>> = Vec::new();
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
        } else if let Some((is_bar, rest)) =
            l.strip_prefix("bar ").map(|r| (true, r)).or_else(|| l.strip_prefix("line ").map(|r| (false, r)))
        {
            if let Some(list) = extract_bracket_list(rest) {
                let (vals, labels): (Vec<_>, Vec<_>) = list.iter().map(|s| parse_xy_value(s)).unzip();
                let name = if is_bar {
                    has_bar = true;
                    bar_idx += 1;
                    xy_series_name(rest, &format!("Bar {}", bar_idx - 1))
                } else {
                    has_line = true;
                    line_idx += 1;
                    xy_series_name(rest, &format!("Line {}", line_idx - 1))
                };
                series.push((name, vals));
                series_kinds.push(if is_bar { NewChartKind::ClusteredColumn } else { NewChartKind::Line });
                point_labels.push(labels);
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
    if point_labels.iter().all(|labels| labels.iter().all(Option::is_none)) {
        point_labels.clear();
    }
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
        point_labels,
        style: xy_style(config),
    })
}

/// `"title" min --> max`, `title`, or `min --> max` of an `xychart` axis line.
pub(super) fn parse_xy_axis(rest: &str) -> (Option<String>, Option<(f64, f64)>) {
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

pub(super) fn xy_series_name(rest: &str, fallback: &str) -> String {
    let Some(list_at) = rest.find('[') else {
        return fallback.to_string();
    };
    let name = unquote(rest[..list_at].trim());
    if name.is_empty() { fallback.to_string() } else { name }
}

pub(super) fn extract_bracket_list(s: &str) -> Option<Vec<String>> {
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


/// The colours the chart fallback draws with, before `config` overrides them.
pub(super) const CHART_PALETTE: &[&str] = &[
    "#3b82f6", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6", "#ec4899", "#06b6d4", "#84cc16",
];
pub(super) const XY_WIDTH: u32 = 520;
pub(super) const XY_HEIGHT: u32 = 280;

/// A number shown on a chart: whole numbers without a fraction.
fn chart_number(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{v:.0}") } else { format!("{v}") }
}

/// Render a parsed `MermaidChartSpec` (`pie` or `xychart-beta`) to a standalone SVG.
///
/// Used as a fallback when a native Office chart cannot be instantiated (e.g., clipboard fragment paste).
pub fn render_chart_spec_svg(spec: &MermaidChartSpec) -> Option<(Vec<u8>, u32, u32)> {
    if spec.series.is_empty() || spec.categories.is_empty() {
        return None;
    }
    let style = &spec.style;
    let color_of = |i: usize| -> String {
        if style.palette.is_empty() {
            CHART_PALETTE[i % CHART_PALETTE.len()].to_string()
        } else {
            style.palette[i % style.palette.len()].clone()
        }
    };
    let title_fill = style.title_color.clone().unwrap_or_else(|| "#0f172a".to_string());

    if matches!(spec.kind, NewChartKind::Pie | NewChartKind::Doughnut) {
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
        // `pie.donutHole`: the inner radius as a fraction of the outer one.
        let hole = style.donut_hole.unwrap_or(0.0) * r;
        // `pie.textPosition`: where the percentage sits, from the centre (0) to the rim (1).
        let text_at = style.text_position.unwrap_or(0.75) * r;

        let mut svg = String::new();
        svg.push_str(&format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width_px}" height="{height_px}" viewBox="0 0 {width_px} {height_px}" font-family="sans-serif">"##
        ));
        svg.push_str(&format!(
            r##"<rect width="{width_px}" height="{height_px}" fill="#ffffff"/>"##
        ));
        if let Some(t) = &spec.title {
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="26" text-anchor="middle" font-size="14" font-weight="bold" fill="{title_fill}">{}</text>"##,
                width_px as f32 / 2.0,
                escape_xml(t)
            ));
        }

        let mut angle = -std::f64::consts::FRAC_PI_2;
        let mut labels = String::new();
        for (i, &val) in vals.iter().enumerate() {
            let frac = val / total;
            if frac <= 0.0 {
                continue;
            }
            let color = color_of(i);
            let next_angle = angle + frac * 2.0 * std::f64::consts::PI;
            let mid = (angle + next_angle) / 2.0;
            // `pie.highlightSlice`: that slice is pulled out of the pie and outlined.
            let highlighted = style.highlight.as_deref() == Some(spec.categories[i].as_str());
            let (ox, oy) = if highlighted { (8.0 * mid.cos(), 8.0 * mid.sin()) } else { (0.0, 0.0) };
            let (scx, scy) = (cx + ox, cy + oy);
            let mark = if highlighted { r##" data-highlight="1" stroke-width="3""## } else { r##" stroke-width="1.5""## };
            if frac >= 0.9999 {
                if hole > 0.0 {
                    svg.push_str(&format!(
                        r##"<path d="M {:.1} {scy:.1} A {r:.1} {r:.1} 0 1 1 {:.1} {scy:.1} A {r:.1} {r:.1} 0 1 1 {:.1} {scy:.1} Z M {:.1} {scy:.1} A {hole:.1} {hole:.1} 0 1 0 {:.1} {scy:.1} A {hole:.1} {hole:.1} 0 1 0 {:.1} {scy:.1} Z" fill="{color}" fill-rule="evenodd" stroke="#ffffff"{mark}/>"##,
                        scx - r, scx + r, scx - r, scx - hole, scx + hole, scx - hole
                    ));
                } else {
                    svg.push_str(&format!(
                        r##"<circle cx="{scx:.1}" cy="{scy:.1}" r="{r:.1}" fill="{color}" stroke="#ffffff"{mark}/>"##
                    ));
                }
            } else {
                let x1 = scx + r * angle.cos();
                let y1 = scy + r * angle.sin();
                let x2 = scx + r * next_angle.cos();
                let y2 = scy + r * next_angle.sin();
                let large_arc = if frac > 0.5 { 1 } else { 0 };
                let d = if hole > 0.0 {
                    let ix1 = scx + hole * next_angle.cos();
                    let iy1 = scy + hole * next_angle.sin();
                    let ix2 = scx + hole * angle.cos();
                    let iy2 = scy + hole * angle.sin();
                    format!(
                        "M {x1:.1} {y1:.1} A {r:.1} {r:.1} 0 {large_arc} 1 {x2:.1} {y2:.1} L {ix1:.1} {iy1:.1} A {hole:.1} {hole:.1} 0 {large_arc} 0 {ix2:.1} {iy2:.1} Z"
                    )
                } else {
                    format!("M {scx:.1} {scy:.1} L {x1:.1} {y1:.1} A {r:.1} {r:.1} 0 {large_arc} 1 {x2:.1} {y2:.1} Z")
                };
                svg.push_str(&format!(r##"<path d="{d}" fill="{color}" stroke="#ffffff"{mark}/>"##));
            }
            // The slice's share, like Mermaid's label on each slice.
            let lr = if frac >= 0.9999 { if hole > 0.0 { (hole + r) / 2.0 } else { 0.0 } } else { text_at.max(hole) };
            let (lx, ly) = (scx + lr * mid.cos(), scy + lr * mid.sin());
            labels.push_str(&format!(
                r##"<text x="{lx:.1}" y="{:.1}" text-anchor="middle" font-size="11" fill="#ffffff" data-role="slice-label">{:.0}%</text>"##,
                ly + 4.0,
                frac * 100.0
            ));
            angle = next_angle;
        }
        svg.push_str(&labels);
        // `themeVariables.pieOuterStrokeWidth`: a rim around the whole pie.
        if let Some(w) = style.outer_stroke_width.filter(|w| *w > 0.0) {
            svg.push_str(&format!(
                r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="{:.1}" fill="none" stroke="#000000" stroke-width="{w:.1}" data-role="pie-rim"/>"##,
                r + w / 2.0
            ));
        }

        // Legend
        let lx = 248.0f32;
        let ly_start = (cy as f32) - (spec.categories.len() as f32 * 22.0) / 2.0;
        for (i, cat) in spec.categories.iter().enumerate() {
            let color = color_of(i);
            let ly = ly_start + i as f32 * 22.0;
            let val = vals.get(i).copied().unwrap_or(0.0);
            let pct = (val / total) * 100.0;
            svg.push_str(&format!(
                r##"<rect x="{lx:.1}" y="{:.1}" width="12" height="12" rx="2" fill="{color}"/>"##,
                ly
            ));
            let shown = if spec.show_data { format!(" [{}]", chart_number(val)) } else { String::new() };
            svg.push_str(&format!(
                r##"<text x="{:.1}" y="{:.1}" font-size="11.5" fill="#1e293b">{}{} ({pct:.1}%)</text>"##,
                lx + 18.0,
                ly + 10.5,
                escape_xml(cat),
                escape_xml(&shown)
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
    let (width_px, height_px) = style.size_px.unwrap_or((
        XY_WIDTH,
        if spec.x_axis_title.is_some() { XY_HEIGHT + 20 } else { XY_HEIGHT },
    ));
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
            r##"<text x="{:.1}" y="24" text-anchor="middle" font-size="14" font-weight="bold" fill="{title_fill}">{}</text>"##,
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

    let is_line_at = |si: usize, sname: &str| {
        spec.series_kinds.get(si) == Some(&NewChartKind::Line)
            || (spec.series_kinds.is_empty() && (spec.kind == NewChartKind::Line || sname.starts_with("Line")))
    };
    // Bars of several series stand side by side in each category.
    let bar_series: Vec<usize> =
        (0..spec.series.len()).filter(|&si| !is_line_at(si, &spec.series[si].0)).collect();
    let mut labels = String::new();
    for (si, (sname, vals)) in spec.series.iter().enumerate() {
        let color = color_of(si);
        let point_label = |ci: usize| spec.point_labels.get(si).and_then(|l| l.get(ci)).and_then(|l| l.as_deref());
        if is_line_at(si, sname) {
            let mut pts = Vec::new();
            for (ci, v_opt) in vals.iter().enumerate() {
                if let Some(v) = v_opt {
                    let px = pad_l + (ci as f32 + 0.5) * slot_w;
                    let py = pad_t + plot_h * (1.0 - y_frac(v.0));
                    pts.push((ci, v.0, px, py));
                }
            }
            if pts.len() >= 2 {
                let poly: String = pts
                    .iter()
                    .map(|(_, _, x, y)| format!("{x:.1},{y:.1}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                svg.push_str(&format!(
                    r##"<polyline points="{poly}" fill="none" stroke="{color}" stroke-width="2.2"/>"##
                ));
            }
            for &(ci, v, px, py) in &pts {
                svg.push_str(&format!(
                    r##"<circle cx="{px:.1}" cy="{py:.1}" r="3.5" fill="{color}" stroke="#ffffff" stroke-width="1"/>"##
                ));
                // A point label (`540 "PaLM"`) and / or the value (`showDataLabel`) above the point.
                let text = match (point_label(ci), style.data_labels) {
                    (Some(l), true) => format!("{} {l}", chart_number(v)),
                    (Some(l), false) => l.to_string(),
                    (None, true) => chart_number(v),
                    (None, false) => continue,
                };
                labels.push_str(&format!(
                    r##"<text x="{px:.1}" y="{:.1}" text-anchor="middle" font-size="10" fill="#1e293b" data-role="data-label">{}</text>"##,
                    py - 8.0,
                    escape_xml(&text)
                ));
            }
        } else {
            let lanes = bar_series.len().max(1) as f32;
            let lane = bar_series.iter().position(|&b| b == si).unwrap_or(0) as f32;
            let bar_w = ((slot_w * 0.54).min(40.0 * lanes)) / lanes;
            for (ci, v_opt) in vals.iter().enumerate() {
                if let Some(v) = v_opt {
                    let bh = (plot_h * y_frac(v.0)).max(2.0);
                    let bx = pad_l + (ci as f32 + 0.5) * slot_w - bar_w * lanes / 2.0 + lane * bar_w;
                    let by = pad_t + plot_h - bh;
                    svg.push_str(&format!(
                        r##"<rect x="{bx:.1}" y="{by:.1}" width="{bar_w:.1}" height="{bh:.1}" rx="2" fill="{color}" fill-opacity="0.88"/>"##
                    ));
                    if style.data_labels || point_label(ci).is_some() {
                        let text = match point_label(ci) {
                            Some(l) if style.data_labels => format!("{} {l}", chart_number(v.0)),
                            Some(l) => l.to_string(),
                            None => chart_number(v.0),
                        };
                        // Inside the top of the bar, or above it with `showDataLabelOutsideBar`.
                        let (ty, fill) = if style.data_labels_outside || bh < 18.0 {
                            (by - 5.0, "#1e293b")
                        } else {
                            (by + 14.0, "#ffffff")
                        };
                        labels.push_str(&format!(
                            r##"<text x="{:.1}" y="{ty:.1}" text-anchor="middle" font-size="10" fill="{fill}" data-role="data-label">{}</text>"##,
                            bx + bar_w / 2.0,
                            escape_xml(&text)
                        ));
                    }
                }
            }
        }
    }
    svg.push_str(&labels);

    svg.push_str("</svg>");
    Some((svg.into_bytes(), width_px, height_px))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::test_support::*;

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

    fn chart_of(src: &str) -> MermaidChartSpec {
        match render_mermaid(src) {
            Some(MermaidOutput::Chart(spec)) => spec,
            _ => panic!("expected chart"),
        }
    }

    fn chart_svg(spec: &MermaidChartSpec) -> String {
        String::from_utf8(render_chart_spec_svg(spec).expect("svg").0).unwrap()
    }

    #[test]
    fn xychart_config_palette_labels_size_and_title_colour() {
        let src = "---
config:
  xyChart:
    width: 900
    height: 600
    showDataLabel: true
    showDataLabelOutsideBar: true
  themeVariables:
    xyChart:
      titleColor: \"#ff0000\"
      plotColorPalette: '#000000, #0000FF, nope, #0F0'
---
xychart
  title \"T\"
  x-axis [a, b]
  bar [12, 20]
  line [5, 15]
";
        let spec = chart_of(src);
        assert_eq!(spec.style.palette, vec!["#000000", "#0000ff", "#00ff00"]);
        assert!(spec.style.data_labels && spec.style.data_labels_outside);
        assert_eq!(spec.style.size_px, Some((900, 600)));
        assert_eq!(spec.style.title_color.as_deref(), Some("#ff0000"));
        let svg = chart_svg(&spec);
        assert!(svg.contains(r#"width="900" height="600""#), "{svg}");
        assert!(svg.contains(r##"font-weight="bold" fill="#ff0000">T<"##), "{svg}");
        assert!(svg.contains(r##"fill="#000000" fill-opacity"##), "bar takes the first palette colour");
        assert!(svg.contains(r##"stroke="#0000ff" stroke-width="2.2""##), "line takes the second");
        let labels: Vec<_> = svg.match_indices(r#"data-role="data-label">"#).collect();
        assert_eq!(labels.len(), 4, "{svg}");
        assert!(svg.contains(r##"fill="#1e293b" data-role="data-label">12<"##), "outside bar: dark text above");

        // Without `config`: no labels, the built-in colours, the default size.
        let plain = chart_of("xychart\n  x-axis [a, b]\n  bar [12, 20]\n");
        assert_eq!(plain.style, MermaidChartStyle::default());
        let svg = chart_svg(&plain);
        assert!(!svg.contains("data-label") && svg.contains(r#"width="520" height="280""#));
    }

    #[test]
    fn xychart_point_labels() {
        let spec = chart_of("xychart\n  x-axis [Q1, Q2, Q3]\n  line [25 \"Launch\", 45, 90 \"Target Hit\"]\n");
        assert_eq!(spec.series[0].1, vec![Some(Number(25.0)), Some(Number(45.0)), Some(Number(90.0))]);
        assert_eq!(spec.point_labels, vec![vec![Some("Launch".to_string()), None, Some("Target Hit".to_string())]]);
        let svg = chart_svg(&spec);
        assert!(svg.contains(">Launch<") && svg.contains(">Target Hit<"), "{svg}");
        assert_eq!(parse_xy_value("540 \"PaLM\""), (Some(Number(540.0)), Some("PaLM".into())));
        assert_eq!(parse_xy_value(" 3.8 "), (Some(Number(3.8)), None));
        // No labels at all: nothing is kept.
        assert!(chart_of("xychart\n  line [1, 2]\n").point_labels.is_empty());
    }

    #[test]
    fn pie_config_donut_text_position_highlight_and_rim() {
        let src = "---
config:
  pie:
    textPosition: 0.5
    donutHole: 0.2
    highlightSlice: Potassium
  themeVariables:
    pieOuterStrokeWidth: \"5px\"
    pie2: \"#123456\"
---
pie showData
    \"Calcium\" : 42.96
    \"Potassium\" : 50.05
    \"Iron\" :  5
";
        let spec = chart_of(src);
        assert_eq!(spec.kind, NewChartKind::Doughnut);
        assert_eq!(spec.style.text_position, Some(0.5));
        assert_eq!(spec.style.donut_hole, Some(0.2));
        assert_eq!(spec.style.highlight.as_deref(), Some("Potassium"));
        assert_eq!(spec.style.outer_stroke_width, Some(5.0));
        assert_eq!(spec.style.palette[1], "#123456");
        let svg = chart_svg(&spec);
        assert_eq!(svg.matches(r#"data-role="slice-label""#).count(), 3, "{svg}");
        assert_eq!(svg.matches(r#"data-highlight="1""#).count(), 1);
        assert!(svg.contains(r#"data-role="pie-rim""#) && svg.contains(r#"stroke-width="5.0""#));
        assert!(svg.contains(r##"fill="#123456""##));
        assert!(svg.contains("[50.05]"), "showData puts the value in the legend");
        // A doughnut's slices are rings: two arcs each.
        let ring = svg.split("<path d=\"").nth(1).unwrap();
        assert_eq!(ring.split('"').next().unwrap().matches(" A ").count(), 2, "{ring}");
        // textPosition 0.5 puts the first label at half the radius from the centre.
        let label = tag_with(&svg, r#"data-role="slice-label""#)[0];
        let (x, y) = (attr_f32(&label, "x").unwrap(), attr_f32(&label, "y").unwrap() - 4.0);
        let cy = (3 * 24 + 80).max(250) as f32 / 2.0;
        assert!(((x - 130.0).hypot(y - cy) - 43.0).abs() < 1.0, "{label}");

        let plain = chart_of("pie\n  \"A\" : 1\n  \"B\" : 3\n");
        assert_eq!(plain.kind, NewChartKind::Pie);
        let svg = chart_svg(&plain);
        assert!(svg.contains(">25%<") && svg.contains(">75%<"), "{svg}");
        assert!(!svg.contains("pie-rim") && !svg.contains("data-highlight"));
    }
}

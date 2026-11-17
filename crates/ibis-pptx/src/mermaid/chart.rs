// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! `pie` / `xychart-beta`: native chart specs and their SVG fallback.

use super::*;

pub(super) fn parse_pie_chart(source: &str) -> Option<MermaidChartSpec> {
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

pub(super) fn parse_xy_chart(source: &str) -> Option<MermaidChartSpec> {
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
}

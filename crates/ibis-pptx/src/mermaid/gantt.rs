// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! `gantt`.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GanttStatus {
    Normal,
    Done,
    Active,
    Crit,
    Milestone,
}

pub(super) struct GanttTask {
    pub(super) section: String,
    pub(super) name: String,
    pub(super) status: GanttStatus,
    pub(super) start_day: i64,
    pub(super) end_day: i64,
}

pub(super) fn render_gantt_svg(source: &str) -> Option<(Vec<u8>, u32, u32)> {
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

pub(super) enum GanttAnchor {
    Date(i64),
    After(Vec<String>),
    Follow,
}

pub(super) struct GanttPending {
    pub(super) section: String,
    pub(super) name: String,
    pub(super) status: GanttStatus,
    pub(super) id: Option<String>,
    pub(super) anchor: GanttAnchor,
    pub(super) span_days: i64,
    pub(super) end_date: Option<i64>,
}

pub(super) fn parse_gantt_chart(source: &str) -> Option<(Option<String>, Vec<GanttTask>, bool)> {
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

pub(super) fn gantt_iso(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Read a date with Mermaid `dateFormat` tokens `YYYY`, `MM` and `DD`.
pub(super) fn parse_gantt_date_formatted(text: &str, format: &str) -> Option<i64> {
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

pub(super) fn take_gantt_digits(text: &[u8], ti: &mut usize, n: usize) -> Option<i64> {
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

pub(super) fn parse_gantt_date(s: &str) -> Option<i64> {
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

pub(super) fn parse_gantt_duration_days(s: &str) -> i64 {
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
pub(super) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(super) fn civil_from_days(z: i64) -> (i64, u32, u32) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::test_support::*;

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
}

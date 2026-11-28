//! ジャストシステム 花子 (`.jhd`/`.jhb`)・三四郎 (`.jsd`/`.jst`) レガシー OLE2 (CFB) 形式の判定・内容抽出・Markdown / SVG 出力。

use std::collections::BTreeMap;

use ibis_ooxml::cfb;

use crate::{Error, Opened};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustLegacyKind {
    Hanako,
    Sanshiro,
}

impl JustLegacyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hanako => "ジャストシステム 花子 (.jhd/.jhb)",
            Self::Sanshiro => "ジャストシステム 三四郎 (.jsd/.jst)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct JustStreamInfo {
    pub path: String,
    pub is_storage: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct HanakoTextBox {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum HanakoShape {
    Line {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: String,
        width: f64,
    },
    Polyline {
        points: Vec<(i32, i32)>,
        closed: bool,
        smooth: bool,
        color: String,
        width: f64,
    },
    Ellipse {
        cx: i32,
        cy: i32,
        rx: i32,
        ry: i32,
        start_deg: f64,
        sweep_deg: f64,
        color: String,
        width: f64,
    },
}

#[derive(Debug, Clone)]
pub struct SanshiroCell {
    pub row: u16,
    pub col: u16,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct SanshiroSheet {
    pub name: String,
    pub cells: Vec<SanshiroCell>,
}

#[derive(Debug, Clone)]
pub struct JustLegacyReport {
    pub kind: JustLegacyKind,
    pub metadata: Vec<(String, String)>,
    pub streams: Vec<JustStreamInfo>,
    pub hanako_texts: Vec<HanakoTextBox>,
    pub hanako_shapes: Vec<HanakoShape>,
    pub sanshiro_sheets: Vec<SanshiroSheet>,
}

fn detect_kind_from_entries(data: &[u8], entries: &[cfb::DirectoryEntry]) -> Option<JustLegacyKind> {
    let mut has_fdm = false;
    let mut has_hanako_name = false;
    let mut has_book = false;
    let mut has_worksheet = false;
    let mut has_sanshiro_name = false;

    for e in entries {
        let lower = e.path.to_ascii_lowercase();
        let base = lower.rsplit('/').next().unwrap_or(&lower);
        if base == "fdmtext" || base == "fdmvector" || base == "fdmindex" {
            has_fdm = true;
        }
        if lower.contains("hanako")
            || lower.contains("hnkbody")
            || lower.contains("hnkinfo")
            || lower.contains("jhdinfo")
            || lower.contains("jhbinfo")
        {
            has_hanako_name = true;
        }
        if base == "book" {
            has_book = true;
        }
        if base.starts_with("worksheet") {
            has_worksheet = true;
        }
        if lower.contains("sanshiro")
            || lower.contains("jsdinfo")
            || lower.contains("snsinfo")
            || lower.contains("snsbook")
        {
            has_sanshiro_name = true;
        }
    }

    if has_sanshiro_name || (has_book && has_worksheet) {
        return Some(JustLegacyKind::Sanshiro);
    }
    if has_hanako_name || has_fdm {
        return Some(JustLegacyKind::Hanako);
    }

    if let Ok(summary) = cfb::stream(data, "\u{5}SummaryInformation") {
        let text = decode_cp932(&summary);
        if text.contains("Sanshiro") || text.contains("三四郎") {
            return Some(JustLegacyKind::Sanshiro);
        }
        if text.contains("Hanako") || text.contains("花子") {
            return Some(JustLegacyKind::Hanako);
        }
    }

    None
}

/// バイト列が花子 (`.jhd`/`.jhb`) または三四郎 (`.jsd`) の CFB コンテナかどうかを判定する。
pub fn is_just_legacy(data: &[u8]) -> bool {
    if !cfb::is_cfb(data) {
        return false;
    }
    let Ok(entries) = cfb::directory(data) else {
        return false;
    };
    detect_kind_from_entries(data, &entries).is_some()
}

/// バイト列が花子 (`.jhd`/`.jhb`) かどうかを判定する。
pub fn is_hanako(data: &[u8]) -> bool {
    if !cfb::is_cfb(data) {
        return false;
    }
    let Ok(entries) = cfb::directory(data) else {
        return false;
    };
    detect_kind_from_entries(data, &entries) == Some(JustLegacyKind::Hanako)
}

/// バイト列が三四郎 (`.jsd`/`.jst`) かどうかを判定する。
pub fn is_sanshiro(data: &[u8]) -> bool {
    if !cfb::is_cfb(data) {
        return false;
    }
    let Ok(entries) = cfb::directory(data) else {
        return false;
    };
    detect_kind_from_entries(data, &entries) == Some(JustLegacyKind::Sanshiro)
}

fn maybe_decompress_zlib(bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() >= 2
        && bytes[0] == 0x78
        && matches!(bytes[1], 0x01 | 0x5e | 0x9c | 0xda)
    {
        if let Ok(decompressed) = miniz_oxide::inflate::decompress_to_vec_zlib(&bytes) {
            return decompressed;
        }
    }
    bytes
}

/// 花子 / 三四郎 CFB コンテナを解析してメタデータ・テキスト・図形・表データを抽出する。
pub fn inspect_just_legacy(data: &[u8]) -> Result<JustLegacyReport, Error> {
    if !cfb::is_cfb(data) {
        return Err(Error::new("not an OLE2 compound file"));
    }
    let reader = cfb::Reader::new(data).map_err(|e| Error::new(format!("CFB read error: {e}")))?;
    let entries = reader
        .directory()
        .map_err(|e| Error::new(format!("CFB directory error: {e}")))?;

    let kind = detect_kind_from_entries(data, entries)
        .ok_or_else(|| Error::new("not a JustSystem Hanako/Sanshiro CFB file"))?;

    let mut streams = Vec::with_capacity(entries.len());
    for entry in entries {
        let size = if entry.is_storage {
            0
        } else {
            reader.stream_size(&entry.path).unwrap_or(0)
        };
        streams.push(JustStreamInfo {
            path: entry.path.clone(),
            is_storage: entry.is_storage,
            size,
        });
    }

    let metadata = entries
        .iter()
        .find(|e| !e.is_storage && e.path.ends_with("\u{5}SummaryInformation"))
        .and_then(|e| reader.stream_path(&e.path).ok())
        .map(|bytes| parse_ole_summary_info(&bytes))
        .unwrap_or_default();

    let mut hanako_texts = Vec::new();
    let mut hanako_shapes = Vec::new();
    let mut sanshiro_sheets = Vec::new();

    match kind {
        JustLegacyKind::Hanako => {
            if let Some(entry) = entries.iter().find(|e| !e.is_storage && e.path == "FDMText") {
                if let Ok(bytes) = reader.stream_path(&entry.path) {
                    hanako_texts = parse_fdm_text(&maybe_decompress_zlib(bytes));
                }
            }
            if let Some(entry) = entries.iter().find(|e| !e.is_storage && e.path == "FDMVector") {
                if let Ok(bytes) = reader.stream_path(&entry.path) {
                    hanako_shapes = parse_fdm_vector(&maybe_decompress_zlib(bytes));
                }
            }
        }
        JustLegacyKind::Sanshiro => {
            let sheet_names = entries
                .iter()
                .find(|e| !e.is_storage && e.path == "Book")
                .and_then(|e| reader.stream_path(&e.path).ok())
                .map(|b| parse_sanshiro_book(&maybe_decompress_zlib(b)))
                .unwrap_or_default();

            let mut ws_entries: Vec<(usize, String)> = entries
                .iter()
                .filter(|e| !e.is_storage && e.path.starts_with("WorkSheet"))
                .filter_map(|e| {
                    e.path
                        .strip_prefix("WorkSheet")
                        .and_then(|n| n.parse::<usize>().ok())
                        .map(|idx| (idx, e.path.clone()))
                })
                .collect();
            ws_entries.sort_by_key(|(idx, _)| *idx);

            for (idx, path) in ws_entries {
                if let Ok(bytes) = reader.stream_path(&path) {
                    let cells = parse_sanshiro_worksheet(&maybe_decompress_zlib(bytes));
                    let name = sheet_names
                        .get(idx.saturating_sub(1))
                        .cloned()
                        .unwrap_or_else(|| path.clone());
                    sanshiro_sheets.push(SanshiroSheet { name, cells });
                }
            }
        }
    }

    Ok(JustLegacyReport {
        kind,
        metadata,
        streams,
        hanako_texts,
        hanako_shapes,
        sanshiro_sheets,
    })
}

/// 花子 / 三四郎の抽出結果を Markdown で返す。
pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let report = inspect_just_legacy(data)?;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", report.kind.label()));

    if !report.metadata.is_empty() {
        out.push_str("## 文書情報\n\n");
        for (k, v) in &report.metadata {
            out.push_str(&format!("- **{k}**: {}\n", escape_md(v)));
        }
        out.push('\n');
    }

    match report.kind {
        JustLegacyKind::Hanako => {
            out.push_str(&format!(
                "- **抽出テキスト要素数**: {}\n- **抽出ベクター図形数**: {}\n\n",
                report.hanako_texts.len(),
                report.hanako_shapes.len()
            ));
            if !report.hanako_texts.is_empty() {
                out.push_str("## 抽出テキスト\n\n");
                for (i, tb) in report.hanako_texts.iter().enumerate() {
                    let text = tb.text.replace('\n', " / ");
                    out.push_str(&format!("{}. {}\n", i + 1, escape_md(&text)));
                }
                out.push('\n');
            }
        }
        JustLegacyKind::Sanshiro => {
            for sheet in &report.sanshiro_sheets {
                out.push_str(&format!(
                    "## シート: {} ({} セル)\n\n",
                    escape_md(&sheet.name),
                    sheet.cells.len()
                ));
                if sheet.cells.is_empty() {
                    out.push_str("*（空のシート）*\n\n");
                    continue;
                }
                // 行ごとにセルをまとめる
                let mut by_row: BTreeMap<u16, Vec<(u16, &str)>> = BTreeMap::new();
                for c in &sheet.cells {
                    by_row.entry(c.row).or_default().push((c.col, c.value.as_str()));
                }
                out.push_str("| セル | 内容 |\n|---|---|\n");
                for (row, cols) in by_row {
                    for (col, val) in cols {
                        let a1 = format!("{}{}", col_to_a1(col), row + 1);
                        out.push_str(&format!("| `{}` | {} |\n", a1, escape_md(val)));
                    }
                }
                out.push('\n');
            }
        }
    }

    out.push_str("## CFB ストリーム一覧\n\n");
    out.push_str("| パス | 種別 | サイズ (bytes) |\n|---|---|---:|\n");
    for s in &report.streams {
        let kind_label = if s.is_storage { "Storage" } else { "Stream" };
        let clean_path = s.path.replace(['\x04', '\x05', '\x03', '\x01', '|', '\r', '\n'], " ");
        out.push_str(&format!("| `{}` | {} | {} |\n", clean_path, kind_label, s.size));
    }

    Ok(out)
}

/// 三四郎 (`.jsd`/`.jst`) の最初の非空シートを CSV/TSV 文字列として出力する。
pub fn to_delimited(data: &[u8], delimiter: char) -> Result<String, Error> {
    let report = inspect_just_legacy(data)?;
    if report.kind != JustLegacyKind::Sanshiro {
        return Err(Error::new("CSV/TSV export is only supported for Sanshiro (.jsd/.jst)"));
    }
    let Some(sheet) = report
        .sanshiro_sheets
        .iter()
        .find(|s| !s.cells.is_empty())
        .or_else(|| report.sanshiro_sheets.first())
    else {
        return Ok(String::new());
    };
    if sheet.cells.is_empty() {
        return Ok(String::new());
    }
    let max_row = sheet.cells.iter().map(|c| c.row).max().unwrap_or(0) as usize;
    let max_col = sheet.cells.iter().map(|c| c.col).max().unwrap_or(0) as usize;
    let mut grid = vec![vec![String::new(); max_col + 1]; max_row + 1];
    for c in &sheet.cells {
        grid[c.row as usize][c.col as usize] = c.value.clone();
    }
    let mut out = String::new();
    for row in grid {
        let line: Vec<String> = row
            .into_iter()
            .map(|cell| {
                if cell.contains(delimiter) || cell.contains('"') || cell.contains('\n') || cell.contains('\r') {
                    format!("\"{}\"", cell.replace('"', "\"\""))
                } else {
                    cell
                }
            })
            .collect();
        out.push_str(&line.join(&delimiter.to_string()));
        out.push_str("\r\n");
    }
    Ok(out)
}

/// 三四郎 (`.jsd`/`.jst`) の最初の非空シートを Shift_JIS (CP932) の CSV/TSV バイト列として出力する。
pub fn to_delimited_cp932(data: &[u8], delimiter: char) -> Result<Vec<u8>, Error> {
    let text = to_delimited(data, delimiter)?;
    let (cow, _, _) = encoding_rs::SHIFT_JIS.encode(&text);
    Ok(cow.into_owned())
}

/// 花子 (`.jhd`/`.jhb`) を SVG 変換してプレゼンテーション図形として開く（図形がない場合は Markdown へフォールバック）。
pub fn open_hanako(data: &[u8]) -> Result<Opened, Error> {
    let report = inspect_just_legacy(data)?;
    if report.kind != JustLegacyKind::Hanako {
        return Err(Error::new("not a Hanako (.jhd/.jhb) file"));
    }
    if report.hanako_shapes.is_empty() && report.hanako_texts.is_empty() {
        let md = to_markdown(data)?;
        return crate::markdown::open_markdown(md.into_bytes());
    }
    let svg = to_svg(data)?;
    let mut opened = crate::save::open(crate::blank::blank_file(crate::Kind::Presentation)?)?;
    let slide_size = opened.presentation.slide_size;
    let frame = ibis_doc_model::EmuRect {
        x: 0,
        y: 0,
        width: slide_size.width,
        height: slide_size.height,
    };
    let mut next_id = 1000u64;
    let shapes = crate::svg_convert::convert_svg_to_shapes(
        || {
            let id = next_id;
            next_id += 1;
            ibis_doc_model::ShapeId(id)
        },
        svg.as_bytes(),
        frame,
    );
    match shapes {
        Ok(shapes) if !shapes.is_empty() => {
            if let Some(slide) = opened.presentation.slides.first_mut() {
                slide.shapes = shapes;
                if let Ok(md) = to_markdown(data) {
                    slide.notes = md;
                }
            }
            Ok(opened)
        }
        _ => {
            let md = to_markdown(data)?;
            crate::markdown::open_markdown(md.into_bytes())
        }
    }
}

/// 三四郎 (`.jsd`/`.jst`) を全シート付きのマルチシート Workbook として開く。
pub fn open_sanshiro(data: &[u8]) -> Result<Opened, Error> {
    let report = inspect_just_legacy(data)?;
    if report.kind != JustLegacyKind::Sanshiro {
        return Err(Error::new("not a Sanshiro (.jsd/.jst) file"));
    }
    if report.sanshiro_sheets.is_empty() {
        let md = to_markdown(data)?;
        return crate::markdown::open_markdown(md.into_bytes());
    }

    let mut wb_md = String::new();
    for (idx, sheet) in report.sanshiro_sheets.iter().enumerate() {
        let safe_name = sheet.name.replace('"', "'");
        wb_md.push_str(&format!("<!-- xlsx-sheet {{\"name\":\"{safe_name}\"}} -->\n\n"));
        if sheet.cells.is_empty() {
            wb_md.push_str("| A |\n| --- |\n| |\n\n");
            continue;
        }
        let max_row = sheet.cells.iter().map(|c| c.row).max().unwrap_or(0) as usize;
        let max_col = sheet.cells.iter().map(|c| c.col).max().unwrap_or(0) as usize;
        if max_col > 64 || max_row > 1000 {
            // 巨大シートの場合は先頭シート CSV フォールバック
            if idx == 0 {
                let csv = to_delimited(data, ',')?;
                return crate::save::open(csv.into_bytes());
            }
            wb_md.push_str("| セル | 内容 |\n| --- | --- |\n");
            for c in sheet.cells.iter().take(500) {
                let a1 = format!("{}{}", col_to_a1(c.col), c.row + 1);
                wb_md.push_str(&format!("| `{}` | {} |\n", a1, escape_md(&c.value)));
            }
            wb_md.push('\n');
            continue;
        }
        let mut grid = vec![vec![String::new(); max_col + 1]; max_row + 1];
        for c in &sheet.cells {
            grid[c.row as usize][c.col as usize] = escape_md(&c.value);
        }
        let header: Vec<String> = (0..=max_col).map(|c| col_to_a1(c as u16)).collect();
        wb_md.push_str(&format!("| {} |\n", header.join(" | ")));
        wb_md.push_str(&format!("| {} |\n", vec!["---"; max_col + 1].join(" | ")));
        for row in grid {
            wb_md.push_str(&format!("| {} |\n", row.join(" | ")));
        }
        wb_md.push('\n');
    }

    crate::markdown::open_markdown_workbook(wb_md.into_bytes()).or_else(|_| {
        let md = to_markdown(data)?;
        crate::markdown::open_markdown(md.into_bytes())
    })
}

/// 花子 (`.jhd`/`.jhb`) のベクター図形・テキストを SVG 文字列として出力する。
pub fn to_svg(data: &[u8]) -> Result<String, Error> {
    let report = inspect_just_legacy(data)?;
    if report.kind != JustLegacyKind::Hanako {
        return Err(Error::new("SVG export is only supported for Hanako (.jhd/.jhb)"));
    }

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let mut expand = |x: i32, y: i32| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };

    for s in &report.hanako_shapes {
        match s {
            HanakoShape::Line { x1, y1, x2, y2, .. } => {
                expand(*x1, *y1);
                expand(*x2, *y2);
            }
            HanakoShape::Polyline { points, .. } => {
                for &(x, y) in points {
                    expand(x, y);
                }
            }
            HanakoShape::Ellipse { cx, cy, rx, ry, .. } => {
                expand(cx - rx, cy - ry);
                expand(cx + rx, cy + ry);
            }
        }
    }
    for tb in &report.hanako_texts {
        expand(tb.left, tb.top);
        expand(tb.right, tb.bottom);
    }

    if min_x == i32::MAX {
        return Ok(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 600"></svg>"#.to_string(),
        );
    }

    let raw_w = (max_x - min_x).max(1) as f64;
    let raw_h = (max_y - min_y).max(1) as f64;
    let scale = (1000.0 / raw_w.max(raw_h)).max(1e-6);
    let vw = raw_w * scale + 40.0;
    let vh = raw_h * scale + 40.0;
    let tx = |x: i32| (x - min_x) as f64 * scale + 20.0;
    let ty = |y: i32| (y - min_y) as f64 * scale + 20.0;

    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {vw:.2} {vh:.2}\" width=\"{vw:.0}\" height=\"{vh:.0}\">\n\
         <rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n"
    );

    for s in &report.hanako_shapes {
        match s {
            HanakoShape::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                width,
            } => {
                out.push_str(&format!(
                    "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{color}\" stroke-width=\"{width:.2}\"/>\n",
                    tx(*x1), ty(*y1), tx(*x2), ty(*y2)
                ));
            }
            HanakoShape::Polyline {
                points,
                closed,
                smooth,
                color,
                width,
            } => {
                if *smooth && points.len() >= 3 && !*closed {
                    let pts: Vec<(f64, f64)> = points.iter().map(|&(x, y)| (tx(x), ty(y))).collect();
                    let mut d = format!("M {:.2} {:.2}", pts[0].0, pts[0].1);
                    for i in 0..pts.len() - 1 {
                        let p0 = if i == 0 { pts[0] } else { pts[i - 1] };
                        let p1 = pts[i];
                        let p2 = pts[i + 1];
                        let p3 = if i + 2 < pts.len() { pts[i + 2] } else { pts[i + 1] };
                        let c1x = p1.0 + (p2.0 - p0.0) / 6.0;
                        let c1y = p1.1 + (p2.1 - p0.1) / 6.0;
                        let c2x = p2.0 - (p3.0 - p1.0) / 6.0;
                        let c2y = p2.1 - (p3.1 - p1.1) / 6.0;
                        d.push_str(&format!(
                            " C {c1x:.2} {c1y:.2}, {c2x:.2} {c2y:.2}, {:.2} {:.2}",
                            p2.0, p2.1
                        ));
                    }
                    out.push_str(&format!(
                        "<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{width:.2}\"/>\n"
                    ));
                } else if points.len() >= 2 {
                    let pts: Vec<String> = points
                        .iter()
                        .map(|&(x, y)| format!("{:.2},{:.2}", tx(x), ty(y)))
                        .collect();
                    let tag = if *closed { "polygon" } else { "polyline" };
                    out.push_str(&format!(
                        "<{tag} points=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{width:.2}\"/>\n",
                        pts.join(" ")
                    ));
                }
            }
            HanakoShape::Ellipse {
                cx,
                cy,
                rx,
                ry,
                start_deg,
                sweep_deg,
                color,
                width,
            } => {
                let srx = (*rx as f64 * scale).max(0.5);
                let sry = (*ry as f64 * scale).max(0.5);
                let scx = tx(*cx);
                let scy = ty(*cy);
                if *sweep_deg <= 0.1 || *sweep_deg >= 359.5 {
                    out.push_str(&format!(
                        "<ellipse cx=\"{scx:.2}\" cy=\"{scy:.2}\" rx=\"{srx:.2}\" ry=\"{sry:.2}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{width:.2}\"/>\n"
                    ));
                } else {
                    let a0 = start_deg.to_radians();
                    let a1 = (start_deg + sweep_deg).to_radians();
                    let x0 = scx + srx * a0.cos();
                    let y0 = scy + sry * a0.sin();
                    let x1 = scx + srx * a1.cos();
                    let y1 = scy + sry * a1.sin();
                    let large = if *sweep_deg > 180.0 { 1 } else { 0 };
                    out.push_str(&format!(
                        "<path d=\"M {x0:.2} {y0:.2} A {srx:.2} {sry:.2} 0 {large} 1 {x1:.2} {y1:.2}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{width:.2}\"/>\n"
                    ));
                }
            }
        }
    }

    for tb in &report.hanako_texts {
        let lines: Vec<&str> = tb.text.lines().collect();
        if lines.is_empty() {
            continue;
        }
        let line_count = lines.len().max(1) as f64;
        let box_h = (tb.bottom - tb.top).abs() as f64 * scale;
        let font_sz = (box_h / (line_count * 1.35)).clamp(8.5, 16.0);
        let x = tx(tb.left);
        let y0 = ty(tb.top) + font_sz;
        out.push_str(&format!(
            "<text x=\"{x:.2}\" y=\"{y0:.2}\" font-size=\"{font_sz:.1}\" font-family=\"sans-serif\" fill=\"#111\">"
        ));
        for (idx, line) in lines.iter().enumerate() {
            if idx == 0 {
                out.push_str(&escape_xml(line));
            } else {
                out.push_str(&format!(
                    "<tspan x=\"{x:.2}\" dy=\"{:.1}\">{}</tspan>",
                    font_sz * 1.3,
                    escape_xml(line)
                ));
            }
        }
        out.push_str("</text>\n");
    }

    out.push_str("</svg>\n");
    Ok(out)
}

fn col_to_a1(mut col: u16) -> String {
    let mut chars = Vec::new();
    loop {
        chars.push((b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    chars.into_iter().rev().collect()
}

fn escape_md(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn decode_cp932(bytes: &[u8]) -> String {
    let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
    cow.into_owned()
}

fn parse_ole_summary_info(data: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if data.len() < 48 {
        return out;
    }
    let num_sections = u32::from_le_bytes(data[24..28].try_into().unwrap_or([0; 4])) as usize;
    if num_sections == 0 || data.len() < 48 {
        return out;
    }
    let sec_offset = u32::from_le_bytes(data[44..48].try_into().unwrap_or([0; 4])) as usize;
    if sec_offset + 8 > data.len() {
        return out;
    }
    let num_props =
        u32::from_le_bytes(data[sec_offset + 4..sec_offset + 8].try_into().unwrap_or([0; 4]))
            as usize;
    let table_start = sec_offset + 8;
    if table_start + num_props * 8 > data.len() {
        return out;
    }

    for i in 0..num_props.min(64) {
        let p = table_start + i * 8;
        let pid = u32::from_le_bytes(data[p..p + 4].try_into().unwrap_or([0; 4]));
        let rel_off = u32::from_le_bytes(data[p + 4..p + 8].try_into().unwrap_or([0; 4])) as usize;
        let abs_off = sec_offset + rel_off;
        if abs_off + 8 > data.len() {
            continue;
        }
        let vtype = u32::from_le_bytes(data[abs_off..abs_off + 4].try_into().unwrap_or([0; 4]));
        let label = match pid {
            2 => "タイトル",
            3 => "件名",
            4 => "作成者",
            6 => "コメント",
            8 => "最終保存者",
            18 => "作成アプリ",
            _ => continue,
        };
        let val = match vtype {
            0x1e => {
                // VT_LPSTR (byte length)
                let slen =
                    u32::from_le_bytes(data[abs_off + 4..abs_off + 8].try_into().unwrap_or([0; 4]))
                        as usize;
                let end = (abs_off + 8 + slen).min(data.len());
                let raw = &data[abs_off + 8..end];
                let trimmed = raw.split(|&b| b == 0).next().unwrap_or(raw);
                decode_cp932(trimmed).trim().to_string()
            }
            0x1f => {
                // VT_LPWSTR (char count)
                let clen =
                    u32::from_le_bytes(data[abs_off + 4..abs_off + 8].try_into().unwrap_or([0; 4]))
                        as usize;
                let end = (abs_off + 8 + clen * 2).min(data.len());
                let mut u16s = Vec::new();
                for ch in data[abs_off + 8..end].chunks_exact(2) {
                    let w = u16::from_le_bytes([ch[0], ch[1]]);
                    if w == 0 {
                        break;
                    }
                    u16s.push(w);
                }
                String::from_utf16_lossy(&u16s).trim().to_string()
            }
            _ => continue,
        };
        if !val.is_empty() {
            out.push((label.to_string(), val));
        }
    }
    out
}

fn parse_fdm_text(data: &[u8]) -> Vec<HanakoTextBox> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let rlen = u32::from_be_bytes(data[pos + 4..pos + 8].try_into().unwrap_or([0; 4])) as usize;
        if rlen < 24 || pos + rlen > data.len() {
            break;
        }
        let rec = &data[pos..pos + rlen];
        let left = i32::from_be_bytes(rec[8..12].try_into().unwrap_or([0; 4]));
        let top = i32::from_be_bytes(rec[12..16].try_into().unwrap_or([0; 4]));
        let right = i32::from_be_bytes(rec[16..20].try_into().unwrap_or([0; 4]));
        let bottom = i32::from_be_bytes(rec[20..24].try_into().unwrap_or([0; 4]));

        if let Some(marker_pos) = rec.windows(4).position(|w| w == [0x00, 0x00, 0x00, 0x0d]) {
            let mut s = marker_pos + 4;
            let mut u16s = Vec::new();
            while s + 2 <= rec.len() {
                let ch = u16::from_be_bytes([rec[s], rec[s + 1]]);
                if ch == 0x0000 || ch == 0x0001 || ch == 0x0002 {
                    break;
                }
                if ch == 0x000d {
                    u16s.push(b'\n' as u16);
                } else {
                    u16s.push(ch);
                }
                s += 2;
            }
            let text = String::from_utf16_lossy(&u16s).trim().to_string();
            if !text.is_empty() {
                out.push(HanakoTextBox {
                    left,
                    top,
                    right,
                    bottom,
                    text,
                });
            }
        }
        pos += rlen;
    }
    out
}

fn parse_fdm_vector(data: &[u8]) -> Vec<HanakoShape> {
    let mut shapes = Vec::new();
    let mut pos = 0usize;
    while pos + 6 <= data.len() {
        let kind = data[pos + 2];
        let rlen = u16::from_be_bytes([data[pos + 4], data[pos + 5]]) as usize;
        if rlen < 6 || pos + rlen > data.len() {
            break;
        }
        let rec = &data[pos..pos + rlen];
        parse_vector_record(kind, rec, &mut shapes);
        pos += rlen;
    }
    shapes
}

fn extract_vector_style(rec: &[u8]) -> (String, f64) {
    let raw_w = if rec.len() >= 10 {
        u16::from_be_bytes([rec[8], rec[9]])
    } else {
        8
    };
    let width = if raw_w == 0 {
        1.0
    } else {
        (raw_w as f64 / 8.0).clamp(0.8, 4.0)
    };
    let color = if rec.len() >= 16 && rec[12] == 0x00 && (rec[13] != 0 || rec[14] != 0 || rec[15] != 0) {
        // 00 BB GG RR (Windows COLORREF) or 00 RR GG BB
        format!("#{:02x}{:02x}{:02x}", rec[15], rec[14], rec[13])
    } else {
        "#222222".to_string()
    };
    (color, width)
}

fn parse_vector_record(kind: u8, rec: &[u8], out: &mut Vec<HanakoShape>) {
    let (color, width) = extract_vector_style(rec);
    match kind {
        0x01 if rec.len() >= 32 => {
            let x1 = i32::from_be_bytes(rec[16..20].try_into().unwrap_or([0; 4]));
            let y1 = i32::from_be_bytes(rec[20..24].try_into().unwrap_or([0; 4]));
            let x2 = i32::from_be_bytes(rec[24..28].try_into().unwrap_or([0; 4]));
            let y2 = i32::from_be_bytes(rec[28..32].try_into().unwrap_or([0; 4]));
            out.push(HanakoShape::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                width,
            });
        }
        0x04 if rec.len() >= 28 => {
            let cx = i32::from_be_bytes(rec[16..20].try_into().unwrap_or([0; 4]));
            let cy = i32::from_be_bytes(rec[20..24].try_into().unwrap_or([0; 4]));
            let rx = u16::from_be_bytes([rec[24], rec[25]]) as i32;
            let ry = u16::from_be_bytes([rec[26], rec[27]]) as i32;
            let (start_deg, sweep_deg) = if rec.len() >= 32 {
                let s = u16::from_be_bytes([rec[28], rec[29]]) as f64 / 256.0;
                let sw = u16::from_be_bytes([rec[30], rec[31]]) as f64 / 256.0;
                (s, sw)
            } else {
                (0.0, 360.0)
            };
            out.push(HanakoShape::Ellipse {
                cx,
                cy,
                rx,
                ry,
                start_deg,
                sweep_deg,
                color,
                width,
            });
        }
        0x05 | 0x06 | 0x09 if rec.len() >= 18 => {
            let count = u16::from_be_bytes([rec[16], rec[17]]) as usize;
            let mut pts = Vec::with_capacity(count);
            let mut off = 18usize;
            for _ in 0..count {
                if off + 8 > rec.len() {
                    break;
                }
                let x = i32::from_be_bytes(rec[off..off + 4].try_into().unwrap_or([0; 4]));
                let y = i32::from_be_bytes(rec[off + 4..off + 8].try_into().unwrap_or([0; 4]));
                pts.push((x, y));
                off += 8;
            }
            if pts.len() >= 2 {
                let closed = kind == 0x06 && pts.first() == pts.last();
                out.push(HanakoShape::Polyline {
                    points: pts,
                    closed,
                    smooth: matches!(kind, 0x05 | 0x09),
                    color,
                    width,
                });
            }
        }
        0x0a | 0x0b => {
            // グループ／複合図形内のサブレコード (ff 00 <kind> 60 <len_be>) を探索
            let mut i = 32usize;
            while i + 6 <= rec.len() {
                if (rec[i] == 0xff || rec[i] == 0x01) && rec[i + 1] == 0x00 && rec[i + 3] == 0x60 {
                    let sub_kind = rec[i + 2];
                    let sub_len = u16::from_be_bytes([rec[i + 4], rec[i + 5]]) as usize;
                    if sub_len >= 6 && i + sub_len <= rec.len() {
                        parse_vector_record(sub_kind, &rec[i..i + sub_len], out);
                        i += sub_len;
                        continue;
                    }
                }
                i += 1;
            }
        }
        _ => {}
    }
}

fn parse_sanshiro_book(data: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    // 先頭 2 バイト (ff ff) をスキップして TLV 走査
    let mut pos = if data.starts_with(&[0xff, 0xff]) { 2 } else { 0 };
    while pos + 4 <= data.len() {
        let tag = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let len = u16::from_le_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if len < 4 || pos + len > data.len() {
            break;
        }
        if tag == 0x000a && len >= 12 {
            // +10 から null 終端 UTF-16LE シート名
            let mut u16s = Vec::new();
            for ch in data[pos + 10..pos + len].chunks_exact(2) {
                let w = u16::from_le_bytes([ch[0], ch[1]]);
                if w == 0 {
                    break;
                }
                u16s.push(w);
            }
            let name = String::from_utf16_lossy(&u16s).trim().to_string();
            if !name.is_empty() {
                names.push(name);
            }
        }
        pos += len;
    }
    names
}

fn parse_sanshiro_worksheet(data: &[u8]) -> Vec<SanshiroCell> {
    let mut cells = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        let tag = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let len = u16::from_le_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if len < 4 || pos + len > data.len() {
            break;
        }
        let body = &data[pos + 4..pos + len];
        if tag == 0x1000 && body.len() >= 8 {
            let row = u16::from_le_bytes([body[4], body[5]]);
            let col = u16::from_le_bytes([body[6], body[7]]);
            let mut sp = 8usize;
            while sp + 4 <= body.len() {
                let st = u16::from_le_bytes([body[sp], body[sp + 1]]);
                let sl = u16::from_le_bytes([body[sp + 2], body[sp + 3]]) as usize;
                if sl < 4 || sp + sl > body.len() {
                    break;
                }
                if st == 0x0001 && sl >= 8 {
                    let mut u16s = Vec::new();
                    for ch in body[sp + 6..sp + sl].chunks_exact(2) {
                        let w = u16::from_le_bytes([ch[0], ch[1]]);
                        if w == 0 {
                            break;
                        }
                        u16s.push(w);
                    }
                    let val = String::from_utf16_lossy(&u16s);
                    if !val.is_empty() {
                        cells.push(SanshiroCell {
                            row,
                            col,
                            value: val,
                        });
                    }
                } else if st == 0x0002 && sl >= 12 {
                    let num = f64::from_le_bytes(
                        body[sp + 4..sp + 12].try_into().unwrap_or([0; 8]),
                    );
                    if num.is_finite() {
                        cells.push(SanshiroCell {
                            row,
                            col,
                            value: num.to_string(),
                        });
                    }
                }
                sp += sl;
            }
        }
        pos += len;
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hanako_text_rec(text: &str) -> Vec<u8> {
        let mut rec = vec![0u8; 40];
        rec[0..4].copy_from_slice(&[0x01, 0x00, 0x16, 0x60]);
        // bbox: (100, 200, 500, 400)
        rec[8..12].copy_from_slice(&100i32.to_be_bytes());
        rec[12..16].copy_from_slice(&200i32.to_be_bytes());
        rec[16..20].copy_from_slice(&500i32.to_be_bytes());
        rec[20..24].copy_from_slice(&400i32.to_be_bytes());
        rec.extend_from_slice(&[0x00, 0x00, 0x00, 0x0d]);
        for u in text.encode_utf16() {
            rec.extend_from_slice(&u.to_be_bytes());
        }
        rec.extend_from_slice(&[0x00, 0x0d, 0x00, 0x01]);
        let rlen = rec.len() as u32;
        rec[4..8].copy_from_slice(&rlen.to_be_bytes());
        rec
    }

    fn make_hanako_line_rec(x1: i32, y1: i32, x2: i32, y2: i32) -> Vec<u8> {
        let mut rec = vec![0u8; 32];
        rec[0..4].copy_from_slice(&[0x01, 0x00, 0x01, 0x60]);
        rec[4..6].copy_from_slice(&32u16.to_be_bytes());
        rec[16..20].copy_from_slice(&x1.to_be_bytes());
        rec[20..24].copy_from_slice(&y1.to_be_bytes());
        rec[24..28].copy_from_slice(&x2.to_be_bytes());
        rec[28..32].copy_from_slice(&y2.to_be_bytes());
        rec
    }

    #[test]
    fn extracts_hanako_text_and_vector_to_md_and_svg() {
        let cfb_bytes = cfb::write(&[
            cfb::Item::stream("FDMText", make_hanako_text_rec("斜視図サンプル")),
            cfb::Item::stream("FDMVector", make_hanako_line_rec(100, 200, 500, 400)),
        ]);
        assert!(is_just_legacy(&cfb_bytes));
        assert!(is_hanako(&cfb_bytes));

        let md = to_markdown(&cfb_bytes).unwrap();
        assert!(md.contains("花子"));
        assert!(md.contains("斜視図サンプル"));

        let svg = to_svg(&cfb_bytes).unwrap();
        assert!(svg.contains("<line"));
        assert!(svg.contains("斜視図サンプル"));
    }

    #[test]
    fn extracts_sanshiro_sheet_names_and_cells_to_md() {
        // Book TLV: ff ff + tag 0x000a
        let mut book = vec![0xff, 0xff];
        let mut sheet_rec = vec![0u8; 10];
        sheet_rec[0..2].copy_from_slice(&0x000au16.to_le_bytes());
        for u in "集計表".encode_utf16() {
            sheet_rec.extend_from_slice(&u.to_le_bytes());
        }
        sheet_rec.extend_from_slice(&[0, 0]);
        let slen = sheet_rec.len() as u16;
        sheet_rec[2..4].copy_from_slice(&slen.to_le_bytes());
        book.extend_from_slice(&sheet_rec);

        // WorkSheet1 TLV: tag 0x1000 cell at (row=1, col=2) with subtag 0x0001
        let mut sub = vec![0u8; 6];
        sub[0..2].copy_from_slice(&0x0001u16.to_le_bytes());
        sub[4..6].copy_from_slice(&0x0027u16.to_le_bytes());
        for u in "東京都庁".encode_utf16() {
            sub.extend_from_slice(&u.to_le_bytes());
        }
        sub.extend_from_slice(&[0, 0]);
        let sub_len = sub.len() as u16;
        sub[2..4].copy_from_slice(&sub_len.to_le_bytes());

        let mut ws = vec![0u8; 12];
        ws[0..2].copy_from_slice(&0x1000u16.to_le_bytes());
        ws[4..8].copy_from_slice(&5u32.to_le_bytes());
        ws[8..10].copy_from_slice(&1u16.to_le_bytes()); // row 1 -> row 2 in A1
        ws[10..12].copy_from_slice(&2u16.to_le_bytes()); // col 2 -> C
        ws.extend_from_slice(&sub);
        let ws_len = ws.len() as u16;
        ws[2..4].copy_from_slice(&ws_len.to_le_bytes());

        let cfb_bytes = cfb::write(&[
            cfb::Item::stream("Book", book),
            cfb::Item::stream("WorkSheet1", ws),
        ]);
        assert!(is_just_legacy(&cfb_bytes));
        assert!(is_sanshiro(&cfb_bytes));
        assert!(!is_hanako(&cfb_bytes));

        let md = to_markdown(&cfb_bytes).unwrap();
        assert!(md.contains("三四郎"));
        assert!(md.contains("集計表"));
        assert!(md.contains("`C2`"));
        assert!(md.contains("東京都庁"));

        let csv = to_delimited(&cfb_bytes, ',').unwrap();
        assert!(csv.contains(",,東京都庁"));
    }
}

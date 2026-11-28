// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust 2D CAD reader and SVG/Markdown exporter for Japanese public-sector
//! electronic delivery formats: DXF, SXF (`.sfc` / `.p21`), and Jw_cad (`.jww`).

use std::collections::BTreeSet;

use crate::{Error, Opened};

#[derive(Clone, Debug, PartialEq)]
pub enum CadEntity {
    Line {
        layer: String,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    Circle {
        layer: String,
        cx: f64,
        cy: f64,
        r: f64,
    },
    Arc {
        layer: String,
        cx: f64,
        cy: f64,
        r: f64,
        start_deg: f64,
        end_deg: f64,
    },
    Polyline {
        layer: String,
        points: Vec<(f64, f64)>,
        closed: bool,
    },
    Text {
        layer: String,
        x: f64,
        y: f64,
        height: f64,
        text: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct CadDrawing {
    pub format: String,
    pub title: String,
    pub entities: Vec<CadEntity>,
    pub layers: BTreeSet<String>,
}

pub fn is_jww(data: &[u8]) -> bool {
    data.starts_with(b"JwwData.")
}

pub fn is_sxf_p21(data: &[u8]) -> bool {
    let head = decode_cad_text(&data[..data.len().min(2048)]);
    head.contains("ISO-10303-21;") && (head.contains("SXF") || head.contains("SCADEC") || head.contains("AP202"))
}

pub fn is_sxf_sfc(data: &[u8]) -> bool {
    let head = decode_cad_text(&data[..data.len().min(4096)]);
    head.contains("SXF") && (head.contains("drawing_sheet_feature") || head.contains("line_feature") || head.contains("SCADEC"))
}

pub fn is_dxf(data: &[u8]) -> bool {
    let head = decode_cad_text(&data[..data.len().min(4096)]);
    head.contains("SECTION") && (head.contains("ENTITIES") || head.contains("HEADER"))
}

pub fn is_cad(data: &[u8]) -> bool {
    is_jww(data) || is_sxf_sfc(data) || is_sxf_p21(data) || is_dxf(data)
}

fn decode_cad_text(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(s) => s.strip_prefix('\u{FEFF}').unwrap_or(s).to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(data);
            cow.into_owned()
        }
    }
}

pub fn parse(data: &[u8]) -> Result<CadDrawing, Error> {
    if is_jww(data) {
        return Ok(parse_jww(data)?);
    }
    if is_sxf_sfc(data) {
        return Ok(parse_sxf_sfc(&decode_cad_text(data)));
    }
    if is_sxf_p21(data) {
        return Ok(parse_sxf_p21(&decode_cad_text(data)));
    }
    if is_dxf(data) {
        return Ok(parse_dxf(&decode_cad_text(data)));
    }
    Err(Error::new("unsupported CAD format (expected DXF, SXF .sfc/.p21, or Jw_cad .jww)"))
}

fn read_jww_cstring(data: &[u8], pos: usize) -> Option<(usize, &[u8])> {
    let b0 = *data.get(pos)?;
    if b0 == 0xff {
        if pos + 3 > data.len() {
            return None;
        }
        let n = u16::from_le_bytes([data[pos + 1], data[pos + 2]]) as usize;
        if pos + 3 + n > data.len() {
            return None;
        }
        Some((pos + 3 + n, &data[pos + 3..pos + 3 + n]))
    } else {
        let n = b0 as usize;
        if pos + 1 + n > data.len() {
            return None;
        }
        Some((pos + 1 + n, &data[pos + 1..pos + 1 + n]))
    }
}

fn jww_f64(data: &[u8], pos: usize) -> f64 {
    f64::from_le_bytes(data[pos..pos + 8].try_into().unwrap_or([0; 8]))
}

fn parse_jww(data: &[u8]) -> Result<CadDrawing, Error> {
    if data.len() < 12 {
        return Err(Error::new("truncated Jw_cad .jww file"));
    }
    let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
    let mut drawing = CadDrawing {
        format: format!("Jw_cad (.jww, version {version})"),
        title: "Jw_cad Drawing".into(),
        entities: Vec::new(),
        layers: BTreeSet::new(),
    };

    let mut i = 12usize;
    let Some((next_i, memo_bytes)) = read_jww_cstring(data, i) else {
        return Ok(drawing);
    };
    let (memo_cow, _, _) = encoding_rs::SHIFT_JIS.decode(memo_bytes);
    let memo = memo_cow.trim().to_string();
    if !memo.is_empty() {
        drawing.title = memo;
    }
    i = next_i;

    // Skip JWW variable-length header up to entity count
    let skip_header = (|| -> Option<usize> {
        let mut p = i.checked_add(8)?; // m_nZumen
        for _ in 0..16 {
            p = p.checked_add(20 + 16 * 8)?;
        }
        p = p.checked_add(14 * 4 + 100)?;
        for _ in 0..16 {
            for _ in 0..16 {
                p = read_jww_cstring(data, p)?.0;
            }
        }
        for _ in 0..16 {
            p = read_jww_cstring(data, p)?.0;
        }
        p = p.checked_add(96)?;
        if version >= 300 {
            p = p.checked_add(8 * 28 + 44 + 12)?;
        } else {
            p = p.checked_add(4 * 24)?;
        }
        p = p.checked_add(10 * 8 + 8 + 10 * 8 + 10 * 16 + 8 * 16 + 5 * 20 + 4 * 16 + 144)?;
        p = p.checked_add(257 * 8)?;
        for _ in 0..257 {
            p = read_jww_cstring(data, p)?.0;
            p = p.checked_add(16)?;
        }
        p = p.checked_add(33 * 16)?;
        for _ in 0..33 {
            p = read_jww_cstring(data, p)?.0;
            p = p.checked_add(4 + 10 * 8)?;
        }
        p = p.checked_add(10 * 28 + 100)?;
        Some(p)
    })();

    let Some(mut p) = skip_header else {
        return Ok(drawing);
    };
    if p + 2 > data.len() {
        return Ok(drawing);
    }

    let cnt16 = u16::from_le_bytes([data[p], data[p + 1]]);
    let n_ent = if cnt16 == 0xFFFF {
        if p + 6 > data.len() {
            return Ok(drawing);
        }
        let n = u32::from_le_bytes(data[p + 2..p + 6].try_into().unwrap()) as usize;
        p += 6;
        n
    } else {
        p += 2;
        cnt16 as usize
    };

    let mut classes: Vec<String> = Vec::new();
    let mut id_map: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut next_class_id = 0usize;

    for no in 1..=n_ent {
        if p + 2 > data.len() {
            break;
        }
        let tag = u16::from_le_bytes([data[p], data[p + 1]]);
        let cname = if tag == 0xFFFF {
            if p + 6 > data.len() {
                break;
            }
            let nlen = u16::from_le_bytes([data[p + 4], data[p + 5]]) as usize;
            if p + 6 + nlen > data.len() {
                break;
            }
            let name = String::from_utf8_lossy(&data[p + 6..p + 6 + nlen]).into_owned();
            p += 6 + nlen;
            let idx = classes.len();
            classes.push(name);
            id_map.insert(no + next_class_id, idx);
            next_class_id += 1;
            classes[idx].as_str()
        } else if tag == 0x7FFF {
            if p + 6 > data.len() {
                break;
            }
            let cid = (u32::from_le_bytes(data[p + 2..p + 6].try_into().unwrap()) & 0x7FFF_FFFF) as usize;
            p += 6;
            let Some(&idx) = id_map.get(&cid) else { break };
            classes[idx].as_str()
        } else {
            let cid = (tag & 0x7FFF) as usize;
            p += 2;
            let Some(&idx) = id_map.get(&cid) else { break };
            classes[idx].as_str()
        };

        if p + 15 > data.len() {
            break;
        }
        let ly = u16::from_le_bytes([data[p + 9], data[p + 10]]);
        let lg = u16::from_le_bytes([data[p + 11], data[p + 12]]);
        let layer = format!("G{lg:X}-L{ly:X}");
        drawing.layers.insert(layer.clone());

        match cname {
            "CDataSen" => {
                if p + 47 > data.len() {
                    break;
                }
                let x1 = jww_f64(data, p + 15);
                let y1 = jww_f64(data, p + 23);
                let x2 = jww_f64(data, p + 31);
                let y2 = jww_f64(data, p + 39);
                drawing.entities.push(CadEntity::Line {
                    layer,
                    x1,
                    y1,
                    x2,
                    y2,
                });
                p += 47;
            }
            "CDataEnko" => {
                if p + 75 > data.len() {
                    break;
                }
                let cx = jww_f64(data, p + 15);
                let cy = jww_f64(data, p + 23);
                let r = jww_f64(data, p + 31).abs();
                let start_rad = jww_f64(data, p + 39);
                let sweep_rad = jww_f64(data, p + 47);
                let zenen = u32::from_le_bytes(data[p + 71..p + 75].try_into().unwrap());
                if zenen != 0 || sweep_rad.abs() >= std::f64::consts::TAU - 1e-4 {
                    drawing.entities.push(CadEntity::Circle { layer, cx, cy, r });
                } else {
                    drawing.entities.push(CadEntity::Arc {
                        layer,
                        cx,
                        cy,
                        r,
                        start_deg: start_rad.to_degrees(),
                        end_deg: (start_rad + sweep_rad).to_degrees(),
                    });
                }
                p += 75;
            }
            "CDataMoji" => {
                if p + 84 > data.len() {
                    break;
                }
                let x = jww_f64(data, p + 15);
                let y = jww_f64(data, p + 23);
                let height = jww_f64(data, p + 59).abs().max(1.5);
                let Some((p_font, _)) = read_jww_cstring(data, p + 83) else {
                    break;
                };
                let Some((p_end, text_bytes)) = read_jww_cstring(data, p_font) else {
                    break;
                };
                let (txt_cow, _, _) = encoding_rs::SHIFT_JIS.decode(text_bytes);
                let text = txt_cow.trim().to_string();
                if !text.is_empty() {
                    drawing.entities.push(CadEntity::Text {
                        layer,
                        x,
                        y,
                        height,
                        text,
                    });
                }
                p = p_end;
            }
            "CDataSolid" => {
                let sz = if data[p + 5] == 10 { 83 } else { 79 };
                if p + sz > data.len() {
                    break;
                }
                let points = vec![
                    (jww_f64(data, p + 15), jww_f64(data, p + 23)),
                    (jww_f64(data, p + 31), jww_f64(data, p + 39)),
                    (jww_f64(data, p + 47), jww_f64(data, p + 55)),
                    (jww_f64(data, p + 63), jww_f64(data, p + 71)),
                ];
                drawing.entities.push(CadEntity::Polyline {
                    layer,
                    points,
                    closed: true,
                });
                p += sz;
            }
            "CDataTen" => {
                let sz = if data[p + 4] == 100 { 55 } else { 35 };
                if p + sz > data.len() {
                    break;
                }
                p += sz;
            }
            "CDataSunpou" => {
                if p + 146 > data.len() {
                    break;
                }
                let Some((p_font, _)) = read_jww_cstring(data, p + 145) else {
                    break;
                };
                let Some((p_end, _)) = read_jww_cstring(data, p_font) else {
                    break;
                };
                p = p_end.saturating_add(236);
            }
            "CDataBlock" => {
                if p + 59 > data.len() {
                    break;
                }
                p += 59;
            }
            _ => break,
        }
    }

    Ok(drawing)
}

fn parse_sxf_p21(text: &str) -> CadDrawing {
    let mut drawing = CadDrawing {
        format: "SXF (STEP AP202 / .p21)".into(),
        ..Default::default()
    };
    for line in text.lines() {
        let t = line.trim();
        if t.contains("FILE_NAME") && drawing.title.is_empty() {
            drawing.title = t.to_string();
        }
    }
    drawing
}

fn parse_sxf_sfc(text: &str) -> CadDrawing {
    let mut drawing = CadDrawing {
        format: "SXF (SCADEC / .sfc)".into(),
        ..Default::default()
    };
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("drawing_sheet_feature") {
            let args = split_sfc_args(rest);
            if let Some(name) = args.first() {
                drawing.title = unquote(name);
            }
        } else if let Some(rest) = line.strip_prefix("line_feature") {
            let args = split_sfc_args(rest);
            // line_feature('layer', color, type, width, x1, y1, x2, y2)
            if args.len() >= 8
                && let (Ok(x1), Ok(y1), Ok(x2), Ok(y2)) = (
                    args[4].parse::<f64>(),
                    args[5].parse::<f64>(),
                    args[6].parse::<f64>(),
                    args[7].parse::<f64>(),
                )
            {
                let layer = unquote(&args[0]);
                drawing.layers.insert(layer.clone());
                drawing.entities.push(CadEntity::Line { layer, x1, y1, x2, y2 });
            }
        } else if let Some(rest) = line.strip_prefix("circle_feature") {
            let args = split_sfc_args(rest);
            // circle_feature('layer', color, type, width, cx, cy, r)
            if args.len() >= 7
                && let (Ok(cx), Ok(cy), Ok(r)) = (
                    args[4].parse::<f64>(),
                    args[5].parse::<f64>(),
                    args[6].parse::<f64>(),
                )
            {
                let layer = unquote(&args[0]);
                drawing.layers.insert(layer.clone());
                drawing.entities.push(CadEntity::Circle { layer, cx, cy, r });
            }
        } else if let Some(rest) = line.strip_prefix("arc_feature") {
            let args = split_sfc_args(rest);
            // arc_feature('layer', color, type, width, cx, cy, r, dir, start, end)
            if args.len() >= 10
                && let (Ok(cx), Ok(cy), Ok(r), Ok(start_deg), Ok(end_deg)) = (
                    args[4].parse::<f64>(),
                    args[5].parse::<f64>(),
                    args[6].parse::<f64>(),
                    args[8].parse::<f64>(),
                    args[9].parse::<f64>(),
                )
            {
                let layer = unquote(&args[0]);
                drawing.layers.insert(layer.clone());
                drawing.entities.push(CadEntity::Arc {
                    layer,
                    cx,
                    cy,
                    r,
                    start_deg,
                    end_deg,
                });
            }
        } else if let Some(rest) = line.strip_prefix("text_string_feature") {
            let args = split_sfc_args(rest);
            // text_string_feature('layer', color, font, 'text', x, y, h, ...)
            if args.len() >= 7
                && let (Ok(x), Ok(y), Ok(height)) = (
                    args[4].parse::<f64>(),
                    args[5].parse::<f64>(),
                    args[6].parse::<f64>(),
                )
            {
                let layer = unquote(&args[0]);
                let text = unquote(&args[3]);
                drawing.layers.insert(layer.clone());
                drawing.entities.push(CadEntity::Text {
                    layer,
                    x,
                    y,
                    height,
                    text,
                });
            }
        }
    }
    drawing
}

fn split_sfc_args(rest: &str) -> Vec<String> {
    let trimmed = rest.trim().trim_start_matches('(').trim_end_matches(';').trim_end_matches(')');
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in trimmed.chars() {
        match ch {
            '\'' => {
                in_quotes = !in_quotes;
                cur.push(ch);
            }
            ',' if !in_quotes => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('\'').trim_matches('"').to_string()
}

fn parse_dxf(text: &str) -> CadDrawing {
    let mut drawing = CadDrawing {
        format: "DXF".into(),
        ..Default::default()
    };
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut i = 0usize;
    let mut in_entities = false;
    let mut kind: Option<String> = None;
    let mut layer = String::from("0");
    let mut x1 = 0.0f64;
    let mut y1 = 0.0f64;
    let mut x2 = 0.0f64;
    let mut y2 = 0.0f64;
    let mut r = 0.0f64;
    let mut start_deg = 0.0f64;
    let mut end_deg = 360.0f64;
    let mut text_val = String::new();
    let mut poly_pts: Vec<(f64, f64)> = Vec::new();
    let mut pending_x: Option<f64> = None;
    let mut closed = false;

    let flush = |drawing: &mut CadDrawing,
                 kind: &mut Option<String>,
                 layer: &mut String,
                 x1: &mut f64,
                 y1: &mut f64,
                 x2: &mut f64,
                 y2: &mut f64,
                 r: &mut f64,
                 start_deg: &mut f64,
                 end_deg: &mut f64,
                 text_val: &mut String,
                 poly_pts: &mut Vec<(f64, f64)>,
                 closed: &mut bool| {
        if let Some(k) = kind.take() {
            let l = layer.clone();
            drawing.layers.insert(l.clone());
            match k.as_str() {
                "LINE" => drawing.entities.push(CadEntity::Line {
                    layer: l,
                    x1: *x1,
                    y1: *y1,
                    x2: *x2,
                    y2: *y2,
                }),
                "CIRCLE" => drawing.entities.push(CadEntity::Circle {
                    layer: l,
                    cx: *x1,
                    cy: *y1,
                    r: *r,
                }),
                "ARC" => drawing.entities.push(CadEntity::Arc {
                    layer: l,
                    cx: *x1,
                    cy: *y1,
                    r: *r,
                    start_deg: *start_deg,
                    end_deg: *end_deg,
                }),
                "LWPOLYLINE" => drawing.entities.push(CadEntity::Polyline {
                    layer: l,
                    points: std::mem::take(poly_pts),
                    closed: *closed,
                }),
                "TEXT" | "MTEXT" => drawing.entities.push(CadEntity::Text {
                    layer: l,
                    x: *x1,
                    y: *y1,
                    height: if *r > 0.0 { *r } else { 2.5 },
                    text: std::mem::take(text_val),
                }),
                _ => {}
            }
        }
        *layer = "0".into();
        *x1 = 0.0;
        *y1 = 0.0;
        *x2 = 0.0;
        *y2 = 0.0;
        *r = 0.0;
        *start_deg = 0.0;
        *end_deg = 360.0;
        text_val.clear();
        poly_pts.clear();
        *closed = false;
    };

    while i + 1 < lines.len() {
        let code = lines[i].trim();
        let val = lines[i + 1].trim();
        i += 2;

        if code == "2" && val == "ENTITIES" {
            in_entities = true;
            continue;
        }
        if !in_entities {
            continue;
        }
        if code == "0" {
            flush(
                &mut drawing,
                &mut kind,
                &mut layer,
                &mut x1,
                &mut y1,
                &mut x2,
                &mut y2,
                &mut r,
                &mut start_deg,
                &mut end_deg,
                &mut text_val,
                &mut poly_pts,
                &mut closed,
            );
            pending_x = None;
            if val == "ENDSEC" || val == "EOF" {
                break;
            }
            kind = Some(val.to_string());
            continue;
        }
        let is_lwpoly = kind.as_deref() == Some("LWPOLYLINE");
        match code {
            "8" => layer = val.to_string(),
            "10" => {
                if let Ok(v) = val.parse::<f64>() {
                    if is_lwpoly {
                        pending_x = Some(v);
                    } else {
                        x1 = v;
                    }
                }
            }
            "20" => {
                if let Ok(v) = val.parse::<f64>() {
                    if is_lwpoly {
                        if let Some(px) = pending_x.take() {
                            poly_pts.push((px, v));
                        }
                    } else {
                        y1 = v;
                    }
                }
            }
            "11" => if let Ok(v) = val.parse() { x2 = v },
            "21" => if let Ok(v) = val.parse() { y2 = v },
            "40" => if let Ok(v) = val.parse() { r = v },
            "50" => if let Ok(v) = val.parse() { start_deg = v },
            "51" => if let Ok(v) = val.parse() { end_deg = v },
            "70" => if let Ok(v) = val.parse::<i32>() { closed = (v & 1) != 0 },
            "1" => text_val = val.to_string(),
            _ => {}
        }
    }
    flush(
        &mut drawing,
        &mut kind,
        &mut layer,
        &mut x1,
        &mut y1,
        &mut x2,
        &mut y2,
        &mut r,
        &mut start_deg,
        &mut end_deg,
        &mut text_val,
        &mut poly_pts,
        &mut closed,
    );
    drawing
}

pub fn to_svg(data: &[u8]) -> Result<String, Error> {
    let drawing = parse(data)?;
    if drawing.entities.is_empty() {
        return Err(Error::new(format!(
            "{}: no 2D vector entities to render to SVG",
            drawing.format
        )));
    }
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut expand = |x: f64, y: f64| {
        if x.is_finite() && y.is_finite() {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    };
    for e in &drawing.entities {
        match e {
            CadEntity::Line { x1, y1, x2, y2, .. } => {
                expand(*x1, *y1);
                expand(*x2, *y2);
            }
            CadEntity::Circle { cx, cy, r, .. } | CadEntity::Arc { cx, cy, r, .. } => {
                expand(cx - r, cy - r);
                expand(cx + r, cy + r);
            }
            CadEntity::Polyline { points, .. } => {
                for &(x, y) in points {
                    expand(x, y);
                }
            }
            CadEntity::Text { x, y, height, .. } => {
                expand(*x, *y);
                expand(x + height, y + height);
            }
        }
    }
    let span_x = (max_x - min_x).max(1.0);
    let span_y = (max_y - min_y).max(1.0);
    let pad = span_x.max(span_y) * 0.04;
    let vb_x = min_x - pad;
    let vb_y = -(max_y + pad);
    let vb_w = span_x + 2.0 * pad;
    let vb_h = span_y + 2.0 * pad;
    let stroke_w = (vb_w.max(vb_h) / 600.0).max(0.05);

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{vb_x:.3} {vb_y:.3} {vb_w:.3} {vb_h:.3}\">\n\
         <rect x=\"{vb_x:.3}\" y=\"{vb_y:.3}\" width=\"{vb_w:.3}\" height=\"{vb_h:.3}\" fill=\"#ffffff\"/>\n\
         <g fill=\"none\" stroke=\"#111827\" stroke-width=\"{stroke_w:.4}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n"
    );
    for e in &drawing.entities {
        match e {
            CadEntity::Line { x1, y1, x2, y2, .. } => {
                svg.push_str(&format!(
                    "  <line x1=\"{x1:.3}\" y1=\"{:.3}\" x2=\"{x2:.3}\" y2=\"{:.3}\"/>\n",
                    -y1, -y2
                ));
            }
            CadEntity::Circle { cx, cy, r, .. } => {
                svg.push_str(&format!(
                    "  <circle cx=\"{cx:.3}\" cy=\"{:.3}\" r=\"{r:.3}\"/>\n",
                    -cy
                ));
            }
            CadEntity::Arc {
                cx,
                cy,
                r,
                start_deg,
                end_deg,
                ..
            } => {
                let s = start_deg.to_radians();
                let e_rad = end_deg.to_radians();
                let sx = cx + r * s.cos();
                let sy = -(cy + r * s.sin());
                let ex = cx + r * e_rad.cos();
                let ey = -(cy + r * e_rad.sin());
                let mut sweep = end_deg - start_deg;
                if sweep <= 0.0 {
                    sweep += 360.0;
                }
                let large = if sweep > 180.0 { 1 } else { 0 };
                svg.push_str(&format!(
                    "  <path d=\"M {sx:.3} {sy:.3} A {r:.3} {r:.3} 0 {large} 0 {ex:.3} {ey:.3}\"/>\n"
                ));
            }
            CadEntity::Polyline { points, closed, .. } => {
                if !points.is_empty() {
                    let pts: String = points
                        .iter()
                        .map(|(x, y)| format!("{x:.3},{:.3}", -y))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let tag = if *closed { "polygon" } else { "polyline" };
                    svg.push_str(&format!("  <{tag} points=\"{pts}\"/>\n"));
                }
            }
            CadEntity::Text { x, y, height, text, .. } => {
                let escaped = text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                svg.push_str(&format!(
                    "  <text x=\"{x:.3}\" y=\"{:.3}\" font-size=\"{height:.3}\" fill=\"#111827\" stroke=\"none\" font-family=\"sans-serif\">{escaped}</text>\n",
                    -y
                ));
            }
        }
    }
    svg.push_str("</g>\n</svg>\n");
    Ok(svg)
}

pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let drawing = parse(data)?;
    let title = if drawing.title.is_empty() {
        drawing.format.clone()
    } else {
        format!("{} — {}", drawing.title, drawing.format)
    };
    let mut md = format!("# {title}\n\n");
    md.push_str(&format!("- **Format:** {}\n", drawing.format));
    md.push_str(&format!("- **Entities:** {}\n", drawing.entities.len()));
    md.push_str(&format!("- **Layers:** {}\n", drawing.layers.len()));
    if !drawing.layers.is_empty() {
        let list: Vec<_> = drawing.layers.iter().cloned().collect();
        md.push_str(&format!("- **Layer names:** {}\n", list.join(", ")));
    }
    let texts: Vec<_> = drawing
        .entities
        .iter()
        .filter_map(|e| match e {
            CadEntity::Text { layer, text, .. } if !text.trim().is_empty() => {
                Some((layer.as_str(), text.trim()))
            }
            _ => None,
        })
        .collect();
    if !texts.is_empty() {
        md.push_str("\n## Extracted Text\n\n| Layer | Text |\n| --- | --- |\n");
        for (layer, t) in texts {
            md.push_str(&format!(
                "| `{}` | {} |\n",
                layer.replace('|', "\\|"),
                t.replace('|', "\\|").replace(['\r', '\n'], " ")
            ));
        }
    }
    Ok(md)
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    let md = to_markdown(data)?;
    crate::markdown::open_markdown(md.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dxf_and_renders_svg() {
        let dxf = concat!(
            "0\nSECTION\n2\nENTITIES\n",
            "0\nLINE\n8\nWALL\n10\n0\n20\n0\n11\n100\n21\n0\n",
            "0\nCIRCLE\n8\nAXIS\n10\n50\n20\n50\n40\n20\n",
            "0\nTEXT\n8\nNOTE\n10\n10\n20\n10\n40\n5\n1\n平面図\n",
            "0\nENDSEC\n0\nEOF\n",
        );
        assert!(is_dxf(dxf.as_bytes()));
        let svg = to_svg(dxf.as_bytes()).unwrap();
        assert!(svg.contains("<line"));
        assert!(svg.contains("<circle"));
        assert!(svg.contains("平面図"));
        let md = to_markdown(dxf.as_bytes()).unwrap();
        assert!(md.contains("WALL"));
    }

    #[test]
    fn parses_sxf_sfc_and_detects_jww() {
        let sfc = concat!(
            "SXF\n",
            "drawing_sheet_feature('一般図', 1, 0, 594.0, 420.0);\n",
            "line_feature('外形線', 1, 1, 1, 0.0, 0.0, 100.0, 50.0);\n",
            "text_string_feature('注記', 1, 1, '橋梁平面図', 10.0, 20.0, 5.0, 3.0, 0.0, 0.0, 1);\n",
        );
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode(sfc);
        assert!(is_sxf_sfc(&sjis));
        let svg = to_svg(&sjis).unwrap();
        assert!(svg.contains("<line"));
        assert!(svg.contains("橋梁平面図"));

        let mut jww = b"JwwData.".to_vec();
        jww.extend_from_slice(&700u32.to_le_bytes());
        assert!(is_jww(&jww));
        let md = to_markdown(&jww).unwrap();
        assert!(md.contains("version 700"));
    }

    #[test]
    fn parses_synthetic_jww_entities_to_svg_and_markdown() {
        let mut jww = b"JwwData.".to_vec();
        jww.extend_from_slice(&700u32.to_le_bytes());
        let (memo_sjis, _, _) = encoding_rs::SHIFT_JIS.encode("建築平面図");
        jww.push(memo_sjis.len() as u8);
        jww.extend_from_slice(&memo_sjis);
        // Zero-filled variable header (all CStrings empty = 1 byte 0x00)
        let header_tail = 8
            + 16 * (20 + 16 * 8)
            + (14 * 4 + 100)
            + 16 * 16
            + 16
            + 96
            + (8 * 28 + 44 + 12)
            + (10 * 8 + 8 + 10 * 8 + 10 * 16 + 8 * 16 + 5 * 20 + 4 * 16 + 144)
            + 257 * 8
            + 257 * 17
            + 33 * 16
            + 33 * (1 + 4 + 10 * 8)
            + (10 * 28 + 100);
        jww.extend(std::iter::repeat_n(0u8, header_tail));

        // 2 entities: CDataSen + CDataMoji
        jww.extend_from_slice(&2u16.to_le_bytes());

        // Entity 1: new class CDataSen
        jww.extend_from_slice(&0xFFFFu16.to_le_bytes());
        jww.extend_from_slice(&700u16.to_le_bytes());
        jww.extend_from_slice(&8u16.to_le_bytes());
        jww.extend_from_slice(b"CDataSen");
        jww.extend(std::iter::repeat_n(0u8, 15));
        for v in [0.0f64, 0.0, 100.0, 50.0] {
            jww.extend_from_slice(&v.to_le_bytes());
        }

        // Entity 2: new class CDataMoji
        jww.extend_from_slice(&0xFFFFu16.to_le_bytes());
        jww.extend_from_slice(&700u16.to_le_bytes());
        jww.extend_from_slice(&9u16.to_le_bytes());
        jww.extend_from_slice(b"CDataMoji");
        let mut moji = vec![0u8; 83];
        moji[15..23].copy_from_slice(&10.0f64.to_le_bytes());
        moji[23..31].copy_from_slice(&20.0f64.to_le_bytes());
        moji[59..67].copy_from_slice(&3.5f64.to_le_bytes());
        jww.extend_from_slice(&moji);
        jww.push(0); // font name empty
        let (txt_sjis, _, _) = encoding_rs::SHIFT_JIS.encode("耐震天井");
        jww.push(txt_sjis.len() as u8);
        jww.extend_from_slice(&txt_sjis);

        let drawing = parse(&jww).unwrap();
        assert_eq!(drawing.title, "建築平面図");
        assert_eq!(drawing.entities.len(), 2);

        let svg = to_svg(&jww).unwrap();
        assert!(svg.contains("<line"));
        assert!(svg.contains("耐震天井"));
        let md = to_markdown(&jww).unwrap();
        assert!(md.contains("建築平面図"));
        assert!(md.contains("耐震天井"));
    }
}

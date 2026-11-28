// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust GIS reader for GeoJSON (`.geojson`), ESRI Shapefile (`.shp`), and dBase (`.dbf`).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{Error, Opened};

#[derive(Clone, Debug, PartialEq)]
pub enum GisGeometry {
    Point(f64, f64),
    LineString(Vec<(f64, f64)>),
    Polygon(Vec<Vec<(f64, f64)>>),
}

#[derive(Clone, Debug, Default)]
pub struct GisFeature {
    pub geometries: Vec<GisGeometry>,
    pub properties: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct GisDataset {
    pub format: String,
    pub features: Vec<GisFeature>,
    pub property_keys: Vec<String>,
}

pub fn is_shp(data: &[u8]) -> bool {
    data.len() >= 100
        && u32::from_be_bytes(data[0..4].try_into().unwrap()) == 9994
        && u32::from_le_bytes(data[28..32].try_into().unwrap()) == 1000
}

pub fn is_dbf(data: &[u8]) -> bool {
    if data.len() < 33 {
        return false;
    }
    let ver = data[0];
    if !matches!(ver, 0x02 | 0x03 | 0x04 | 0x05 | 0x30 | 0x31 | 0x83 | 0x8B | 0xF5) {
        return false;
    }
    let header_len = u16::from_le_bytes(data[8..10].try_into().unwrap()) as usize;
    let record_len = u16::from_le_bytes(data[10..12].try_into().unwrap()) as usize;
    header_len >= 33
        && record_len > 0
        && header_len <= data.len()
        && data[header_len - 1] == 0x0D
}

pub fn is_geojson(data: &[u8]) -> bool {
    let trimmed = data
        .strip_prefix(b"\xEF\xBB\xBF")
        .unwrap_or(data);
    let Ok(text) = std::str::from_utf8(&trimmed[..trimmed.len().min(4096)]) else {
        return false;
    };
    let t = text.trim_start();
    if !t.starts_with('{') {
        return false;
    }
    let Ok(val) = serde_json::from_slice::<Value>(trimmed) else {
        return false;
    };
    matches!(
        val.get("type").and_then(|v| v.as_str()),
        Some(
            "FeatureCollection"
                | "Feature"
                | "Point"
                | "MultiPoint"
                | "LineString"
                | "MultiLineString"
                | "Polygon"
                | "MultiPolygon"
        )
    )
}

pub fn is_gis(data: &[u8]) -> bool {
    is_shp(data) || is_dbf(data) || is_geojson(data)
}

pub fn parse_geojson(data: &[u8]) -> Result<GisDataset, Error> {
    let trimmed = data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(data);
    let val: Value = serde_json::from_slice(trimmed)
        .map_err(|e| Error::new(format!("invalid GeoJSON: {e}")))?;
    let mut ds = GisDataset {
        format: "GeoJSON".into(),
        ..Default::default()
    };
    match val.get("type").and_then(|v| v.as_str()) {
        Some("FeatureCollection") => {
            if let Some(arr) = val.get("features").and_then(|v| v.as_array()) {
                for f in arr {
                    push_geojson_feature(f, &mut ds);
                }
            }
        }
        Some("Feature") => push_geojson_feature(&val, &mut ds),
        Some(_) => {
            let mut feat = GisFeature::default();
            collect_geojson_geometry(&val, &mut feat.geometries);
            ds.features.push(feat);
        }
        None => return Err(Error::new("GeoJSON missing \"type\" field")),
    }
    Ok(ds)
}

fn push_geojson_feature(f: &Value, ds: &mut GisDataset) {
    let mut feat = GisFeature::default();
    if let Some(geom) = f.get("geometry") {
        collect_geojson_geometry(geom, &mut feat.geometries);
    }
    if let Some(props) = f.get("properties").and_then(|v| v.as_object()) {
        for (k, v) in props {
            if !ds.property_keys.contains(k) {
                ds.property_keys.push(k.clone());
            }
            let s = match v {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            feat.properties.insert(k.clone(), s);
        }
    }
    ds.features.push(feat);
}

fn collect_geojson_geometry(geom: &Value, out: &mut Vec<GisGeometry>) {
    let Some(kind) = geom.get("type").and_then(|v| v.as_str()) else {
        return;
    };
    let coords = geom.get("coordinates");
    match kind {
        "Point" => {
            if let Some(pt) = coords.and_then(as_point) {
                out.push(GisGeometry::Point(pt.0, pt.1));
            }
        }
        "MultiPoint" => {
            if let Some(arr) = coords.and_then(|v| v.as_array()) {
                for p in arr.iter().filter_map(as_point) {
                    out.push(GisGeometry::Point(p.0, p.1));
                }
            }
        }
        "LineString" => {
            if let Some(pts) = coords.and_then(as_ring) {
                out.push(GisGeometry::LineString(pts));
            }
        }
        "MultiLineString" => {
            if let Some(arr) = coords.and_then(|v| v.as_array()) {
                for line in arr.iter().filter_map(as_ring) {
                    out.push(GisGeometry::LineString(line));
                }
            }
        }
        "Polygon" => {
            if let Some(rings) = coords.and_then(as_rings) {
                out.push(GisGeometry::Polygon(rings));
            }
        }
        "MultiPolygon" => {
            if let Some(arr) = coords.and_then(|v| v.as_array()) {
                for rings in arr.iter().filter_map(as_rings) {
                    out.push(GisGeometry::Polygon(rings));
                }
            }
        }
        "GeometryCollection" => {
            if let Some(geoms) = geom.get("geometries").and_then(|v| v.as_array()) {
                for g in geoms {
                    collect_geojson_geometry(g, out);
                }
            }
        }
        _ => {}
    }
}

fn as_point(v: &Value) -> Option<(f64, f64)> {
    let arr = v.as_array()?;
    Some((arr.first()?.as_f64()?, arr.get(1)?.as_f64()?))
}

fn as_ring(v: &Value) -> Option<Vec<(f64, f64)>> {
    let arr = v.as_array()?;
    Some(arr.iter().filter_map(as_point).collect())
}

fn as_rings(v: &Value) -> Option<Vec<Vec<(f64, f64)>>> {
    let arr = v.as_array()?;
    Some(arr.iter().filter_map(as_ring).collect())
}

pub fn parse_shp(data: &[u8]) -> Result<GisDataset, Error> {
    if !is_shp(data) {
        return Err(Error::new("not an ESRI Shapefile (.shp)"));
    }
    let shape_type = u32::from_le_bytes(data[32..36].try_into().unwrap());
    let mut ds = GisDataset {
        format: format!("ESRI Shapefile (type {shape_type})"),
        ..Default::default()
    };
    let mut pos = 100usize;
    while pos + 8 <= data.len() {
        let _rec_num = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
        let content_words = u32::from_be_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        let content_bytes = content_words.saturating_mul(2);
        if content_bytes < 4 || pos + content_bytes > data.len() {
            break;
        }
        let rec = &data[pos..pos + content_bytes];
        pos += content_bytes;
        let rec_type = u32::from_le_bytes(rec[0..4].try_into().unwrap());
        let mut feat = GisFeature::default();
        match rec_type {
            1 | 11 | 21 if rec.len() >= 20 => {
                let x = f64::from_le_bytes(rec[4..12].try_into().unwrap());
                let y = f64::from_le_bytes(rec[12..20].try_into().unwrap());
                feat.geometries.push(GisGeometry::Point(x, y));
            }
            3 | 5 | 13 | 15 | 23 | 25 if rec.len() >= 44 => {
                let num_parts = u32::from_le_bytes(rec[36..40].try_into().unwrap()) as usize;
                let num_points = u32::from_le_bytes(rec[40..44].try_into().unwrap()) as usize;
                let parts_start = 44usize;
                let pts_start = parts_start + num_parts * 4;
                if pts_start + num_points * 16 <= rec.len() {
                    let mut parts = Vec::with_capacity(num_parts);
                    for p in 0..num_parts {
                        let off = parts_start + p * 4;
                        parts.push(u32::from_le_bytes(rec[off..off + 4].try_into().unwrap()) as usize);
                    }
                    let mut pts = Vec::with_capacity(num_points);
                    for p in 0..num_points {
                        let off = pts_start + p * 16;
                        let x = f64::from_le_bytes(rec[off..off + 8].try_into().unwrap());
                        let y = f64::from_le_bytes(rec[off + 8..off + 16].try_into().unwrap());
                        pts.push((x, y));
                    }
                    let mut rings = Vec::with_capacity(num_parts);
                    for idx in 0..num_parts {
                        let start = parts[idx];
                        let end = parts.get(idx + 1).copied().unwrap_or(num_points);
                        if start < end && end <= pts.len() {
                            rings.push(pts[start..end].to_vec());
                        }
                    }
                    if matches!(rec_type, 3 | 13 | 23) {
                        for r in rings {
                            feat.geometries.push(GisGeometry::LineString(r));
                        }
                    } else if !rings.is_empty() {
                        feat.geometries.push(GisGeometry::Polygon(rings));
                    }
                }
            }
            _ => {}
        }
        ds.features.push(feat);
    }
    Ok(ds)
}

pub fn parse_dbf(data: &[u8]) -> Result<(Vec<String>, Vec<Vec<String>>), Error> {
    if !is_dbf(data) {
        return Err(Error::new("not a dBase (.dbf) file"));
    }
    let num_records = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let header_len = u16::from_le_bytes(data[8..10].try_into().unwrap()) as usize;
    let record_len = u16::from_le_bytes(data[10..12].try_into().unwrap()) as usize;

    let mut fields: Vec<(String, usize)> = Vec::new();
    let mut off = 32usize;
    while off + 32 <= header_len && data[off] != 0x0D {
        let name_raw = &data[off..off + 11];
        let name_end = name_raw.iter().position(|&b| b == 0).unwrap_or(11);
        let name = decode_dbf_str(&name_raw[..name_end]);
        let field_len = data[off + 16] as usize;
        fields.push((name, field_len));
        off += 32;
    }
    let headers: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
    let mut rows = Vec::new();
    let mut rec_off = header_len;
    for _ in 0..num_records {
        if rec_off + record_len > data.len() {
            break;
        }
        let rec = &data[rec_off..rec_off + record_len];
        rec_off += record_len;
        if rec[0] == b'*' {
            continue;
        }
        let mut row = Vec::with_capacity(fields.len());
        let mut f_off = 1usize;
        for (_, flen) in &fields {
            if f_off + *flen > rec.len() {
                row.push(String::new());
            } else {
                row.push(decode_dbf_str(&rec[f_off..f_off + *flen]).trim().to_string());
            }
            f_off += *flen;
        }
        rows.push(row);
    }
    Ok((headers, rows))
}

fn decode_dbf_str(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
            cow.into_owned()
        }
    }
}

pub fn to_svg(data: &[u8]) -> Result<String, Error> {
    let ds = if is_shp(data) {
        parse_shp(data)?
    } else if is_geojson(data) {
        parse_geojson(data)?
    } else {
        return Err(Error::new("SVG export requires GeoJSON or Shapefile (.shp)"));
    };
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
    for f in &ds.features {
        for g in &f.geometries {
            match g {
                GisGeometry::Point(x, y) => expand(*x, *y),
                GisGeometry::LineString(pts) => {
                    for &(x, y) in pts {
                        expand(x, y);
                    }
                }
                GisGeometry::Polygon(rings) => {
                    for r in rings {
                        for &(x, y) in r {
                            expand(x, y);
                        }
                    }
                }
            }
        }
    }
    if !min_x.is_finite() {
        return Err(Error::new("GIS dataset contains no coordinates"));
    }
    let span_x = (max_x - min_x).max(0.0001);
    let span_y = (max_y - min_y).max(0.0001);
    let pad = span_x.max(span_y) * 0.05;
    let vb_x = min_x - pad;
    let vb_y = -(max_y + pad);
    let vb_w = span_x + 2.0 * pad;
    let vb_h = span_y + 2.0 * pad;
    let stroke_w = vb_w.max(vb_h) / 400.0;
    let pt_r = vb_w.max(vb_h) / 140.0;

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{vb_x:.6} {vb_y:.6} {vb_w:.6} {vb_h:.6}\">\n\
         <rect x=\"{vb_x:.6}\" y=\"{vb_y:.6}\" width=\"{vb_w:.6}\" height=\"{vb_h:.6}\" fill=\"#ffffff\"/>\n\
         <g stroke=\"#1d4ed8\" stroke-width=\"{stroke_w:.6}\" fill=\"#93c5fd\" fill-opacity=\"0.35\">\n"
    );
    for f in &ds.features {
        for g in &f.geometries {
            match g {
                GisGeometry::Point(x, y) => {
                    svg.push_str(&format!(
                        "  <circle cx=\"{x:.6}\" cy=\"{:.6}\" r=\"{pt_r:.6}\" fill=\"#dc2626\" stroke=\"none\"/>\n",
                        -y
                    ));
                }
                GisGeometry::LineString(pts) if !pts.is_empty() => {
                    let s: String = pts
                        .iter()
                        .map(|(x, y)| format!("{x:.6},{:.6}", -y))
                        .collect::<Vec<_>>()
                        .join(" ");
                    svg.push_str(&format!("  <polyline points=\"{s}\" fill=\"none\"/>\n"));
                }
                GisGeometry::Polygon(rings) => {
                    let mut d = String::new();
                    for ring in rings {
                        for (idx, (x, y)) in ring.iter().enumerate() {
                            if idx == 0 {
                                d.push_str(&format!("M {x:.6} {:.6} ", -y));
                            } else {
                                d.push_str(&format!("L {x:.6} {:.6} ", -y));
                            }
                        }
                        if !ring.is_empty() {
                            d.push_str("Z ");
                        }
                    }
                    if !d.is_empty() {
                        svg.push_str(&format!("  <path d=\"{}\" fill-rule=\"evenodd\"/>\n", d.trim()));
                    }
                }
                _ => {}
            }
        }
    }
    svg.push_str("</g>\n</svg>\n");
    Ok(svg)
}

pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    if is_dbf(data) {
        let (headers, rows) = parse_dbf(data)?;
        return Ok(render_table_markdown("dBase (.dbf) Attribute Table", &headers, &rows));
    }
    let ds = if is_shp(data) {
        parse_shp(data)?
    } else if is_geojson(data) {
        parse_geojson(data)?
    } else {
        return Err(Error::new("unsupported GIS format"));
    };
    let mut md = format!("# {}\n\n- **Features:** {}\n", ds.format, ds.features.len());
    if !ds.property_keys.is_empty() {
        let rows: Vec<Vec<String>> = ds
            .features
            .iter()
            .take(500)
            .map(|f| {
                ds.property_keys
                    .iter()
                    .map(|k| f.properties.get(k).cloned().unwrap_or_default())
                    .collect()
            })
            .collect();
        md.push('\n');
        md.push_str(&render_table_markdown("Attributes", &ds.property_keys, &rows));
    }
    Ok(md)
}

fn render_table_markdown(title: &str, headers: &[String], rows: &[Vec<String>]) -> String {
    let mut md = format!("## {title} ({} rows)\n\n|", rows.len());
    for h in headers {
        md.push_str(&format!(" {} |", h.replace('|', "\\|")));
    }
    md.push_str("\n|");
    for _ in headers {
        md.push_str(" --- |");
    }
    md.push('\n');
    for row in rows.iter().take(500) {
        md.push('|');
        for cell in row {
            md.push_str(&format!(
                " {} |",
                cell.replace('|', "\\|").replace(['\r', '\n'], " ")
            ));
        }
        md.push('\n');
    }
    md
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    let md = to_markdown(data)?;
    crate::markdown::open_markdown(md.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_geojson_and_renders_svg_and_markdown() {
        let gj = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"name":"庁舎"},"geometry":{"type":"Point","coordinates":[139.75,35.68]}},
            {"type":"Feature","properties":{"name":"道路"},"geometry":{"type":"LineString","coordinates":[[139.7,35.6],[139.8,35.7]]}}
        ]}"#.as_bytes();
        assert!(is_geojson(gj));
        let svg = to_svg(gj).unwrap();
        assert!(svg.contains("<circle"));
        assert!(svg.contains("<polyline"));
        let md = to_markdown(gj).unwrap();
        assert!(md.contains("庁舎"));
    }

    #[test]
    fn parses_synthetic_shp_point_and_dbf() {
        let mut shp = vec![0u8; 100 + 28];
        shp[0..4].copy_from_slice(&9994u32.to_be_bytes());
        shp[28..32].copy_from_slice(&1000u32.to_le_bytes());
        shp[32..36].copy_from_slice(&1u32.to_le_bytes());
        shp[100..104].copy_from_slice(&1u32.to_be_bytes());
        shp[104..108].copy_from_slice(&10u32.to_be_bytes()); // 20 bytes
        shp[108..112].copy_from_slice(&1u32.to_le_bytes()); // Point
        shp[112..120].copy_from_slice(&135.0f64.to_le_bytes());
        shp[120..128].copy_from_slice(&35.0f64.to_le_bytes());
        assert!(is_shp(&shp));
        let svg = to_svg(&shp).unwrap();
        assert!(svg.contains("<circle"));

        let mut dbf = vec![0u8; 65 + 11];
        dbf[0] = 0x03;
        dbf[4..8].copy_from_slice(&1u32.to_le_bytes());
        dbf[8..10].copy_from_slice(&65u16.to_le_bytes());
        dbf[10..12].copy_from_slice(&11u16.to_le_bytes());
        dbf[32..36].copy_from_slice(b"NAME");
        dbf[32 + 11] = b'C';
        dbf[32 + 16] = 10;
        dbf[64] = 0x0D;
        dbf[65] = b' ';
        dbf[66..76].copy_from_slice(b"Tokyo     ");
        assert!(is_dbf(&dbf));
        let (headers, rows) = parse_dbf(&dbf).unwrap();
        assert_eq!(headers, vec!["NAME"]);
        assert_eq!(rows[0][0], "Tokyo");
    }
}

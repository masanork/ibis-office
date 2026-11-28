//! CityGML / PLATEAU (`.gml`) 3D 都市モデルの解析・2D フットプリント SVG 出力・Markdown サマリ出力。

use std::collections::BTreeMap;

use roxmltree::{Document, Node};

use crate::Error;

#[derive(Debug, Clone)]
pub struct CityFeatureSummary {
    pub kind: String,
    pub id: String,
    pub name: Option<String>,
    pub height: Option<f64>,
    pub storeys: Option<String>,
    pub usage: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CityGmlReport {
    pub is_plateau: bool,
    pub model_name: Option<String>,
    pub srs_name: Option<String>,
    pub lower_corner: Option<String>,
    pub upper_corner: Option<String>,
    pub feature_counts: BTreeMap<String, usize>,
    pub features: Vec<CityFeatureSummary>,
    pub footprints: Vec<Vec<(f64, f64)>>,
}

/// バイト列が CityGML / PLATEAU GML かどうかを判定する。
pub fn is_citygml(data: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&data[..data.len().min(4096)]);
    if !head.contains('<') {
        return false;
    }
    head.contains("opengis.net/citygml/")
        || head.contains("<CityModel")
        || head.contains("<core:CityModel")
        || (head.contains("opengis.net/gml")
            && (head.contains("bldg:Building")
                || head.contains("tran:Road")
                || head.contains("luse:LandUse")
                || head.contains("cityObjectMember")))
}

fn find_descendant_text(node: Node<'_, '_>, local_name: &str) -> Option<String> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == local_name)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_pos_list(text: &str) -> Vec<(f64, f64)> {
    let nums: Vec<f64> = text
        .split_whitespace()
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    if nums.len() < 6 {
        return Vec::new();
    }
    // CityGML posList は通常 3D (lat lon elev または x y z)
    let stride = if nums.len() % 3 == 0 { 3 } else { 2 };
    let mut pts = Vec::with_capacity(nums.len() / stride);
    for chunk in nums.chunks_exact(stride) {
        // CityGML / JGD2011 の地理座標 (lat, lon) は (y=lat, x=lon) の順が多いため、
        // 日本付近 (20..46, 122..154) なら (lon, lat) に正規化する
        let (a, b) = (chunk[0], chunk[1]);
        if (20.0..=46.0).contains(&a) && (122.0..=154.0).contains(&b) {
            pts.push((b, a));
        } else {
            pts.push((a, b));
        }
    }
    pts
}

/// CityGML / PLATEAU GML を解析する。
pub fn inspect_citygml(data: &[u8]) -> Result<CityGmlReport, Error> {
    let text = std::str::from_utf8(data)
        .map_err(|_| Error::new("CityGML ファイルが有効な UTF-8 ではありません"))?
        .trim_start_matches('\u{FEFF}');
    let doc = Document::parse(text).map_err(|e| Error::xml("CityGML", e))?;
    let root = doc.root_element();

    let is_plateau = text[..text.len().min(8192)].contains("chisou")
        || text[..text.len().min(8192)].contains("uro")
        || text[..text.len().min(8192)].contains("plateau");

    let model_name = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "name")
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let env = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Envelope");
    let srs_name = env.and_then(|e| e.attribute("srsName")).map(str::to_string);
    let lower_corner = env.and_then(|e| find_descendant_text(e, "lowerCorner"));
    let upper_corner = env.and_then(|e| find_descendant_text(e, "upperCorner"));

    let mut feature_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut features = Vec::new();
    let mut footprints = Vec::new();

    for member in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "cityObjectMember")
    {
        for feat in member.children().filter(|n| n.is_element()) {
            let kind = feat.tag_name().name().to_string();
            *feature_counts.entry(kind.clone()).or_insert(0) += 1;

            if features.len() < 100 {
                let id = feat
                    .attribute(("http://www.opengis.net/gml", "id"))
                    .or_else(|| feat.attribute("id"))
                    .unwrap_or("")
                    .to_string();
                let name = find_descendant_text(feat, "name");
                let height = find_descendant_text(feat, "measuredHeight")
                    .and_then(|s| s.parse::<f64>().ok());
                let storeys = find_descendant_text(feat, "storeysAboveGround");
                let usage = find_descendant_text(feat, "usage");
                features.push(CityFeatureSummary {
                    kind,
                    id,
                    name,
                    height,
                    storeys,
                    usage,
                });
            }

            if footprints.len() < 2000 {
                if let Some(pos_node) = feat
                    .descendants()
                    .find(|n| n.is_element() && n.tag_name().name() == "posList")
                {
                    if let Some(txt) = pos_node.text() {
                        let pts = parse_pos_list(txt);
                        if pts.len() >= 3 {
                            footprints.push(pts);
                        }
                    }
                }
            }
        }
    }

    Ok(CityGmlReport {
        is_plateau,
        model_name,
        srs_name,
        lower_corner,
        upper_corner,
        feature_counts,
        features,
        footprints,
    })
}

/// CityGML の 2D フットプリントをベクター SVG として出力する。
pub fn to_svg(data: &[u8]) -> Result<String, Error> {
    let report = inspect_citygml(data)?;
    if report.footprints.is_empty() {
        return Ok(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 600"></svg>"#.to_string(),
        );
    }

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for poly in &report.footprints {
        for &(x, y) in poly {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    let dx = (max_x - min_x).max(1e-9);
    let dy = (max_y - min_y).max(1e-9);
    let scale = 960.0 / dx.max(dy);
    let vw = dx * scale + 40.0;
    let vh = dy * scale + 40.0;

    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {vw:.2} {vh:.2}\" width=\"{vw:.0}\" height=\"{vh:.0}\">\n\
         <rect width=\"100%\" height=\"100%\" fill=\"#f8fafc\"/>\n"
    );
    for poly in &report.footprints {
        let pts: Vec<String> = poly
            .iter()
            .map(|&(x, y)| {
                let sx = (x - min_x) * scale + 20.0;
                let sy = (max_y - y) * scale + 20.0;
                format!("{sx:.2},{sy:.2}")
            })
            .collect();
        out.push_str(&format!(
            "<polygon points=\"{}\" fill=\"#cbd5e1\" fill-opacity=\"0.7\" stroke=\"#334155\" stroke-width=\"1\"/>\n",
            pts.join(" ")
        ));
    }
    out.push_str("</svg>\n");
    Ok(out)
}

/// CityGML / PLATEAU GML の検査結果を Markdown で返す。
pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let r = inspect_citygml(data)?;
    let title = if r.is_plateau {
        "CityGML / PLATEAU 3D 都市モデル"
    } else {
        "CityGML 3D 都市モデル"
    };
    let mut out = format!("# {title}\n\n");
    if let Some(name) = &r.model_name {
        out.push_str(&format!("- **モデル名**: {name}\n"));
    }
    if let Some(srs) = &r.srs_name {
        out.push_str(&format!("- **座標参照系 (`srsName`)**: `{srs}`\n"));
    }
    if let (Some(lo), Some(hi)) = (&r.lower_corner, &r.upper_corner) {
        out.push_str(&format!("- **空間範囲**: `{lo}` 〜 `{hi}`\n"));
    }
    out.push_str(&format!("- **抽出 2D フットプリント数**: {}\n", r.footprints.len()));

    if !r.feature_counts.is_empty() {
        out.push_str("\n## 地物種別集計\n\n| 地物型 | 件数 |\n|---|---:|\n");
        for (k, v) in &r.feature_counts {
            out.push_str(&format!("| `{k}` | {v} |\n"));
        }
    }

    if !r.features.is_empty() {
        out.push_str("\n## 地物一覧 (先頭最大 100 件)\n\n");
        out.push_str("| 種別 | ID | 名称 | 高さ (m) | 階数 | 用途 |\n|---|---|---|---:|---:|---|\n");
        for f in &r.features {
            let h = f.height.map(|v| format!("{v:.1}")).unwrap_or_default();
            out.push_str(&format!(
                "| `{}` | `{}` | {} | {} | {} | {} |\n",
                f.kind,
                f.id,
                f.name.as_deref().unwrap_or(""),
                h,
                f.storeys.as_deref().unwrap_or(""),
                f.usage.as_deref().unwrap_or(""),
            ));
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plateau_citygml_and_renders_svg_and_markdown() {
        let gml = r#"<?xml version="1.0" encoding="UTF-8"?>
<core:CityModel xmlns:core="http://www.opengis.net/citygml/2.0"
                xmlns:gml="http://www.opengis.net/gml"
                xmlns:bldg="http://www.opengis.net/citygml/building/2.0"
                xmlns:uro="https://www.geospatial.jp/iur/uro/3.0">
  <gml:name>東京都千代田区サンプル</gml:name>
  <gml:boundedBy>
    <gml:Envelope srsName="http://www.opengis.net/def/crs/EPSG/0/6697">
      <gml:lowerCorner>35.6800 139.7500 0.0</gml:lowerCorner>
      <gml:upperCorner>35.6900 139.7600 120.0</gml:upperCorner>
    </gml:Envelope>
  </gml:boundedBy>
  <core:cityObjectMember>
    <bldg:Building gml:id="bldg_001">
      <gml:name>庁舎本館</gml:name>
      <bldg:usage>401</bldg:usage>
      <bldg:measuredHeight uom="m">45.5</bldg:measuredHeight>
      <bldg:storeysAboveGround>10</bldg:storeysAboveGround>
      <bldg:lod0FootPrint>
        <gml:Polygon><gml:exterior><gml:LinearRing>
          <gml:posList>35.6810 139.7510 0 35.6810 139.7520 0 35.6820 139.7520 0 35.6820 139.7510 0 35.6810 139.7510 0</gml:posList>
        </gml:LinearRing></gml:exterior></gml:Polygon>
      </bldg:lod0FootPrint>
    </bldg:Building>
  </core:cityObjectMember>
</core:CityModel>"#;

        assert!(is_citygml(gml.as_bytes()));
        let md = to_markdown(gml.as_bytes()).unwrap();
        assert!(md.contains("PLATEAU"));
        assert!(md.contains("庁舎本館"));
        assert!(md.contains("45.5"));

        let svg = to_svg(gml.as_bytes()).unwrap();
        assert!(svg.contains("<polygon"));
    }
}

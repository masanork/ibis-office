// SPDX-License-Identifier: AGPL-3.0-only

use ibis_doc_model::{FloatingAnchor, WrapPoint, WrapPolygon};
use serde_json::{Value, json};

use super::{decode_base64, encode_base64};

pub(super) const ATTRIBUTE: &str = "docx-anchor=";

pub(super) fn encode(anchor: &FloatingAnchor) -> String {
    let point = |point: WrapPoint| [point.x, point.y];
    let polygon = anchor.wrap_polygon.as_ref().map(|polygon| {
        json!({
            "start": point(polygon.start),
            "lineTo": polygon.line_to.iter().copied().map(point).collect::<Vec<_>>(),
            "edited": polygon.edited,
        })
    });
    let value = json!({
        "offset": [anchor.offset_x, anchor.offset_y],
        "align": [anchor.align_h, anchor.align_v],
        "relative": [anchor.relative_h, anchor.relative_v],
        "wrap": anchor.wrap,
        "wrapText": anchor.wrap_text,
        "distance": [anchor.dist_t, anchor.dist_r, anchor.dist_b, anchor.dist_l],
        "behind": anchor.behind_doc,
        "locked": anchor.locked,
        "overlap": anchor.allow_overlap,
        "inCell": anchor.layout_in_cell,
        "relativeHeight": anchor.relative_height,
        "polygon": polygon,
    });
    encode_base64(value.to_string().as_bytes())
}

fn integers<const N: usize>(value: &Value) -> Option<[i64; N]> {
    let entries = value.as_array()?;
    if entries.len() != N {
        return None;
    }
    let mut result = [0; N];
    for (slot, entry) in result.iter_mut().zip(entries) {
        *slot = entry.as_i64()?;
    }
    Some(result)
}

fn setting(value: &Value, allowed: &[&str]) -> Option<String> {
    let value = value.as_str()?;
    allowed.contains(&value).then(|| value.to_owned())
}

fn optional_setting(value: &Value, allowed: &[&str]) -> Option<Option<String>> {
    if value.is_null() {
        Some(None)
    } else {
        Some(Some(setting(value, allowed)?))
    }
}

pub(super) fn decode(encoded: &str) -> Option<FloatingAnchor> {
    let value: Value = serde_json::from_slice(&decode_base64(encoded)?).ok()?;
    let [offset_x, offset_y] = integers(&value["offset"])?;
    let [dist_t, dist_r, dist_b, dist_l] = integers(&value["distance"])?;
    let align = value["align"].as_array()?;
    let [align_h, align_v] = align.as_slice() else {
        return None;
    };
    let relative = value["relative"].as_array()?;
    let [relative_h, relative_v] = relative.as_slice() else {
        return None;
    };
    let polygon = &value["polygon"];
    let wrap_polygon = if polygon.is_null() {
        None
    } else {
        let point = |value: &Value| {
            let [x, y] = integers(value)?;
            Some(WrapPoint::new(x, y))
        };
        Some(WrapPolygon::new(
            point(polygon.get("start")?)?,
            polygon["lineTo"]
                .as_array()?
                .iter()
                .map(point)
                .collect::<Option<_>>()?,
            polygon["edited"].as_bool()?,
        ))
    };

    Some(FloatingAnchor {
        offset_x,
        offset_y,
        frame_size: None,
        align_h: optional_setting(align_h, &["left", "center", "right", "inside", "outside"])?,
        align_v: optional_setting(align_v, &["top", "center", "bottom", "inside", "outside"])?,
        relative_h: setting(
            relative_h,
            &[
                "character",
                "column",
                "margin",
                "page",
                "leftMargin",
                "rightMargin",
                "insideMargin",
                "outsideMargin",
            ],
        )?,
        relative_v: setting(
            relative_v,
            &[
                "line",
                "paragraph",
                "margin",
                "page",
                "topMargin",
                "bottomMargin",
                "insideMargin",
                "outsideMargin",
            ],
        )?,
        wrap: setting(
            &value["wrap"],
            &["none", "square", "tight", "through", "topAndBottom"],
        )?,
        wrap_text: optional_setting(
            &value["wrapText"],
            &["bothSides", "left", "right", "largest"],
        )?,
        behind_doc: value["behind"].as_bool()?,
        dist_t,
        dist_b,
        dist_l,
        dist_r,
        locked: value["locked"].as_bool()?,
        allow_overlap: value["overlap"].as_bool()?,
        layout_in_cell: value["inCell"].as_bool()?,
        relative_height: u32::try_from(value["relativeHeight"].as_u64()?).ok()?,
        wrap_polygon,
        glue: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_wrap_polygon_and_rejects_invalid_metadata() {
        let anchor = FloatingAnchor {
            wrap: "tight".into(),
            align_h: Some("right".into()),
            relative_h: "margin".into(),
            wrap_polygon: Some(WrapPolygon::new(
                WrapPoint::new(0, 0),
                vec![
                    WrapPoint::new(0, 21600),
                    WrapPoint::new(21600, 0),
                    WrapPoint::new(0, 0),
                ],
                true,
            )),
            ..Default::default()
        };
        assert_eq!(decode(&encode(&anchor)), Some(anchor));
        assert!(decode("not-base64").is_none());
        assert!(decode(&encode_base64(b"{}")).is_none());
    }
}

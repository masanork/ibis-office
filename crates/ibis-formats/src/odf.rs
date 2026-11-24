// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use roxmltree::Node;

pub mod enhanced_geometry;

pub const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";

/// Visit elements in source order, replacing selected containers with their
/// children. Other elements are yielded whole, without visiting their children.
pub fn through<'a, 'i>(
    root: Node<'a, 'i>,
    mut descend: impl FnMut(Node<'a, 'i>) -> bool,
) -> impl Iterator<Item = Node<'a, 'i>> {
    let mut stack = vec![root.children()];
    std::iter::from_fn(move || {
        while let Some(children) = stack.last_mut() {
            let Some(node) = children.find(Node::is_element) else {
                stack.pop();
                continue;
            };
            if descend(node) {
                stack.push(node.children());
            } else {
                return Some(node);
            }
        }
        None
    })
}

pub fn visible_section(node: Node) -> bool {
    node.has_tag_name((TEXT, "section"))
        && !matches!(
            node.attribute((TEXT, "display")),
            Some("none" | "condition")
        )
}

pub fn text_container(node: Node) -> bool {
    visible_section(node)
        || (node.tag_name().namespace() == Some(TEXT)
            && matches!(
                node.tag_name().name(),
                "list" | "list-item" | "list-header" | "numbered-paragraph"
            ))
}

pub fn table_rows<'a, 'i>(table: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    through(table, |node| {
        node.tag_name().namespace() == Some(TABLE)
            && matches!(
                node.tag_name().name(),
                "table-row-group" | "table-header-rows" | "table-rows"
            )
    })
    .filter(|node| node.has_tag_name((TABLE, "table-row")))
}

pub fn table_columns<'a, 'i>(table: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    through(table, |node| {
        node.tag_name().namespace() == Some(TABLE)
            && matches!(
                node.tag_name().name(),
                "table-column-group" | "table-header-columns" | "table-columns"
            )
    })
    .filter(|node| node.has_tag_name((TABLE, "table-column")))
}

pub fn row_hidden(row: Node) -> bool {
    axis_hidden(row, "table-row-group")
}

pub fn column_hidden(column: Node) -> bool {
    axis_hidden(column, "table-column-group")
}

fn axis_hidden(node: Node, group: &str) -> bool {
    matches!(
        node.attribute((TABLE, "visibility")),
        Some("collapse" | "filter")
    ) || node
        .ancestors()
        .skip(1)
        .take_while(|n| !n.has_tag_name((TABLE, "table")))
        .any(|n| {
            n.has_tag_name((TABLE, group))
                && matches!(n.attribute((TABLE, "display")), Some("false" | "0"))
        })
}

/// The value of a number list "a b c" in a transform.
pub fn numbers(s: &str) -> Vec<f64> {
    s.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| t.trim_end_matches(|c: char| c.is_ascii_alphabetic()).parse::<f64>().ok()).collect()
}

/// Where the `draw:transform` puts a box of `w` x `h` (EMU) that is drawn with its top-left corner at the origin: the centre of the
/// box and the angle it is turned by (radians, counterclockwise). The operations apply in the order they are listed, to the box in turn:
/// `rotate(a)` turns about the origin, `translate(x y)` moves (LibreOffice writes `rotate(a) translate(x y)`, Word `translate(-w/2 -h/2)
/// rotate(a) translate(cx cy)`, which both put the centre where the last translation says; measured with `-SaveAs todocx`, where the
/// shape's offset is that centre minus half the size).
pub fn placed_box(text: &str, w: f64, h: f64, emu: fn(&str) -> Option<i64>) -> (f64, f64, f64) {
    let (mut angle, mut cx, mut cy) = (0.0f64, w / 2.0, h / 2.0);
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().to_string();
        let Some(close) = rest[open..].find(')') else { break };
        let args = &rest[open + 1..open + close];
        match name.as_str() {
            "rotate" => {
                let a = numbers(args).first().copied().unwrap_or(0.0);
                let (cos, sin) = (a.cos(), a.sin());
                (cx, cy) = (cx * cos + cy * sin, -cx * sin + cy * cos);
                angle += a;
            }
            "translate" => {
                let parts: Vec<&str> = args.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).collect();
                cx += parts.first().and_then(|p| emu(p)).unwrap_or(0) as f64;
                cy += parts.get(1).and_then(|p| emu(p)).unwrap_or(0) as f64;
            }
            _ => {}
        }
        rest = &rest[open + close + 1..];
    }
    (cx, cy, angle)
}

/// `rot` of an `a:xfrm` (1/60000 degree, clockwise) for an angle of a `draw:transform` (radians, counterclockwise).
pub fn ooxml_rotation(angle: f64) -> i64 {
    ((-angle.to_degrees()).rem_euclid(360.0) * 60_000.0).round() as i64 % 21_600_000
}

/// The pixel size of a picture file (PNG and JPEG), and the density if it says one, as the picture's size in points.
pub fn picture_size(data: &[u8]) -> Option<(f64, f64)> {
    if data.starts_with(b"\x89PNG") {
        let w = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?) as f64;
        let h = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?) as f64;
        // The physical size, when given (pHYs: pixels per metre).
        let mut at = 8;
        while at + 12 <= data.len() {
            let len = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize;
            if data.get(at + 4..at + 8)? == b"pHYs" {
                let ppx = u32::from_be_bytes(data.get(at + 8..at + 12)?.try_into().ok()?) as f64;
                let ppy = u32::from_be_bytes(data.get(at + 12..at + 16)?.try_into().ok()?) as f64;
                let unit = *data.get(at + 16)?;
                if unit == 1 && ppx > 0.0 && ppy > 0.0 {
                    return Some((w / ppx * 72.0 / 0.0254, h / ppy * 72.0 / 0.0254));
                }
            }
            at += 12 + len;
        }
        return Some((w * 0.75, h * 0.75));
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        let mut at = 2;
        let mut density: Option<(f64, f64)> = None;
        while at + 9 < data.len() {
            if data[at] != 0xFF {
                at += 1;
                continue;
            }
            let marker = data[at + 1];
            let len = usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
            if marker == 0xE0 && data.get(at + 4..at + 9) == Some(b"JFIF\0") {
                let unit = *data.get(at + 11)?;
                let dx = f64::from(u16::from_be_bytes([*data.get(at + 12)?, *data.get(at + 13)?]));
                let dy = f64::from(u16::from_be_bytes([*data.get(at + 14)?, *data.get(at + 15)?]));
                density = match unit {
                    1 if dx > 0.0 && dy > 0.0 => Some((72.0 / dx, 72.0 / dy)),
                    2 if dx > 0.0 && dy > 0.0 => Some((72.0 / 2.54 / dx, 72.0 / 2.54 / dy)),
                    _ => None,
                };
            }
            if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                let h = f64::from(u16::from_be_bytes([data[at + 5], data[at + 6]]));
                let w = f64::from(u16::from_be_bytes([data[at + 7], data[at + 8]]));
                let (px, py) = density.unwrap_or((0.75, 0.75));
                return Some((w * px, h * py));
            }
            at += 2 + len;
        }
    }
    None
}

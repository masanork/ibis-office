// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::ops::Range;

use ibis_pdf::{Error, Result};
use roxmltree::Node;

use crate::{VsdxContainer, child, named, xml};

type Patch = (Range<usize>, String);
type XmlNode<'a, 'i> = Node<'a, 'i>;

#[derive(Debug, Clone, Copy)]
pub enum Endpoint {
    Begin,
    End,
}

#[derive(Debug, Clone, Copy)]
pub enum GeometryEdit {
    /// Translation in page units (inches, positive upward).
    Move { shape: u32, dx: f64, dy: f64 },
    Reconnect {
        connector: u32,
        endpoint: Endpoint,
        target: u32,
    },
}

impl VsdxContainer {
    /// Patch one page and retain the other parts of the original package.
    pub fn edit_geometry(&self, page: usize, edit: GeometryEdit) -> Result<Vec<u8>> {
        self.update_pages([(page, move |source| patch_page(source, edit))])
    }
}

fn unsupported() -> Error {
    Error::new("This Visio shape or connector uses unsupported geometry, routing, or formulas")
}

fn patch_page(mut source: String, edit: GeometryEdit) -> Result<Option<Vec<u8>>> {
    let mut patches = Vec::new();
    {
        let doc = xml(&source)?;
        let root = doc.root_element();
        let shapes = child(root, "Shapes").ok_or_else(unsupported)?;
        let links: Vec<_> = child(root, "Connects")
            .into_iter()
            .flat_map(|node| node.children())
            .filter(|&node| named(node, "Connect"))
            .collect();
        match edit {
            GeometryEdit::Move { shape, dx, dy } => {
                let node = shape_by_id(shapes, shape)?;
                let (x, y) = movable_pin(node)?;
                if !dx.is_finite() || !dy.is_finite() || dx.abs().max(dy.abs()) > 1000.0 {
                    return Err(unsupported());
                }
                if dx == 0.0 && dy == 0.0 {
                    return Ok(None);
                }
                if links
                    .iter()
                    .any(|&link| id(link, "FromSheet") == Some(shape))
                {
                    return Err(unsupported());
                }
                number_patch(&mut patches, cell(node, "PinX")?, x + dx)?;
                number_patch(&mut patches, cell(node, "PinY")?, y + dy)?;
                let mut connectors = std::collections::BTreeSet::new();
                for &link in &links {
                    if id(link, "ToSheet") == Some(shape) {
                        connectors.insert(id(link, "FromSheet").ok_or_else(unsupported)?);
                    }
                }
                for connector in connectors {
                    let line = horizontal_line(shapes, &links, connector)?;
                    let [mut start, mut end] = line.points;
                    for (point, link) in [(&mut start, line.links[0]), (&mut end, line.links[1])] {
                        if id(link, "ToSheet") == Some(shape) {
                            point.0 += dx;
                            point.1 += dy;
                        }
                    }
                    line.patch(&mut patches, start, end)?;
                }
            }
            GeometryEdit::Reconnect {
                connector,
                endpoint,
                target,
            } => {
                let line = horizontal_line(shapes, &links, connector)?;
                let idx = match endpoint {
                    Endpoint::Begin => 0,
                    Endpoint::End => 1,
                };
                if id(line.links[1 - idx], "ToSheet") == Some(target) {
                    return Err(unsupported());
                }
                let old = id(line.links[idx], "ToSheet").ok_or_else(unsupported)?;
                if old == target {
                    return Ok(None);
                }
                let old_node = shape_by_id(shapes, old)?;
                let new_node = shape_by_id(shapes, target)?;
                let (old_x, old_y) = movable_pin(old_node)?;
                let (new_x, new_y) = movable_pin(new_node)?;
                let old_width = value(old_node, "Width")?;
                let new_width = value(new_node, "Width")?;
                let old_height = value(old_node, "Height")?;
                let new_height = value(new_node, "Height")?;
                if [old_width, new_width, old_height, new_height]
                    .into_iter()
                    .any(|v| v <= 0.0)
                    || [old_node, new_node].into_iter().any(|n| {
                        ["Angle", "FlipX", "FlipY"].into_iter().any(|name| {
                            cell(n, name).is_ok_and(|c| {
                                c.attribute("V").and_then(|v| v.parse::<f64>().ok()) != Some(0.0)
                            })
                        })
                    })
                {
                    return Err(unsupported());
                }
                let mut points = line.points;
                let (x, y) = points[idx];
                points[idx] = (
                    new_x + (x - old_x) * new_width / old_width,
                    new_y + (y - old_y) * new_height / old_height,
                );
                line.patch(&mut patches, points[0], points[1])?;
                attribute_patch(&mut patches, line.links[idx], "ToSheet", target.to_string())?;
                let trigger = cell(
                    line.shape,
                    if idx == 0 { "BegTrigger" } else { "EndTrigger" },
                )?;
                attribute_patch(
                    &mut patches,
                    trigger,
                    "F",
                    format!("_XFTRIGGER(Sheet.{target}!EventXFMod)"),
                )?;
            }
        }
    }
    patches.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, value) in patches {
        source.replace_range(range, &value);
    }
    Ok(Some(source.into_bytes()))
}

fn id(node: XmlNode<'_, '_>, key: &str) -> Option<u32> {
    node.attribute(key)?.parse().ok()
}

fn shape_by_id<'a, 'i>(shapes: XmlNode<'a, 'i>, wanted: u32) -> Result<XmlNode<'a, 'i>> {
    let mut matches = shapes
        .children()
        .filter(|&node| named(node, "Shape") && id(node, "ID") == Some(wanted));
    matches
        .next()
        .filter(|_| matches.next().is_none())
        .ok_or_else(unsupported)
}

fn cell<'a, 'i>(node: XmlNode<'a, 'i>, name: &str) -> Result<XmlNode<'a, 'i>> {
    let mut matches = node
        .children()
        .filter(|&child| named(child, "Cell") && child.attribute("N") == Some(name));
    matches
        .next()
        .filter(|_| matches.next().is_none())
        .ok_or_else(unsupported)
}

fn value(node: XmlNode<'_, '_>, name: &str) -> Result<f64> {
    cell(node, name)?
        .attribute("V")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .ok_or_else(unsupported)
}

fn movable_pin(node: XmlNode<'_, '_>) -> Result<(f64, f64)> {
    if node.attribute("Type") == Some("Group")
        || child(node, "Shapes").is_some()
        || cell(node, "BeginX").is_ok()
        || ["PinX", "PinY"]
            .into_iter()
            .any(|key| cell(node, key).is_ok_and(|c| c.attribute("F").is_some()))
        || cell(node, "LockMoveX").is_ok_and(|_| value(node, "LockMoveX").ok() != Some(0.0))
        || cell(node, "LockMoveY").is_ok_and(|_| value(node, "LockMoveY").ok() != Some(0.0))
    {
        return Err(unsupported());
    }
    Ok((value(node, "PinX")?, value(node, "PinY")?))
}

fn attribute_patch(
    patches: &mut Vec<Patch>,
    node: XmlNode<'_, '_>,
    key: &str,
    value: String,
) -> Result<()> {
    let range = node
        .attribute_node(key)
        .ok_or_else(unsupported)?
        .range_value();
    patches.push((range, value));
    Ok(())
}

fn number_patch(patches: &mut Vec<Patch>, node: XmlNode<'_, '_>, value: f64) -> Result<()> {
    if !value.is_finite() || value.abs() > 1_000_000.0 {
        return Err(unsupported());
    }
    attribute_patch(patches, node, "V", value.to_string())
}

struct HorizontalLine<'a, 'i> {
    shape: XmlNode<'a, 'i>,
    links: [XmlNode<'a, 'i>; 2],
    points: [(f64, f64); 2],
    line_x: XmlNode<'a, 'i>,
}

fn horizontal_line<'a, 'i>(
    shapes: XmlNode<'a, 'i>,
    links: &[XmlNode<'a, 'i>],
    connector: u32,
) -> Result<HorizontalLine<'a, 'i>> {
    let shape = shape_by_id(shapes, connector)?;
    if child(shape, "Shapes").is_some()
        || links
            .iter()
            .any(|&link| id(link, "ToSheet") == Some(connector))
    {
        return Err(unsupported());
    }
    let outgoing: Vec<_> = links
        .iter()
        .copied()
        .filter(|&link| id(link, "FromSheet") == Some(connector))
        .collect();
    let [first, second] = outgoing.as_slice() else {
        return Err(unsupported());
    };
    let [begin, end] = if first.attribute("FromCell") == Some("BeginX") {
        [*first, *second]
    } else {
        [*second, *first]
    };
    for (link, prefix, trigger, opposite) in [
        (begin, "Begin", "BegTrigger", "EndTrigger"),
        (end, "End", "EndTrigger", "BegTrigger"),
    ] {
        let target = id(link, "ToSheet").ok_or_else(unsupported)?;
        if link.attribute("FromCell") != Some(if prefix == "Begin" { "BeginX" } else { "EndX" })
            || !matches!(link.attribute("ToCell"), None | Some("PinX"))
            || cell(shape, trigger)?.attribute("F")
                != Some(&format!("_XFTRIGGER(Sheet.{target}!EventXFMod)"))
        {
            return Err(unsupported());
        }
        let formula = format!("_WALKGLUE({trigger},{opposite},WalkPreference)");
        for axis in ["X", "Y"] {
            if cell(shape, &format!("{prefix}{axis}"))?.attribute("F") != Some(formula.as_str()) {
                return Err(unsupported());
            }
        }
    }
    let points = [
        (value(shape, "BeginX")?, value(shape, "BeginY")?),
        (value(shape, "EndX")?, value(shape, "EndY")?),
    ];
    let mut sections = shape
        .children()
        .filter(|&n| named(n, "Section") && n.attribute("N") == Some("Geometry"));
    let geometry = sections
        .next()
        .filter(|_| sections.next().is_none())
        .ok_or_else(unsupported)?;
    let rows: Vec<_> = geometry.children().filter(|&n| named(n, "Row")).collect();
    let [start, end_row] = rows.as_slice() else {
        return Err(unsupported());
    };
    let width = value(shape, "Width")?;
    let close = |a: f64, b: f64| (a - b).abs() < 1e-5;
    if start.attribute("T") != Some("MoveTo")
        || end_row.attribute("T") != Some("LineTo")
        || !close(value(*start, "X")?, 0.0)
        || !close(value(*end_row, "X")?, width)
        || !close(value(*start, "Y")?, value(*end_row, "Y")?)
        || !close(points[0].1, points[1].1)
        || !close(points[1].0 - points[0].0, width)
        || !close(value(shape, "PinX")?, (points[0].0 + points[1].0) / 2.0)
        || !close(value(shape, "PinY")?, points[0].1)
        || !close(value(shape, "LocPinX")?, width / 2.0)
        || !close(value(shape, "LocPinY")?, value(*start, "Y")?)
        || ["Angle", "FlipX", "FlipY"]
            .into_iter()
            .any(|key| cell(shape, key).is_ok() && value(shape, key).ok() != Some(0.0))
        || ["PinX", "PinY", "Width", "LocPinX"]
            .into_iter()
            .any(|key| cell(shape, key).is_ok_and(|c| c.attribute("F").is_some()))
        || cell(*end_row, "X")?.attribute("F").is_some()
    {
        return Err(unsupported());
    }
    Ok(HorizontalLine {
        shape,
        links: [begin, end],
        points,
        line_x: cell(*end_row, "X")?,
    })
}

impl HorizontalLine<'_, '_> {
    fn patch(&self, patches: &mut Vec<Patch>, start: (f64, f64), end: (f64, f64)) -> Result<()> {
        let width = end.0 - start.0;
        if width <= 1e-5 || (end.1 - start.1).abs() >= 1e-5 {
            return Err(unsupported());
        }
        for (name, value) in [
            ("BeginX", start.0),
            ("BeginY", start.1),
            ("EndX", end.0),
            ("EndY", end.1),
            ("PinX", (start.0 + end.0) / 2.0),
            ("PinY", start.1),
            ("Width", width),
            ("LocPinX", width / 2.0),
        ] {
            number_patch(patches, cell(self.shape, name)?, value)?;
        }
        number_patch(patches, self.line_x, width)
    }
}

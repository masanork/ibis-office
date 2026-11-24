// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::{collections::BTreeSet, ops::Range};

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
    /// Translation in parent coordinates (inches, positive upward).
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
                let mut candidates = shapes
                    .descendants()
                    .filter(|&node| named(node, "Shape") && id(node, "ID") == Some(shape));
                let node = candidates
                    .next()
                    .filter(|_| candidates.next().is_none())
                    .ok_or_else(unsupported)?;
                let nested = node.parent() != Some(shapes);
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
                if nested || is_group(node) {
                    let subtree: BTreeSet<_> = node
                        .descendants()
                        .filter(|&item| named(item, "Shape"))
                        .filter_map(|item| id(item, "ID"))
                        .collect();
                    if links.iter().any(|&link| {
                        let from = id(link, "FromSheet").is_some_and(|id| subtree.contains(&id));
                        let to = id(link, "ToSheet").is_some_and(|id| subtree.contains(&id));
                        from != to
                    }) {
                        return Err(unsupported());
                    }
                }
                number_patch(&mut patches, cell(node, "PinX")?, x + dx)?;
                number_patch(&mut patches, cell(node, "PinY")?, y + dy)?;
                let mut connectors = BTreeSet::new();
                for &link in &links {
                    if id(link, "ToSheet") == Some(shape) {
                        connectors.insert(id(link, "FromSheet").ok_or_else(unsupported)?);
                    }
                }
                for connector in connectors {
                    let line = editable_route(shapes, &links, connector)?;
                    let [mut start, mut end] = line.points;
                    for (point, link) in [(&mut start, line.links[0]), (&mut end, line.links[1])] {
                        if id(link, "ToSheet") == Some(shape) {
                            if let Some(row) = fixed_row(link) {
                                let saved = connection_point(node, row)?;
                                if !close(saved.0, point.0) || !close(saved.1, point.1) {
                                    return Err(unsupported());
                                }
                            }
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
                let line = editable_route(shapes, &links, connector)?;
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
                if is_group(old_node) || is_group(new_node) {
                    return Err(unsupported());
                }
                let link = line.links[idx];
                let mut points = line.points;
                points[idx] = if let Some(row) = fixed_row(link) {
                    let old_point = connection_point(old_node, row)?;
                    if !close(old_point.0, points[idx].0) || !close(old_point.1, points[idx].1) {
                        return Err(unsupported());
                    }
                    let new_point = connection_point(new_node, row)?;
                    let prefix = if idx == 0 { "Begin" } else { "End" };
                    for axis in ["X", "Y"] {
                        attribute_patch(
                            &mut patches,
                            cell(line.shape, &format!("{prefix}{axis}"))?,
                            "F",
                            fixed_formula(target, row),
                        )?;
                    }
                    if let Ok(trigger) = cell(
                        line.shape,
                        if idx == 0 { "BegTrigger" } else { "EndTrigger" },
                    ) {
                        if trigger.attribute("F")
                            != Some(&format!("_XFTRIGGER(Sheet.{old}!EventXFMod)"))
                        {
                            return Err(unsupported());
                        }
                        attribute_patch(
                            &mut patches,
                            trigger,
                            "F",
                            format!("_XFTRIGGER(Sheet.{target}!EventXFMod)"),
                        )?;
                    }
                    new_point
                } else {
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
                                    c.attribute("V").and_then(|v| v.parse::<f64>().ok())
                                        != Some(0.0)
                                })
                            })
                        })
                    {
                        return Err(unsupported());
                    }
                    let (x, y) = points[idx];
                    (
                        new_x + (x - old_x) * new_width / old_width,
                        new_y + (y - old_y) * new_height / old_height,
                    )
                };
                line.patch(&mut patches, points[0], points[1])?;
                attribute_patch(&mut patches, link, "ToSheet", target.to_string())?;
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

fn is_group(node: XmlNode<'_, '_>) -> bool {
    node.attribute("Type") == Some("Group") || child(node, "Shapes").is_some()
}

fn movable_pin(node: XmlNode<'_, '_>) -> Result<(f64, f64)> {
    if cell(node, "BeginX").is_ok()
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

struct EditableRoute<'a, 'i> {
    shape: XmlNode<'a, 'i>,
    links: [XmlNode<'a, 'i>; 2],
    points: [(f64, f64); 2],
    vertices: Vec<([XmlNode<'a, 'i>; 2], [f64; 2])>,
    size: [f64; 2],
}

fn fixed_row(link: XmlNode<'_, '_>) -> Option<u32> {
    let row: u32 = link
        .attribute("ToCell")?
        .strip_prefix("Connections.X")?
        .parse()
        .ok()?;
    let part = 99_u32.checked_add(row)?;
    (row != 0 && id(link, "ToPart") == Some(part)).then_some(row)
}

fn fixed_formula(target: u32, row: u32) -> String {
    format!("PAR(PNT(Sheet.{target}!Connections.X{row},Sheet.{target}!Connections.Y{row}))")
}

fn connection_point(shape: XmlNode<'_, '_>, row: u32) -> Result<(f64, f64)> {
    if ["Angle", "FlipX", "FlipY"].into_iter().any(|key| {
        cell(shape, key)
            .is_ok_and(|c| c.attribute("F").is_some() || value(shape, key).ok() != Some(0.0))
    }) {
        return Err(unsupported());
    }
    let mut sections = shape
        .children()
        .filter(|&n| named(n, "Section") && n.attribute("N") == Some("Connection"));
    let section = sections
        .next()
        .filter(|_| sections.next().is_none())
        .ok_or_else(unsupported)?;
    let mut rows = section
        .children()
        .filter(|&n| named(n, "Row") && id(n, "IX") == Some(row - 1));
    let point = rows
        .next()
        .filter(|_| rows.next().is_none())
        .ok_or_else(unsupported)?;
    if point.attribute("Del") == Some("1") {
        return Err(unsupported());
    }
    let (x, y) = movable_pin(shape)?;
    Ok((
        x + value(point, "X")? - value(shape, "LocPinX")?,
        y + value(point, "Y")? - value(shape, "LocPinY")?,
    ))
}

fn editable_route<'a, 'i>(
    shapes: XmlNode<'a, 'i>,
    links: &[XmlNode<'a, 'i>],
    connector: u32,
) -> Result<EditableRoute<'a, 'i>> {
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
    let [begin, end_link] = if first.attribute("FromCell") == Some("BeginX") {
        [*first, *second]
    } else {
        [*second, *first]
    };
    for (link, prefix, trigger, opposite) in [
        (begin, "Begin", "BegTrigger", "EndTrigger"),
        (end_link, "End", "EndTrigger", "BegTrigger"),
    ] {
        let target = id(link, "ToSheet").ok_or_else(unsupported)?;
        if link.attribute("FromCell") != Some(if prefix == "Begin" { "BeginX" } else { "EndX" }) {
            return Err(unsupported());
        }
        let formula = if let Some(row) = fixed_row(link) {
            fixed_formula(target, row)
        } else if matches!(link.attribute("ToCell"), None | Some("PinX"))
            && cell(shape, trigger)?.attribute("F")
                == Some(&format!("_XFTRIGGER(Sheet.{target}!EventXFMod)"))
        {
            format!("_WALKGLUE({trigger},{opposite},WalkPreference)")
        } else {
            return Err(unsupported());
        };
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
    let width = value(shape, "Width")?;
    let height = value(shape, "Height")?;
    let [start, end] = points;
    let low = [start.0.min(end.0), start.1.min(end.1)];
    let flat = [close(start.0, end.0), close(start.1, end.1)];
    let size = [width, height];
    if width <= 0.0
        || height <= 0.0
        || (flat[0] && flat[1])
        || (!flat[0] && !close((end.0 - start.0).abs(), width))
        || (!flat[1] && !close((end.1 - start.1).abs(), height))
        || !close(value(shape, "PinX")?, (start.0 + end.0) / 2.0)
        || !close(value(shape, "PinY")?, (start.1 + end.1) / 2.0)
        || !close(value(shape, "LocPinX")?, width / 2.0)
        || !close(value(shape, "LocPinY")?, height / 2.0)
        || ["Angle", "FlipX", "FlipY"].into_iter().any(|key| {
            cell(shape, key)
                .is_ok_and(|c| c.attribute("F").is_some() || value(shape, key).ok() != Some(0.0))
        })
        || ["PinX", "PinY", "Width", "Height", "LocPinX", "LocPinY"]
            .into_iter()
            .any(|key| cell(shape, key).is_ok_and(|c| c.attribute("F").is_some()))
    {
        return Err(unsupported());
    }

    let mut vertices = Vec::new();
    let mut first = None;
    let mut previous: Option<[f64; 2]> = None;
    let mut count = 0;
    for row in geometry.children().filter(|&node| named(node, "Row")) {
        let kind = match (count, row.attribute("T")) {
            (0, Some("MoveTo")) | (1.., Some("LineTo")) => false,
            (0, Some("RelMoveTo")) | (1.., Some("RelLineTo" | "RelCubBezTo")) => true,
            _ => return Err(unsupported()),
        };
        let cells = [cell(row, "X")?, cell(row, "Y")?];
        let raw = [value(row, "X")?, value(row, "Y")?];
        let point = if kind {
            [raw[0] * width, raw[1] * height]
        } else {
            raw
        };
        let cubic = row.attribute("T") == Some("RelCubBezTo");
        if cubic {
            for key in ["A", "B", "C", "D"] {
                if cell(row, key)?.attribute("F").is_some() {
                    return Err(unsupported());
                }
                value(row, key)?;
            }
        }
        if cells.iter().any(|cell| cell.attribute("F").is_some())
            || (0..2).any(|axis| {
                point[axis] < -1e-5
                    || point[axis] > size[axis] + 1e-5
                    || (flat[axis] && !close(point[axis], size[axis] / 2.0))
            })
            || (!cubic
                && previous
                    .is_some_and(|last| !close(last[0], point[0]) && !close(last[1], point[1])))
        {
            return Err(unsupported());
        }
        if !kind {
            vertices.push((cells, point));
        }
        first.get_or_insert(point);
        previous = Some(point);
        count += 1;
    }
    let locally = |point: (f64, f64)| {
        let value = [point.0, point.1];
        [0, 1].map(|i| {
            if flat[i] {
                size[i] / 2.0
            } else {
                value[i] - low[i]
            }
        })
    };
    let first = first.ok_or_else(unsupported)?;
    let last = previous.ok_or_else(unsupported)?;
    let expected_first = locally(start);
    let expected_last = locally(end);
    if count < 2
        || (0..2).any(|i| !close(first[i], expected_first[i]) || !close(last[i], expected_last[i]))
    {
        return Err(unsupported());
    }
    Ok(EditableRoute {
        shape,
        links: [begin, end_link],
        points,
        vertices,
        size,
    })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-5
}

impl EditableRoute<'_, '_> {
    fn patch(&self, patches: &mut Vec<Patch>, start: (f64, f64), end: (f64, f64)) -> Result<()> {
        let old = [
            self.points[1].0 - self.points[0].0,
            self.points[1].1 - self.points[0].1,
        ];
        let new = [end.0 - start.0, end.1 - start.1];
        let mut size = self.size;
        for i in 0..2 {
            if close(old[i], 0.0) {
                if !close(new[i], 0.0) {
                    return Err(unsupported());
                }
            } else {
                if close(new[i], 0.0) || old[i].signum() != new[i].signum() {
                    return Err(unsupported());
                }
                size[i] = new[i].abs();
            }
        }
        let [width, height] = size;
        let x_scale = width / self.size[0];
        let y_scale = height / self.size[1];
        for (name, value) in [
            ("BeginX", start.0),
            ("BeginY", start.1),
            ("EndX", end.0),
            ("EndY", end.1),
            ("PinX", (start.0 + end.0) / 2.0),
            ("PinY", (start.1 + end.1) / 2.0),
            ("Width", width),
            ("Height", height),
            ("LocPinX", width / 2.0),
            ("LocPinY", height / 2.0),
        ] {
            number_patch(patches, cell(self.shape, name)?, value)?;
        }
        for &([x, y], [local_x, local_y]) in &self.vertices {
            number_patch(patches, x, local_x * x_scale)?;
            number_patch(patches, y, local_y * y_scale)?;
        }
        Ok(())
    }
}

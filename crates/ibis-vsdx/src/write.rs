// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::fmt::Write;

use ibis_ooxml::zip::{EntryMeta, Writer};
use ibis_pdf::{Error, PdfRect, Result};

use super::{MAX_LABEL_CHARS, MAX_PART, PACKAGE_REL, REL, VISIO, VISIO_REL};

// Keep the generated XML under the preview parser's node budget.
const MAX_DRAWING_ITEMS: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Diamond,
    Parallelogram,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Top, Self::Bottom];

    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Top => 2,
            Self::Bottom => 3,
        }
    }

    fn relative(self) -> (f64, f64) {
        match self {
            Self::Left => (0.0, 0.5),
            Self::Right => (1.0, 0.5),
            Self::Top => (0.5, 1.0),
            Self::Bottom => (0.5, 0.0),
        }
    }
}

struct Shape {
    kind: ShapeKind,
    frame: PdfRect,
    fill: [u8; 3],
    label: String,
}

#[derive(Clone, Copy)]
struct End {
    shape: ShapeId,
    side: Side,
}

struct Connector {
    start: End,
    end: End,
    arrows: (bool, bool),
}

/// A new single-page diagram. Coordinates are points relative to the page's top left.
pub struct Drawing {
    width: f64,
    height: f64,
    shapes: Vec<Shape>,
    connectors: Vec<Connector>,
    label_bytes: usize,
}

impl Drawing {
    pub fn new(width: f64, height: f64) -> Result<Self> {
        if !(1.0..=72_000.0).contains(&width) || !(1.0..=72_000.0).contains(&height) {
            return Err(Error::new("Invalid drawing page size"));
        }
        Ok(Self {
            width,
            height,
            shapes: Vec::new(),
            connectors: Vec::new(),
            label_bytes: 0,
        })
    }

    pub fn add_shape(
        &mut self,
        kind: ShapeKind,
        frame: PdfRect,
        label: impl Into<String>,
        fill: [u8; 3],
    ) -> Result<ShapeId> {
        self.check_item_limit()?;
        self.check_frame(frame)?;
        let label = label.into();
        if self.label_bytes.saturating_add(label.len()) > MAX_LABEL_CHARS
            || label.chars().any(|c| !matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
        {
            return Err(Error::new("Invalid or oversized diagram label"));
        }
        self.label_bytes += label.len();
        self.shapes.push(Shape {
            kind,
            frame,
            fill,
            label,
        });
        Ok(ShapeId(self.shapes.len()))
    }

    pub fn move_shape(&mut self, shape: ShapeId, frame: PdfRect) -> Result<()> {
        self.check_frame(frame)?;
        self.shapes
            .get_mut(shape.0 - 1)
            .ok_or_else(|| Error::new("Unknown drawing shape"))?
            .frame = frame;
        Ok(())
    }

    pub fn connect(&mut self, start: (ShapeId, Side), end: (ShapeId, Side)) -> Result<()> {
        self.connect_with_arrows(start, end, (false, true))
    }

    /// Connect two shapes with an optional triangular arrow at each end.
    pub fn connect_with_arrows(
        &mut self,
        start: (ShapeId, Side),
        end: (ShapeId, Side),
        arrows: (bool, bool),
    ) -> Result<()> {
        self.check_item_limit()?;
        if start.0 == end.0
            || start.0.0 == 0
            || end.0.0 == 0
            || start.0.0 > self.shapes.len()
            || end.0.0 > self.shapes.len()
        {
            return Err(Error::new(
                "A connector must join two different drawing shapes",
            ));
        }
        self.connectors.push(Connector {
            start: End {
                shape: start.0,
                side: start.1,
            },
            end: End {
                shape: end.0,
                side: end.1,
            },
            arrows,
        });
        Ok(())
    }

    pub fn to_vsdx(&self) -> Result<Vec<u8>> {
        let mut zip = Writer::new();
        let mut add = |name, xml: &str| {
            zip.add(name, EntryMeta::new(), xml.as_bytes())
                .map_err(|e| Error::new(e.to_string()))
        };
        add("[Content_Types].xml", CONTENT_TYPES)?;
        add(
            "_rels/.rels",
            &relationship("document", "visio/document.xml"),
        )?;
        add(
            "visio/document.xml",
            &format!(r#"<VisioDocument xmlns="{VISIO}"/>"#),
        )?;
        add(
            "visio/_rels/document.xml.rels",
            &relationship("pages", "pages/pages.xml"),
        )?;
        add(
            "visio/pages/pages.xml",
            &format!(
                r#"<Pages xmlns="{VISIO}" xmlns:r="{REL}"><Page ID="0" Name="Page-1" NameU="Page-1"><PageSheet><Cell N="PageWidth" V="{}"/><Cell N="PageHeight" V="{}"/></PageSheet><Rel r:id="rId1"/></Page></Pages>"#,
                self.width / 72.0,
                self.height / 72.0,
            ),
        )?;
        add(
            "visio/pages/_rels/pages.xml.rels",
            &relationship("page", "page1.xml"),
        )?;
        let page = self.page_xml();
        if page.len() > MAX_PART {
            return Err(Error::new("Drawing page is too large"));
        }
        add("visio/pages/page1.xml", &page)?;
        zip.finish().map_err(|e| Error::new(e.to_string()))
    }

    fn check_item_limit(&self) -> Result<()> {
        if self.shapes.len() + self.connectors.len() >= MAX_DRAWING_ITEMS {
            return Err(Error::new("Too many objects on the drawing page"));
        }
        Ok(())
    }

    fn check_frame(&self, frame: PdfRect) -> Result<()> {
        if (0.0..f64::from(frame.x1)).contains(&f64::from(frame.x0))
            && (0.0..f64::from(frame.y1)).contains(&f64::from(frame.y0))
            && f64::from(frame.x1) <= self.width
            && f64::from(frame.y1) <= self.height
        {
            Ok(())
        } else {
            Err(Error::new("Shape must fit on the drawing page"))
        }
    }

    fn page_xml(&self) -> String {
        let mut out = format!(r#"<PageContents xmlns="{VISIO}"><Shapes>"#);
        // Lay connectors under the shapes. IDs are independent of paint order.
        for (i, connector) in self.connectors.iter().enumerate() {
            self.write_connector(&mut out, self.shapes.len() + 1 + i, connector);
        }
        for (i, shape) in self.shapes.iter().enumerate() {
            self.write_shape(&mut out, i + 1, shape);
        }
        out.push_str("</Shapes>");
        if !self.connectors.is_empty() {
            out.push_str("<Connects>");
            for (i, connector) in self.connectors.iter().enumerate() {
                let id = self.shapes.len() + 1 + i;
                for (cell, part, end) in
                    [("BeginX", 9, connector.start), ("EndX", 12, connector.end)]
                {
                    let row = end.side.index();
                    write!(out, r#"<Connect FromSheet="{id}" FromCell="{cell}" FromPart="{part}" ToSheet="{}" ToCell="Connections.X{}" ToPart="{}"/>"#,
                        end.shape.0, row + 1, 100 + row).unwrap();
                }
            }
            out.push_str("</Connects>");
        }
        out.push_str("</PageContents>");
        out
    }

    fn write_shape(&self, out: &mut String, id: usize, shape: &Shape) {
        let PdfRect { x0, y0, x1, y1 } = shape.frame;
        let (w, h) = (
            (f64::from(x1) - f64::from(x0)) / 72.0,
            (f64::from(y1) - f64::from(y0)) / 72.0,
        );
        let top = self.height - f64::from(y0);
        write!(out, r#"<Shape ID="{id}" Type="Shape">"#).unwrap();
        for (key, value) in [
            ("PinX", f64::from(x0) / 72.0 + w / 2.0),
            ("PinY", top / 72.0 - h / 2.0),
            ("Width", w),
            ("Height", h),
            ("LineWeight", 1.0 / 72.0),
        ] {
            cell(out, key, value);
        }
        for (key, value, formula) in [
            ("LocPinX", w / 2.0, "Width/2"),
            ("LocPinY", h / 2.0, "Height/2"),
            ("TxtWidth", w, "Width"),
            ("TxtHeight", h, "Height"),
        ] {
            formula_cell(out, key, value, formula);
        }
        let [r, g, b] = shape.fill;
        write!(
            out,
            r##"<Cell N="FillForegnd" V="#{r:02X}{g:02X}{b:02X}"/>"##
        )
        .unwrap();
        if shape.kind == ShapeKind::RoundedRectangle {
            cell(out, "Rounding", w.min(h) * 0.166667);
        }
        out.push_str(r#"<Cell N="FillPattern" V="1"/><Cell N="LinePattern" V="1"/><Section N="Geometry" IX="0">"#);
        match shape.kind {
            ShapeKind::Rectangle | ShapeKind::RoundedRectangle => {
                polygon(out, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])
            }
            ShapeKind::Diamond => polygon(out, &[(0.0, 0.5), (0.5, 0.0), (1.0, 0.5), (0.5, 1.0)]),
            ShapeKind::Parallelogram => {
                polygon(out, &[(0.0, 0.0), (0.85, 0.0), (1.0, 1.0), (0.15, 1.0)])
            }
            ShapeKind::Ellipse => {
                out.push_str(r#"<Row T="Ellipse" IX="1">"#);
                for (key, value, formula) in [
                    ("X", w / 2.0, "Width/2"),
                    ("Y", h / 2.0, "Height/2"),
                    ("A", w, "Width"),
                    ("B", h / 2.0, "Height/2"),
                    ("C", w / 2.0, "Width/2"),
                    ("D", h, "Height"),
                ] {
                    formula_cell(out, key, value, formula);
                }
                out.push_str("</Row>");
            }
        }
        out.push_str(r#"</Section><Section N="Connection">"#);
        for side in Side::ALL {
            let (x, y) = side.relative();
            write!(out, r#"<Row IX="{}"><Cell N="X" V="{}" F="Width*{x}"/><Cell N="Y" V="{}" F="Height*{y}"/><Cell N="Type" V="0"/></Row>"#, side.index(), w * x, h * y).unwrap();
        }
        out.push_str("</Section>");
        if !shape.label.is_empty() {
            out.push_str(r##"<Section N="Character"><Row IX="0"><Cell N="Font" V="Arial"/><Cell N="AsianFont" V="Yu Gothic"/><Cell N="Size" V="0.1666666666666667"/></Row></Section>"##);
            write!(
                out,
                r#"<Text xml:space="preserve"><cp IX="0"/>{}</Text>"#,
                ibis_formats::docx_convert::xml_escape(&shape.label)
            )
            .unwrap();
        }
        out.push_str("</Shape>");
    }

    fn endpoint(&self, end: End) -> (f64, f64) {
        let frame = self.shapes[end.shape.0 - 1].frame;
        let (x, y) = end.side.relative();
        (
            (f64::from(frame.x0) + f64::from(frame.width()) * x) / 72.0,
            (self.height - f64::from(frame.y1) + f64::from(frame.height()) * y) / 72.0,
        )
    }

    fn write_connector(&self, out: &mut String, id: usize, connector: &Connector) {
        let (x0, y0) = self.endpoint(connector.start);
        let (x1, y1) = self.endpoint(connector.end);
        let length = (x1 - x0).hypot(y1 - y0);
        write!(out, r#"<Shape ID="{id}" Type="Shape"><Cell N="FillPattern" V="0"/><Cell N="LinePattern" V="1"/>"#).unwrap();
        cell(
            out,
            "BeginArrow",
            if connector.arrows.0 { 4.0 } else { 0.0 },
        );
        cell(out, "EndArrow", if connector.arrows.1 { 4.0 } else { 0.0 });
        for (prefix, end, (x, y)) in [
            ("Begin", connector.start, (x0, y0)),
            ("End", connector.end, (x1, y1)),
        ] {
            let sheet = end.shape.0;
            let row = end.side.index() + 1;
            let formula = format!(
                "PAR(PNT(Sheet.{sheet}!Connections.X{row},Sheet.{sheet}!Connections.Y{row}))"
            );
            write!(out, r#"<Cell N="{prefix}X" V="{x}" F="{formula}"/><Cell N="{prefix}Y" V="{y}" F="{formula}"/>"#).unwrap();
        }
        for (key, value, formula) in [
            ("PinX", (x0 + x1) / 2.0, "(BeginX+EndX)/2"),
            ("PinY", (y0 + y1) / 2.0, "(BeginY+EndY)/2"),
            ("Width", length, "SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)"),
            ("LocPinX", length / 2.0, "Width/2"),
            (
                "Angle",
                (y1 - y0).atan2(x1 - x0),
                "ATAN2(EndY-BeginY,EndX-BeginX)",
            ),
        ] {
            formula_cell(out, key, value, formula);
        }
        write!(out, r#"<Cell N="Height" V="0"/><Cell N="LocPinY" V="0"/><Section N="Geometry" IX="0"><Cell N="NoFill" V="1"/><Row T="MoveTo" IX="1"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row><Row T="LineTo" IX="2"><Cell N="X" V="{length}" F="Width"/><Cell N="Y" V="0"/></Row></Section></Shape>"#).unwrap();
    }
}

fn polygon(out: &mut String, points: &[(f64, f64)]) {
    for (i, &(x, y)) in points.iter().chain(points.first()).enumerate() {
        let kind = if i == 0 { "RelMoveTo" } else { "RelLineTo" };
        write!(out, r#"<Row T="{kind}" IX="{}">"#, i + 1).unwrap();
        cell(out, "X", x);
        cell(out, "Y", y);
        out.push_str("</Row>");
    }
}

fn cell(out: &mut String, name: &str, value: f64) {
    write!(out, r#"<Cell N="{name}" V="{value}"/>"#).unwrap();
}

fn formula_cell(out: &mut String, name: &str, value: f64, formula: &str) {
    write!(out, r#"<Cell N="{name}" V="{value}" F="{formula}"/>"#).unwrap();
}

fn relationship(kind: &str, target: &str) -> String {
    format!(
        r#"<Relationships xmlns="{PACKAGE_REL}"><Relationship Id="rId1" Type="{VISIO_REL}{kind}" Target="{target}"/></Relationships>"#
    )
}

const CONTENT_TYPES: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Override PartName="/visio/document.xml" ContentType="application/vnd.ms-visio.drawing.main+xml"/>
<Override PartName="/visio/pages/pages.xml" ContentType="application/vnd.ms-visio.pages+xml"/>
<Override PartName="/visio/pages/page1.xml" ContentType="application/vnd.ms-visio.page+xml"/>
</Types>"#;

impl Drawing {
    /// Read an unchanged drawing from this writer. Reject packages with any
    /// parts or geometry the editor would discard when saving it again.
    pub fn from_vsdx(bytes: &[u8]) -> Result<Self> {
        use super::{child, named, open_archive, part, xml};

        let zip = open_archive(bytes)?;
        let pages = part(&zip, "visio/pages/pages.xml")?;
        let index = xml(&pages)?;
        let size = child(
            child(index.root_element(), "Page").ok_or_else(super::invalid)?,
            "PageSheet",
        )
        .ok_or_else(super::invalid)?;
        let number = |node: roxmltree::Node<'_, '_>, key| {
            super::number(super::value(Some(node), key)).ok_or_else(super::invalid)
        };
        let mut drawing = Self::new(
            number(size, "PageWidth")? * 72.0,
            number(size, "PageHeight")? * 72.0,
        )?;
        let content = part(&zip, "visio/pages/page1.xml")?;
        let page = xml(&content)?;
        let root = page.root_element();
        let shapes = child(root, "Shapes").ok_or_else(super::invalid)?;
        let mut nodes: Vec<_> = shapes
            .children()
            .filter(|&n| named(n, "Shape"))
            .map(|node| {
                let id = node
                    .attribute("ID")
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or_else(super::invalid)?;
                Ok((id, node))
            })
            .collect::<Result<_>>()?;
        if nodes.len() > MAX_DRAWING_ITEMS {
            return Err(super::invalid());
        }
        nodes.sort_by_key(|(id, _)| *id);
        for (id, node) in nodes
            .iter()
            .copied()
            .take_while(|(_, n)| super::value(Some(*n), "BeginX").is_none())
        {
            if id != drawing.shapes.len() + 1 {
                return Err(super::invalid());
            }
            let geometry = node
                .children()
                .find(|&n| named(n, "Section") && n.attribute("N") == Some("Geometry"))
                .ok_or_else(super::invalid)?;
            let mut rows = geometry.children().filter(|&n| named(n, "Row"));
            let first = rows.next().ok_or_else(super::invalid)?;
            let second = rows.next();
            let kind = match first.attribute("T") {
                Some("Ellipse") => ShapeKind::Ellipse,
                Some("RelMoveTo") if number(first, "Y")? == 0.0 => {
                    if super::value(Some(node), "Rounding").is_some() {
                        ShapeKind::RoundedRectangle
                    } else if second.is_some_and(|row| number(row, "X").is_ok_and(|x| (x - 0.85).abs() < 1e-6)) {
                        ShapeKind::Parallelogram
                    } else {
                        ShapeKind::Rectangle
                    }
                }
                Some("RelMoveTo") => ShapeKind::Diamond,
                _ => return Err(super::invalid()),
            };
            let (w, h) = (
                number(node, "Width")? * 72.0,
                number(node, "Height")? * 72.0,
            );
            let (x, y) = (
                number(node, "PinX")? * 72.0,
                drawing.height - number(node, "PinY")? * 72.0,
            );
            let x0 = x - w / 2.0;
            let y0 = y - h / 2.0;
            let zero = |v: f64| if v.abs() < 1e-9 { 0.0 } else { v as f32 };
            let frame = PdfRect {
                x0: zero(x0),
                y0: zero(y0),
                x1: (x + w / 2.0) as f32,
                y1: (y + h / 2.0) as f32,
            };
            let color = super::value(Some(node), "FillForegnd")
                .and_then(|v| v.strip_prefix('#'))
                .and_then(|v| u32::from_str_radix(v, 16).ok())
                .ok_or_else(super::invalid)?;
            let label: String = child(node, "Text")
                .into_iter()
                .flat_map(|n| n.children())
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect();
            drawing.add_shape(
                kind,
                frame,
                label,
                [(color >> 16) as u8, (color >> 8) as u8, color as u8],
            )?;
        }

        let mut links = std::collections::HashMap::new();
        for link in child(root, "Connects")
            .into_iter()
            .flat_map(|n| n.children())
            .filter(|&n| named(n, "Connect"))
        {
            let parse = |key| {
                link.attribute(key)
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or_else(super::invalid)
            };
            let from = match link.attribute("FromCell") {
                Some("BeginX") => false,
                Some("EndX") => true,
                _ => return Err(super::invalid()),
            };
            let row = link
                .attribute("ToCell")
                .and_then(|s| s.strip_prefix("Connections.X"))
                .and_then(|s| s.parse::<usize>().ok())
                .and_then(|n| n.checked_sub(1))
                .and_then(|n| Side::ALL.get(n).copied())
                .ok_or_else(super::invalid)?;
            if links
                .insert(
                    (parse("FromSheet")?, from),
                    (ShapeId(parse("ToSheet")?), row),
                )
                .is_some()
            {
                return Err(super::invalid());
            }
        }
        for (id, node) in nodes.into_iter().skip(drawing.shapes.len()) {
            if id != drawing.shapes.len() + drawing.connectors.len() + 1 {
                return Err(super::invalid());
            }
            let start = links.remove(&(id, false)).ok_or_else(super::invalid)?;
            let end = links.remove(&(id, true)).ok_or_else(super::invalid)?;
            drawing.connect_with_arrows(
                start,
                end,
                (
                    number(node, "BeginArrow")? == 4.0,
                    number(node, "EndArrow")? == 4.0,
                ),
            )?;
        }
        if drawing.to_vsdx()?.as_slice() != bytes {
            return Err(super::invalid());
        }
        Ok(drawing)
    }

    pub fn page_size(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    pub fn shapes(&self) -> impl Iterator<Item = (ShapeKind, PdfRect, [u8; 3], &str)> {
        self.shapes
            .iter()
            .map(|shape| (shape.kind, shape.frame, shape.fill, shape.label.as_str()))
    }

    /// Return zero-based shape indices, attachment sides, and arrowheads.
    pub fn connectors(&self) -> impl Iterator<Item = ((usize, Side), (usize, Side), (bool, bool))> {
        self.connectors.iter().map(|c| {
            (
                (c.start.shape.0 - 1, c.start.side),
                (c.end.shape.0 - 1, c.end.side),
                c.arrows,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_flowchart_shapes_and_connectors() {
        let mut drawing = Drawing::new(800.0, 600.0).unwrap();
        let s1 = drawing
            .add_shape(
                ShapeKind::RoundedRectangle,
                PdfRect { x0: 40.0, y0: 40.0, x1: 180.0, y1: 110.0 },
                "Subprocess",
                [0x2e, 0x8b, 0x8b],
            )
            .unwrap();
        let s2 = drawing
            .add_shape(
                ShapeKind::Parallelogram,
                PdfRect { x0: 240.0, y0: 40.0, x1: 380.0, y1: 110.0 },
                "Input Data",
                [0x70, 0x30, 0xa0],
            )
            .unwrap();
        let s3 = drawing
            .add_shape(
                ShapeKind::Rectangle,
                PdfRect { x0: 440.0, y0: 40.0, x1: 580.0, y1: 110.0 },
                "Process",
                [0x44, 0x72, 0xc4],
            )
            .unwrap();
        let s4 = drawing
            .add_shape(
                ShapeKind::Diamond,
                PdfRect { x0: 240.0, y0: 200.0, x1: 380.0, y1: 290.0 },
                "Check?",
                [0xed, 0x7d, 0x31],
            )
            .unwrap();
        let s5 = drawing
            .add_shape(
                ShapeKind::Ellipse,
                PdfRect { x0: 440.0, y0: 200.0, x1: 580.0, y1: 270.0 },
                "Done",
                [0x70, 0xad, 0x47],
            )
            .unwrap();
        drawing.connect((s1, Side::Right), (s2, Side::Left)).unwrap();
        drawing.connect((s2, Side::Right), (s3, Side::Left)).unwrap();
        drawing.connect((s2, Side::Bottom), (s4, Side::Top)).unwrap();
        drawing.connect((s4, Side::Right), (s5, Side::Left)).unwrap();

        let bytes = drawing.to_vsdx().unwrap();
        let parsed = Drawing::from_vsdx(&bytes).unwrap();
        let kinds: Vec<ShapeKind> = parsed.shapes().map(|(k, _, _, _)| k).collect();
        assert_eq!(
            kinds,
            vec![
                ShapeKind::RoundedRectangle,
                ShapeKind::Parallelogram,
                ShapeKind::Rectangle,
                ShapeKind::Diamond,
                ShapeKind::Ellipse,
            ]
        );
        assert_eq!(parsed.connectors().count(), 4);

        let container = crate::VsdxContainer::from_bytes(bytes.into()).unwrap();
        container.interpret_page_scene(0).unwrap();
    }
}

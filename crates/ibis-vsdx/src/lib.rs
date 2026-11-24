// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Read-only Visio VSDX pages, using cached ShapeSheet values from the package.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use ibis_formats::zip::Archive;
use ibis_ooxml::Relationship;
use ibis_pdf::scene::{CharBox, FillRule, PathCmd, TextAnchor, estimated_advance_em};
use ibis_pdf::{DisplayCmd, Error, ObjectId, PageInfo, PageScene, PdfRect, Result};
use roxmltree::{Document, Node};

mod geometry;

use geometry::{dashed_line, elliptical_arc, rounded_polygon, straight_vertices, triangle_arrow};

const VISIO: &str = "http://schemas.microsoft.com/office/visio/2012/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const VISIO_REL: &str = "http://schemas.microsoft.com/visio/2010/relationships/";
const MAX_PART: usize = 8 * 1024 * 1024;
const MAX_MASTER_XML: usize = 24 * 1024 * 1024;
const MAX_PAGES: usize = 512;
const MAX_SHAPES: usize = 20_000;
const MAX_PAGE_PATH_COMMANDS: usize = 250_000;
const MAX_LABEL_CHARS: usize = 100_000;

fn invalid() -> Error {
    Error::new("Invalid or unsupported Visio VSDX document")
}

fn named(n: Node<'_, '_>, name: &str) -> bool {
    n.has_tag_name((VISIO, name))
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|&n| named(n, name))
}

fn part(zip: &Archive<'_>, name: &str) -> Result<String> {
    let bytes = zip
        .read_bounded(name, MAX_PART)
        .map_err(|e| Error::new(e.to_string()))?
        .ok_or_else(invalid)?;
    String::from_utf8(bytes).map_err(|_| invalid())
}

fn xml(text: &str) -> Result<Document<'_>> {
    Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            nodes_limit: 250_000,
            ..Default::default()
        },
    )
    .map_err(|_| invalid())
}

fn relationships(zip: &Archive<'_>, source: &str) -> Result<Vec<Relationship>> {
    let text = part(zip, &ibis_ooxml::package::relationships_part(source))?;
    let doc = xml(&text)?;
    if !doc
        .root_element()
        .has_tag_name((PACKAGE_REL, "Relationships"))
    {
        return Err(invalid());
    }
    Ok(doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name((PACKAGE_REL, "Relationship")))
        .filter_map(|n| {
            Some(Relationship {
                id: n.attribute("Id")?.into(),
                kind: n.attribute("Type")?.into(),
                target: n.attribute("Target")?.into(),
                external: n.attribute("TargetMode") == Some("External"),
            })
        })
        .collect())
}

fn target(rels: &[Relationship], source: &str, id: Option<&str>, kind: &str) -> Option<String> {
    rels.iter()
        .find(|r| r.kind.strip_prefix(VISIO_REL) == Some(kind) && id.is_none_or(|id| id == r.id))
        .and_then(|r| r.target_part(source))
}

fn rel_id<'a, 'i>(node: Node<'a, 'i>) -> Option<&'a str> {
    child(node, "Rel")?.attribute((REL, "id"))
}

fn main_part(zip: &Archive<'_>) -> Result<String> {
    let name = target(&relationships(zip, "")?, "", None, "document").ok_or_else(invalid)?;
    let types = part(zip, "[Content_Types].xml")?;
    let path = format!("/{name}");
    let namespace = "http://schemas.openxmlformats.org/package/2006/content-types";
    let has_drawing = xml(&types)?.root_element().children().any(|n| {
        n.has_tag_name((namespace, "Override"))
            && n.attribute("PartName") == Some(path.as_str())
            && n.attribute("ContentType") == Some("application/vnd.ms-visio.drawing.main+xml")
    });
    if !has_drawing || !zip.contains(&name) {
        return Err(invalid());
    }
    Ok(name)
}

fn open_archive(bytes: &[u8]) -> Result<Archive<'_>> {
    if bytes.len() > 128 * 1024 * 1024 || !bytes.starts_with(b"PK") {
        return Err(invalid());
    }
    Archive::new(bytes).map_err(|_| invalid())
}

/// Recognize a VSDX drawing by its document relationship and declared content type.
pub fn is_vsdx(bytes: &[u8]) -> bool {
    open_archive(bytes).and_then(|zip| main_part(&zip)).is_ok()
}

struct Page {
    name: String,
    info: PageInfo,
}

pub struct VsdxContainer {
    bytes: Arc<[u8]>,
    pages: Vec<Page>,
    masters: HashMap<String, String>,
    document: String,
}

impl VsdxContainer {
    pub fn from_bytes(bytes: Arc<[u8]>) -> Result<Self> {
        let zip = open_archive(&bytes)?;
        let main = main_part(&zip)?;
        let document = part(&zip, &main)?;
        if !named(xml(&document)?.root_element(), "VisioDocument") {
            return Err(invalid());
        }
        let rels = relationships(&zip, &main)?;
        let index = target(&rels, &main, None, "pages").ok_or_else(invalid)?;
        let page_index = part(&zip, &index)?;
        let page_rels = relationships(&zip, &index)?;
        let page_xml = xml(&page_index)?;
        let root = page_xml.root_element();
        if !named(root, "Pages") {
            return Err(invalid());
        }
        let mut pages = Vec::new();
        for page in root.children().filter(|&n| named(n, "Page")) {
            let sheet = child(page, "PageSheet").ok_or_else(invalid)?;
            let dimension = |key| {
                number(value(Some(sheet), key))
                    .filter(|&v| v > 0.0 && v <= 1000.0)
                    .map(|v| (72.0 * v) as f32)
                    .ok_or_else(invalid)
            };
            let bounds = PdfRect {
                x0: 0.0,
                y0: 0.0,
                x1: dimension("PageWidth")?,
                y1: dimension("PageHeight")?,
            };
            let name = target(
                &page_rels,
                &index,
                Some(rel_id(page).ok_or_else(invalid)?),
                "page",
            )
            .ok_or_else(invalid)?;
            let index = pages.len();
            if index >= MAX_PAGES || !zip.contains(&name) {
                return Err(invalid());
            }
            pages.push(Page {
                name,
                info: PageInfo {
                    index,
                    object_id: ObjectId::new(index as u32 + 1, 0),
                    media_box: bounds,
                    crop_box: bounds,
                    rotate: 0,
                },
            });
        }
        if pages.is_empty() {
            return Err(invalid());
        }
        let mut masters = HashMap::new();
        if let Some(index) = target(&rels, &main, None, "masters") {
            let index_xml = part(&zip, &index)?;
            let master_rels = relationships(&zip, &index)?;
            for master in xml(&index_xml)?
                .root_element()
                .children()
                .filter(|&n| named(n, "Master"))
            {
                if let (Some(id), Some(rel)) = (master.attribute("ID"), rel_id(master))
                    && let Some(name) = target(&master_rels, &index, Some(rel), "master")
                {
                    masters.insert(id.into(), name);
                }
            }
        }
        Ok(Self {
            bytes,
            pages,
            masters,
            document,
        })
    }

    pub fn page_infos(&self) -> Vec<PageInfo> {
        self.pages.iter().map(|p| p.info.clone()).collect()
    }

    pub fn interpret_page_scene(&self, index: usize) -> Result<PageScene> {
        let page = self.pages.get(index).ok_or_else(invalid)?;
        let zip = Archive::new(&self.bytes).map_err(|_| invalid())?;
        let content = part(&zip, &page.name)?;
        let doc = xml(&content)?;
        if !named(doc.root_element(), "PageContents") {
            return Err(invalid());
        }
        let mut master_parts = HashMap::new();
        let mut total = 0;
        for id in doc
            .root_element()
            .descendants()
            .filter(|&n| named(n, "Shape"))
            .filter_map(|n| n.attribute("Master"))
        {
            if let Some(name) = self.masters.get(id)
                && !master_parts.contains_key(id)
            {
                let text = part(&zip, name)?;
                total += text.len();
                if total > MAX_MASTER_XML {
                    return Err(invalid());
                }
                master_parts.insert(id, text);
            }
        }
        let masters: HashMap<_, _> = master_parts
            .iter()
            .map(|(&id, text)| Ok((id, xml(text)?)))
            .collect::<Result<_>>()?;
        let main = xml(&self.document)?;
        let styles = main
            .root_element()
            .descendants()
            .filter(|&n| named(n, "StyleSheet"))
            .filter_map(|n| Some((n.attribute("ID")?, n)))
            .collect();
        let mut render = Render {
            scene: PageScene {
                info: page.info.clone(),
                display_list: Vec::new(),
                text_index: Default::default(),
            },
            styles,
            shapes: 0,
            path_commands: 0,
        };
        let page_height = f64::from(page.info.crop_box.height());
        let to_points = Matrix([72.0, 0.0, 0.0, -72.0, 0.0, page_height]);
        if let Some(shapes) = child(doc.root_element(), "Shapes") {
            render.shapes(shapes, to_points, &masters, None, 0)?;
        }
        Ok(render.scene)
    }
}

fn value<'a>(n: Option<Node<'a, '_>>, key: &str) -> Option<&'a str> {
    n?.children()
        .find(|&c| named(c, "Cell") && c.attribute("N") == Some(key))?
        .attribute("V")
}

fn number(s: Option<&str>) -> Option<f64> {
    s?.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && v.abs() <= 1_000_000.0)
}

#[derive(Clone, Copy)]
struct Matrix([f64; 6]);

impl Matrix {
    fn point(self, x: f64, y: f64) -> (f32, f32) {
        let [a, b, c, d, tx, ty] = self.0;
        ((a * x + c * y + tx) as f32, (b * x + d * y + ty) as f32)
    }

    fn child(self, rhs: Self) -> Self {
        let [a, b, c, d, tx, ty] = self.0;
        let [e, f, g, h, ux, uy] = rhs.0;
        Self([
            a * e + c * f,
            b * e + d * f,
            a * g + c * h,
            b * g + d * h,
            a * ux + c * uy + tx,
            b * ux + d * uy + ty,
        ])
    }
}

struct Render<'a, 'i> {
    scene: PageScene,
    styles: HashMap<&'a str, Node<'a, 'i>>,
    shapes: usize,
    path_commands: usize,
}

impl Render<'_, '_> {
    fn shapes(
        &mut self,
        shapes: Node<'_, '_>,
        parent: Matrix,
        masters: &HashMap<&str, Document<'_>>,
        parent_master: Option<Node<'_, '_>>,
        depth: usize,
    ) -> Result<()> {
        if depth > 32 {
            return Err(invalid());
        }
        for shape in shapes.children().filter(|&n| named(n, "Shape")) {
            self.shapes += 1;
            if self.shapes > MAX_SHAPES {
                return Err(invalid());
            }
            let master = shape
                .attribute("Master")
                .and_then(|id| masters.get(id))
                .and_then(|doc| child(doc.root_element(), "Shapes"))
                .and_then(|shapes| child(shapes, "Shape"))
                .or_else(|| {
                    let id = shape.attribute("MasterShape")?;
                    child(parent_master?, "Shapes")?
                        .children()
                        .find(|&n| named(n, "Shape") && n.attribute("ID") == Some(id))
                });
            let get = |key| value(Some(shape), key).or_else(|| value(master, key));
            let num = |key| number(get(key));
            let w = num("Width").unwrap_or(0.0);
            let h = num("Height").unwrap_or(0.0);
            let px = num("PinX").unwrap_or(w / 2.0);
            let py = num("PinY").unwrap_or(h / 2.0);
            let lx = num("LocPinX").unwrap_or(w / 2.0);
            let ly = num("LocPinY").unwrap_or(h / 2.0);
            let (sin, cos) = num("Angle").unwrap_or(0.0).sin_cos();
            let fx = if get("FlipX") == Some("1") { -1.0 } else { 1.0 };
            let fy = if get("FlipY") == Some("1") { -1.0 } else { 1.0 };
            let m = parent.child(Matrix([
                cos * fx,
                sin * fx,
                -sin * fy,
                cos * fy,
                px - cos * fx * lx + sin * fy * ly,
                py - sin * fx * lx - cos * fy * ly,
            ]));
            let paint = |style, key| {
                get(key).or_else(|| {
                    let id = shape
                        .attribute(style)
                        .or_else(|| master?.attribute(style))?;
                    value(self.styles.get(id).copied(), key)
                })
            };
            let fill = (paint("FillStyle", "FillPattern") != Some("0"))
                .then(|| color(paint("FillStyle", "FillForegnd"), [255, 255, 255, 255]));
            let stroke = (paint("LineStyle", "LinePattern") != Some("0"))
                .then(|| color(paint("LineStyle", "LineColor"), [0, 0, 0, 255]));
            let weight = number(paint("LineStyle", "LineWeight"))
                .unwrap_or(0.01)
                .clamp(0.0, 5.0) as f32
                * 72.0;
            let line = |key| paint("LineStyle", key);
            let dash = line("LinePattern") == Some("2");
            let arrows = (
                line("BeginArrow") == Some("4"),
                line("EndArrow") == Some("4"),
            );
            let rounding = num("Rounding").unwrap_or_default() as f32 * 72.0;
            let geometry = indexed_children(master, Some(shape), "Section", |node| {
                node.attribute("N") == Some("Geometry")
            });
            let scales = (
                inherited_scale(w, master, "Width"),
                inherited_scale(h, master, "Height"),
            );
            let mut drew_geometry = false;
            let mut hidden_geometry = false;
            for (base, local) in geometry.into_values() {
                let get = |key| value(local, key).or_else(|| value(base, key));
                if local.and_then(|n| n.attribute("Del")) == Some("1") || get("NoShow") == Some("1")
                {
                    hidden_geometry = true;
                    continue;
                }
                let commands = geometry_path(base, local, m, (w, h), scales);
                let fill = fill.filter(|_| get("NoFill") != Some("1"));
                let stroke = stroke.filter(|_| get("NoLine") != Some("1"));
                drew_geometry |= commands.len() >= 2;
                self.draw_geometry(commands, fill, stroke, weight, (dash, arrows), rounding)?;
            }
            if !drew_geometry
                && !hidden_geometry
                && let (Some(x0), Some(y0), Some(x1), Some(y1)) =
                    (num("BeginX"), num("BeginY"), num("EndX"), num("EndY"))
            {
                let (x0, y0) = parent.point(x0, y0);
                let (x1, y1) = parent.point(x1, y1);
                self.draw_geometry(
                    vec![PathCmd::MoveTo(x0, y0), PathCmd::LineTo(x1, y1)],
                    None,
                    stroke,
                    weight,
                    (dash, arrows),
                    0.0,
                )?;
            }
            if let Some(sub) = child(shape, "Shapes") {
                self.shapes(sub, m, masters, master, depth + 1)?;
            }
            if let Some(label) = child(shape, "Text").or_else(|| child(master?, "Text"))
                && label
                    .descendants()
                    .filter(Node::is_text)
                    .filter_map(|n| n.text())
                    .any(|text| !text.trim().is_empty())
            {
                let row = |section| {
                    shape
                        .children()
                        .find(|&n| named(n, "Section") && n.attribute("N") == Some(section))
                        .and_then(|sec| child(sec, "Row"))
                };
                let character = row("Character");
                let paragraph = row("Paragraph");
                let txt_width = num("TxtWidth");
                let left_margin = num("LeftMargin").unwrap_or(0.0);
                let right_margin = num("RightMargin").unwrap_or(0.0);
                let pin = (
                    num("TxtPinX").unwrap_or(w / 2.0),
                    num("TxtPinY").unwrap_or(h / 2.0),
                );
                let (sin, cos) = num("TxtAngle").unwrap_or(0.0).sin_cos();
                let text_matrix = m.child(Matrix([cos, sin, -sin, cos, pin.0, pin.1]));
                let left = -num("TxtLocPinX").unwrap_or(txt_width.unwrap_or(w) / 2.0);
                let height = num("TxtHeight").unwrap_or(h);
                let bottom = -num("TxtLocPinY").unwrap_or(height / 2.0);
                let top_margin = num("TopMargin").unwrap_or(0.0);
                let bottom_margin = num("BottomMargin").unwrap_or(0.0);
                if let Some(background) = get("TextBkgnd").filter(|v| v.starts_with('#')) {
                    let mut rgba = color(Some(background), [255, 255, 255, 255]);
                    let transparency = num("TextBkgndTrans").unwrap_or(0.0).clamp(0.0, 1.0);
                    rgba[3] = ((1.0 - transparency) * 255.0).round() as u8;
                    let right = left + txt_width.unwrap_or(w);
                    let top = bottom + height;
                    let a = text_matrix.point(left, top);
                    let b = text_matrix.point(right, top);
                    let c = text_matrix.point(right, bottom);
                    let d = text_matrix.point(left, bottom);
                    self.path(
                        vec![
                            PathCmd::MoveTo(a.0, a.1),
                            PathCmd::LineTo(b.0, b.1),
                            PathCmd::LineTo(c.0, c.1),
                            PathCmd::LineTo(d.0, d.1),
                            PathCmd::Close,
                        ],
                        Some(rgba),
                        None,
                        0.0,
                    )?;
                }
                self.label(
                    label,
                    text_matrix,
                    LabelStyle {
                        top: bottom + height - top_margin,
                        bottom: bottom + bottom_margin,
                        vertical_alignment: get("VerticalAlign").unwrap_or("1"),
                        left: (left + left_margin) as f32 * 72.0,
                        width: (txt_width.unwrap_or(w) - left_margin - right_margin).max(0.0)
                            as f32
                            * 72.0,
                        wrap: txt_width.is_some(),
                        alignment: value(paragraph, "HorzAlign").unwrap_or("1"),
                        spacing: number(value(paragraph, "SpLine")).unwrap_or(-1.2),
                        font_size: (number(value(character, "Size")).unwrap_or(1.0 / 6.0) * 72.0)
                            .clamp(1.0, 400.0) as f32,
                        family: value(character, "Font"),
                        color: color(value(character, "Color"), [0, 0, 0, 255]),
                        flags: value(character, "Style")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0),
                    },
                )?;
            }
        }
        Ok(())
    }

    fn draw_geometry(
        &mut self,
        mut commands: Vec<PathCmd>,
        fill: Option<[u8; 4]>,
        stroke: Option<[u8; 4]>,
        weight: f32,
        (dash, arrows): (bool, (bool, bool)),
        rounding: f32,
    ) -> Result<()> {
        if !dash && !arrows.0 && !arrows.1 && rounding <= 0.0 {
            return self.path(commands, fill, stroke, weight);
        }
        let points = straight_vertices(&commands);
        if rounding > 0.0
            && matches!(commands.last(), Some(PathCmd::Close))
            && let Some(points) = &points
            && points.len() >= 4
        {
            commands = rounded_polygon(&points[..points.len() - 1], rounding);
        }
        if dash && let (Some(points), Some(stroke)) = (&points, stroke) {
            if fill.is_some() {
                self.path(commands, fill, None, weight)?;
            }
            self.path(dashed_line(points, weight), None, Some(stroke), weight)?;
        } else {
            self.path(commands, fill, stroke, weight)?;
        }
        if let (Some(points), Some(stroke)) = (&points, stroke)
            && points.len() >= 2
            && points.first() != points.last()
        {
            let last = points.len() - 1;
            for (enabled, tip, adjacent) in [
                (arrows.0, points[0], points[1]),
                (arrows.1, points[last], points[last - 1]),
            ] {
                if enabled {
                    self.path(
                        triangle_arrow(tip, adjacent, weight),
                        Some(stroke),
                        None,
                        0.0,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn path(
        &mut self,
        commands: Vec<PathCmd>,
        fill: Option<[u8; 4]>,
        stroke: Option<[u8; 4]>,
        weight: f32,
    ) -> Result<()> {
        if commands.len() < 2 || fill.is_none() && stroke.is_none() {
            return Ok(());
        }
        self.path_commands += commands.len();
        if self.path_commands > MAX_PAGE_PATH_COMMANDS {
            return Err(invalid());
        }
        let mut bounds = PdfRect {
            x0: f32::INFINITY,
            y0: f32::INFINITY,
            x1: f32::NEG_INFINITY,
            y1: f32::NEG_INFINITY,
        };
        let mut point = |x: f32, y: f32| {
            bounds.x0 = bounds.x0.min(x);
            bounds.y0 = bounds.y0.min(y);
            bounds.x1 = bounds.x1.max(x);
            bounds.y1 = bounds.y1.max(y);
        };
        for cmd in &commands {
            match *cmd {
                PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => point(x, y),
                PathCmd::CubicTo(x, y, u, v, a, b) => {
                    point(x, y);
                    point(u, v);
                    point(a, b);
                }
                _ => {}
            }
        }
        if bounds.x0.is_finite()
            && bounds.y0.is_finite()
            && bounds.x1.is_finite()
            && bounds.y1.is_finite()
        {
            self.scene.display_list.push(DisplayCmd::DrawPath {
                bounds,
                commands,
                fill_rule: FillRule::NonZero,
                fill_rgba: fill,
                stroke_rgba: stroke,
                stroke_width: weight,
            });
        }
        Ok(())
    }

    fn label(&mut self, node: Node<'_, '_>, matrix: Matrix, style: LabelStyle<'_>) -> Result<()> {
        let text: String = node
            .descendants()
            .filter(Node::is_text)
            .filter_map(|n| n.text())
            .collect();
        let text = text.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            return Ok(());
        }
        if self.scene.text_index.chars.len().saturating_add(text.len()) > MAX_LABEL_CHARS {
            return Err(invalid());
        }
        let font_size = style.font_size;
        let lines = wrap_lines(text, style.wrap.then_some(style.width), font_size);
        let distance = if style.spacing < 0.0 {
            -style.spacing as f32 * font_size
        } else {
            style.spacing as f32 * 72.0
        };
        let line_count = lines.len() as f32;
        let block_height = f64::from(font_size + (line_count - 1.0) * distance) / 72.0;
        let center_y = match style.vertical_alignment {
            "0" => style.top - block_height / 2.0,
            "2" => style.bottom + block_height / 2.0,
            _ => (style.top + style.bottom) / 2.0,
        };
        let angle = (-matrix.0[1]).atan2(matrix.0[0]) as f32;
        for (index, (line, continues_text)) in lines.into_iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let visible = line.trim_end();
            let measured_width = text_width(visible, font_size);
            let max_width = (style.wrap && style.width > 0.0).then_some(style.width);
            let width = max_width.map_or(measured_width, |limit| measured_width.min(limit));
            let index_scale = if measured_width > 0.0 {
                width / measured_width
            } else {
                1.0
            };
            let (anchor, fraction) = match style.alignment {
                "0" => (TextAnchor::Start, 0.0),
                "2" => (TextAnchor::End, 1.0),
                _ => (TextAnchor::Center, 0.5),
            };
            let anchor_x = style.left + style.width * fraction;
            let x = anchor_x - width * fraction;
            let y = (index as f32 - (line_count - 1.0) / 2.0) * distance + font_size * 0.34;
            let origin = matrix.point(f64::from(anchor_x) / 72.0, center_y - f64::from(y) / 72.0);
            let stream_op_index = self.scene.display_list.len();
            self.scene
                .display_list
                .push(DisplayCmd::DrawStyledGlyphRun {
                    text: visible.into(),
                    origin,
                    font_size,
                    family: style.family.unwrap_or_default().into(),
                    weight: if style.flags & 1 != 0 { 700 } else { 400 },
                    italic: style.flags & 2 != 0,
                    angle,
                    horizontal_scale: 1.0,
                    anchor,
                    continues_text,
                    max_width,
                    rgba: style.color,
                });
            let mut cursor = x;
            for ch in line.chars() {
                let end = cursor + estimated_advance_em(ch) * font_size * index_scale;
                let pt = |x: f32, y: f32| {
                    matrix.point(f64::from(x) / 72.0, center_y - f64::from(y) / 72.0)
                };
                let corners = [
                    pt(cursor, y),
                    pt(end, y),
                    pt(cursor, y - font_size),
                    pt(end, y - font_size),
                ];
                self.scene.text_index.chars.push(CharBox {
                    ch,
                    rect_pt: PdfRect {
                        x0: corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min),
                        y0: corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min),
                        x1: corners
                            .iter()
                            .map(|p| p.0)
                            .fold(f32::NEG_INFINITY, f32::max),
                        y1: corners
                            .iter()
                            .map(|p| p.1)
                            .fold(f32::NEG_INFINITY, f32::max),
                    },
                    vertical: false,
                    stream_op_index,
                });
                cursor = end;
            }
        }
        Ok(())
    }
}

struct LabelStyle<'a> {
    top: f64,
    bottom: f64,
    vertical_alignment: &'a str,
    left: f32,
    width: f32,
    wrap: bool,
    alignment: &'a str,
    spacing: f64,
    font_size: f32,
    family: Option<&'a str>,
    color: [u8; 4],
    flags: u32,
}

fn text_width(text: &str, font_size: f32) -> f32 {
    text.chars().map(estimated_advance_em).sum::<f32>() * font_size
}

// The boolean marks a visual continuation of the same source line for search.
fn wrap_lines(text: &str, width: Option<f32>, font_size: f32) -> Vec<(&str, bool)> {
    let Some(width) = width.filter(|&w| w > 0.0) else {
        return text.lines().map(|line| (line, false)).collect();
    };
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut start = 0;
        let mut used = 0.0;
        let mut previous_break = 0;
        for (next_break, _) in unicode_linebreak::linebreaks(paragraph) {
            let segment = &paragraph[previous_break..next_break];
            if previous_break > start && used + text_width(segment.trim_end(), font_size) > width {
                lines.push((&paragraph[start..previous_break], start > 0));
                start = previous_break;
                used = 0.0;
            }
            used += text_width(segment, font_size);
            previous_break = next_break;
        }
        lines.push((&paragraph[start..], start > 0));
    }
    lines
}

fn color(value: Option<&str>, default: [u8; 4]) -> [u8; 4] {
    match value {
        Some("0") => [0, 0, 0, 255],
        Some("1") => [255, 255, 255, 255],
        Some(s) if s.len() == 7 && s.starts_with('#') => u32::from_str_radix(&s[1..], 16)
            .map_or(default, |n| [(n >> 16) as u8, (n >> 8) as u8, n as u8, 255]),
        _ => default,
    }
}

fn inherited_scale(size: f64, master: Option<Node<'_, '_>>, key: &str) -> f64 {
    number(value(master, key))
        .filter(|&v| v > 0.0)
        .map_or(1.0, |v| (size / v).clamp(0.0, 1_000_000.0))
}

type InheritedNode<'a, 'i> = (Option<Node<'a, 'i>>, Option<Node<'a, 'i>>);

fn indexed_children<'a, 'i>(
    base: Option<Node<'a, 'i>>,
    local: Option<Node<'a, 'i>>,
    tag: &str,
    filter: impl Fn(Node<'a, 'i>) -> bool,
) -> BTreeMap<u32, InheritedNode<'a, 'i>> {
    let mut nodes = BTreeMap::<_, InheritedNode<'a, 'i>>::new();
    for (is_local, parent) in [(false, base), (true, local)] {
        for (ordinal, node) in parent
            .into_iter()
            .flat_map(|n| n.children())
            .filter(|&n| named(n, tag) && filter(n))
            .enumerate()
        {
            let index = node
                .attribute("IX")
                .and_then(|v| v.parse().ok())
                .unwrap_or(ordinal as u32);
            let pair = nodes.entry(index).or_default();
            if is_local {
                pair.1 = Some(node);
            } else {
                pair.0 = Some(node);
            }
        }
    }
    nodes
}

fn geometry_path(
    base: Option<Node<'_, '_>>,
    local: Option<Node<'_, '_>>,
    m: Matrix,
    size: (f64, f64),
    scales: (f64, f64),
) -> Vec<PathCmd> {
    let mut out = Vec::new();
    let mut start = None;
    let mut previous = None;
    for (base, local) in indexed_children(base, local, "Row", |_| true).into_values() {
        if local.or(base).and_then(|n| n.attribute("Del")) == Some("1") {
            continue;
        }
        let kind = local
            .and_then(|n| n.attribute("T"))
            .or_else(|| base.and_then(|n| n.attribute("T")))
            .unwrap_or_default();
        let relative = kind.starts_with("Rel");
        let num = |key, axis_scale| {
            number(value(local, key)).or_else(|| {
                number(value(base, key)).map(|v| if relative { v } else { v * axis_scale })
            })
        };
        let (Some(x), Some(y)) = (num("X", scales.0), num("Y", scales.1)) else {
            continue;
        };
        let end = if relative {
            (x * size.0, y * size.1)
        } else {
            (x, y)
        };
        let p = m.point(end.0, end.1);
        match kind {
            "MoveTo" | "RelMoveTo" => {
                out.push(PathCmd::MoveTo(p.0, p.1));
                start = Some(p);
            }
            "LineTo" | "RelLineTo" if start.is_some() => {
                out.push(PathCmd::LineTo(p.0, p.1));
                if start == Some(p) {
                    out.push(PathCmd::Close);
                }
            }
            "RelCubBezTo" if start.is_some() => {
                if let (Some(a), Some(b), Some(c), Some(d)) =
                    (num("A", 1.0), num("B", 1.0), num("C", 1.0), num("D", 1.0))
                {
                    let first = m.point(a * size.0, b * size.1);
                    let second = m.point(c * size.0, d * size.1);
                    out.push(PathCmd::CubicTo(
                        first.0, first.1, second.0, second.1, p.0, p.1,
                    ));
                    if start == Some(p) {
                        out.push(PathCmd::Close);
                    }
                }
            }
            "EllipticalArcTo" if start.is_some() => {
                let arc = previous.and_then(|from| {
                    elliptical_arc(
                        m,
                        from,
                        (num("A", scales.0)?, num("B", scales.1)?),
                        end,
                        num("C", 1.0)?,
                        num("D", 1.0)?,
                    )
                });
                if let Some(arc) = arc {
                    out.extend(arc);
                } else {
                    out.push(PathCmd::LineTo(p.0, p.1));
                }
                if start == Some(p) {
                    out.push(PathCmd::Close);
                }
            }
            "Ellipse" => {
                if let (Some(a), Some(b), Some(c), Some(d)) = (
                    num("A", scales.0),
                    num("B", scales.1),
                    num("C", scales.0),
                    num("D", scales.1),
                ) {
                    let (ux, uy, vx, vy) = (a - x, b - y, c - x, d - y);
                    let k = 0.5522847498307936;
                    let p = m.point(a, b);
                    out.push(PathCmd::MoveTo(p.0, p.1));
                    let axes = [(ux, uy), (vx, vy), (-ux, -uy), (-vx, -vy), (ux, uy)];
                    for pair in axes.windows(2) {
                        let [(u, v), (s, t)] = [pair[0], pair[1]];
                        let p1 = m.point(x + u + k * s, y + v + k * t);
                        let p2 = m.point(x + s + k * u, y + t + k * v);
                        let p = m.point(x + s, y + t);
                        out.push(PathCmd::CubicTo(p1.0, p1.1, p2.0, p2.1, p.0, p.1));
                    }
                    out.push(PathCmd::Close);
                }
            }
            _ => {
                start = None;
            }
        }
        previous = Some(end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resized_master_keeps_its_inherited_geometry_rows_and_cells() {
        let base = format!(
            r#"<Section xmlns="{VISIO}">
              <Row T="MoveTo" IX="1"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row>
              <Row T="LineTo" IX="2"><Cell N="X" V="1"/><Cell N="Y" V="0"/></Row>
              <Row T="LineTo" IX="3"><Cell N="X" V="1"/><Cell N="Y" V="1"/></Row>
              <Row T="LineTo" IX="4"><Cell N="X" V="0"/><Cell N="Y" V="1"/></Row>
              <Row T="LineTo" IX="5"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row>
            </Section>"#
        );
        let local = format!(
            r#"<Section xmlns="{VISIO}">
              <Row T="LineTo" IX="2"><Cell N="X" V="3"/></Row>
              <Row IX="3"><Cell N="X" V="3"/></Row>
            </Section>"#
        );
        let base = xml(&base).unwrap();
        let local = xml(&local).unwrap();
        let commands = geometry_path(
            Some(base.root_element()),
            Some(local.root_element()),
            Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            (3.0, 2.0),
            (3.0, 2.0),
        );
        assert!(matches!(
            commands.as_slice(),
            [
                PathCmd::MoveTo(0.0, 0.0),
                PathCmd::LineTo(3.0, 0.0),
                PathCmd::LineTo(3.0, 2.0),
                PathCmd::LineTo(0.0, 2.0),
                PathCmd::LineTo(0.0, 0.0),
                PathCmd::Close
            ]
        ));
    }
}

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Playing an Enhanced Metafile ([MS-EMF]) into SVG: the GDI records for lines, shapes, paths, text and bitmaps, with the
//! coordinate transforms, the objects selected and the clips.

use std::collections::HashMap;
use std::fmt::Write as _;

mod plus;

use crate::ImageFormat;
use crate::svg::{Canvas, Font, IDENTITY, Matrix, Paint, Stroke, apply, dib_png, scale_of, then};

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(d: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn f32_at(d: &[u8], at: usize) -> Option<f64> {
    Some(f64::from(f32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?)))
}

#[derive(Clone, Debug)]
struct Pen {
    style: u32,
    width: f64,
    color: u32,
}

#[derive(Clone, Debug)]
struct Brush {
    style: u32,
    color: u32,
}

#[derive(Clone, Debug, Default)]
struct FontObj {
    height: f64,
    escapement: f64,
    weight: u32,
    italic: bool,
    underline: bool,
    strike: bool,
    charset: u8,
    face: String,
}

#[derive(Clone, Debug)]
enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(FontObj),
    Other,
}

#[derive(Clone)]
struct Dc {
    map_mode: u32,
    win_org: (f64, f64),
    win_ext: (f64, f64),
    vp_org: (f64, f64),
    vp_ext: (f64, f64),
    world: Matrix,
    pen: Pen,
    brush: Brush,
    font: FontObj,
    text_color: u32,
    bk_color: u32,
    bk_mode: u32,
    text_align: u32,
    poly_fill: u32,
    cur: (f64, f64),
    clip: Option<usize>,
    /// The clip's box in device units, to cut further clips down.
    clip_box: Option<(f64, f64, f64, f64)>,
}

impl Default for Dc {
    fn default() -> Self {
        Dc {
            map_mode: 1,
            win_org: (0.0, 0.0),
            win_ext: (1.0, 1.0),
            vp_org: (0.0, 0.0),
            vp_ext: (1.0, 1.0),
            world: IDENTITY,
            pen: Pen { style: 0, width: 0.0, color: 0 },
            brush: Brush { style: 1, color: 0xFFFFFF },
            font: FontObj::default(),
            text_color: 0,
            bk_color: 0xFFFFFF,
            bk_mode: 2,
            text_align: 0,
            poly_fill: 1,
            cur: (0.0, 0.0),
            clip: None,
            clip_box: None,
        }
    }
}

struct Player {
    canvas: Canvas,
    dc: Dc,
    stack: Vec<Dc>,
    objects: HashMap<u32, Object>,
    /// Device pixels per millimetre.
    px_per_mm: (f64, f64),
    /// The path being built (a bracket), in device units.
    path: Option<String>,
    path_start: Option<(f64, f64)>,
    /// EMF+ records are played (else only the GDI records of a dual metafile).
    use_plus: bool,
    plus: plus::Plus,
    /// The device rectangle (left, top, width, height) the picture covers.
    view: (f64, f64, f64, f64),
}

const ENHMETA_SIGNATURE: &[u8] = b" EMF";

/// What an Enhanced Metafile shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Picture {
    /// The metafile is one bitmap shown as it is, in an image file.
    Image(ImageFormat, Vec<u8>),
    Svg(String),
}

/// The SVG an Enhanced Metafile draws, or `None` when it is not one or draws nothing.
pub fn emf_to_svg(emf: &[u8]) -> Option<String> {
    match emf_picture_with(emf, false)? {
        Picture::Svg(svg) => Some(svg),
        Picture::Image(..) => None,
    }
}

/// What the Enhanced Metafile shows: the bitmap when that is all it holds, else the SVG of its drawing.
pub fn emf_picture(emf: &[u8]) -> Option<Picture> {
    emf_picture_with(emf, true)
}

fn emf_picture_with(emf: &[u8], bitmap_as_image: bool) -> Option<Picture> {
    let played = play(emf, true)?;
    // A dual metafile's GDI records stand in for EMF+ records that are not played.
    let played = if played.fallback { play(emf, false).or(Some(played))? } else { played };
    if bitmap_as_image {
        if let Some((format, bytes)) = played.sole_image {
            return Some(Picture::Image(format, bytes));
        }
    }
    played.svg.map(Picture::Svg)
}

struct Played {
    svg: Option<String>,
    fallback: bool,
    sole_image: Option<(ImageFormat, Vec<u8>)>,
}

/// Play the metafile; what was drawn, and whether the EMF+ part had something unplayed in a dual metafile.
fn play(emf: &[u8], use_plus: bool) -> Option<Played> {
    if u32_at(emf, 0)? != 1 || emf.get(40..44)? != ENHMETA_SIGNATURE {
        return None;
    }
    let bounds = (i32_at(emf, 8)?, i32_at(emf, 12)?, i32_at(emf, 16)?, i32_at(emf, 20)?);
    let (device, millimetres) = crate::gdi::emf_device_size(emf)?;
    let [dx, dy] = device.map(|v| f64::from(v.max(1)));
    let [mx, my] = millimetres.map(|v| f64::from(v.max(1)));
    let (l, t, r, b) = bounds;
    let mut player = Player {
        canvas: Canvas::default(),
        dc: Dc::default(),
        stack: Vec::new(),
        objects: HashMap::new(),
        px_per_mm: (dx / mx, dy / my),
        path: None,
        path_start: None,
        use_plus,
        plus: plus::Plus::default(),
        view: (f64::from(l), f64::from(t), f64::from(r - l + 1), f64::from(b - t + 1)),
    };
    let mut rest = emf;
    while let Some((kind, record)) = crate::gdi::take_emf_record(&mut rest) {
        if kind == 14 {
            break;
        }
        player.record(kind, record);
    }
    let fallback = player.plus.unsupported && player.plus.dual;
    let sole_image = if player.canvas.drawn == 1 && player.plus.draws == 1 { player.plus.sole_image.take() } else { None };
    if player.canvas.drawn == 0 {
        return if fallback { Some(Played { svg: None, fallback: true, sole_image: None }) } else { None };
    }
    // GDI plays a metafile into a picture's box by its frame (`rclFrame`, in 0.01 mm), not by the bounds of what was
    // drawn: content that stops short of the frame keeps its place in it. The frame is taken in device units.
    let frame = (i32_at(emf, 24), i32_at(emf, 28), i32_at(emf, 32), i32_at(emf, 36));
    let frame_view = match frame {
        (Some(fl), Some(ft), Some(fr), Some(fb)) if fr > fl && fb > ft => Some((
            f64::from(fl) / 100.0 * player.px_per_mm.0,
            f64::from(ft) / 100.0 * player.px_per_mm.1,
            f64::from(fr - fl) / 100.0 * player.px_per_mm.0,
            f64::from(fb - ft) / 100.0 * player.px_per_mm.1,
        )),
        _ => None,
    };
    let view = if let Some(v) = frame_view.filter(|v| v.2 >= 1.0 && v.3 >= 1.0) {
        v
    } else if r > l && b > t {
        (f64::from(l), f64::from(t), f64::from(r - l + 1), f64::from(b - t + 1))
    } else {
        let (x0, y0, x1, y1) = player.canvas.extent?;
        (x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0))
    };
    Some(Played { svg: Some(player.canvas.finish(view)), fallback, sole_image })
}

impl Player {
    /// The matrix from logical units to device units.
    fn matrix(&self) -> Matrix {
        let dc = &self.dc;
        let (sx, sy, flip) = match dc.map_mode {
            // Text: one unit, one device unit.
            1 => (1.0, 1.0, false),
            2..=6 => {
                let mm = match dc.map_mode {
                    2 => 0.1,
                    3 => 0.01,
                    4 => 0.254,
                    5 => 0.0254,
                    _ => 25.4 / 1440.0,
                };
                (self.px_per_mm.0 * mm, self.px_per_mm.1 * mm, true)
            }
            _ => {
                let safe = |v: f64| if v == 0.0 { 1.0 } else { v };
                (dc.vp_ext.0 / safe(dc.win_ext.0), dc.vp_ext.1 / safe(dc.win_ext.1), false)
            }
        };
        let sy = if flip { -sy } else { sy };
        let page = [sx, 0.0, 0.0, sy, dc.vp_org.0 - dc.win_org.0 * sx, dc.vp_org.1 - dc.win_org.1 * sy];
        then(dc.world, page)
    }

    fn point(&self, x: f64, y: f64) -> (f64, f64) {
        apply(&self.matrix(), x, y)
    }

    fn pen_stroke(&self) -> Option<Stroke> {
        let pen = &self.dc.pen;
        let kind = pen.style & 0xF;
        if kind == 5 {
            return None;
        }
        let scale = scale_of(&self.matrix());
        // A cosmetic pen is one device unit wide.
        let width = if pen.width <= 1.0 && (pen.style & 0x1_0000) == 0 { 1.0 } else { (pen.width * scale).max(1.0) };
        let dash = match kind {
            1 => vec![width * 6.0, width * 2.0],
            2 => vec![width, width * 2.0],
            3 => vec![width * 6.0, width * 2.0, width, width * 2.0],
            4 => vec![width * 6.0, width * 2.0, width, width * 2.0, width, width * 2.0],
            _ => Vec::new(),
        };
        let cap = match pen.style & 0xF00 {
            0x100 => "square",
            0x200 => "butt",
            _ => "round",
        };
        let join = match pen.style & 0xF000 {
            0x1000 => "bevel",
            0x2000 => "miter",
            _ => "round",
        };
        Some(Stroke { color: pen.color, alpha: 1.0, miter: None, width, dash, cap, join })
    }

    fn fill_color(&self) -> Option<u32> {
        let brush = &self.dc.brush;
        match brush.style {
            1 => None,
            0 => Some(brush.color),
            // A hatch or a pattern: its colour, as a stand-in.
            2 => Some(brush.color),
            _ => Some(0x80_8080),
        }
    }

    fn paint(&self, filled: bool, stroked: bool) -> Paint {
        Paint {
            fill: if filled { self.fill_color() } else { None },
            fill_url: None,
            fill_alpha: if filled && self.dc.brush.style == 2 { 0.4 } else { 1.0 },
            even_odd: self.dc.poly_fill == 1,
            stroke: if stroked { self.pen_stroke() } else { None },
        }
    }

    /// Draw (or add to the path being built) `d`, which is in device units.
    fn emit(&mut self, d: String, filled: bool, stroked: bool, extent: &[(f64, f64)]) {
        if let Some(path) = &mut self.path {
            path.push_str(&d);
            return;
        }
        self.canvas.grow(&IDENTITY, extent);
        let paint = self.paint(filled, stroked);
        let clip = self.dc.clip;
        self.canvas.path(&IDENTITY, clip, &d, &paint);
    }

    fn lines(&self, points: &[(f64, f64)], close: bool, from_current: bool) -> String {
        let mut d = String::new();
        for (i, &(x, y)) in points.iter().enumerate() {
            let (px, py) = self.point(x, y);
            let cmd = if i == 0 && !from_current { "M" } else { "L" };
            let _ = write!(d, "{cmd}{} {} ", fmt(px), fmt(py));
        }
        if close {
            d.push('Z');
        }
        d
    }

    fn beziers(&self, points: &[(f64, f64)], from_current: bool) -> String {
        let mut d = String::new();
        let mut rest = points;
        if !from_current {
            let Some(first) = points.first() else { return d };
            let (px, py) = self.point(first.0, first.1);
            let _ = write!(d, "M{} {} ", fmt(px), fmt(py));
            rest = &points[1..];
        }
        for c in rest.chunks_exact(3) {
            let (a, b, e) = (self.point(c[0].0, c[0].1), self.point(c[1].0, c[1].1), self.point(c[2].0, c[2].1));
            let _ = write!(d, "C{} {} {} {} {} {} ", fmt(a.0), fmt(a.1), fmt(b.0), fmt(b.1), fmt(e.0), fmt(e.1));
        }
        d
    }

    fn start_from_current(&self) -> String {
        let (x, y) = self.point(self.dc.cur.0, self.dc.cur.1);
        format!("M{} {} ", fmt(x), fmt(y))
    }

    /// An ellipse (or its arc from angle `a0` to `a1`, in radians counterclockwise on the page, from the x axis) as bézier curves.
    fn ellipse_path(&self, l: f64, t: f64, r: f64, b: f64, arc: Option<(f64, f64)>, close_to_centre: bool, chord: bool) -> String {
        let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l).abs() / 2.0, (b - t).abs() / 2.0);
        let (a0, a1) = match arc {
            Some((a, b)) => (a, if b <= a { b + std::f64::consts::TAU } else { b }),
            None => (0.0, std::f64::consts::TAU),
        };
        let mut d = String::new();
        let pt = |a: f64| self.point(cx + rx * a.cos(), cy - ry * a.sin());
        let (sx, sy) = pt(a0);
        if arc.is_some() && close_to_centre {
            let (mx, my) = self.point(cx, cy);
            let _ = write!(d, "M{} {} L{} {} ", fmt(mx), fmt(my), fmt(sx), fmt(sy));
        } else {
            let _ = write!(d, "M{} {} ", fmt(sx), fmt(sy));
        }
        let pieces = (((a1 - a0) / (std::f64::consts::FRAC_PI_2)).ceil() as usize).max(1);
        let step = (a1 - a0) / pieces as f64;
        for i in 0..pieces {
            let (s, e) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
            let k = 4.0 / 3.0 * ((e - s) / 4.0).tan();
            let p1 = self.point(cx + rx * (s.cos() - k * s.sin()), cy - ry * (s.sin() + k * s.cos()));
            let p2 = self.point(cx + rx * (e.cos() + k * e.sin()), cy - ry * (e.sin() - k * e.cos()));
            let p3 = pt(e);
            let _ = write!(d, "C{} {} {} {} {} {} ", fmt(p1.0), fmt(p1.1), fmt(p2.0), fmt(p2.1), fmt(p3.0), fmt(p3.1));
        }
        if arc.is_none() || close_to_centre || chord {
            d.push('Z');
        }
        d
    }

    fn rect_extent(&self, l: f64, t: f64, r: f64, b: f64) -> Vec<(f64, f64)> {
        [(l, t), (r, t), (r, b), (l, b)].iter().map(|&(x, y)| self.point(x, y)).collect()
    }

    fn set_clip_box(&mut self, d: String, device_box: (f64, f64, f64, f64)) {
        let (x0, y0, x1, y1) = match self.dc.clip_box {
            Some((a, b, c, e)) => (a.max(device_box.0), b.max(device_box.1), c.min(device_box.2), e.min(device_box.3)),
            None => device_box,
        };
        let _ = d;
        let rect = format!("M{} {} L{} {} L{} {} L{} {} Z", fmt(x0), fmt(y0), fmt(x1), fmt(y0), fmt(x1), fmt(y1), fmt(x0), fmt(y1));
        self.dc.clip_box = Some((x0, y0, x1, y1));
        self.dc.clip = Some(self.canvas.clip_path(&rect, false));
    }

    fn select(&mut self, handle: u32) {
        if handle & 0x8000_0000 != 0 {
            // A stock object.
            match handle & 0x7FFF_FFFF {
                0 => self.dc.brush = Brush { style: 0, color: 0xFFFFFF },
                1 => self.dc.brush = Brush { style: 0, color: 0xC0C0C0 },
                2 => self.dc.brush = Brush { style: 0, color: 0x808080 },
                3 => self.dc.brush = Brush { style: 0, color: 0x404040 },
                4 => self.dc.brush = Brush { style: 0, color: 0 },
                5 => self.dc.brush = Brush { style: 1, color: 0 },
                6 => self.dc.pen = Pen { style: 0, width: 0.0, color: 0xFFFFFF },
                7 => self.dc.pen = Pen { style: 0, width: 0.0, color: 0 },
                8 => self.dc.pen = Pen { style: 5, width: 0.0, color: 0 },
                _ => {}
            }
            return;
        }
        match self.objects.get(&handle).cloned() {
            Some(Object::Pen(p)) => self.dc.pen = p,
            Some(Object::Brush(b)) => self.dc.brush = b,
            Some(Object::Font(f)) => self.dc.font = f,
            _ => {}
        }
    }

    fn record(&mut self, kind: u32, r: &[u8]) {
        let f = |at: usize| f64::from(i32_at(r, at).unwrap_or(0));
        if kind == 0x46 {
            if self.use_plus && r.get(12..16) == Some(b"EMF+") {
                self.plus_comment(r);
            }
            return;
        }
        // A metafile with EMF+ records is drawn by them; its GDI records are the fallback (but for those after EmfPlusGetDC).
        if self.use_plus && self.plus.present && !self.plus.get_dc {
            return;
        }
        match kind {
            // State.
            0x09 => self.dc.win_ext = (f(8), f(12)),
            0x0A => self.dc.win_org = (f(8), f(12)),
            0x0B => self.dc.vp_ext = (f(8), f(12)),
            0x0C => self.dc.vp_org = (f(8), f(12)),
            0x11 => self.dc.map_mode = u32_at(r, 8).unwrap_or(1),
            0x12 => self.dc.bk_mode = u32_at(r, 8).unwrap_or(2),
            0x13 => self.dc.poly_fill = u32_at(r, 8).unwrap_or(1),
            0x16 => self.dc.text_align = u32_at(r, 8).unwrap_or(0),
            0x18 => self.dc.text_color = u32_at(r, 8).unwrap_or(0) & 0xFF_FFFF,
            0x19 => self.dc.bk_color = u32_at(r, 8).unwrap_or(0xFFFFFF) & 0xFF_FFFF,
            0x1F => {
                // Scale the viewport extent by numerator / denominator (x then y).
                let (nx, dx, ny, dy) = (f(8), f(12), f(16), f(20));
                if dx != 0.0 && dy != 0.0 {
                    self.dc.vp_ext = (self.dc.vp_ext.0 * nx / dx, self.dc.vp_ext.1 * ny / dy);
                }
            }
            0x20 => {
                let (nx, dx, ny, dy) = (f(8), f(12), f(16), f(20));
                if dx != 0.0 && dy != 0.0 {
                    self.dc.win_ext = (self.dc.win_ext.0 * nx / dx, self.dc.win_ext.1 * ny / dy);
                }
            }
            0x21 => self.stack.push(self.dc.clone()),
            0x22 => {
                let n = i32_at(r, 8).unwrap_or(-1);
                let n = if n < 0 { (-n) as usize } else { 0 };
                for _ in 0..n.max(1) {
                    if let Some(dc) = self.stack.pop() {
                        self.dc = dc;
                    }
                }
            }
            0x23 => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(g)) = (f32_at(r, 8), f32_at(r, 12), f32_at(r, 16), f32_at(r, 20), f32_at(r, 24), f32_at(r, 28)) {
                    self.dc.world = [a, b, c, d, e, g];
                }
            }
            0x24 => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(g), Some(mode)) = (f32_at(r, 8), f32_at(r, 12), f32_at(r, 16), f32_at(r, 20), f32_at(r, 24), f32_at(r, 28), u32_at(r, 32)) {
                    let rec = [a, b, c, d, e, g];
                    self.dc.world = match mode {
                        1 => IDENTITY,
                        // The record's matrix on the left: it applies first.
                        2 => then(rec, self.dc.world),
                        3 => then(self.dc.world, rec),
                        _ => rec,
                    };
                }
            }
            // Objects.
            0x25 => {
                if let Some(h) = u32_at(r, 8) {
                    self.select(h);
                }
            }
            0x28 => {
                if let Some(h) = u32_at(r, 8) {
                    self.objects.remove(&h);
                }
            }
            0x26 => {
                // EMR_CREATEPEN: handle, then a LogPen (style, width, colour).
                if let (Some(h), Some(style), Some(width), Some(c)) = (u32_at(r, 8), u32_at(r, 12), i32_at(r, 16), u32_at(r, 24)) {
                    self.objects.insert(h, Object::Pen(Pen { style, width: f64::from(width), color: c & 0xFF_FFFF }));
                }
            }
            0x5F => {
                // EMR_EXTCREATEPEN: handle, offBmi, cbBmi, offBits, cbBits, then a LogPenEx (style, width, brush style, colour).
                if let (Some(h), Some(style), Some(width), Some(c)) = (u32_at(r, 8), u32_at(r, 28), u32_at(r, 32), u32_at(r, 40)) {
                    self.objects.insert(h, Object::Pen(Pen { style, width: f64::from(width), color: c & 0xFF_FFFF }));
                }
            }
            0x27 => {
                if let (Some(h), Some(style), Some(c)) = (u32_at(r, 8), u32_at(r, 12), u32_at(r, 16)) {
                    self.objects.insert(h, Object::Brush(Brush { style, color: c & 0xFF_FFFF }));
                }
            }
            0x5D | 0x5E => {
                if let Some(h) = u32_at(r, 8) {
                    self.objects.insert(h, Object::Brush(Brush { style: 3, color: 0x80_8080 }));
                }
            }
            0x52 => {
                // EMR_EXTCREATEFONTINDIRECTW: handle, then a LogFont.
                if let Some(h) = u32_at(r, 8) {
                    let face: Vec<u16> = (0..32).filter_map(|i| u16_at(r, 12 + 28 + i * 2)).take_while(|u| *u != 0).collect();
                    self.objects.insert(
                        h,
                        Object::Font(FontObj {
                            height: f64::from(i32_at(r, 12).unwrap_or(0)),
                            escapement: f64::from(i32_at(r, 20).unwrap_or(0)),
                            weight: u32_at(r, 28).unwrap_or(400),
                            italic: r.get(32).copied().unwrap_or(0) != 0,
                            underline: r.get(33).copied().unwrap_or(0) != 0,
                            strike: r.get(34).copied().unwrap_or(0) != 0,
                            charset: r.get(35).copied().unwrap_or(0),
                            face: String::from_utf16_lossy(&face),
                        }),
                    );
                }
            }
            0x31 | 0x63 | 0x7A => {
                if let Some(h) = u32_at(r, 8) {
                    self.objects.insert(h, Object::Other);
                }
            }
            // Positions and lines.
            0x1B => {
                self.dc.cur = (f(8), f(12));
                if self.path.is_some() {
                    let (x, y) = self.point(f(8), f(12));
                    let d = format!("M{} {} ", fmt(x), fmt(y));
                    if let Some(path) = &mut self.path {
                        path.push_str(&d);
                    }
                }
            }
            0x36 => {
                let to = (f(8), f(12));
                let mut d = self.start_from_current();
                let (x, y) = self.point(to.0, to.1);
                let _ = write!(d, "L{} {} ", fmt(x), fmt(y));
                let from = self.point(self.dc.cur.0, self.dc.cur.1);
                if self.path.is_some() {
                    // In a path the segment continues the figure.
                    d = format!("L{} {} ", fmt(x), fmt(y));
                }
                self.emit(d, false, true, &[from, (x, y)]);
                self.dc.cur = to;
            }
            // Polygons and lines with 32-bit points.
            0x02 | 0x03 | 0x04 | 0x05 | 0x06 => self.poly(kind, r, false),
            0x55..=0x59 => self.poly(kind - 0x53, r, true),
            0x07 | 0x08 => self.polypoly(kind, r, false),
            0x5A | 0x5B => self.polypoly(kind - 0x53, r, true),
            // Shapes.
            0x2B | 0x2C | 0x2A => self.shape(kind, r),
            0x2D | 0x2E | 0x2F | 0x37 => self.arc(kind, r),
            // Paths.
            0x3B => {
                self.path = Some(String::new());
                self.path_start = None;
            }
            0x3C => {}
            0x3D => {
                if let Some(path) = &mut self.path {
                    path.push('Z');
                }
            }
            0x44 => self.path = None,
            0x3E | 0x3F | 0x40 => {
                if let Some(d) = self.path.take() {
                    let (fill, stroke) = (kind != 0x40, kind != 0x3E);
                    let extent: Vec<(f64, f64)> = Vec::new();
                    let _ = extent;
                    let paint = self.paint(fill, stroke);
                    let clip = self.dc.clip;
                    self.canvas.path(&IDENTITY, clip, &d, &paint);
                    // (The extent: the bounds the record gives, in device units.)
                    let b = [(f(8), f(12)), (f(16), f(20))];
                    let pts: Vec<(f64, f64)> = b.iter().map(|&(x, y)| self.point(x, y)).collect();
                    self.canvas.grow(&IDENTITY, &pts);
                }
            }
            0x43 => {
                if let Some(d) = self.path.take() {
                    let id = self.canvas.clip_path(&d, self.dc.poly_fill == 1);
                    self.dc.clip = Some(id);
                    self.dc.clip_box = None;
                }
            }
            // Clips.
            0x1E => {
                let (l, t, rr, b) = (f(8), f(12), f(16), f(20));
                let pts = self.rect_extent(l, t, rr, b);
                let (x0, y0) = pts.iter().fold((f64::MAX, f64::MAX), |a, p| (a.0.min(p.0), a.1.min(p.1)));
                let (x1, y1) = pts.iter().fold((f64::MIN, f64::MIN), |a, p| (a.0.max(p.0), a.1.max(p.1)));
                self.set_clip_box(String::new(), (x0, y0, x1, y1));
            }
            0x4B => self.select_clip_region(r),
            // Text.
            0x54 => self.text(r, true),
            0x53 => self.text(r, false),
            // Bitmaps.
            0x51 => self.stretch_dibits(r),
            0x4C | 0x4D => self.blit(r, kind == 0x4D),
            0x50 => self.set_dibits(r),
            _ => {}
        }
    }

    fn points(&self, r: &[u8], at: usize, count: usize, small: bool) -> Vec<(f64, f64)> {
        (0..count)
            .filter_map(|i| {
                if small {
                    Some((f64::from(u16_at(r, at + i * 4)? as i16), f64::from(u16_at(r, at + i * 4 + 2)? as i16)))
                } else {
                    Some((f64::from(i32_at(r, at + i * 8)?), f64::from(i32_at(r, at + i * 8 + 4)?)))
                }
            })
            .collect()
    }

    /// EMR_POLYBEZIER (2), POLYGON (3), POLYLINE (4), POLYBEZIERTO (5), POLYLINETO (6) and their 16-bit forms.
    fn poly(&mut self, kind: u32, r: &[u8], small: bool) {
        let Some(count) = u32_at(r, 24) else { return };
        let points = self.points(r, 28, (count as usize).min(r.len() / 4), small);
        if points.is_empty() {
            return;
        }
        let extent: Vec<(f64, f64)> = points.iter().map(|&(x, y)| self.point(x, y)).collect();
        match kind {
            2 | 5 => {
                let to = kind == 5;
                let mut d = String::new();
                if to && self.path.is_none() {
                    d = self.start_from_current();
                }
                d += &self.beziers(&points, to);
                self.emit(d, false, true, &extent);
                self.dc.cur = *points.last().unwrap_or(&self.dc.cur);
            }
            3 => {
                let d = self.lines(&points, true, false);
                self.emit(d, true, true, &extent);
                self.dc.cur = *points.last().unwrap_or(&self.dc.cur);
            }
            4 => {
                let d = self.lines(&points, false, false);
                self.emit(d, false, true, &extent);
                self.dc.cur = *points.last().unwrap_or(&self.dc.cur);
            }
            _ => {
                let mut d = String::new();
                if self.path.is_none() {
                    d = self.start_from_current();
                }
                d += &self.lines(&points, false, true);
                self.emit(d, false, true, &extent);
                self.dc.cur = *points.last().unwrap_or(&self.dc.cur);
            }
        }
    }

    /// EMR_POLYPOLYLINE (7) and POLYPOLYGON (8).
    fn polypoly(&mut self, kind: u32, r: &[u8], small: bool) {
        let (Some(n), Some(total)) = (u32_at(r, 24), u32_at(r, 28)) else { return };
        let n = (n as usize).min(r.len() / 4);
        let counts: Vec<usize> = (0..n).filter_map(|i| u32_at(r, 32 + i * 4)).map(|c| c as usize).collect();
        let points = self.points(r, 32 + n * 4, (total as usize).min(r.len() / 4), small);
        let mut d = String::new();
        let mut extent = Vec::new();
        let mut at = 0;
        for c in counts {
            let Some(part) = points.get(at..at + c) else { break };
            d += &self.lines(part, kind == 8, false);
            extent.extend(part.iter().map(|&(x, y)| self.point(x, y)));
            at += c;
        }
        self.emit(d, kind == 8, true, &extent);
    }

    /// EMR_ELLIPSE (0x2A), RECTANGLE (0x2B), ROUNDRECT (0x2C).
    fn shape(&mut self, kind: u32, r: &[u8]) {
        let f = |at: usize| f64::from(i32_at(r, at).unwrap_or(0));
        let (l, t, rr, b) = (f(8), f(12), f(16), f(20));
        let (l, rr) = (l.min(rr), l.max(rr));
        let (t, b) = (t.min(b), t.max(b));
        let extent = self.rect_extent(l, t, rr, b);
        let d = match kind {
            0x2A => self.ellipse_path(l, t, rr, b, None, false, false),
            0x2B => self.lines(&[(l, t), (rr, t), (rr, b), (l, b)], true, false),
            _ => {
                // A rounded rectangle: corners of width cx and height cy (ellipses of that size).
                let (cx, cy) = ((f(24).abs() / 2.0).min((rr - l) / 2.0), (f(28).abs() / 2.0).min((b - t) / 2.0));
                let mut d = String::new();
                let corner = |p: &Player, cxp: f64, cyp: f64, a0: f64| -> String {
                    let k = 0.552_284_75;
                    let (dx, dy) = (a0.cos(), -a0.sin());
                    let _ = (dx, dy);
                    let _ = (p, cxp, cyp, k);
                    String::new()
                };
                let _ = corner;
                let p = |x: f64, y: f64| self.point(x, y);
                let k = 0.552_284_75;
                let (x0, y0) = p(l + cx, t);
                let _ = write!(d, "M{} {} ", fmt(x0), fmt(y0));
                let line = |d: &mut String, x: f64, y: f64| {
                    let (px, py) = p(x, y);
                    let _ = write!(d, "L{} {} ", fmt(px), fmt(py));
                };
                let curve = |d: &mut String, a: (f64, f64), b2: (f64, f64), c: (f64, f64)| {
                    let (a, b2, c) = (p(a.0, a.1), p(b2.0, b2.1), p(c.0, c.1));
                    let _ = write!(d, "C{} {} {} {} {} {} ", fmt(a.0), fmt(a.1), fmt(b2.0), fmt(b2.1), fmt(c.0), fmt(c.1));
                };
                line(&mut d, rr - cx, t);
                curve(&mut d, (rr - cx + cx * k, t), (rr, t + cy - cy * k), (rr, t + cy));
                line(&mut d, rr, b - cy);
                curve(&mut d, (rr, b - cy + cy * k), (rr - cx + cx * k, b), (rr - cx, b));
                line(&mut d, l + cx, b);
                curve(&mut d, (l + cx - cx * k, b), (l, b - cy + cy * k), (l, b - cy));
                line(&mut d, l, t + cy);
                curve(&mut d, (l, t + cy - cy * k), (l + cx - cx * k, t), (l + cx, t));
                d.push('Z');
                d
            }
        };
        self.emit(d, true, true, &extent);
    }

    /// EMR_ARC (0x2D), CHORD (0x2E), PIE (0x2F), ARCTO (0x37).
    fn arc(&mut self, kind: u32, r: &[u8]) {
        let f = |at: usize| f64::from(i32_at(r, at).unwrap_or(0));
        let (l, t, rr, b) = (f(8), f(12), f(16), f(20));
        let (l, rr) = (l.min(rr), l.max(rr));
        let (t, b) = (t.min(b), t.max(b));
        let (cx, cy, rx, ry) = ((l + rr) / 2.0, (t + b) / 2.0, ((rr - l) / 2.0).max(0.001), ((b - t) / 2.0).max(0.001));
        let angle = |x: f64, y: f64| (-(y - cy) / ry).atan2((x - cx) / rx);
        let (a0, a1) = (angle(f(24), f(28)), angle(f(32), f(36)));
        let extent = self.rect_extent(l, t, rr, b);
        // Arcs run counterclockwise (the default direction).
        let d = self.ellipse_path(l, t, rr, b, Some((a0, a1)), kind == 0x2F, kind == 0x2E);
        let (filled, stroked) = match kind {
            0x2D | 0x37 => (false, true),
            _ => (true, true),
        };
        self.emit(d, filled, stroked, &extent);
    }

    fn text_font(&self) -> Font {
        let fo = &self.dc.font;
        let scale = scale_of(&self.matrix());
        let size = if fo.height == 0.0 { 12.0 } else { fo.height.abs() };
        // A positive height counts the line's whole height, a negative one the letters'.
        let (ascent, descent) = crate::svg::font_metrics(&fo.face, fo.weight >= 600, fo.italic);
        let size = if fo.height > 0.0 { size / (ascent + descent) } else { size };
        Font { face: fo.face.clone(), size: size * scale, bold: fo.weight >= 600, italic: fo.italic, underline: fo.underline, strike: fo.strike, angle: fo.escapement / 10.0 }
    }

    /// EMR_EXTTEXTOUTW (`wide`) and EMR_EXTTEXTOUTA.
    fn text(&mut self, r: &[u8], wide: bool) {
        let (Some(rx), Some(ry), Some(count), Some(off_string), Some(options)) = (i32_at(r, 36), i32_at(r, 40), u32_at(r, 44), u32_at(r, 48), u32_at(r, 52)) else { return };
        let count = count as usize;
        let mut text = String::new();
        let mut byte_text: Vec<u8> = Vec::new();
        if wide {
            let units: Vec<u16> = (0..count).filter_map(|i| u16_at(r, off_string as usize + i * 2)).collect();
            text = String::from_utf16_lossy(&units);
        } else {
            byte_text = (0..count).filter_map(|i| r.get(off_string as usize + i).copied()).collect();
        }
        if self.path.is_some() {
            return;
        }
        // The spacing of the characters: the offset is where the record says (after the optional rectangle).
        let off_dx = [u32_at(r, 72), u32_at(r, 56)].into_iter().flatten().find(|&o| o as usize >= 56 && (o as usize) < r.len() && o as usize + count * 4 <= r.len()).unwrap_or(0) as usize;
        let pdy = options & 0x2000 != 0;
        let mut dx: Vec<f64> = if off_dx != 0 { (0..count).filter_map(|i| u32_at(r, off_dx + i * if pdy { 8 } else { 4 })).map(|v| f64::from(v as i32)).collect() } else { Vec::new() };
        if !wide {
            // Text in the character set of the font; the spacing is per byte.
            let (t, merged) = crate::charset::decode_with_spacing(&byte_text, self.dc.font.charset, &dx);
            text = t;
            dx = merged;
        }
        let matrix = self.matrix();
        let mut font = self.text_font();
        // The page's own turn adds to the font's.
        font.angle -= matrix[1].atan2(matrix[0]).to_degrees();
        let align = self.dc.text_align;
        let (x, y) = self.point(f64::from(rx), f64::from(ry));
        let scale_x = (matrix[0] * matrix[0] + matrix[1] * matrix[1]).sqrt();
        let vertical = match align & 24 {
            8 => -crate::svg::font_metrics(&self.dc.font.face, font.bold, font.italic).1 * font.size,
            24 => 0.0,
            _ => crate::svg::font_metrics(&self.dc.font.face, font.bold, font.italic).0 * font.size,
        };
        let (xs, anchor): (Vec<f64>, &str) = if dx.is_empty() {
            (vec![x], match align & 6 {
                2 => "end",
                6 => "middle",
                _ => "start",
            })
        } else {
            let total: f64 = dx.iter().sum::<f64>() * scale_x;
            let start = match align & 6 {
                2 => x - total,
                6 => x - total / 2.0,
                _ => x,
            };
            let mut at = start;
            let mut xs = Vec::with_capacity(dx.len());
            for d in &dx {
                xs.push(at);
                at += d * scale_x;
            }
            (xs, "start")
        };
        let clip = self.dc.clip;
        let color = self.dc.text_color;
        self.canvas.grow(&IDENTITY, &[(x, y)]);
        self.canvas.text(&IDENTITY, clip, &xs, y + vertical, (x, y), &text, &font, color, 1.0, anchor);
    }

    fn place_dib(&mut self, r: &[u8], dest: (f64, f64, f64, f64), info_at: usize, source: Option<(i32, i32, i32, i32)>, rop: u32) {
        let Some((info, bits)) = crate::gdi::dib_parts(r, info_at) else { return };
        let Some(png) = dib_png(info, bits, source, rop) else { return };
        let matrix = self.matrix();
        let extent = [(dest.0, dest.1), (dest.0 + dest.2, dest.1 + dest.3)];
        self.canvas.grow(&matrix, &extent);
        let clip = self.dc.clip;
        self.canvas.image(&matrix, clip, dest.0, dest.1, dest.2, dest.3, "image/png", &png);
    }

    fn stretch_dibits(&mut self, r: &[u8]) {
        let f = |at: usize| i32_at(r, at).unwrap_or(0);
        let dest = (f64::from(f(24)), f64::from(f(28)), f64::from(f(72)), f64::from(f(76)));
        let rop = u32_at(r, 68).unwrap_or(0x00CC_0020);
        self.place_dib(r, dest, 48, Some((f(32), f(36), f(40), f(44))), rop);
    }

    fn set_dibits(&mut self, r: &[u8]) {
        let f = |at: usize| i32_at(r, at).unwrap_or(0);
        let dest = (f64::from(f(24)), f64::from(f(28)), f64::from(f(40)), f64::from(f(44)));
        self.place_dib(r, dest, 48, Some((f(32), f(36), f(40), f(44))), 0x00CC_0020);
    }

    /// EMR_BITBLT and STRETCHBLT.
    fn blit(&mut self, r: &[u8], stretch: bool) {
        let f = |at: usize| i32_at(r, at).unwrap_or(0);
        let dest = (f64::from(f(24)), f64::from(f(28)), f64::from(f(32)), f64::from(f(36)));
        let rop = u32_at(r, 40).unwrap_or(0);
        if u32_at(r, 88).unwrap_or(0) == 0 {
            // No bitmap: a fill with the brush, black or white.
            let color = match rop {
                0x00F0_0021 => self.fill_color(),
                0x0000_0042 => Some(0),
                0x00FF_0062 => Some(0xFF_FFFF),
                _ => None,
            };
            if let Some(color) = color {
                let (x, y, w, h) = dest;
                let d = self.lines(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)], true, false);
                let extent = self.rect_extent(x, y, x + w, y + h);
                self.canvas.grow(&IDENTITY, &extent);
                let paint = Paint { fill: Some(color), fill_url: None, fill_alpha: 1.0, even_odd: false, stroke: None };
                let clip = self.dc.clip;
                self.canvas.path(&IDENTITY, clip, &d, &paint);
            }
            return;
        }
        let source = if stretch { Some((f(44), f(48), f(100), f(104))) } else { Some((f(44), f(48), f(32), f(36))) };
        self.place_dib(r, dest, 84, source, rop);
    }

    /// EMR_EXTSELECTCLIPRGN: the region's rectangles as the clip.
    fn select_clip_region(&mut self, r: &[u8]) {
        let (Some(size), Some(mode)) = (u32_at(r, 8), u32_at(r, 12)) else { return };
        // Mode RGN_COPY (5) with no data resets the clip.
        if size == 0 {
            if mode == 5 {
                self.dc.clip = None;
                self.dc.clip_box = None;
            }
            return;
        }
        // RGNDATA: a 32-byte header (the rectangle count at 8) and the rectangles.
        let data = &r[16.min(r.len())..];
        let Some(count) = u32_at(data, 8) else { return };
        let mut d = String::new();
        let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for i in 0..(count as usize).min(r.len() / 16) {
            let at = 32 + i * 16;
            let (Some(l), Some(t), Some(rr), Some(b)) = (i32_at(data, at), i32_at(data, at + 4), i32_at(data, at + 8), i32_at(data, at + 12)) else { break };
            let pts = self.rect_extent(f64::from(l), f64::from(t), f64::from(rr), f64::from(b));
            for (j, (x, y)) in pts.iter().enumerate() {
                let _ = write!(d, "{}{} {} ", if j == 0 { "M" } else { "L" }, fmt(*x), fmt(*y));
                bounds = (bounds.0.min(*x), bounds.1.min(*y), bounds.2.max(*x), bounds.3.max(*y));
            }
            d.push('Z');
        }
        if d.is_empty() {
            return;
        }
        self.dc.clip = Some(self.canvas.clip_path(&d, false));
        self.dc.clip_box = Some(bounds);
    }
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".to_string() } else { s.to_string() }
}

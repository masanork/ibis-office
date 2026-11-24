// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! What the metafile players draw with: an SVG canvas (shapes, text, pictures and clips), and the decoding of the device-independent
//! bitmaps the records carry.

use std::fmt::Write as _;

/// A matrix `[a, b, c, d, e, f]`: x' = a·x + c·y + e, y' = b·x + d·y + f (SVG's order).
pub(crate) type Matrix = [f64; 6];

pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` applied after `n` (first `n`, then `m`).
pub(crate) fn then(n: Matrix, m: Matrix) -> Matrix {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

pub(crate) fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// How much `m` scales lengths, on average.
pub(crate) fn scale_of(m: &Matrix) -> f64 {
    (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
}

/// Gives the ascent and descent of a typeface (as fractions of its em size) from the font that will draw it:
/// for a face name, bold and italic; `None` when the font is not known.
pub type FontMetricsFn = fn(&str, bool, bool) -> Option<(f64, f64)>;

static FONT_METRICS: std::sync::OnceLock<FontMetricsFn> = std::sync::OnceLock::new();

/// Set where the metrics of fonts come from (once; the first call stands).
pub fn set_font_metrics(f: FontMetricsFn) {
    let _ = FONT_METRICS.set(f);
}

/// The ascent and descent of a typeface, as fractions of its em size (the line's cell above and below the baseline):
/// from the font (`set_font_metrics`), else those of a common sans-serif.
pub(crate) fn font_metrics(face: &str, bold: bool, italic: bool) -> (f64, f64) {
    FONT_METRICS.get().and_then(|f| f(face, bold, italic)).filter(|(a, d)| *a > 0.0 && *d >= 0.0 && a + d < 3.0).unwrap_or((0.905, 0.212))
}

pub(crate) fn color(c: u32) -> String {
    let [r, g, b] = crate::gdi::colorref(c);
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".to_string() } else { s.to_string() }
}

/// How shapes are painted.
#[derive(Clone, Debug)]
pub(crate) struct Paint {
    pub(crate) fill: Option<u32>,
    /// A paint server (`url(#...)`) in place of the colour.
    pub(crate) fill_url: Option<String>,
    pub(crate) fill_alpha: f64,
    pub(crate) even_odd: bool,
    /// Colour, width (in the shape's own units), dash pattern, cap and join.
    pub(crate) stroke: Option<Stroke>,
}

#[derive(Clone, Debug)]
pub(crate) struct Stroke {
    pub(crate) color: u32,
    pub(crate) alpha: f64,
    pub(crate) miter: Option<f64>,
    pub(crate) width: f64,
    pub(crate) dash: Vec<f64>,
    pub(crate) cap: &'static str,
    pub(crate) join: &'static str,
}

/// A font for text.
#[derive(Clone, Debug, Default)]
pub(crate) struct Font {
    pub(crate) face: String,
    /// The size of the text, in the units of the text's own space.
    pub(crate) size: f64,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strike: bool,
    /// Degrees counterclockwise.
    pub(crate) angle: f64,
}

#[derive(Default)]
pub(crate) struct Canvas {
    body: String,
    defs: String,
    clips: usize,
    paints: usize,
    /// How many things were drawn.
    pub(crate) drawn: usize,
    /// The extent of what was drawn, in device units.
    pub(crate) extent: Option<(f64, f64, f64, f64)>,
}

impl Canvas {
    pub(crate) fn grow(&mut self, m: &Matrix, points: &[(f64, f64)]) {
        for &(x, y) in points {
            let (x, y) = apply(m, x, y);
            self.extent = Some(match self.extent {
                None => (x, y, x, y),
                Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
            });
        }
    }

    /// A clip made of a path (in device units); its id.
    /// Add a paint server whose XML has `{id}` where its id goes; the reference to fill with is returned.
    pub(crate) fn paint_server(&mut self, xml: &str) -> String {
        self.paints += 1;
        let id = format!("p{}", self.paints);
        self.defs += &xml.replace("{id}", &id);
        format!("url(#{id})")
    }

    /// Open a group that clips what follows (until `close_group`).
    pub(crate) fn open_group(&mut self, clip: Option<usize>) {
        match clip {
            Some(c) => {
                let _ = write!(self.body, r#"<g clip-path="url(#c{c})">"#);
            }
            None => self.body += "<g>",
        }
    }

    pub(crate) fn close_group(&mut self) {
        self.body += "</g>";
    }

    pub(crate) fn clip_path(&mut self, d: &str, even_odd: bool) -> usize {
        self.clips += 1;
        let _ = write!(self.defs, r#"<clipPath id="c{}"><path d="{}" clip-rule="{}"/></clipPath>"#, self.clips, d, if even_odd { "evenodd" } else { "nonzero" });
        self.clips
    }

    /// The attributes of an element drawn in the units `m` maps to the device. The clip is in device units, but a
    /// clip-path next to a transform is in the element's own units, so a clipped, transformed element is wrapped in a
    /// group that carries the clip (`wrap`: whether to close the group after the element).
    fn attrs(&mut self, m: &Matrix, clip: Option<usize>) -> (String, bool) {
        let mut s = String::new();
        if *m != IDENTITY {
            let _ = write!(s, r#" transform="matrix({} {} {} {} {} {})""#, num(m[0]), num(m[1]), num(m[2]), num(m[3]), num(m[4]), num(m[5]));
        }
        let mut wrap = false;
        if let Some(c) = clip {
            if *m != IDENTITY {
                let _ = write!(self.body, r##"<g clip-path="url(#c{c})">"##);
                wrap = true;
            } else {
                let _ = write!(s, r#" clip-path="url(#c{c})""#);
            }
        }
        (s, wrap)
    }

    /// A path in the units `m` maps to the device, painted as `paint` says.
    pub(crate) fn path(&mut self, m: &Matrix, clip: Option<usize>, d: &str, paint: &Paint) {
        if d.is_empty() || (paint.fill.is_none() && paint.fill_url.is_none() && paint.stroke.is_none()) {
            return;
        }
        let (attrs, wrap) = self.attrs(m, clip);
        let mut s = format!("<path{attrs} d=\"{d}\"");
        if let Some(url) = &paint.fill_url {
            let _ = write!(s, r#" fill="{url}" fill-rule="{}""#, if paint.even_odd { "evenodd" } else { "nonzero" });
        } else {
        match paint.fill {
            Some(c) => {
                let _ = write!(s, r#" fill="{}" fill-rule="{}""#, color(c), if paint.even_odd { "evenodd" } else { "nonzero" });
                if paint.fill_alpha < 1.0 {
                    let _ = write!(s, r#" fill-opacity="{}""#, num(paint.fill_alpha));
                }
            }
            None => s += r#" fill="none""#,
        }
        }
        if let Some(st) = &paint.stroke {
            let _ = write!(s, r#" stroke="{}" stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}""#, color(st.color), num(st.width), st.cap, st.join);
            if st.alpha < 1.0 {
                let _ = write!(s, r#" stroke-opacity="{}""#, num(st.alpha));
            }
            if let Some(m) = st.miter {
                let _ = write!(s, r#" stroke-miterlimit="{}""#, num(m));
            }
            if !st.dash.is_empty() {
                let dash: Vec<String> = st.dash.iter().map(|v| num(*v)).collect();
                let _ = write!(s, r#" stroke-dasharray="{}""#, dash.join(" "));
            }
        }
        s += "/>";
        if wrap {
            s += "</g>";
        }
        self.body += &s;
        self.drawn += 1;
    }

    /// Text at (`x`, `y`) in the units `m` maps to the device: one `x` for each character, or just the first. `anchor` is
    /// `start`, `middle` or `end`.
    pub(crate) fn text(&mut self, m: &Matrix, clip: Option<usize>, xs: &[f64], y: f64, center: (f64, f64), text: &str, font: &Font, fill: u32, alpha: f64, anchor: &str) {
        if text.trim().is_empty() || xs.is_empty() {
            return;
        }
        // The matrix, then a turn about the reference point.
        let mut transform = String::new();
        if *m != IDENTITY {
            let _ = write!(transform, "matrix({} {} {} {} {} {})", num(m[0]), num(m[1]), num(m[2]), num(m[3]), num(m[4]), num(m[5]));
        }
        if font.angle != 0.0 {
            if !transform.is_empty() {
                transform.push(' ');
            }
            let _ = write!(transform, "rotate({} {} {})", num(-font.angle), num(center.0), num(center.1));
        }
        let mut attrs = if transform.is_empty() { String::new() } else { format!(r#" transform="{transform}""#) };
        // (A clip next to a transform would be in the element's own units: the group carries it.)
        let wrap = clip.is_some() && !transform.is_empty();
        if let Some(c) = clip {
            if wrap {
                let _ = write!(self.body, r##"<g clip-path="url(#c{c})">"##);
            } else {
                let _ = write!(attrs, r#" clip-path="url(#c{c})""#);
            }
        }
        let xlist: Vec<String> = xs.iter().map(|v| num(*v)).collect();
        let mut s = format!(
            "<text{attrs} x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\" text-anchor=\"{anchor}\" xml:space=\"preserve\"",
            xlist.join(" "),
            num(y),
            family(&font.face),
            num(font.size),
            color(fill)
        );
        if alpha < 1.0 {
            let _ = write!(s, r#" fill-opacity="{}""#, num(alpha));
        }
        if font.bold {
            s += r#" font-weight="bold""#;
        }
        if font.italic {
            s += r#" font-style="italic""#;
        }
        match (font.underline, font.strike) {
            (true, true) => s += r#" text-decoration="underline line-through""#,
            (true, false) => s += r#" text-decoration="underline""#,
            (false, true) => s += r#" text-decoration="line-through""#,
            _ => {}
        }
        let _ = write!(s, ">{}</text>{}", escape(text), if wrap { "</g>" } else { "" });
        self.body += &s;
        self.drawn += 1;
    }

    /// A picture (PNG) in the rectangle `x, y, width, height` (negative sizes flip it) in the units `m` maps to the device.
    pub(crate) fn image(&mut self, m: &Matrix, clip: Option<usize>, x: f64, y: f64, width: f64, height: f64, mime: &str, png: &[u8]) {
        if width == 0.0 || height == 0.0 {
            return;
        }
        // A flip is a scale about the rectangle's own corner.
        let (w, h) = (width.abs(), height.abs());
        let (ox, oy) = (if width < 0.0 { x + width } else { x }, if height < 0.0 { y + height } else { y });
        let mut local = IDENTITY;
        if width < 0.0 {
            local = then(local, [-1.0, 0.0, 0.0, 1.0, 2.0 * ox + w, 0.0]);
        }
        if height < 0.0 {
            local = then(local, [1.0, 0.0, 0.0, -1.0, 0.0, 2.0 * oy + h]);
        }
        let all = then(local, *m);
        let (attrs, wrap) = self.attrs(&all, clip);
        let _ = write!(
            self.body,
            r#"<image{} x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none" href="data:{};base64,{}"/>{}"#,
            attrs,
            num(ox),
            num(oy),
            num(w),
            num(h),
            mime,
            base64(png),
            if wrap { "</g>" } else { "" }
        );
        self.drawn += 1;
    }

    /// The document, over the rectangle `view` (device units).
    pub(crate) fn finish(self, view: (f64, f64, f64, f64)) -> String {
        let (x, y, w, h) = view;
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{}" height="{}" preserveAspectRatio="none"><defs>{}</defs>{}</svg>"#,
            num(x),
            num(y),
            num(w),
            num(h),
            num(w),
            num(h),
            self.defs,
            self.body
        )
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

fn family(face: &str) -> String {
    let face = face.replace('\'', "");
    if face.is_empty() { "sans-serif".to_string() } else { format!("'{face}', sans-serif") }
}

pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(d: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// A device-independent bitmap (its header, palette and bits as the records carry them) as a PNG; `source` is the part of it used
/// (`x, y, width, height`) when it is not the whole, and `rop` is the GDI raster operation (`SRCPAINT` leaves black transparent;
/// `SRCAND` leaves white transparent).
pub(crate) fn dib_png(info: &[u8], bits: &[u8], source: Option<(i32, i32, i32, i32)>, rop: u32) -> Option<Vec<u8>> {
    let header_size = u32_at(info, 0)? as usize;
    let (width, height, bit_count, compression, used) = if header_size == 12 {
        (i32::from(u16_at(info, 4)?), i32::from(u16_at(info, 6)?), u16_at(info, 10)?, 0, 0)
    } else {
        (i32_at(info, 4)?, i32_at(info, 8)?, u16_at(info, 14)?, u32_at(info, 16)?, u32_at(info, 32)?)
    };
    if width <= 0 || height == 0 || width > 16_384 || height.abs() > 16_384 {
        return None;
    }
    let top_down = height < 0;
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let palette_at = header_size + if compression == 3 && header_size == 40 { 12 } else { 0 };
    let entry = if header_size == 12 { 3 } else { 4 };
    let colors = if bit_count <= 8 { if used != 0 { used as usize } else { 1usize << bit_count } } else { 0 };
    let palette: Vec<[u8; 3]> = (0..colors).filter_map(|i| info.get(palette_at + i * entry..palette_at + i * entry + 3).map(|b| [b[2], b[1], b[0]])).collect();
    // Bit masks of 16- and 32-bit pixels.
    let masks = |at: usize| -> Option<[u32; 3]> { Some([u32_at(info, at)?, u32_at(info, at + 4)?, u32_at(info, at + 8)?]) };
    let field = if compression == 3 && header_size >= 40 { masks(40.min(header_size)) } else { None };
    let stride = (w * bit_count as usize).div_ceil(32) * 4;
    if compression != 0 && compression != 3 {
        return None;
    }
    if bits.len() < stride * (h - 1) + (w * bit_count as usize).div_ceil(8) {
        return None;
    }
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let src_row = if top_down { y } else { h - 1 - y };
        let row = &bits[src_row * stride..];
        for x in 0..w {
            let (r, g, b, a) = match bit_count {
                1 | 4 | 8 => {
                    let per = 8 / bit_count as usize;
                    let byte = row[x / per];
                    let shift = 8 - bit_count as usize * (x % per + 1);
                    let index = ((byte >> shift) as usize) & ((1 << bit_count) - 1);
                    let c = palette.get(index).copied().unwrap_or([0, 0, 0]);
                    (c[0], c[1], c[2], 255)
                }
                16 => {
                    let v = u32::from(u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]));
                    let pick = |mask: u32| -> u8 {
                        if mask == 0 {
                            return 0;
                        }
                        let shift = mask.trailing_zeros();
                        let max = mask >> shift;
                        (((v & mask) >> shift) * 255 / max.max(1)) as u8
                    };
                    match field {
                        Some([r, g, b]) => (pick(r), pick(g), pick(b), 255),
                        None => (pick(0x7C00), pick(0x03E0), pick(0x001F), 255),
                    }
                }
                24 => (row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255),
                32 => {
                    let p = &row[x * 4..x * 4 + 4];
                    (p[2], p[1], p[0], 255)
                }
                _ => return None,
            };
            let a = match rop {
                // SRCPAINT (dest | src) and SRCINVERT (dest ^ src): black (0) leaves the destination unchanged.
                0x00EE_0086 | 0x0066_0046 if (r, g, b) == (0, 0, 0) => 0,
                // SRCAND (dest & src): white (0xFF) leaves the destination unchanged.
                0x0088_00C6 if (r, g, b) == (255, 255, 255) => 0,
                _ => a,
            };
            let at = (y * w + x) * 4;
            rgba[at..at + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    // The part used.
    let (sx, sy, sw, sh) = match source {
        Some((x, y, cw, ch)) if cw > 0 && ch > 0 && x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h => (x as usize, y as usize, (cw as usize).min(w - x as usize), (ch as usize).min(h - y as usize)),
        _ => (0, 0, w, h),
    };
    // (A bottom-up picture counts rows from its bottom.)
    let sy = if top_down { sy } else { h.saturating_sub(sy + sh) };
    if (sx, sy, sw, sh) == (0, 0, w, h) {
        return Some(crate::png::encode(w as u32, h as u32, &rgba));
    }
    // (The source rectangle counts from the top of the picture as it is shown.)
    let mut part = Vec::with_capacity(sw * sh * 4);
    for y in sy..sy + sh {
        part.extend_from_slice(&rgba[(y * w + sx) * 4..(y * w + sx + sw) * 4]);
    }
    Some(crate::png::encode(sw as u32, sh as u32, &part))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clip is in device units, a transform scales the element: the clip-path must not sit on the transformed
    /// element (it would be scaled with it, and a picture clipped to a small box would vanish).
    #[test]
    fn a_clip_next_to_a_transform_is_on_a_group_of_its_own() {
        let mut canvas = Canvas::default();
        let clip = canvas.clip_path("M0 0 L0 100 L100 100 L100 0 Z", false);
        canvas.image(&[0.1, 0.0, 0.0, 0.1, 0.0, 0.0], Some(clip), 0.0, 0.0, 1000.0, 1000.0, "image/png", &[1, 2, 3]);
        let svg = canvas.finish((0.0, 0.0, 100.0, 100.0));
        let group = svg.find("<g clip-path=").expect("a group carries the clip");
        let image = svg.find("<image transform=").expect("the image carries the transform");
        assert!(group < image && !svg[image..svg[image..].find('>').map_or(0, |n| image + n)].contains("clip-path"), "{svg}");
        assert!(svg.contains("</g>"));
    }
}

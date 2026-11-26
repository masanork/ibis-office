// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! 色の動作（`p:animClr`、色を値に持つ `p:set`）の評価（ECMA-376 Part 1 19.5.3 `p:animClr`）。
//!
//! `p:animClr` は、`from`（なければ図形の今の色）から `to` まで、`clrSpc`（`rgb` か `hsl`）の空間で一様に動かす。`by` は、
//! 今の色に足す量（`p:hsl` は色相が 1/60000 度、彩度と輝度が 1/100000。`p:rgb` は各チャネルが 1/100000）。
//! 色相の動く向きは `dir`（`cw` で増える向き、`ccw` で減る向き）。

use crate::drawing::Rgb;

/// A colour an animation names: a theme colour (`a:schemeClr`) or a plain one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimColor {
    Scheme(String),
    Rgb(Rgb),
}

/// What a colour behaviour changes (`attrName`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSlot {
    /// `fillcolor`, `fillColor`.
    Fill,
    /// `stroke.color`.
    Line,
    /// `style.color`.
    Text,
}

/// A colour behaviour at a moment: `q` (0 to 1) of the way from `from` to `to` (or by `by`).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorChange {
    pub slot: ColorSlot,
    pub from: Option<AnimColor>,
    pub to: Option<AnimColor>,
    /// `p:by` in hue (degrees), saturation and lightness (0 to 1 each).
    pub by_hsl: Option<(f64, f64, f64)>,
    /// `p:by` in red, green and blue (0 to 1 each).
    pub by_rgb: Option<(f64, f64, f64)>,
    /// `clrSpc="hsl"`.
    pub hsl: bool,
    /// `dir="ccw"`.
    pub ccw: bool,
    /// The colours a `p:anim` of colours (`valueType="clr"`, `p:tavLst`) goes through, each at its time (0 to 1).
    pub steps: Vec<(f64, AnimColor)>,
    /// The steps are taken at once (`calcmode="discrete"`), not run into one another.
    pub discrete: bool,
    pub q: f64,
}

/// A colour behaviour as written (before its time).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorSpec {
    pub change: ColorChange,
    /// A `p:set`: the colour is `to` from the start (no moving).
    pub is_set: bool,
}

fn slot_of(attr: &str) -> Option<ColorSlot> {
    match attr {
        "fillcolor" | "fillColor" => Some(ColorSlot::Fill),
        "stroke.color" => Some(ColorSlot::Line),
        "style.color" => Some(ColorSlot::Text),
        _ => None,
    }
}

/// Whether `s` starts with an XML start tag whose local name (ignoring any namespace prefix) is `local`.
pub(crate) fn starts_with_local_tag(s: &str, local: &str) -> bool {
    let Some(rest) = s.trim_start().strip_prefix('<') else { return false };
    let name_end = rest.find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/').unwrap_or(rest.len());
    let qname = &rest[..name_end];
    let lname = qname.rsplit_once(':').map_or(qname, |(_, l)| l);
    lname == local
}

/// Finds the byte offset of the first XML start tag in `s` whose local name is `local`.
fn find_local_tag(s: &str, local: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(rel) = s[i..].find('<') {
        let pos = i + rel;
        let rest = &s[pos + 1..];
        if !rest.starts_with('/') && !rest.starts_with('!') && !rest.starts_with('?') {
            let name_end = rest.find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/').unwrap_or(rest.len());
            let qname = &rest[..name_end];
            let lname = qname.rsplit_once(':').map_or(qname, |(_, l)| l);
            if lname == local {
                return Some(pos);
            }
        }
        i = pos + 1;
        if i >= bytes.len() {
            break;
        }
    }
    None
}

/// The value of attribute `key` in the first start tag whose local name is `local` (supports both `"` and `'`).
pub(crate) fn attr_of_local<'a>(s: &'a str, local: &str, key: &str) -> Option<&'a str> {
    let at = find_local_tag(s, local)?;
    let rest = &s[at..];
    let end = rest.find('>')?;
    let head = &rest[..end];
    let bytes = head.as_bytes();
    let mut i = head.find(|c: char| c.is_ascii_whitespace())?;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'/' || bytes[i] == b'>' {
            break;
        }
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'/' && bytes[i] != b'>' {
            i += 1;
        }
        let attr_name = &head[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        i += 1;
        let val_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        let val = &head[val_start..i];
        if i < bytes.len() {
            i += 1;
        }
        let attr_local = attr_name.rsplit_once(':').map_or(attr_name, |(_, l)| l);
        if attr_name == key || attr_local == key {
            return Some(val);
        }
    }
    None
}

/// The part of `raw` inside `<*:local ...>` … `</*:local>` (any namespace prefix).
fn section_local<'a>(raw: &'a str, local: &str) -> Option<&'a str> {
    let at = find_local_tag(raw, local)?;
    let rest = &raw[at..];
    let open_end = rest.find('>')?;
    if rest[..open_end].ends_with('/') {
        return None;
    }
    let inner = &rest[open_end + 1..];
    // Find matching closing tag `</*:local>`.
    let mut scan = 0;
    while let Some(rel) = inner[scan..].find("</") {
        let close_pos = scan + rel;
        let after = &inner[close_pos + 2..];
        if let Some(gt) = after.find('>') {
            let qname = after[..gt].trim();
            let lname = qname.rsplit_once(':').map_or(qname, |(_, l)| l);
            if lname == local {
                return Some(&inner[..close_pos]);
            }
        }
        scan = close_pos + 2;
    }
    None
}

fn linear_to_srgb(linear: f64) -> f64 {
    let x = linear.clamp(0.0, 1.0);
    if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
}

fn system_color_fallback(name: &str) -> Option<Rgb> {
    Some(Rgb(match name {
        "window" | "windowFrame" | "highlightText" | "btnHighlight" | "3dLight" => 0xFFFFFF,
        "windowText" | "captionText" | "menuText" | "btnText" | "infoText" => 0x000000,
        "highlight" | "hotLight" => 0x0078D7,
        "grayText" | "btnShadow" => 0x808080,
        "btnFace" | "menu" | "scrollBar" => 0xF0F0F0,
        "infoBk" => 0xFFFFE1,
        _ => return None,
    }))
}

fn preset_color(name: &str) -> Option<Rgb> {
    let short;
    let name = match (name.strip_prefix("dark"), name.strip_prefix("light"), name.strip_prefix("medium")) {
        (Some(rest), _, _) => {
            short = format!("dk{rest}");
            short.as_str()
        }
        (_, Some(rest), _) => {
            short = format!("lt{rest}");
            short.as_str()
        }
        (_, _, Some(rest)) => {
            short = format!("med{rest}");
            short.as_str()
        }
        _ => name,
    };
    Some(Rgb(match name {
        "aliceBlue" => 0xf0f8ff, "antiqueWhite" => 0xfaebd7, "aqua" => 0x00ffff, "aquamarine" => 0x7fffd4,
        "azure" => 0xf0ffff, "beige" => 0xf5f5dc, "bisque" => 0xffe4c4, "black" => 0x000000,
        "blanchedAlmond" => 0xffebcd, "blue" => 0x0000ff, "blueViolet" => 0x8a2be2, "brown" => 0xa52a2a,
        "burlyWood" => 0xdeb887, "cadetBlue" => 0x5f9ea0, "chartreuse" => 0x7fff00, "chocolate" => 0xd2691e,
        "coral" => 0xff7f50, "cornflowerBlue" => 0x6495ed, "cornsilk" => 0xfff8dc, "crimson" => 0xdc143c,
        "cyan" => 0x00ffff, "dkBlue" => 0x00008b, "dkCyan" => 0x008b8b, "dkGoldenrod" => 0xb8860b,
        "dkGray" | "dkGrey" => 0xa9a9a9, "dkGreen" => 0x006400, "dkKhaki" => 0xbdb76b, "dkMagenta" => 0x8b008b,
        "dkOliveGreen" => 0x556b2f, "dkOrange" => 0xff8c00, "dkOrchid" => 0x9932cc, "dkRed" => 0x8b0000,
        "dkSalmon" => 0xe9967a, "dkSeaGreen" => 0x8fbc8f, "dkSlateBlue" => 0x483d8b, "dkSlateGray" | "dkSlateGrey" => 0x2f4f4f,
        "dkTurquoise" => 0x00ced1, "dkViolet" => 0x9400d3, "deepPink" => 0xff1493, "deepSkyBlue" => 0x00bfff,
        "dimGray" | "dimGrey" => 0x696969, "dodgerBlue" => 0x1e90ff, "firebrick" => 0xb22222, "floralWhite" => 0xfffaf0,
        "forestGreen" => 0x228b22, "fuchsia" => 0xff00ff, "gainsboro" => 0xdcdcdc, "ghostWhite" => 0xf8f8ff,
        "gold" => 0xffd700, "goldenrod" => 0xdaa520, "gray" | "grey" => 0x808080, "green" => 0x008000,
        "greenYellow" => 0xadff2f, "honeydew" => 0xf0fff0, "hotPink" => 0xff69b4, "indianRed" => 0xcd5c5c,
        "indigo" => 0x4b0082, "ivory" => 0xfffff0, "khaki" => 0xf0e68c, "lavender" => 0xe6e6fa,
        "lavenderBlush" => 0xfff0f5, "lawnGreen" => 0x7cfc00, "lemonChiffon" => 0xfffacd, "ltBlue" => 0xadd8e6,
        "ltCoral" => 0xf08080, "ltCyan" => 0xe0ffff, "ltGoldenrodYellow" => 0xfafad2, "ltGray" | "ltGrey" => 0xd3d3d3,
        "ltGreen" => 0x90ee90, "ltPink" => 0xffb6c1, "ltSalmon" => 0xffa07a, "ltSeaGreen" => 0x20b2aa,
        "ltSkyBlue" => 0x87cefa, "ltSlateGray" | "ltSlateGrey" => 0x778899, "ltSteelBlue" => 0xb0c4de, "ltYellow" => 0xffffe0,
        "lime" => 0x00ff00, "limeGreen" => 0x32cd32, "linen" => 0xfaf0e6, "magenta" => 0xff00ff,
        "maroon" => 0x800000, "medAquamarine" => 0x66cdaa, "medBlue" => 0x0000cd, "medOrchid" => 0xba55d3,
        "medPurple" => 0x9370db, "medSeaGreen" => 0x3cb371, "medSlateBlue" => 0x7b68ee, "medSpringGreen" => 0x00fa9a,
        "medTurquoise" => 0x48d1cc, "medVioletRed" => 0xc71585, "midnightBlue" => 0x191970, "mintCream" => 0xf5fffa,
        "mistyRose" => 0xffe4e1, "moccasin" => 0xffe4b5, "navajoWhite" => 0xffdead, "navy" => 0x000080,
        "oldLace" => 0xfdf5e6, "olive" => 0x808000, "oliveDrab" => 0x6b8e23, "orange" => 0xffa500,
        "orangeRed" => 0xff4500, "orchid" => 0xda70d6, "paleGoldenrod" => 0xeee8aa, "paleGreen" => 0x98fb98,
        "paleTurquoise" => 0xafeeee, "paleVioletRed" => 0xdb7093, "papayaWhip" => 0xffefd5, "peachPuff" => 0xffdab9,
        "peru" => 0xcd853f, "pink" => 0xffc0cb, "plum" => 0xdda0dd, "powderBlue" => 0xb0e0e6,
        "purple" => 0x800080, "red" => 0xff0000, "rosyBrown" => 0xbc8f8f, "royalBlue" => 0x4169e1,
        "saddleBrown" => 0x8b4513, "salmon" => 0xfa8072, "sandyBrown" => 0xf4a460, "seaGreen" => 0x2e8b57,
        "seaShell" => 0xfff5ee, "sienna" => 0xa0522d, "silver" => 0xc0c0c0, "skyBlue" => 0x87ceeb,
        "slateBlue" => 0x6a5acd, "slateGray" | "slateGrey" => 0x708090, "snow" => 0xfffafa, "springGreen" => 0x00ff7f,
        "steelBlue" => 0x4682b4, "tan" => 0xd2b48c, "teal" => 0x008080, "thistle" => 0xd8bfd8,
        "tomato" => 0xff6347, "turquoise" => 0x40e0d0, "violet" => 0xee82ee, "wheat" => 0xf5deb3,
        "white" => 0xffffff, "whiteSmoke" => 0xf5f5f5, "yellow" => 0xffff00, "yellowGreen" => 0x9acd32,
        _ => return None,
    }))
}

fn color_in(s: &str) -> Option<AnimColor> {
    if let Some(v) = attr_of_local(s, "schemeClr", "val") {
        return Some(AnimColor::Scheme(v.to_string()));
    }
    if let Some(v) = attr_of_local(s, "srgbClr", "val") {
        return u32::from_str_radix(v, 16).ok().map(|n| AnimColor::Rgb(Rgb(n)));
    }
    if let Some(v) = attr_of_local(s, "prstClr", "val") {
        return preset_color(v).map(AnimColor::Rgb);
    }
    if find_local_tag(s, "sysClr").is_some() {
        let rgb = attr_of_local(s, "sysClr", "lastClr")
            .and_then(|v| u32::from_str_radix(v, 16).ok())
            .map(Rgb)
            .or_else(|| attr_of_local(s, "sysClr", "val").and_then(system_color_fallback))?;
        return Some(AnimColor::Rgb(rgb));
    }
    if find_local_tag(s, "hslClr").is_some() {
        let h = attr_of_local(s, "hslClr", "hue")?.parse::<f64>().ok()? / 60_000.0;
        let sat = attr_of_local(s, "hslClr", "sat")?.parse::<f64>().ok()? / 100_000.0;
        let lum = attr_of_local(s, "hslClr", "lum")?.parse::<f64>().ok()? / 100_000.0;
        return Some(AnimColor::Rgb(from_hsl(h, sat, lum)));
    }
    if find_local_tag(s, "scrgbClr").is_some() {
        let ch = |k: &str| {
            let lin = attr_of_local(s, "scrgbClr", k)?.parse::<f64>().ok()? / 100_000.0;
            Some((linear_to_srgb(lin) * 255.0).round().clamp(0.0, 255.0) as u8)
        };
        return Some(AnimColor::Rgb(Rgb::from_rgb(ch("r")?, ch("g")?, ch("b")?)));
    }
    if find_local_tag(s, "rgb").is_some() {
        let c = |k: &str| attr_of_local(s, "rgb", k)?.parse::<f64>().ok().map(|v| (v / 100_000.0 * 255.0).round().clamp(0.0, 255.0) as u8);
        return Some(AnimColor::Rgb(Rgb::from_rgb(c("r")?, c("g")?, c("b")?)));
    }
    None
}

/// The colour behaviour an opaque `p:animClr` or a `p:set` of a colour (`<p:to><p:clrVal>`) says.
pub fn color_spec(raw: &str) -> Option<ColorSpec> {
    let is_set = starts_with_local_tag(raw, "set");
    let is_anim = starts_with_local_tag(raw, "anim");
    if !is_set && !is_anim && !starts_with_local_tag(raw, "animClr") {
        return None;
    }
    let attr = section_local(raw, "attrName")?;
    let slot = slot_of(attr.trim())?;
    if is_anim {
        let list = section_local(raw, "tavLst")?;
        let mut steps = Vec::new();
        let mut rest = list;
        while let Some(at) = find_local_tag(rest, "tav") {
            let tav = &rest[at..];
            let end = tav.find("</").and_then(|c| {
                // Advance to closing `</*:tav>`.
                let mut scan = c;
                while let Some(rel) = tav[scan..].find("</") {
                    let cp = scan + rel;
                    let after = &tav[cp + 2..];
                    if let Some(gt) = after.find('>') {
                        let q = after[..gt].trim();
                        if q.rsplit_once(':').map_or(q, |(_, l)| l) == "tav" {
                            return Some(cp + 2 + gt + 1);
                        }
                    }
                    scan = cp + 2;
                }
                None
            }).unwrap_or(tav.len());
            let tm = attr_of_local(&tav[..end], "tav", "tm")?.parse::<f64>().ok()? / 100_000.0;
            steps.push((tm, color_in(&tav[..end])?));
            rest = &tav[end..];
        }
        if steps.is_empty() {
            return None;
        }
        let discrete = attr_of_local(raw, "anim", "calcmode") == Some("discrete");
        return Some(ColorSpec { change: ColorChange { slot, from: None, to: None, by_hsl: None, by_rgb: None, hsl: false, ccw: false, steps, discrete, q: 0.0 }, is_set: false });
    }
    let (from, to) = (section_local(raw, "from").and_then(color_in), section_local(raw, "to").and_then(color_in));
    let by = section_local(raw, "by");
    let by_hsl = by.filter(|b| find_local_tag(b, "hsl").is_some()).and_then(|b| {
        let n = |k: &str| attr_of_local(b, "hsl", k)?.parse::<f64>().ok();
        Some((n("h")? / 60_000.0, n("s")? / 100_000.0, n("l")? / 100_000.0))
    });
    let by_rgb = by.filter(|b| find_local_tag(b, "rgb").is_some()).and_then(|b| {
        let n = |k: &str| attr_of_local(b, "rgb", k)?.parse::<f64>().ok().map(|v| v / 100_000.0);
        Some((n("r")?, n("g")?, n("b")?))
    });
    if to.is_none() && by_hsl.is_none() && by_rgb.is_none() {
        return None;
    }
    let hsl = attr_of_local(raw, "animClr", "clrSpc") == Some("hsl");
    let ccw = attr_of_local(raw, "animClr", "dir") == Some("ccw");
    Some(ColorSpec { change: ColorChange { slot, from, to, by_hsl, by_rgb, hsl, ccw, steps: Vec::new(), discrete: false, q: 0.0 }, is_set })
}

/// Theme colour `name` of an animation (`bg1`, `tx2`, `accent3`…) as the theme scheme names it (`lt1`, `dk2`, `accent3`…).
pub fn scheme_slot(name: &str) -> &str {
    match name {
        "bg1" => "lt1",
        "tx1" => "dk1",
        "bg2" => "lt2",
        "tx2" => "dk2",
        other => other,
    }
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (f64::from(c.r()) / 255.0, f64::from(c.g()) / 255.0, f64::from(c.b()) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    (h * 60.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let q = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb::from_rgb(q(r), q(g), q(b))
}

impl ColorChange {
    /// The colour the shape has with this change at `q` of its time, where its own colour is `base`; `theme` gives a theme
    /// colour (by the scheme's name, `lt1`, `accent2`…).
    pub fn resolve(&self, base: Rgb, theme: &dyn Fn(&str) -> Option<Rgb>) -> Rgb {
        let rgb_of = |c: &AnimColor| match c {
            AnimColor::Rgb(c) => Some(*c),
            AnimColor::Scheme(n) => theme(scheme_slot(n)),
        };
        let q = self.q.clamp(0.0, 1.0);
        // The colour of text does not run: PowerPoint draws the text in its own colour for the whole time of the behaviour and in the new
        // one from its end (measured, フォントの色: black to 1.9 s of 2 s, then the accent colour; the fill and the outline do run).
        let q = if self.slot == ColorSlot::Text && self.steps.is_empty() && q < 1.0 { 0.0 } else { q };
        if !self.steps.is_empty() {
            // The colour of the last step whose time has come; between two steps, run from the one to the next (not `discrete`).
            let at = self.steps.iter().rposition(|(t, _)| *t <= q).unwrap_or(0);
            let here = rgb_of(&self.steps[at].1).unwrap_or(base);
            return match self.steps.get(at + 1) {
                Some((t1, next)) if !self.discrete => {
                    let (t0, end) = (self.steps[at].0, rgb_of(next).unwrap_or(here));
                    let k = if *t1 > t0 { (q - t0) / (t1 - t0) } else { 1.0 };
                    ColorChange { steps: Vec::new(), from: Some(AnimColor::Rgb(here)), to: Some(AnimColor::Rgb(end)), q: k, ..self.clone() }.resolve(base, theme)
                }
                _ => here,
            };
        }
        let start = self.from.as_ref().and_then(rgb_of).unwrap_or(base);
        let lerp = |a: f64, b: f64| a + (b - a) * q;
        let (h0, s0, l0) = to_hsl(start);
        if let Some((dh, ds, dl)) = self.by_hsl {
            // The hue ends `dh` away but goes the way `dir` says (measured on PowerPoint: 補色 2, `h="-7200000"` and `dir="cw"`, takes the hue
            // 240 degrees round through violet and orange to the green 120 degrees below the start, not the short way back).
            let travel = if self.ccw { -((-dh).rem_euclid(360.0)) } else { dh.rem_euclid(360.0) };
            return from_hsl(h0 + travel * q, s0 + ds * q, l0 + dl * q);
        }
        if let Some((dr, dg, db)) = self.by_rgb {
            let ch = |v: u8, d: f64| (f64::from(v) + d * 255.0 * q).round().clamp(0.0, 255.0) as u8;
            return Rgb::from_rgb(ch(start.r(), dr), ch(start.g(), dg), ch(start.b(), db));
        }
        let Some(end) = self.to.as_ref().and_then(rgb_of) else { return start };
        if self.hsl {
            let (h1, s1, l1) = to_hsl(end);
            // The hue goes the way `dir` says (clockwise: it grows).
            let mut dh = h1 - h0;
            if self.ccw {
                if dh > 0.0 {
                    dh -= 360.0;
                }
            } else if dh < 0.0 {
                dh += 360.0;
            }
            return from_hsl(h0 + dh * q, lerp(s0, s1), lerp(l0, l1));
        }
        let ch = |a: u8, b: u8| lerp(f64::from(a), f64::from(b)).round().clamp(0.0, 255.0) as u8;
        Rgb::from_rgb(ch(start.r(), end.r()), ch(start.g(), end.g()), ch(start.b(), end.b()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = r#"<p:animClr clrSpc="rgb" dir="cw"><p:cBhvr><p:cTn id="" dur="2000" fill="hold"/><p:attrNameLst><p:attrName>fillcolor</p:attrName></p:attrNameLst></p:cBhvr><p:to><a:schemeClr val="accent2"/></p:to></p:animClr>"#;

    #[test]
    fn a_colour_goes_from_its_own_to_the_named_one_in_rgb() {
        let spec = color_spec(RAW).unwrap();
        assert_eq!(spec.change.slot, ColorSlot::Fill);
        let theme = |n: &str| (n == "accent2").then_some(Rgb::from_rgb(200, 100, 0));
        let at = |q: f64| ColorChange { q, ..spec.change.clone() }.resolve(Rgb::from_rgb(0, 100, 200), &theme);
        assert_eq!(at(0.0), Rgb::from_rgb(0, 100, 200));
        assert_eq!(at(0.5), Rgb::from_rgb(100, 100, 100));
        assert_eq!(at(1.0), Rgb::from_rgb(200, 100, 0));
    }

    #[test]
    fn by_hsl_adds_to_hue_saturation_and_lightness() {
        let raw = r#"<p:animClr clrSpc="hsl" dir="cw"><p:cBhvr><p:attrNameLst><p:attrName>fillcolor</p:attrName></p:attrNameLst></p:cBhvr><p:by><p:hsl h="7200000" s="0" l="0"/></p:by></p:animClr>"#;
        let spec = color_spec(raw).unwrap();
        assert_eq!(spec.change.by_hsl, Some((120.0, 0.0, 0.0)));
        // Red turned 120 degrees is green.
        let red = Rgb::from_rgb(255, 0, 0);
        assert_eq!(ColorChange { q: 1.0, ..spec.change.clone() }.resolve(red, &|_| None), Rgb::from_rgb(0, 255, 0));
        // By -120 degrees going clockwise: red turns the long way (240 degrees) to the blue 120 below it, through green at half way, to the blue at the end.
        let back = ColorChange { by_hsl: Some((-120.0, 0.0, 0.0)), q: 0.5, ..spec.change };
        assert_eq!(back.resolve(red, &|_| None), Rgb::from_rgb(0, 255, 0));
        assert_eq!(ColorChange { q: 1.0, ..back }.resolve(red, &|_| None), Rgb::from_rgb(0, 0, 255));
    }

    #[test]
    fn a_set_of_a_colour_is_read() {
        let raw = r#"<p:set><p:cBhvr><p:cTn id="" dur="500" fill="hold"/><p:attrNameLst><p:attrName>style.color</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:clrVal><a:schemeClr val="bg1"/></p:clrVal></p:to></p:set>"#;
        let spec = color_spec(raw).unwrap();
        assert!(spec.is_set);
        assert_eq!((spec.change.slot, spec.change.to), (ColorSlot::Text, Some(AnimColor::Scheme("bg1".into()))));
        assert_eq!(scheme_slot("bg1"), "lt1");
    }

    #[test]
    fn colours_that_step_are_read_and_resolved() {
        let raw = r#"<p:anim calcmode="discrete" valueType="clr"><p:cBhvr><p:attrNameLst><p:attrName>fillcolor</p:attrName></p:attrNameLst></p:cBhvr><p:tavLst><p:tav tm="0"><p:val><p:clrVal><a:schemeClr val="accent2"/></p:clrVal></p:val></p:tav><p:tav tm="50000"><p:val><p:clrVal><a:schemeClr val="hlink"/></p:clrVal></p:val></p:tav></p:tavLst></p:anim>"#;
        let spec = color_spec(raw).unwrap();
        assert_eq!(spec.change.steps.len(), 2);
        let theme = |n: &str| match n {
            "accent2" => Some(Rgb::from_rgb(1, 2, 3)),
            "hlink" => Some(Rgb::from_rgb(9, 8, 7)),
            _ => None,
        };
        let at = |q: f64| ColorChange { q, ..spec.change.clone() }.resolve(Rgb::BLACK, &theme);
        assert_eq!((at(0.2), at(0.5), at(1.0)), (Rgb::from_rgb(1, 2, 3), Rgb::from_rgb(9, 8, 7), Rgb::from_rgb(9, 8, 7)));
    }

    #[test]
    fn the_colour_of_text_changes_at_the_end_not_along_the_way() {
        let raw = RAW.replace("fillcolor", "style.color");
        let spec = color_spec(&raw).unwrap();
        assert_eq!(spec.change.slot, ColorSlot::Text);
        let theme = |n: &str| (n == "accent2").then_some(Rgb::from_rgb(200, 100, 0));
        let base = Rgb::from_rgb(0, 0, 0);
        let at = |q: f64| ColorChange { q, ..spec.change.clone() }.resolve(base, &theme);
        assert_eq!((at(0.0), at(0.5), at(0.99)), (base, base, base));
        assert_eq!(at(1.0), Rgb::from_rgb(200, 100, 0));
    }

    #[test]
    fn hsl_round_trips() {
        for c in [Rgb::from_rgb(1, 80, 197), Rgb::from_rgb(237, 125, 49), Rgb::from_rgb(10, 10, 10)] {
            let (h, s, l) = to_hsl(c);
            assert_eq!(from_hsl(h, s, l), c);
        }
    }

    #[test]
    fn prst_sys_hsl_scrgb_single_quoted_and_prefixed_colours_are_read() {
        let wrap = |prefix: &str, to: &str| {
            format!(
                "<{prefix}:animClr clrSpc='rgb' dir='cw'><{prefix}:cBhvr><{prefix}:attrNameLst><{prefix}:attrName>fillcolor</{prefix}:attrName></{prefix}:attrNameLst></{prefix}:cBhvr><{prefix}:to>{to}</{prefix}:to></{prefix}:animClr>"
            )
        };
        let resolve_end = |raw: &str| {
            let mut spec = color_spec(raw).expect(raw);
            spec.change.q = 1.0;
            spec.change.resolve(Rgb::BLACK, &|_| None)
        };
        assert_eq!(resolve_end(&wrap("p", r#"<a:prstClr val="red"/>"#)), Rgb::from_rgb(255, 0, 0));
        assert_eq!(resolve_end(&wrap("p", r#"<a:sysClr val="highlight" lastClr="0078D7"/>"#)), Rgb(0x0078D7));
        assert_eq!(resolve_end(&wrap("p", r#"<a:hslClr hue="14400000" sat="100000" lum="50000"/>"#)), Rgb::from_rgb(0, 0, 255));
        assert_eq!(resolve_end(&wrap("p", r#"<a:scrgbClr r="0" g="100000" b="0"/>"#)), Rgb::from_rgb(0, 255, 0));
        assert_eq!(resolve_end(&wrap("p", "<a:srgbClr val='FF0000'/>")), Rgb::from_rgb(255, 0, 0));
        assert_eq!(resolve_end(&wrap("pp", r#"<a:srgbClr val="FF0000"/>"#)), Rgb::from_rgb(255, 0, 0));
    }
}

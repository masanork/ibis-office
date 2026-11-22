// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/235)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Bridge from `hayro-interpret` (`Device` trait) to `ibis-pdf` [`PageScene`].

use std::sync::Arc;

use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::hayro_syntax::Pdf;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, FillRule as HayroFillRule, GlyphDrawMode, Image,
    ImageData, InterpreterCache, InterpreterSettings, LumaData, Paint, PathDrawMode, SoftMask,
    interpret_page,
};
use kurbo::{Affine, BezPath, PathEl, Rect, Shape};

use crate::container::PdfContainer;
use crate::scene::{
    CharBox, DisplayCmd, FillRule, PageInfo, PageScene, PageTextIndex, PathCmd, PdfRect,
    collect_pages,
};
use crate::{Error, Result};

/// Interpret page `page_index` (0-based) into a [`PageScene`] containing both
/// GPUI-ready display commands and a character-level [`PageTextIndex`].
pub fn interpret_page_scene(
    container: &mut PdfContainer,
    page_index: usize,
) -> Result<PageScene> {
    let pages = collect_pages(container)?;
    let info = pages
        .get(page_index)
        .cloned()
        .ok_or_else(|| Error::new(format!("page index {page_index} out of range")))?;

    let pdf = parse_for_interpretation(container)?;
    interpret_scene(&pdf, &InterpreterCache::new(), info, Purpose::View)
}

/// Visit each page for export, keeping glyph outlines only when Unicode is unavailable.
pub fn visit_export_scenes(
    container: &mut PdfContainer,
    mut visit: impl FnMut(PageScene) -> Result<()>,
) -> Result<()> {
    let pages = collect_pages(container)?;
    let pdf = parse_for_interpretation(container)?;
    let cache = InterpreterCache::new();
    for info in pages {
        visit(interpret_scene(&pdf, &cache, info, Purpose::Export)?)?;
    }
    Ok(())
}

/// Visit each page's characters only: nothing is drawn, no image decoded (the plain text export).
/// The scenes have an empty display list.
pub fn visit_text_scenes(
    container: &mut PdfContainer,
    mut visit: impl FnMut(PageScene) -> Result<()>,
) -> Result<()> {
    let pages = collect_pages(container)?;
    let pdf = parse_for_interpretation(container)?;
    let cache = InterpreterCache::new();
    for info in pages {
        visit(interpret_scene(&pdf, &cache, info, Purpose::Text)?)?;
    }
    Ok(())
}

/// What a scene is made for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// The viewer: every glyph drawn, each character's box reaching the next character (for selection).
    View,
    /// The Markdown export: the drawings, glyphs only where they have no Unicode; the boxes as wide as the font says.
    Export,
    /// The text export: the characters only.
    Text,
}

fn parse_for_interpretation(container: &PdfContainer) -> Result<Pdf> {
    Pdf::new(Arc::new(container.raw_bytes().to_vec()))
        .map_err(|error| Error::new(format!("hayro-syntax failed to parse PDF: {error:?}")))
}

fn interpret_scene<'a>(
    pdf: &'a Pdf,
    cache: &'a InterpreterCache<'a>,
    info: PageInfo,
    purpose: Purpose,
) -> Result<PageScene> {
    let page_index = info.index;
    let page = pdf
        .pages()
        .get(page_index)
        .ok_or_else(|| Error::new(format!("page {page_index} missing in hayro-syntax")))?;

    let settings = InterpreterSettings::default();
    let initial_transform = Affine::new(page.initial_transform(true).as_coeffs());
    let (pw, ph) = page.render_dimensions();
    let bbox = Rect::new(0.0, 0.0, pw as f64, ph as f64);

    let mut ctx = Context::new(initial_transform, bbox, cache, pdf.xref(), settings);
    let mut device = SceneDevice::new(info, purpose);
    interpret_page(page, &mut ctx, &mut device);
    Ok(device.finish())
}

struct SceneDevice {
    info: PageInfo,
    display_list: Vec<DisplayCmd>,
    text_index: PageTextIndex,
    op_counter: usize,
    purpose: Purpose,
}

impl SceneDevice {
    fn new(info: PageInfo, purpose: Purpose) -> Self {
        Self {
            info,
            display_list: Vec::new(),
            text_index: PageTextIndex::default(),
            op_counter: 0,
            purpose,
        }
    }

    fn finish(mut self) -> PageScene {
        // The viewer's selection wants each box to reach the next character (ibisOffice). The export reads the gaps
        // between characters as spaces and keeps the widths the font gives: a box widened over a word space hides it.
        if self.purpose == Purpose::View {
            self.text_index.fit_widths();
        }
        PageScene {
            info: self.info,
            display_list: self.display_list,
            text_index: self.text_index,
        }
    }

    fn push_bezpath(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'_>,
        draw_mode: &PathDrawMode,
    ) {
        let transformed = transform * path;
        let bounds = rect_to_pdf_rect(transformed.bounding_box());
        let commands: Vec<PathCmd> = transformed
            .elements()
            .iter()
            .map(|el| match el {
                PathEl::MoveTo(p) => PathCmd::MoveTo(p.x as f32, p.y as f32),
                PathEl::LineTo(p) => PathCmd::LineTo(p.x as f32, p.y as f32),
                PathEl::QuadTo(c, p) => {
                    PathCmd::QuadTo(c.x as f32, c.y as f32, p.x as f32, p.y as f32)
                }
                PathEl::CurveTo(c1, c2, p) => PathCmd::CubicTo(
                    c1.x as f32,
                    c1.y as f32,
                    c2.x as f32,
                    c2.y as f32,
                    p.x as f32,
                    p.y as f32,
                ),
                PathEl::ClosePath => PathCmd::Close,
            })
            .collect();

        if commands.is_empty() {
            return;
        }
        let rgba = paint_rgba(paint);
        let (fill_rule, fill_rgba, stroke_rgba, stroke_width) = match draw_mode {
            PathDrawMode::Fill(rule) => (
                match rule {
                    HayroFillRule::NonZero => FillRule::NonZero,
                    HayroFillRule::EvenOdd => FillRule::EvenOdd,
                },
                Some(rgba),
                None,
                0.0,
            ),
            PathDrawMode::Stroke(props) => (FillRule::NonZero, None, Some(rgba), props.line_width),
        };
        self.display_list.push(DisplayCmd::DrawPath {
            bounds,
            commands,
            fill_rule,
            fill_rgba,
            stroke_rgba,
            stroke_width,
        });
    }
}

fn rect_to_pdf_rect(r: Rect) -> PdfRect {
    PdfRect {
        x0: r.x0 as f32,
        y0: r.y0 as f32,
        x1: r.x1 as f32,
        y1: r.y1 as f32,
    }
}

fn paint_rgba(paint: &Paint<'_>) -> [u8; 4] {
    match paint {
        Paint::Color(c) => c.to_rgba().to_rgba8(),
        Paint::Pattern(_) => [0, 0, 0, 255],
    }
}

impl<'a> Device<'a> for SceneDevice {
    fn set_soft_mask(&mut self, _mask: Option<SoftMask<'a>>) {}

    fn set_blend_mode(&mut self, _blend_mode: BlendMode) {}

    fn draw_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &PathDrawMode,
    ) {
        self.op_counter += 1;
        if self.purpose != Purpose::Text {
            self.push_bezpath(path, transform, paint, draw_mode);
        }
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        if self.purpose == Purpose::Text {
            return;
        }
        let bounds = rect_to_pdf_rect(clip_path.path.bounding_box());
        self.display_list.push(DisplayCmd::PushClip(bounds));
    }

    fn push_transparency_group(
        &mut self,
        opacity: f32,
        _mask: Option<SoftMask<'a>>,
        _blend_mode: BlendMode,
    ) {
        if self.purpose == Purpose::Text {
            return;
        }
        self.display_list
            .push(DisplayCmd::PushTransparencyGroup { opacity });
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        self.op_counter += 1;
        let combined = transform * glyph_transform;
        let coeffs = combined.as_coeffs();
        let origin = (coeffs[4] as f32, coeffs[5] as f32);
        let font_size = (coeffs[0].hypot(coeffs[1]) * 1000.0).max(1.0) as f32;

        // Always index text (including invisible OCR text layers) for search and selection.
        if let Some(unicode) = glyph.as_unicode() {
            let text = match unicode {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            };
            // The glyph's advance where the font gives it (in thousandths of the size), else what the character
            // usually takes: a full-width character one em, any other 0.55.
            let advance = match glyph {
                Glyph::Outline(outline) => outline.advance_width().filter(|w| *w > 0.0 && *w < 4000.0).map(|w| w / 1000.0),
                Glyph::Type3(_) => None,
            }
            .unwrap_or_else(|| text.chars().map(crate::scene::estimated_advance_em).sum());
            let rect_pt = PdfRect {
                x0: origin.0,
                y0: origin.1 - font_size,
                x1: origin.0 + font_size * advance,
                y1: origin.1,
            };
            for ch in text.chars() {
                self.text_index.chars.push(CharBox {
                    ch,
                    rect_pt,
                    vertical: false,
                    stream_op_index: self.op_counter,
                });
            }
            if self.purpose == Purpose::Export && !text.is_empty() && !text.contains('\u{fffd}') {
                return;
            }
        }
        if self.purpose == Purpose::Text {
            return;
        }

        // Emit vector outlines / Type3 instructions for GPUI rendering unless invisible.
        let path_mode = match draw_mode {
            GlyphDrawMode::Fill => Some(PathDrawMode::Fill(HayroFillRule::NonZero)),
            GlyphDrawMode::Stroke(props) => Some(PathDrawMode::Stroke(props.clone())),
            GlyphDrawMode::Invisible => None,
        };
        if let Some(mode) = path_mode {
            match glyph {
                Glyph::Outline(outline_glyph) => {
                    let outline = outline_glyph.outline();
                    self.push_bezpath(&outline, combined, paint, &mode);
                }
                Glyph::Type3(t3) => {
                    t3.interpret(self, transform, glyph_transform, paint);
                }
            }
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
        self.op_counter += 1;
        if self.purpose == Purpose::Text {
            return;
        }
        match image {
            Image::Stencil(s) => {
                let unit = Rect::new(0.0, 0.0, s.width() as f64, s.height() as f64);
                let bounds = rect_to_pdf_rect(transform.transform_rect_bbox(unit));
                self.display_list.push(DisplayCmd::DrawImage {
                    bounds,
                    width: s.width(),
                    height: s.height(),
                    rgba: Vec::new(),
                });
            }
            Image::Raster(r) => {
                let mut decoded_out = None;
                r.with_rgba(
                    |img, alpha| {
                        decoded_out = Some(image_to_rgba(img, alpha));
                    },
                    None,
                );
                if let Some((width, height, rgba)) = decoded_out {
                    let unit = Rect::new(0.0, 0.0, width as f64, height as f64);
                    let bounds = rect_to_pdf_rect(transform.transform_rect_bbox(unit));
                    self.display_list.push(DisplayCmd::DrawImage {
                        bounds,
                        width,
                        height,
                        rgba,
                    });
                }
            }
        }
    }

    fn pop_clip_path(&mut self) {
        if self.purpose != Purpose::Text {
            self.display_list.push(DisplayCmd::PopClip);
        }
    }

    fn pop_transparency_group(&mut self) {
        if self.purpose != Purpose::Text {
            self.display_list.push(DisplayCmd::PopTransparencyGroup);
        }
    }
}

fn image_to_rgba(img: ImageData, alpha: Option<LumaData>) -> (u32, u32, Vec<u8>) {
    match img {
        ImageData::Rgb(rgb) => {
            let count = (rgb.width as usize) * (rgb.height as usize);
            let mut rgba = Vec::with_capacity(count * 4);
            for i in 0..count {
                let base = i * 3;
                let a = alpha
                    .as_ref()
                    .and_then(|al| al.data.get(i).copied())
                    .unwrap_or(255);
                rgba.extend_from_slice(&[
                    rgb.data[base],
                    rgb.data[base + 1],
                    rgb.data[base + 2],
                    a,
                ]);
            }
            (rgb.width, rgb.height, rgba)
        }
        ImageData::Luma(luma) => {
            let count = (luma.width as usize) * (luma.height as usize);
            let mut rgba = Vec::with_capacity(count * 4);
            for i in 0..count {
                let v = luma.data[i];
                let a = alpha
                    .as_ref()
                    .and_then(|al| al.data.get(i).copied())
                    .unwrap_or(255);
                rgba.extend_from_slice(&[v, v, v, a]);
            }
            (luma.width, luma.height, rgba)
        }
    }
}

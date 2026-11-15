// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Bridge from `hayro-interpret` (`Device` trait) to `ibis-pdf` [`PageScene`].

use std::sync::Arc;

use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::hayro_syntax::Pdf;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use kurbo::{Affine, BezPath, Rect, Shape};

use crate::container::PdfContainer;
use crate::scene::{
    CharBox, DisplayCmd, PageInfo, PageScene, PageTextIndex, PdfRect, collect_pages,
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

    let raw_arc = Arc::new(container.raw_bytes().to_vec());
    let pdf = Pdf::new(raw_arc)
        .map_err(|e| Error::new(format!("hayro-syntax failed to parse PDF: {e:?}")))?;
    let page = pdf
        .pages()
        .get(page_index)
        .ok_or_else(|| Error::new(format!("page {page_index} missing in hayro-syntax")))?;

    let settings = InterpreterSettings::default();
    let cache = InterpreterCache::new();
    let initial_transform = Affine::new(page.initial_transform(true).as_coeffs());
    let (pw, ph) = page.render_dimensions();
    let bbox = Rect::new(0.0, 0.0, pw as f64, ph as f64);

    let mut ctx = Context::new(initial_transform, bbox, &cache, pdf.xref(), settings);
    let mut device = SceneDevice::new(info);
    interpret_page(page, &mut ctx, &mut device);
    Ok(device.finish())
}

struct SceneDevice {
    info: PageInfo,
    display_list: Vec<DisplayCmd>,
    text_index: PageTextIndex,
    op_counter: usize,
}

impl SceneDevice {
    fn new(info: PageInfo) -> Self {
        Self {
            info,
            display_list: Vec::new(),
            text_index: PageTextIndex::default(),
            op_counter: 0,
        }
    }

    fn finish(self) -> PageScene {
        PageScene {
            info: self.info,
            display_list: self.display_list,
            text_index: self.text_index,
        }
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
        let transformed = transform * path;
        let bounds = rect_to_pdf_rect(transformed.bounding_box());
        let rgba = paint_rgba(paint);
        let (fill_rgba, stroke_rgba, stroke_width) = match draw_mode {
            PathDrawMode::Fill(_) => (Some(rgba), None, 0.0),
            PathDrawMode::Stroke(props) => (None, Some(rgba), props.line_width),
        };
        self.display_list.push(DisplayCmd::DrawPath {
            bounds,
            fill_rgba,
            stroke_rgba,
            stroke_width,
            segment_count: path.elements().len(),
        });
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        let bounds = rect_to_pdf_rect(clip_path.path.bounding_box());
        self.display_list.push(DisplayCmd::PushClip(bounds));
    }

    fn push_transparency_group(
        &mut self,
        opacity: f32,
        _mask: Option<SoftMask<'a>>,
        _blend_mode: BlendMode,
    ) {
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
        let font_size = coeffs[0].hypot(coeffs[1]).max(1.0) as f32;
        let rgba = match draw_mode {
            GlyphDrawMode::Invisible => [0, 0, 0, 0],
            _ => paint_rgba(paint),
        };

        if let Some(unicode) = glyph.as_unicode() {
            let text = match unicode {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            };
            let rect_pt = PdfRect {
                x0: origin.0,
                y0: origin.1 - font_size,
                x1: origin.0 + font_size * 0.6,
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
            self.display_list.push(DisplayCmd::DrawGlyphRun {
                text,
                origin,
                font_size,
                rgba,
            });
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
        self.op_counter += 1;
        let (width, height) = match &image {
            Image::Stencil(s) => (s.width(), s.height()),
            Image::Raster(r) => (r.width(), r.height()),
        };
        let unit = Rect::new(0.0, 0.0, width as f64, height as f64);
        let bounds = rect_to_pdf_rect(transform.transform_rect_bbox(unit));
        self.display_list.push(DisplayCmd::DrawImage {
            bounds,
            width,
            height,
        });
    }

    fn pop_clip_path(&mut self) {
        self.display_list.push(DisplayCmd::PopClip);
    }

    fn pop_transparency_group(&mut self) {
        self.display_list.push(DisplayCmd::PopTransparencyGroup);
    }
}

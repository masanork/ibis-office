// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Bridge between [`ibis_ocr::NdlOcrEngine`] and [`ibis_pdf::scene::PageScene`].
//!
//! Runs OCR on raster `DrawImage` commands in pages that lack extractable text
//! (e.g. scanned PDFs, OFDs, XPS, or XDW pages) and populates [`PageTextIndex`]
//! so selection, search, copy, and Markdown export work seamlessly.

use ibis_ocr::engine::NdlOcrEngine;
use ibis_ocr::{OcrLine, OcrPage};
use ibis_pdf::scene::{CharBox, DisplayCmd, PageScene, PageTextIndex, PdfRect};
use image::{DynamicImage, RgbaImage};

/// Recognize text from all raster images in `scene` and return a [`PageTextIndex`].
///
/// Pages that already have extractable text are returned unchanged unless `force` is `true`.
pub fn ocr_page_scene(
    engine: &mut NdlOcrEngine,
    scene: &PageScene,
    force: bool,
) -> ibis_ocr::Result<PageTextIndex> {
    if !force && !scene.text_index.chars.is_empty() {
        return Ok(scene.text_index.clone());
    }

    let mut out = PageTextIndex::default();
    for (op_idx, cmd) in scene.display_list.iter().enumerate() {
        let DisplayCmd::DrawImage {
            bounds,
            width,
            height,
            rgba,
        } = cmd
        else {
            continue;
        };
        if *width == 0 || *height == 0 {
            continue;
        }
        let Some(img) = RgbaImage::from_raw(*width, *height, rgba.clone()) else {
            continue;
        };
        let dyn_img = DynamicImage::ImageRgba8(img);
        let ocr_page = engine.recognize_image(&dyn_img)?;
        append_ocr_page_to_index(&ocr_page, *bounds, op_idx, &mut out);
    }

    Ok(out)
}

/// Map an [`OcrPage`] (pixel coordinates) into `bounds` (PDF points) and append [`CharBox`]es.
pub fn append_ocr_page_to_index(
    ocr_page: &OcrPage,
    bounds: PdfRect,
    stream_op_index: usize,
    out: &mut PageTextIndex,
) {
    if ocr_page.width == 0 || ocr_page.height == 0 {
        return;
    }
    let sx = bounds.width() / ocr_page.width as f32;
    let sy = bounds.height() / ocr_page.height as f32;
    let origin_x = bounds.x0.min(bounds.x1);
    let origin_y = bounds.y0.min(bounds.y1);

    for line in &ocr_page.lines {
        append_ocr_line_to_index(line, origin_x, origin_y, sx, sy, stream_op_index, out);
    }
}

fn append_ocr_line_to_index(
    line: &OcrLine,
    origin_x: f32,
    origin_y: f32,
    sx: f32,
    sy: f32,
    stream_op_index: usize,
    out: &mut PageTextIndex,
) {
    let chars: Vec<char> = line.text.chars().collect();
    if chars.is_empty() {
        return;
    }
    let [px0, py0, px1, py1] = line.box_xyxy;
    let x0 = origin_x + px0 as f32 * sx;
    let y0 = origin_y + py0 as f32 * sy;
    let x1 = origin_x + px1 as f32 * sx;
    let y1 = origin_y + py1 as f32 * sy;
    let n = chars.len() as f32;

    for (i, ch) in chars.into_iter().enumerate() {
        let t0 = i as f32 / n;
        let t1 = (i + 1) as f32 / n;
        let rect_pt = if line.is_vertical {
            let cy0 = y0 + (y1 - y0) * t0;
            let cy1 = y0 + (y1 - y0) * t1;
            PdfRect { x0, y0: cy0, x1, y1: cy1 }
        } else {
            let cx0 = x0 + (x1 - x0) * t0;
            let cx1 = x0 + (x1 - x0) * t1;
            PdfRect { x0: cx0, y0, x1: cx1, y1 }
        };
        out.chars.push(CharBox {
            ch,
            rect_pt,
            vertical: line.is_vertical,
            stream_op_index,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_ocr_page_maps_horizontal_and_vertical_chars() {
        let page = OcrPage {
            width: 1000,
            height: 1000,
            lines: vec![
                OcrLine {
                    box_xyxy: [100, 100, 500, 150],
                    class_index: 1,
                    class_name: "line_main",
                    confidence: 0.9,
                    pred_char_count: 2.0,
                    is_vertical: false,
                    text: "日本".to_string(),
                    order: 0,
                },
                OcrLine {
                    box_xyxy: [800, 100, 850, 500],
                    class_index: 1,
                    class_name: "line_main",
                    confidence: 0.9,
                    pred_char_count: 2.0,
                    is_vertical: true,
                    text: "縦書".to_string(),
                    order: 1,
                },
            ],
        };
        let mut idx = PageTextIndex::default();
        append_ocr_page_to_index(
            &page,
            PdfRect {
                x0: 0.0,
                y0: 0.0,
                x1: 500.0,
                y1: 500.0,
            },
            0,
            &mut idx,
        );
        assert_eq!(idx.chars.len(), 4);
        assert_eq!(idx.chars[0].ch, '日');
        assert!(!idx.chars[0].vertical);
        assert_eq!(idx.chars[2].ch, '縦');
        assert!(idx.chars[2].vertical);
    }
}

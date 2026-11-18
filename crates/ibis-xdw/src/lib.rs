// SPDX-License-Identifier: AGPL-3.0-only
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Read-only DocuWorks page adapter for the fixed-layout viewer.
//!
//! The preview preserves horizontal glyph positions, rectangular clips and
//! quarter-turn images. Other clip shapes use their bounding rectangles.

use ezpzxdw_core::{
    doc::Document,
    gfx::{ImageData, Item, Path, Seg},
};
use ibis_pdf::{
    DisplayCmd, Error, ObjectId, PageInfo, PageScene, PdfRect, Result,
    scene::{FillRule, PathCmd},
};

const PT_PER_UNIT: f32 = 72.0 / 2540.0;
const PAGE_IMAGE_BUDGET: usize = 64 << 20;

pub struct XdwContainer(Document);

/// Recognize the common DocuWorks header, including generations 7 and 10.
pub fn is_xdw(bytes: &[u8]) -> bool {
    bytes.get(..5).is_some_and(|header| {
        header[..4] == [0x60, 0x0e, 0x82, 0x01] && matches!(header[4], 7 | 10)
    }) && bytes.get(10..16) == Some(&[0x83, 0x04, 0x01, 0x0d, 0x0a, 0x01])
}

impl XdwContainer {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if !is_xdw(bytes) {
            return Err(Error::new("Unrecognized DocuWorks header"));
        }
        let limits = ezpzxdw_core::limits::Limits::default();
        if bytes.len() > limits.max_bytes {
            return Err(Error::new(
                "DocuWorks document exceeds the input size limit",
            ));
        }
        let document = Document::open_with(bytes.to_vec(), &limits)
            .map_err(|err| Error::new(err.to_string()))?;
        if document.is_binder() {
            return Err(Error::new("DocuWorks binders (.xbd) are unsupported"));
        }
        if document.pages.is_empty() || document.pages.iter().any(|p| p.w <= 0 || p.h <= 0) {
            return Err(Error::new("Invalid DocuWorks page dimensions"));
        }
        Ok(Self(document))
    }

    pub fn page_infos(&self) -> Vec<PageInfo> {
        self.0
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| page_info(index, page.w, page.h))
            .collect()
    }

    pub fn interpret_page_scene(&self, index: usize) -> Result<PageScene> {
        let page = self
            .0
            .pages
            .get(index)
            .ok_or_else(|| Error::new("DocuWorks page out of range"))?;
        let display = self
            .0
            .render(index)
            .map_err(|err| Error::new(err.to_string()))?;
        let mut display_list = Vec::with_capacity(display.items.len());
        let mut image_budget = PAGE_IMAGE_BUDGET;
        let clips: Vec<_> = display
            .clips
            .iter()
            .map(|clip| clip_rect(&clip.0))
            .collect();
        let mut current_clip = None;
        for item in display.items {
            let clip = match &item {
                Item::Fill { clip, .. }
                | Item::Stroke { clip, .. }
                | Item::Text { clip, .. }
                | Item::Image { clip, .. } => *clip,
            };
            let next_clip = clips.get(clip as usize).copied().flatten();
            if next_clip != current_clip {
                display_list.push(DisplayCmd::SetClipRect(next_clip));
                current_clip = next_clip;
            }
            let command = match item {
                Item::Fill {
                    path,
                    color,
                    evenodd,
                    mul,
                    ..
                } => path_command(
                    path,
                    // The GPUI viewer uses source-over. Keep text visible under marker ink.
                    Some(if mul {
                        let [r, g, b, _] = rgba(color);
                        [r, g, b, 96]
                    } else {
                        rgba(color)
                    }),
                    None,
                    0.0,
                    if evenodd {
                        FillRule::EvenOdd
                    } else {
                        FillRule::NonZero
                    },
                ),
                Item::Stroke {
                    path, color, width, ..
                } => path_command(path, None, Some(rgba(color)), width, FillRule::NonZero),
                Item::Text {
                    x,
                    y,
                    angle,
                    size,
                    sx,
                    face,
                    weight,
                    italic,
                    vertical,
                    color,
                    text,
                    xs,
                    ..
                } => {
                    let sx = if sx == 0.0 { 1.0 } else { sx };
                    let angle = -angle.to_radians();
                    let (sin, cos) = angle.sin_cos();
                    let glyph = |text, dx, max_width| DisplayCmd::DrawStyledGlyphRun {
                        text,
                        origin: ((x + dx * cos) * PT_PER_UNIT, (y + dx * sin) * PT_PER_UNIT),
                        font_size: size * PT_PER_UNIT,
                        family: face.clone(),
                        weight,
                        italic,
                        angle,
                        horizontal_scale: sx,
                        max_width,
                        rgba: rgba(color),
                    };
                    if !vertical && !xs.is_empty() && xs.len() == text.chars().count() {
                        for ((index, ch), &dx) in text.chars().enumerate().zip(&xs) {
                            if ch.is_whitespace() {
                                continue;
                            }
                            // Offsets are already in page units. sx scales glyph outlines.
                            let advance = xs.get(index + 1).map(|next| (next - dx) * PT_PER_UNIT);
                            let remaining = next_clip
                                .filter(|_| angle == 0.0)
                                .map(|clip| clip.x1 - (x + dx) * PT_PER_UNIT);
                            let max_width = match (advance, remaining) {
                                (Some(a), Some(b)) if b > 0.0 => Some(a.min(b)),
                                (Some(a), _) => Some(a),
                                (None, remaining) => remaining,
                            }
                            .filter(|width| *width > 0.0);
                            display_list.push(glyph(ch.to_string(), dx, max_width));
                        }
                        None
                    } else {
                        Some(glyph(text, 0.0, None))
                    }
                }
                Item::Image {
                    image,
                    m: [a, b, c, d, e, f],
                    alpha,
                    ..
                } => {
                    let image = display
                        .images
                        .get(image as usize)
                        .ok_or_else(|| Error::new("Missing DocuWorks page image"))?;
                    let (width, height, mut pixels) = match &image.data {
                        ImageData::Rgba(data) => {
                            charge_image(image.w, image.h, &mut image_budget)?;
                            (image.w, image.h, data.clone())
                        }
                        ImageData::Jpeg(data) => {
                            let reader = || {
                                image::ImageReader::with_format(
                                    std::io::Cursor::new(data),
                                    image::ImageFormat::Jpeg,
                                )
                            };
                            let (width, height) = reader()
                                .into_dimensions()
                                .map_err(|err| Error::new(err.to_string()))?;
                            charge_image(width, height, &mut image_budget)?;
                            let decoded = reader()
                                .decode()
                                .map_err(|err| Error::new(err.to_string()))?
                                .into_rgba8();
                            (width, height, decoded.into_raw())
                        }
                    };
                    if alpha < 1.0 {
                        for pixel in pixels.chunks_exact_mut(4) {
                            pixel[3] = (f32::from(pixel[3]) * alpha).round() as u8;
                        }
                    }
                    let (width, height, pixels) =
                        orient_image(width, height, pixels, [a, b, c, d])?;
                    Some(DisplayCmd::DrawImage {
                        bounds: rect([
                            e + a.min(0.0) + c.min(0.0),
                            f + b.min(0.0) + d.min(0.0),
                            e + a.max(0.0) + c.max(0.0),
                            f + b.max(0.0) + d.max(0.0),
                        ]),
                        width,
                        height,
                        rgba: pixels,
                    })
                }
            };
            display_list.extend(command);
        }
        Ok(PageScene {
            info: page_info(index, page.w, page.h),
            display_list,
            text_index: Default::default(),
        })
    }
}

// Straight and quarter-turn transforms can be expressed as a rectangular
// image placement once the pixel rows and columns are oriented.
fn orient_image(
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    [a, b, c, d]: [f32; 4],
) -> Result<(u32, u32, Vec<u8>)> {
    let quarter_turn = a == 0.0 && d == 0.0 && b != 0.0 && c != 0.0;
    let axis_aligned = b == 0.0 && c == 0.0;
    let (flip_x, flip_y) = if quarter_turn {
        (c > 0.0, b < 0.0)
    } else if axis_aligned {
        (a < 0.0, d < 0.0)
    } else {
        (false, false)
    };
    if !quarter_turn && !flip_x && !flip_y {
        return Ok((width, height, pixels));
    }
    let source = image::RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| Error::new("Invalid DocuWorks image dimensions"))?;
    let mut result = if quarter_turn {
        image::imageops::rotate90(&source)
    } else {
        source
    };
    if flip_x {
        image::imageops::flip_horizontal_in_place(&mut result);
    }
    if flip_y {
        image::imageops::flip_vertical_in_place(&mut result);
    }
    Ok((result.width(), result.height(), result.into_raw()))
}

fn clip_rect(paths: &[Path]) -> Option<PdfRect> {
    paths
        .iter()
        .map(|path| {
            // Empty paths clip everything. GPUI supports rectangular content masks.
            rect(path.bounds().unwrap_or([0.0; 4]))
        })
        .reduce(|a, b| {
            let x0 = a.x0.max(b.x0);
            let y0 = a.y0.max(b.y0);
            PdfRect {
                x0,
                y0,
                x1: a.x1.min(b.x1).max(x0),
                y1: a.y1.min(b.y1).max(y0),
            }
        })
}

fn charge_image(width: u32, height: u32, budget: &mut usize) -> Result<()> {
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    *budget = bytes
        .and_then(|bytes| budget.checked_sub(bytes))
        .ok_or_else(|| Error::new("DocuWorks page exceeds the decoded image limit"))?;
    Ok(())
}

fn page_info(index: usize, width: i64, height: i64) -> PageInfo {
    let bounds = rect([0.0, 0.0, width as f32, height as f32]);
    PageInfo {
        index,
        object_id: ObjectId::new(index as u32 + 1, 0),
        media_box: bounds,
        crop_box: bounds,
        rotate: 0,
    }
}

fn rect([x0, y0, x1, y1]: [f32; 4]) -> PdfRect {
    PdfRect {
        x0: x0 * PT_PER_UNIT,
        y0: y0 * PT_PER_UNIT,
        x1: x1 * PT_PER_UNIT,
        y1: y1 * PT_PER_UNIT,
    }
}

fn rgba(color: u32) -> [u8; 4] {
    [(color >> 16) as u8, (color >> 8) as u8, color as u8, 255]
}

fn path_command(
    path: Path,
    fill_rgba: Option<[u8; 4]>,
    stroke_rgba: Option<[u8; 4]>,
    width: f32,
    fill_rule: FillRule,
) -> Option<DisplayCmd> {
    let bounds = rect(path.bounds()?);
    let pt = |value| value * PT_PER_UNIT;
    let commands = path
        .0
        .into_iter()
        .map(|segment| match segment {
            Seg::M(x, y) => PathCmd::MoveTo(pt(x), pt(y)),
            Seg::L(x, y) => PathCmd::LineTo(pt(x), pt(y)),
            Seg::C(a, b, c, d, x, y) => PathCmd::CubicTo(pt(a), pt(b), pt(c), pt(d), pt(x), pt(y)),
            Seg::Z => PathCmd::Close,
        })
        .collect();
    Some(DisplayCmd::DrawPath {
        bounds,
        commands,
        fill_rule,
        fill_rgba,
        stroke_rgba,
        stroke_width: pt(width),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ezpzxdw_core::edit::Shape;

    #[test]
    fn orients_quarter_turns_and_mirrors() {
        let pixels: Vec<_> = (1..=6).flat_map(|red| [red, 0, 0, 255]).collect();
        for (matrix, dimensions, expected) in [
            ([0.0, 1.0, -1.0, 0.0], (3, 2), [5, 3, 1, 6, 4, 2]),
            ([0.0, -1.0, 1.0, 0.0], (3, 2), [2, 4, 6, 1, 3, 5]),
            ([-1.0, 0.0, 0.0, 1.0], (2, 3), [2, 1, 4, 3, 6, 5]),
        ] {
            let (w, h, data) = orient_image(2, 3, pixels.clone(), matrix).unwrap();
            assert_eq!((w, h), dimensions);
            assert_eq!(
                data.chunks_exact(4)
                    .map(|pixel| pixel[0])
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn intersects_nested_clips_and_preserves_empty_intersections() {
        assert_eq!(clip_rect(&[]), None);
        let page = Path::rect(0.0, 0.0, 21_000.0, 29_700.0);
        let cell = Path::rect(200.0, 300.0, 500.0, 600.0);
        assert_eq!(
            clip_rect(&[page, cell.clone()]),
            Some(rect([200.0, 300.0, 500.0, 600.0]))
        );
        let separate = Path::rect(700.0, 300.0, 800.0, 600.0);
        assert_eq!(clip_rect(&[cell, separate]).unwrap().width(), 0.0);
    }

    #[test]
    fn opens_and_interprets_synthetic_docuworks() {
        let mut document = Document::blank(21_000.0, 29_700.0).unwrap();
        document
            .add_annotation(
                0,
                1000.0,
                1000.0,
                4000.0,
                1000.0,
                &Shape::Text {
                    text: "閲覧テスト".into(),
                    size: 12.0,
                    color: 0x113355,
                    bold: false,
                    background: None,
                    frame: None,
                },
            )
            .unwrap();
        document
            .add_annotation(
                0,
                2000.0,
                3000.0,
                1000.0,
                500.0,
                &Shape::Rect {
                    stroke: Some(0x112233),
                    width: 1.0,
                    fill: Some(0x445566),
                    highlight: false,
                },
            )
            .unwrap();
        document
            .add_annotation(
                0,
                1000.0,
                1000.0,
                2000.0,
                500.0,
                &Shape::Rect {
                    stroke: None,
                    width: 1.0,
                    fill: Some(0xffff00),
                    highlight: true,
                },
            )
            .unwrap();
        let bytes = document.save().unwrap();
        assert!(is_xdw(&bytes));
        let xdw = XdwContainer::from_bytes(&bytes).unwrap();
        let pages = xdw.page_infos();
        assert_eq!(pages.len(), 1);
        assert!((pages[0].crop_box.width() - 595.28).abs() < 0.02);
        assert!((pages[0].crop_box.height() - 841.89).abs() < 0.02);
        let scene = xdw.interpret_page_scene(0).unwrap();
        assert_eq!(scene.info, pages[0]);
        let display = xdw.0.render(0).unwrap();
        let (x, y, offsets) = display
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text { text, x, y, xs, .. } if text == "閲覧テスト" => {
                    Some((*x, *y, xs))
                }
                _ => None,
            })
            .unwrap();
        let glyphs: Vec<_> = scene
            .display_list
            .iter()
            .filter_map(|cmd| match cmd {
                DisplayCmd::DrawStyledGlyphRun { text, origin, .. } => {
                    Some((text.as_str(), *origin))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            glyphs.iter().map(|(ch, _)| *ch).collect::<String>(),
            "閲覧テスト"
        );
        assert_eq!(glyphs.len(), offsets.len());
        for ((_, (gx, gy)), offset) in glyphs.iter().zip(offsets) {
            assert_eq!(*gx, (x + offset) * PT_PER_UNIT);
            assert_eq!(*gy, y * PT_PER_UNIT);
        }
        assert!(scene.display_list.iter().any(|cmd| matches!(
            cmd,
            DisplayCmd::DrawPath {
                fill_rgba: Some([0x44, 0x55, 0x66, 255]),
                ..
            }
        )));
        assert!(scene.display_list.iter().any(|cmd| matches!(
            cmd,
            DisplayCmd::DrawPath {
                stroke_rgba: Some([0x11, 0x22, 0x33, 255]),
                ..
            }
        )));
        assert!(scene.display_list.iter().any(|cmd| matches!(
            cmd,
            DisplayCmd::DrawPath {
                fill_rgba: Some([255, 255, 0, 96]),
                ..
            }
        )));
        assert!(xdw.interpret_page_scene(1).is_err());
        assert!(XdwContainer::from_bytes(&bytes[..16]).is_err());
    }

    #[test]
    fn decodes_a_synthetic_scan_page() {
        use ezpzxdw_core::pages::Jpeg;
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&[200; 6], 2, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        let mut document = Document::blank(21_000.0, 29_700.0).unwrap();
        document
            .insert_image_page(
                1,
                21_000.0,
                29_700.0,
                &Jpeg {
                    data: &jpeg,
                    w: 2,
                    h: 1,
                },
                None,
                None,
            )
            .unwrap();
        document.rotate_page(1, 1).unwrap();
        let xdw = XdwContainer::from_bytes(&document.save().unwrap()).unwrap();
        assert_eq!(xdw.page_infos().len(), 2);
        let scene = xdw.interpret_page_scene(1).unwrap();
        assert!(scene.display_list.iter().any(|cmd| matches!(
            cmd, DisplayCmd::DrawImage { width: 1, height: 2, rgba, .. } if rgba.len() == 8
        )));
    }
}

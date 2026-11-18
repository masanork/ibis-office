// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Viewer/editor intermediate representation (Layer 2 bridge) and page tree resolution.

use crate::container::PdfContainer;
use crate::cos::{CosDict, CosObj, ObjectId};
use crate::{Error, Result};

/// Axis-aligned rectangle in PDF user-space points (1/72 inch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl PdfRect {
    pub fn width(self) -> f32 {
        (self.x1 - self.x0).abs()
    }

    pub fn height(self) -> f32 {
        (self.y1 - self.y0).abs()
    }
}

/// Resolved metadata for a single page in the `/Pages` tree.
#[derive(Clone, Debug, PartialEq)]
pub struct PageInfo {
    /// 0-based page index.
    pub index: usize,
    pub object_id: ObjectId,
    pub media_box: PdfRect,
    pub crop_box: PdfRect,
    pub rotate: u16,
}

/// Path segment in PDF user/device space points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// Drawing commands emitted by a page interpreter for GPUI rendering.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayCmd {
    /// Replace the current device-space rectangular clip. None clears it.
    SetClipRect(Option<PdfRect>),
    PushClip(PdfRect),
    PopClip,
    PushTransparencyGroup {
        opacity: f32,
    },
    PopTransparencyGroup,
    DrawRect {
        rect: PdfRect,
        fill_rgba: Option<[u8; 4]>,
    },
    DrawPath {
        bounds: PdfRect,
        commands: Vec<PathCmd>,
        fill_rule: FillRule,
        fill_rgba: Option<[u8; 4]>,
        stroke_rgba: Option<[u8; 4]>,
        stroke_width: f32,
    },
    DrawGlyphRun {
        text: String,
        origin: (f32, f32),
        font_size: f32,
        rgba: [u8; 4],
    },
    DrawStyledGlyphRun {
        text: String,
        origin: (f32, f32),
        font_size: f32,
        family: String,
        weight: u16,
        italic: bool,
        angle: f32,
        horizontal_scale: f32,
        max_width: Option<f32>,
        rgba: [u8; 4],
    },
    DrawImage {
        bounds: PdfRect,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

/// Character bounding box for text selection, search, and highlight annotation creation.
#[derive(Clone, Debug, PartialEq)]
pub struct CharBox {
    pub ch: char,
    pub rect_pt: PdfRect,
    pub vertical: bool,
    pub stream_op_index: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageTextIndex {
    pub chars: Vec<CharBox>,
}

impl PageTextIndex {
    pub fn plain_text(&self) -> String {
        self.chars.iter().map(|c| c.ch).collect()
    }
}

/// Combined rendering + text-index output for one PDF page.
#[derive(Clone, Debug, PartialEq)]
pub struct PageScene {
    pub info: PageInfo,
    pub display_list: Vec<DisplayCmd>,
    pub text_index: PageTextIndex,
}

/// Collect all pages in order from `/Root -> /Pages`, inheriting `MediaBox`/`CropBox`/`Rotate`.
pub fn collect_pages(container: &mut PdfContainer) -> Result<Vec<PageInfo>> {
    let (_, catalog) = container.catalog()?;
    let pages_id = catalog
        .get_ref(b"Pages")
        .ok_or_else(|| Error::new("catalog missing /Pages root"))?;
    let mut out = Vec::new();
    collect_page_node(container, pages_id, None, None, 0, &mut out)?;
    Ok(out)
}

fn collect_page_node(
    container: &mut PdfContainer,
    node_id: ObjectId,
    inherited_media: Option<PdfRect>,
    inherited_crop: Option<PdfRect>,
    inherited_rotate: u16,
    out: &mut Vec<PageInfo>,
) -> Result<()> {
    let node = container.resolve(node_id)?;
    let Some(dict) = node.as_dict().cloned() else {
        return Ok(());
    };

    let media = parse_rect(&dict, b"MediaBox").or(inherited_media);
    let crop = parse_rect(&dict, b"CropBox").or(inherited_crop);
    let rotate = dict
        .get_int(b"Rotate")
        .map(|r| (r.rem_euclid(360)) as u16)
        .unwrap_or(inherited_rotate);

    match dict.get_name(b"Type") {
        Some(b"Pages") => {
            if let Some(kids) = dict.get_array(b"Kids").map(|k| k.to_vec()) {
                for kid in kids {
                    if let Some(kid_id) = kid.as_ref() {
                        collect_page_node(container, kid_id, media, crop, rotate, out)?;
                    }
                }
            }
        }
        Some(b"Page") => {
            let media_box = media.unwrap_or(PdfRect {
                x0: 0.0,
                y0: 0.0,
                x1: 595.0,
                y1: 842.0,
            });
            let crop_box = crop.unwrap_or(media_box);
            let index = out.len();
            out.push(PageInfo {
                index,
                object_id: node_id,
                media_box,
                crop_box,
                rotate,
            });
        }
        _ => {}
    }
    Ok(())
}

fn parse_rect(dict: &CosDict, key: &[u8]) -> Option<PdfRect> {
    let arr = dict.get_array(key)?;
    if arr.len() != 4 {
        return None;
    }
    let nums: Option<Vec<f64>> = arr.iter().map(CosObj::as_f64).collect();
    let nums = nums?;
    Some(PdfRect {
        x0: nums[0] as f32,
        y0: nums[1] as f32,
        x1: nums[2] as f32,
        y1: nums[3] as f32,
    })
}

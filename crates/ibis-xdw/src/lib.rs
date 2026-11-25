// SPDX-License-Identifier: AGPL-3.0-only
//! Read-only DocuWorks page adapter for the fixed-layout viewer.

use std::sync::Arc;

use ibis_pdf::{CharBox, DisplayCmd, Error, ObjectId, PageInfo, PageScene, PdfRect, Result};
pub use ibis_xdw_read::BinderDocument;
use ibis_xdw_read::Document;

const POINTS_PER_UNIT: f32 = 72.0 / 2540.0;

pub struct XdwContainer {
    document: Document<Arc<[u8]>>,
    pages: Vec<PageInfo>,
    binder_documents: Option<Vec<BinderDocument>>,
}

/// Recognize the common DocuWorks header, including generations 7 and 10.
pub fn is_xdw(bytes: &[u8]) -> bool {
    bytes.get(..5).is_some_and(|header| {
        header[..4] == [0x60, 0x0e, 0x82, 0x01] && matches!(header[4], 7 | 10)
    }) && bytes.get(10..16) == Some(&[0x83, 0x04, 0x01, 0x0d, 0x0a, 0x01])
}

impl XdwContainer {
    pub fn from_bytes(bytes: Arc<[u8]>) -> Result<Self> {
        let document = Document::open(bytes).map_err(map_error)?;
        let ibis_xdw_read::Content {
            pages,
            binder_documents,
        } = document.content().map_err(map_error)?;
        let pages: Vec<_> = pages
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                let bounds = PdfRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: page.width as f32 * POINTS_PER_UNIT,
                    y1: page.height as f32 * POINTS_PER_UNIT,
                };
                PageInfo {
                    index,
                    object_id: ObjectId::new(index as u32 + 1, 0),
                    media_box: bounds,
                    crop_box: bounds,
                    rotate: 0,
                }
            })
            .collect();
        if pages.is_empty() {
            return Err(Error::new("DocuWorks document contains no pages"));
        }
        Ok(Self {
            document,
            pages,
            binder_documents,
        })
    }

    pub fn binder_documents(&self) -> Option<&[BinderDocument]> {
        self.binder_documents.as_deref()
    }

    pub fn page_infos(&self) -> Vec<PageInfo> {
        self.pages.clone()
    }

    pub fn interpret_page_scene(&self, index: usize) -> Result<PageScene> {
        let pages = self.document.pages().map_err(map_error)?;
        let page = pages
            .get(index)
            .ok_or_else(|| Error::new("DocuWorks page out of range"))?;
        let mut scene = self.document.page_scene(index, page).map_err(map_error)?;
        if self.binder_documents.is_some() {
            index_binder_text(&mut scene);
        }
        Ok(scene)
    }
}

// This preview indexes horizontal, left-aligned text from the metafile.
fn index_binder_text(scene: &mut PageScene) {
    use ibis_pdf::scene::{TextAnchor, estimated_advance_em};
    let mut clip = scene.info.crop_box;
    let chars = &mut scene.text_index.chars;
    for (order, cmd) in scene.display_list.iter().enumerate() {
        let (text, (mut x, y), size, width, scale) = match cmd {
            DisplayCmd::SetClipRect(rect) => {
                clip = rect.unwrap_or(scene.info.crop_box);
                continue;
            }
            DisplayCmd::DrawGlyphRun {
                text,
                origin,
                font_size,
                rgba,
            } if rgba[3] > 0 => (text, *origin, *font_size, None, 1.0),
            DisplayCmd::DrawStyledGlyphRun {
                text,
                origin,
                font_size,
                angle,
                anchor: TextAnchor::Start,
                max_width,
                horizontal_scale,
                rgba,
                ..
            } if angle.abs() < 0.01 && rgba[3] > 0 => {
                (text, *origin, *font_size, *max_width, *horizontal_scale)
            }
            _ => continue,
        };
        let estimated: f32 = text.chars().map(estimated_advance_em).sum();
        if estimated == 0.0 || size <= 0.0 || scale <= 0.0 {
            continue;
        }
        let em = width.map_or(size * scale, |width| width / estimated);
        for ch in text.chars() {
            let right = x + em * estimated_advance_em(ch);
            let rect_pt = PdfRect {
                x0: x.max(clip.x0),
                y0: (y - size).max(clip.y0),
                x1: right.min(clip.x1),
                y1: y.min(clip.y1),
            };
            x = right;
            if rect_pt.x1 <= rect_pt.x0 || rect_pt.y1 <= rect_pt.y0 {
                continue;
            }
            if let Some(previous) = chars.last()
                && !previous.ch.is_whitespace()
                && !ch.is_whitespace()
                && (previous.rect_pt.y1 - rect_pt.y1).abs() < size * 0.2
                && rect_pt.x0 - previous.rect_pt.x1 > size * 0.15
                && rect_pt.x0 - previous.rect_pt.x1 < size * 2.0
            {
                chars.push(CharBox {
                    ch: ' ',
                    rect_pt: PdfRect {
                        x0: previous.rect_pt.x1,
                        ..rect_pt
                    },
                    vertical: false,
                    stream_op_index: order,
                });
            }
            chars.push(CharBox {
                ch,
                rect_pt,
                vertical: false,
                stream_op_index: order,
            });
        }
    }
    scene.text_index.fit_widths();
}

fn map_error(error: ibis_xdw_read::Error) -> Error {
    use ibis_xdw_read::Error::*;
    Error::new(match error {
        Invalid => "Invalid DocuWorks document",
        Protected => "This DocuWorks document is protected",
        UnsupportedDrawing => "Unsupported DocuWorks drawing",
        Limit => "DocuWorks document exceeds the resource limits",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_scan_without_a_legacy_decoder() {
        let bytes = include_bytes!("../../ibis-xdw-read/tests/fixtures/rotated-emf-scan.xdw");
        let viewer = XdwContainer::from_bytes(Arc::from(bytes.as_slice())).unwrap();
        let pages = viewer.page_infos();
        assert_eq!(pages.len(), 2);
        let scene = viewer.interpret_page_scene(1).unwrap();
        assert_eq!(scene.info, pages[1]);
        assert!(matches!(
            scene.display_list.as_slice(),
            [ibis_pdf::DisplayCmd::DrawImage {
                width: 48,
                height: 32,
                ..
            }]
        ));
        assert!(viewer.interpret_page_scene(2).is_err());
    }
}

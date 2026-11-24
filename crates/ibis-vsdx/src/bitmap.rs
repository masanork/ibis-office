// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::io::Cursor;

use ibis_formats::zip::Archive;
use ibis_ooxml::Relationship;
use ibis_pdf::{DisplayCmd, PdfRect};
use image::{ImageFormat, ImageReader, Limits};
use roxmltree::Node;

use super::{MAX_PART, Matrix, rel_id, relationships};

const IMAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const MAX_PAGE_ENCODED: usize = 16 * 1024 * 1024;
const MAX_PAGE_PIXELS: usize = 4_194_304;

pub(super) struct Bitmaps<'a, 'i> {
    archive: &'a Archive<'i>,
    page: &'a str,
    rels: Vec<Relationship>,
    remaining_pixels: usize,
    remaining_encoded: usize,
}

impl<'a, 'i> Bitmaps<'a, 'i> {
    pub(super) fn new(archive: &'a Archive<'i>, page: &'a str) -> Self {
        let rels = if archive.contains(&ibis_ooxml::package::relationships_part(page)) {
            relationships(archive, page).unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            archive,
            page,
            rels,
            remaining_pixels: MAX_PAGE_PIXELS,
            remaining_encoded: MAX_PAGE_ENCODED,
        }
    }

    pub(super) fn draw(
        &mut self,
        foreign: Node<'_, '_>,
        transform: Matrix,
        (x, y, width, height): (f64, f64, f64, f64),
    ) -> Option<DisplayCmd> {
        let [a, b, c, d, _, _] = transform.0;
        if b.abs() > 1e-7
            || c.abs() > 1e-7
            || width <= 0.0
            || height <= 0.0
            || self.remaining_pixels == 0
            || self.remaining_encoded == 0
        {
            return None;
        }
        if foreign.attribute("ForeignType") != Some("Bitmap") {
            return None;
        }
        let id = rel_id(foreign)?;
        let target = self
            .rels
            .iter()
            .find(|rel| rel.id == id && rel.kind == IMAGE_REL)?
            .target_part(self.page)?;
        let bytes = self
            .archive
            .read_bounded(&target, MAX_PART.min(self.remaining_encoded))
            .ok()??;
        self.remaining_encoded -= bytes.len();
        let format = match image::guess_format(&bytes).ok()? {
            format @ (ImageFormat::Png | ImageFormat::Jpeg) => format,
            _ => return None,
        };
        let reader = || ImageReader::with_format(Cursor::new(&bytes), format);
        let (pixels_x, pixels_y) = reader().into_dimensions().ok()?;
        let count = usize::try_from(u64::from(pixels_x) * u64::from(pixels_y)).ok()?;
        if count == 0 || count > self.remaining_pixels {
            return None;
        }
        // Account for attempted decoding and for each copy retained in the page scene.
        self.remaining_pixels -= count;
        let mut decoder = reader();
        let mut limits = Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some((MAX_PAGE_PIXELS * 8) as u64);
        decoder.limits(limits);
        let mut image = decoder.decode().ok()?.into_rgba8();
        if a < 0.0 {
            image::imageops::flip_horizontal_in_place(&mut image);
        }
        if d > 0.0 {
            image::imageops::flip_vertical_in_place(&mut image);
        }
        let (left, bottom) = transform.point(x, y);
        let (right, top) = transform.point(x + width, y + height);
        Some(DisplayCmd::DrawImage {
            bounds: PdfRect {
                x0: left.min(right),
                y0: top.min(bottom),
                x1: left.max(right),
                y1: top.max(bottom),
            },
            width: pixels_x,
            height: pixels_y,
            rgba: image.into_raw(),
        })
    }
}

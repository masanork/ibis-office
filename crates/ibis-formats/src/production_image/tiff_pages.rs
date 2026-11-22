// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use base64::Engine as _;
use image::{DynamicImage, ImageFormat};
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use tiff::decoder::Decoder;

const MAX_PAGES: usize = 32;
const MAX_PAGE_EDGE: u32 = 2_048;
const MAX_PREVIEW_PNG_BYTES: usize = 32 * 1024 * 1024;

pub(super) fn open(bytes: &[u8]) -> Result<crate::Opened, String> {
    let mut pages = Decoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let first = || super::decode_raster_with_limits(bytes, ImageFormat::Tiff, 4);
    if !pages.more_images() {
        let image = match first() {
            Ok(image) => image,
            Err(image::ImageError::Limits(_)) => super::large_tiff::preview(bytes, pages)?,
            Err(error) => return Err(error.to_string()),
        };
        return super::open_preview_png(&super::encode_png(image)?);
    }

    let mut markdown = String::new();
    let mut png_bytes = 0;
    let mut pixels = 0;
    for page in 1..=MAX_PAGES {
        let image =
            page_image(bytes, &mut pages).map_err(|error| format!("TIFF page {page}: {error}"))?;
        let (width, height) = (image.width(), image.height());
        pixels += u64::from(width) * u64::from(height);
        if pixels > super::MAX_PIXELS {
            return Err("TIFF document exceeds the total preview pixel limit".into());
        }
        let png = super::encode_png(image)?;
        png_bytes += png.len();
        if png_bytes > MAX_PREVIEW_PNG_BYTES {
            return Err("TIFF document exceeds the total preview size limit".into());
        }
        if page > 1 {
            markdown.push_str("\n\\newpage\n\n");
        }
        use std::fmt::Write as _;
        // Fit the image within the Word page's printable area while keeping its aspect ratio.
        let scale = (5.0 / f64::from(width)).min(8.0 / f64::from(height));
        write!(markdown, "![TIFF page {page}](data:image/png;base64,").unwrap();
        base64::engine::general_purpose::STANDARD.encode_string(png, &mut markdown);
        writeln!(
            markdown,
            "){{width={:.6}in height={:.6}in}}",
            f64::from(width) * scale,
            f64::from(height) * scale
        )
        .unwrap();
        if !pages.more_images() {
            return crate::markdown::open_markdown(markdown.into_bytes())
                .map_err(|error| error.to_string());
        }
        if page == MAX_PAGES {
            break;
        }
        pages
            .next_image()
            .map_err(|error| format!("TIFF page {}: {error}", page + 1))?;
    }
    Err(format!(
        "TIFF document exceeds the {MAX_PAGES}-page preview limit"
    ))
}

fn page_image(bytes: &[u8], pages: &mut Decoder<Cursor<&[u8]>>) -> Result<DynamicImage, String> {
    let offset = pages
        .ifd_pointer()
        .ok_or("TIFF page directory is missing")?
        .0;
    let reader = || HeaderOverlay::new(bytes, offset).map(BufReader::new);
    let image = match super::decode_raster_reader(reader()?, ImageFormat::Tiff, 4) {
        Ok(image) => image,
        Err(image::ImageError::Limits(_)) => {
            let decoder = Decoder::new(reader()?).map_err(|error| error.to_string())?;
            super::large_tiff::preview(bytes, decoder)?
        }
        Err(error) => return Err(error.to_string()),
    };
    Ok(if image.width().max(image.height()) > MAX_PAGE_EDGE {
        image.thumbnail(MAX_PAGE_EDGE, MAX_PAGE_EDGE)
    } else {
        image
    })
}

/// Present the selected IFD as the first image while keeping all absolute
/// TIFF offsets and sharing the original bytes with the input file reader.
struct HeaderOverlay<'a> {
    data: Cursor<&'a [u8]>,
    start: usize,
    offset: [u8; 8],
    len: usize,
}

impl<'a> HeaderOverlay<'a> {
    fn new(bytes: &'a [u8], offset: u64) -> Result<Self, String> {
        let (start, len, little_endian) = match bytes.get(..4) {
            Some(b"II*\0") => (4, 4, true),
            Some(b"MM\0*") => (4, 4, false),
            Some(b"II+\0") => (8, 8, true),
            Some(b"MM\0+") => (8, 8, false),
            _ => return Err("Invalid TIFF header".into()),
        };
        let mut encoded = [0; 8];
        if len == 4 {
            let offset = u32::try_from(offset).map_err(|_| "Invalid classic TIFF page offset")?;
            encoded[..4].copy_from_slice(&if little_endian {
                offset.to_le_bytes()
            } else {
                offset.to_be_bytes()
            });
        } else {
            encoded = if little_endian {
                offset.to_le_bytes()
            } else {
                offset.to_be_bytes()
            };
        }
        Ok(Self {
            data: Cursor::new(bytes),
            start,
            offset: encoded,
            len,
        })
    }
}

impl Read for HeaderOverlay<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let position = self.data.position();
        let count = self.data.read(buffer)?;
        if position < (self.start + self.len) as u64 && count != 0 {
            let position = position as usize;
            let begin = position.max(self.start);
            let end = (position + count).min(self.start + self.len);
            if begin < end {
                buffer[begin - position..end - position]
                    .copy_from_slice(&self.offset[begin - self.start..end - self.start]);
            }
        }
        Ok(count)
    }
}

impl Seek for HeaderOverlay<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.data.seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibis_doc_model::{FlowRunKind, flow::Block};
    use tiff::encoder::{Compression, TiffEncoder, colortype};

    #[test]
    fn imports_all_pages_in_order_and_saves_page_breaks() {
        let mut file = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new(&mut file).unwrap();
        encoder
            .write_image::<colortype::Gray8>(2, 3, &[19; 6])
            .unwrap();
        encoder
            .write_image::<colortype::RGB8>(3, 2, &[42; 18])
            .unwrap();
        encoder
            .write_image::<colortype::Gray8>(8_200, 80, &vec![71; 8_200 * 80])
            .unwrap();
        let file = file.into_inner();
        let opened = super::super::open(Some("tiff"), &file).unwrap().unwrap();
        let verify = |opened: &crate::Opened| {
            let flow = opened.presentation.flow.as_ref().unwrap();
            let breaks = flow
                .body
                .iter()
                .filter_map(|block| match block {
                    Block::Paragraph(paragraph) => Some(&paragraph.runs),
                    _ => None,
                })
                .flatten()
                .filter(|run| matches!(run.kind, FlowRunKind::PageBreak))
                .count();
            assert_eq!(breaks, 2);
            let actual: Vec<_> = opened
                .presentation
                .media
                .iter()
                .map(|media| {
                    let image = image::load_from_memory(&media.data).unwrap().to_rgb8();
                    (image.dimensions(), image.get_pixel(0, 0).0)
                })
                .collect();
            assert_eq!(
                actual,
                [((2, 3), [19; 3]), ((3, 2), [42; 3]), ((2_048, 20), [71; 3])]
            );
        };
        verify(&opened);
        let docx = opened
            .source
            .save(&opened.presentation, Default::default())
            .unwrap();
        verify(&crate::open(docx).unwrap());
    }

    #[test]
    fn follows_bigtiff_page_offsets() {
        let mut file = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new_big(&mut file)
            .unwrap()
            .with_compression(Compression::Lzw);
        for gray in [17, 88] {
            encoder
                .write_image::<colortype::Gray8>(2, 2, &[gray; 4])
                .unwrap();
        }
        let opened = open(&file.into_inner()).unwrap();
        let samples: Vec<_> = opened
            .presentation
            .media
            .iter()
            .map(|media| {
                image::load_from_memory(&media.data)
                    .unwrap()
                    .to_luma8()
                    .get_pixel(0, 0)
                    .0[0]
            })
            .collect();
        assert_eq!(samples, [17, 88]);
    }

    #[test]
    fn reports_the_page_limit_instead_of_silently_dropping_pages() {
        let mut file = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new(&mut file).unwrap();
        for _ in 0..=MAX_PAGES {
            encoder.write_image::<colortype::Gray8>(1, 1, &[0]).unwrap();
        }
        let error = open(&file.into_inner()).err().unwrap();
        assert!(error.contains("32-page"), "{error}");
    }
}

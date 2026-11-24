// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::{collections::HashMap, io::Cursor};

use ibis_doc_model::{
    EmuSize, FloatingAnchor, MediaFormat, MediaId, Presentation, ShapeId,
    flow::{FlowRunKind, PictureLook},
};

use super::{
    appearance::{Packets, references, word},
    table::{payload, units},
    wpg,
};

#[derive(Clone, Copy)]
struct PictureContent {
    media: MediaId,
    size: Option<EmuSize>,
}

#[derive(Default)]
pub(super) struct Pictures {
    cache: HashMap<u16, Option<PictureContent>>,
}

#[derive(Default)]
struct ImageBox {
    kind: u8,
    horizontal: (u8, i16),
    vertical: (u8, i16),
    width: u16,
    height: u16,
    native: (u16, u16),
}

impl ImageBox {
    fn style(packet: &[u8]) -> Option<Self> {
        let (_, mut data) = children(packet)?;
        let mut data = section(&mut data)?;
        section(&mut data)?; // Name/library
        section(&mut data)?; // Counter
        let positioning = section(&mut data)?;
        let content = section(&mut data)?;
        let mut style = Self {
            kind: *content.get(1)?,
            horizontal: (*positioning.get(2)?, word(positioning, 3)? as i16),
            vertical: (*positioning.get(7)?, word(positioning, 8)? as i16),
            width: if positioning.get(10)? & 1 == 0 {
                word(positioning, 11)?
            } else {
                0
            },
            height: if positioning.get(13)? & 1 == 0 {
                word(positioning, 14)?
            } else {
                0
            },
            ..Default::default()
        };
        if style.kind == 3 {
            let render = section(&mut content.get(3..)?)?;
            if render.first() == Some(&1) {
                style.native = (word(render, 1)?, word(render, 3)?);
            }
        }
        Some(style)
    }

    fn apply(&mut self, record: &[u8]) -> Option<()> {
        let mut data = record.get(14..)?;
        let mut wrapping = section(&mut data)?;
        let mut overrides = section(&mut wrapping)?;
        let flags = take_word(&mut overrides)?;
        if flags & 0x8000 != 0 {
            section(&mut overrides)?;
        }
        if flags & 0x4000 != 0 {
            let mut position = section(&mut overrides)?;
            let bits = take_word(&mut position)?;
            if bits & 0x8000 != 0 {
                wpg::split(&mut position, 2)?;
            }
            if bits & 0x4000 != 0 {
                wpg::split(&mut position, 2)?;
            }
            if bits & 0x2000 != 0 {
                let horizontal = wpg::split(&mut position, 5)?;
                self.horizontal = (horizontal[0], word(horizontal, 1)? as i16);
            }
            if bits & 0x1000 != 0 {
                let vertical = wpg::split(&mut position, 3)?;
                self.vertical = (vertical[0], word(vertical, 1)? as i16);
            }
            if bits & 0x0800 != 0 {
                self.width = dimension(&mut position)?;
            }
            if bits & 0x0400 != 0 {
                self.height = dimension(&mut position)?;
            }
        }
        if flags & 0x2000 != 0 {
            let mut content = section(&mut overrides)?;
            let bits = take_word(&mut content)?;
            if bits & 0x8000 != 0 {
                wpg::split(&mut content, 2)?;
            }
            if bits & 0x4000 != 0 {
                self.kind = wpg::split(&mut content, 1)?[0];
            }
            if bits & 0x2000 != 0 && self.kind == 3 {
                let mut render = section(&mut content)?;
                let fields = take_word(&mut render)?;
                if fields & 0x8000 != 0 {
                    wpg::split(&mut render, 2)?;
                }
                if fields & 0x4000 != 0 {
                    self.native = (take_word(&mut render)?, take_word(&mut render)?);
                }
            }
        }
        Some(())
    }

    fn size(&self, pixels: Option<EmuSize>) -> Option<EmuSize> {
        let original = match self.native {
            (w, h) if w != 0 && h != 0 => Some(EmuSize::new(units(w), units(h))),
            _ => pixels,
        };
        let size = match (self.width, self.height) {
            (0, 0) => original?,
            (w, 0) => {
                let original = original?;
                EmuSize::new(
                    units(w),
                    units(w).checked_mul(original.height)? / original.width,
                )
            }
            (0, h) => {
                let original = original?;
                EmuSize::new(
                    units(h).checked_mul(original.width)? / original.height,
                    units(h),
                )
            }
            (w, h) => EmuSize::new(units(w), units(h)),
        };
        (size.width > 0 && size.height > 0).then_some(size)
    }

    fn anchor(&self, kind: u8) -> FloatingAnchor {
        let (horizontal, x) = self.horizontal;
        let (vertical, y) = self.vertical;
        let page_h = horizontal & 3 == 0;
        let page_v = vertical & 3 == 0;
        let align_h = if page_h {
            None
        } else {
            match (horizontal >> 2) & 7 {
                1 if x == 0 => Some("right".into()),
                2 => Some("center".into()),
                _ => None,
            }
        };
        let align_v = if page_v {
            None
        } else {
            match (vertical >> 2) & 7 {
                1 => Some("bottom".into()),
                2 => Some("center".into()),
                _ => None,
            }
        };
        FloatingAnchor {
            offset_x: i64::from(x) * 762,
            offset_y: i64::from(y) * 762,
            align_h,
            align_v,
            relative_h: if page_h { "page" } else { "margin" }.into(),
            relative_v: if page_v {
                "page"
            } else if kind == 2 {
                "margin"
            } else {
                "paragraph"
            }
            .into(),
            wrap: "square".into(),
            allow_overlap: true,
            layout_in_cell: true,
            relative_height: FloatingAnchor::DEFAULT_RELATIVE_HEIGHT,
            ..Default::default()
        }
    }
}

impl Pictures {
    pub(super) fn read(
        &mut self,
        record: &[u8],
        packets: Packets<'_>,
        presentation: &mut Presentation,
        id: ShapeId,
    ) -> Option<FlowRunKind> {
        let ids = references(record)?;
        let mut frame = ids
            .chunks_exact(2)
            .filter_map(|id| packets.packet(word(id, 0)?, 0x41))
            .find_map(ImageBox::style)
            .unwrap_or_default();
        frame.apply(payload(record).ok()?)?;
        if frame.kind != 3 {
            return None;
        }
        let pid = ids
            .chunks_exact(2)
            .filter_map(|id| packets.packet(word(id, 0)?, 0x40))
            .filter_map(|packet| children(packet).map(|(ids, _)| ids))
            .flat_map(|ids| ids.chunks_exact(2))
            .filter_map(|id| word(id, 0))
            .find(|&pid| packets.packet(pid, 0x6f).is_some())?;
        let picture = *self
            .cache
            .entry(pid)
            .or_insert_with(|| {
                let bytes = packets.packet(pid, 0x6f)?;
                if let Some(format) = MediaFormat::sniff(bytes) {
                    let size = image::ImageReader::new(Cursor::new(bytes))
                        .with_guessed_format()
                        .ok()
                        .and_then(|image| image.into_dimensions().ok())
                        .filter(|&(w, h)| w > 0 && h > 0)
                        .map(|(w, h)| EmuSize::new(i64::from(w) * 9525, i64::from(h) * 9525));
                    return Some(PictureContent {
                        media: presentation.add_media(format, bytes),
                        size,
                    });
                }
                let (png, size) = wpg::decode(bytes)?;
                Some(PictureContent {
                    media: presentation.add_media(MediaFormat::Png, png),
                    size: Some(size),
                })
            })
            .as_ref()?;
        let size = frame.size(picture.size)?;
        let kind = record[1];
        let floating = (kind != 0).then(|| (id, frame.anchor(kind)));
        Some(crate::hangul_structure::imported_picture(
            picture.media,
            size,
            PictureLook::default(),
            floating,
        ))
    }
}

fn children(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let end = 2 + usize::from(word(data, 0)?) * 2;
    Some((data.get(2..end)?, data.get(end..)?))
}

fn take_word(data: &mut &[u8]) -> Option<u16> {
    word(wpg::split(data, 2)?, 0)
}

fn section<'a>(data: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = usize::from(take_word(data)?);
    wpg::split(data, len)
}

fn dimension(data: &mut &[u8]) -> Option<u16> {
    let record = wpg::split(data, 3)?;
    if record[0] & 1 != 0 {
        Some(0)
    } else {
        word(record, 1)
    }
}

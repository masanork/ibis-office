// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::{
    Rgb,
    flow::{BorderStyle, FlowBorder},
};

use super::table::units;

pub(super) fn word(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn dword(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize)
}

pub(super) fn rgb(bytes: &[u8]) -> Option<Rgb> {
    let [r, g, b, shade] = *bytes.first_chunk::<4>()?;
    let s = u32::from(shade.min(100));
    let channel = |n: u8| (u32::from(n) * s + 255 * (100 - s)) / 100;
    Some(Rgb((channel(r) << 16) | (channel(g) << 8) | channel(b)))
}

pub(super) fn references(record: &[u8]) -> Option<&[u8]> {
    if record[4] & 0x80 == 0 {
        return Some(&[]);
    }
    record.get(6..6 + usize::from(record[5]) * 2)
}

#[derive(Clone, Copy, Default)]
pub(super) struct Packets<'a> {
    prefix: &'a [u8],
    entries: &'a [u8],
}

impl<'a> Packets<'a> {
    pub(super) fn read(prefix: &'a [u8]) -> Self {
        let Some(index) = word(prefix, 14).map(usize::from) else {
            return Self::default();
        };
        let Some(count) = word(prefix, index + 2).map(usize::from) else {
            return Self::default();
        };
        let Some(entries) = prefix.get(index + 14..index + 14 * count) else {
            return Self::default();
        };
        Self { prefix, entries }
    }

    fn packet(self, pid: u16, kind: u8) -> Option<&'a [u8]> {
        let index = usize::from(pid.checked_sub(1)?) * 14;
        let entry = self.entries.get(index..index + 14)?;
        if entry[1] != kind {
            return None;
        }
        let size = dword(entry, 6)?;
        let offset = dword(entry, 10)?;
        self.prefix.get(offset..offset.checked_add(size)?)
    }

    pub(super) fn line(self, pid: u16) -> Option<Option<FlowBorder>> {
        let (_, name, data) = sections(self.packet(pid, 0x42)?)?;
        let name = word(name, 2)? as i16;
        let count = *data.get(3)?;
        let segment = sized(data.get(4..)?)?.0;
        let width = word(segment, 1)?;
        let color = rgb(segment.get(5..)?)?;
        if width == 0 {
            return Some(None);
        }
        let style = match (name, count) {
            (-6, _) => BorderStyle::ThinThickSmallGap,
            (-7, _) => BorderStyle::ThickThinSmallGap,
            (-1, _) | (_, 2) => BorderStyle::Double,
            (_, 3..) => BorderStyle::Triple,
            (-2, _) => BorderStyle::Dashed,
            (-3, _) => BorderStyle::Dotted,
            (_, _) if segment[0] & 3 == 1 => match segment.get(9).copied().unwrap_or(0) {
                0 | 1 => BorderStyle::Single,
                2 if word(segment, 10)? <= width => BorderStyle::Dotted,
                2 => BorderStyle::Dashed,
                4 => BorderStyle::DotDash,
                6 => BorderStyle::DotDotDash,
                _ => BorderStyle::Dashed,
            },
            _ => match segment.get(10).copied().unwrap_or(0) {
                1..=5 => BorderStyle::Dashed,
                6 | 7 => BorderStyle::Dotted,
                8 | 10 | 12 | 14 => BorderStyle::DotDash,
                9 | 11 | 13 | 15 => BorderStyle::DotDotDash,
                _ => BorderStyle::Single,
            },
        };
        Some(Some(FlowBorder {
            width: units(width).clamp(3_175, 152_400),
            color,
            style,
            space: 0,
            art: None,
        }))
    }

    pub(super) fn fill(self, pid: u16) -> Option<Rgb> {
        let (_, _, data) = sections(self.packet(pid, 0x43)?)?;
        rgb(data.get(1..)?)
    }

    pub(super) fn border(self, pid: u16) -> Option<[Option<FlowBorder>; 4]> {
        let (ids, _, data) = sections(self.packet(pid, 0x44)?)?;
        let flags = *data.first()?;
        let mut lines = [None; 4];
        let mut index = 0;
        let override_color = (data.get(1)? & 1 != 0)
            .then(|| rgb(data.get(22..)?))
            .flatten();
        // The packet stores left, right, top, bottom. FlowCell starts at top.
        for (bit, side) in [3, 1, 0, 2].into_iter().enumerate() {
            if flags & (1 << bit) != 0 {
                lines[side] = self.line(word(ids, index)?).flatten().map(|mut line| {
                    line.color = override_color.unwrap_or(line.color);
                    line
                });
                index += 2;
            }
        }
        Some(lines)
    }
}

fn sized(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let end = 2 + usize::from(word(data, 0)?);
    Some((data.get(2..end)?, data.get(end..)?))
}

fn sections(packet: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let index_end = 2 + 2 * usize::from(word(packet, 0)?);
    let ids = packet.get(2..index_end)?;
    let (body, _) = sized(packet.get(index_end..)?)?;
    let (name, rest) = sized(body)?;
    let (data, _) = sized(rest)?;
    Some((ids, name, data))
}

pub(super) fn default_line() -> FlowBorder {
    FlowBorder {
        width: 3_175,
        color: Rgb(0),
        style: BorderStyle::Single,
        space: 0,
        art: None,
    }
}

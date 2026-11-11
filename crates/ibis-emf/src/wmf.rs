// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Playing a Windows Metafile ([MS-WMF]) into SVG. The 16-bit records are turned into the Enhanced Metafile records they
//! stand for and played by the Enhanced Metafile player.

use crate::emf::emf_to_svg;

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// An Enhanced Metafile being assembled.
struct Out {
    bytes: Vec<u8>,
}

impl Out {
    fn record(&mut self, kind: u32, body: &[u8]) {
        let mut body = body.to_vec();
        while body.len() % 4 != 0 {
            body.push(0);
        }
        self.bytes.extend_from_slice(&kind.to_le_bytes());
        self.bytes.extend_from_slice(&((body.len() + 8) as u32).to_le_bytes());
        self.bytes.extend_from_slice(&body);
    }

    /// EMR_EXTTEXTOUTW at (x, y); characters beyond ASCII stand as their single-byte code.
    fn text(&mut self, x: i32, y: i32, text: &str, dx: &[i32]) {
        let units: Vec<u16> = text.encode_utf16().collect();
        let n = units.len();
        let mut body = vec![0u8; 68];
        let mut put = |at: usize, v: i32| body[at - 8..at - 4].copy_from_slice(&v.to_le_bytes());
        put(36, x);
        put(40, y);
        put(44, n as i32);
        put(48, 76);
        let spacing = if dx.len() == n && n > 0 { 76 + n * 2 + (n * 2) % 4 } else { 0 };
        put(72, spacing as i32);
        for u in &units {
            body.extend_from_slice(&u.to_le_bytes());
        }
        if spacing != 0 {
            while body.len() + 8 < spacing {
                body.push(0);
            }
            for v in dx {
                body.extend_from_slice(&v.to_le_bytes());
            }
        }
        self.record(0x54, &body);
    }

    /// EMR_STRETCHDIBITS: a bitmap (`info` header, `bits` pixels) from `src` to `dest` (x, y, width, height).
    fn dib(&mut self, dest: (i32, i32, i32, i32), src: (i32, i32, i32, i32), info: &[u8], bits: &[u8]) {
        let mut body = vec![0u8; 72];
        let mut put = |at: usize, v: i32| body[at - 8..at - 4].copy_from_slice(&v.to_le_bytes());
        put(24, dest.0);
        put(28, dest.1);
        put(32, src.0);
        put(36, src.1);
        put(40, src.2);
        put(44, src.3);
        put(48, 80);
        put(52, info.len() as i32);
        let pad = (4 - info.len() % 4) % 4;
        put(56, (80 + info.len() + pad) as i32);
        put(60, bits.len() as i32);
        put(68, 0x00CC_0020);
        put(72, dest.2);
        put(76, dest.3);
        body.extend_from_slice(info);
        body.extend(std::iter::repeat_n(0, pad));
        body.extend_from_slice(bits);
        self.record(0x51, &body);
    }

    fn ints(&mut self, kind: u32, values: &[i32]) {
        let body: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.record(kind, &body);
    }
}

fn i16_at(d: &[u8], at: usize) -> Option<i32> {
    Some(i32::from(u16_at(d, at)? as i16))
}

/// The bytes of a device-independent bitmap split into its header (with colours) and its pixels.
fn split_dib(dib: &[u8]) -> Option<(&[u8], &[u8])> {
    let header = u32_at(dib, 0)? as usize;
    let (bits, used, compression) = if header == 12 {
        (u32::from(u16_at(dib, 10)?), 0, 0)
    } else {
        (u32::from(u16_at(dib, 14)?), u32_at(dib, 32)?, u32_at(dib, 16)?)
    };
    let entries = if used != 0 { used as usize } else if bits <= 8 { 1usize << bits } else { 0 };
    let mut info = header.checked_add(entries.checked_mul(if header == 12 { 3 } else { 4 })?)?;
    if header == 40 && (compression == 3 || compression == 6) {
        info = info.checked_add(if compression == 3 { 12 } else { 16 })?;
    }
    Some((dib.get(..info)?, dib.get(info..)?))
}

#[cfg(test)]
mod bounds_tests {
    use super::split_dib;

    #[test]
    fn rejects_palette_count_larger_than_the_input() {
        let mut dib = vec![0u8; 40];
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[14..16].copy_from_slice(&8u16.to_le_bytes());
        dib[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_dib(&dib).is_none());
    }
}

/// The SVG a Windows Metafile draws, or `None` when it is not one or draws nothing.
pub fn wmf_to_svg(wmf: &[u8]) -> Option<String> {
    let (start, placeable) = if u32_at(wmf, 0)? == 0x9AC6_CDD7 {
        let bbox = (i16_at(wmf, 6)?, i16_at(wmf, 8)?, i16_at(wmf, 10)?, i16_at(wmf, 12)?);
        (22, Some(bbox))
    } else {
        (0, None)
    };
    // The 18-byte header: type, header size (words), version, size, objects, largest record, members.
    let header_size = usize::from(u16_at(wmf, start + 2)?) * 2;
    if header_size != 18 || !matches!(u16_at(wmf, start)?, 1 | 2) {
        return None;
    }
    let mut out = Out { bytes: vec![0; 88] };
    out.bytes[0..4].copy_from_slice(&1u32.to_le_bytes());
    out.bytes[4..8].copy_from_slice(&88u32.to_le_bytes());
    out.bytes[40..44].copy_from_slice(b" EMF");
    // The display: 960 pixels over 254 millimetres, so that the metric map modes work.
    for (at, v) in [(72, 960i32), (76, 960), (80, 254), (84, 254)] {
        out.bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    // EMF object handles are one more than the WMF indexes. The WMF table fills its lowest free slot.
    let mut slots: Vec<bool> = Vec::new();
    let mut free_slots = std::collections::BinaryHeap::<std::cmp::Reverse<usize>>::new();
    let take = |slots: &mut Vec<bool>, free: &mut std::collections::BinaryHeap<std::cmp::Reverse<usize>>| -> u32 {
        let i = free.pop().map(|std::cmp::Reverse(i)| i).unwrap_or_else(|| {
            slots.push(false);
            slots.len() - 1
        });
        slots[i] = true;
        i as u32 + 1
    };
    let mut fonts: std::collections::HashMap<u32, u8> = std::collections::HashMap::new();
    let mut charset = 0u8;
    let mut window = ((0i32, 0i32), (0i32, 0i32));
    let mut mode_set = false;
    let mut at = start + header_size;
    while let Some(words) = u32_at(wmf, at) {
        let size = usize::try_from(words).ok()?.checked_mul(2)?;
        if size < 6 || at.checked_add(size)? > wmf.len() {
            break;
        }
        let r = &wmf[at..at + size];
        let func = u16_at(r, 4).unwrap_or(0);
        let p = |i: usize| i16_at(r, 6 + i * 2).unwrap_or(0);
        match func {
            0 => break,
            0x0103 => {
                out.ints(0x11, &[i32::from(u16_at(r, 6).unwrap_or(1))]);
                mode_set = true;
            }
            0x0102 => out.ints(0x12, &[p(0)]),
            0x0106 => out.ints(0x13, &[p(0)]),
            0x0201 => out.ints(0x19, &[u32_at(r, 6).unwrap_or(0) as i32]),
            0x0209 => out.ints(0x18, &[u32_at(r, 6).unwrap_or(0) as i32]),
            0x012E => out.ints(0x16, &[p(0)]),
            0x020B => {
                window.0 = (p(1), p(0));
                out.ints(0x0A, &[p(1), p(0)]);
            }
            0x020C => {
                window.1 = (p(1), p(0));
                out.ints(0x09, &[p(1), p(0)]);
                // Free scaling: the window is shown whole.
                if !mode_set {
                    out.ints(0x11, &[8]);
                    mode_set = true;
                }
                out.ints(0x0C, &[0, 0]);
                out.ints(0x0B, &[p(1).abs(), p(0).abs()]);
            }
            0x020D => out.ints(0x0C, &[p(1), p(0)]),
            0x020E => out.ints(0x0B, &[p(1), p(0)]),
            0x020F => {
                window.0 = (window.0.0 + p(1), window.0.1 + p(0));
                out.ints(0x0A, &[window.0.0, window.0.1]);
            }
            0x0410 => out.ints(0x20, &[p(3), p(2), p(1), p(0)]),
            0x0214 => out.ints(0x1B, &[p(1), p(0)]),
            0x0213 => out.ints(0x36, &[p(1), p(0)]),
            0x001E => out.ints(0x21, &[]),
            0x0127 => out.ints(0x22, &[p(0)]),
            0x0416 => out.ints(0x1E, &[p(3), p(2), p(1), p(0)]),
            0x0324 | 0x0325 => {
                let n = usize::from(u16_at(r, 6).unwrap_or(0));
                if 8usize.checked_add(n.checked_mul(4)?)? > r.len() { return None; }
                let mut v = vec![0, 0, 0, 0, n as i32];
                for i in 0..n {
                    v.push(i16_at(r, 8 + i * 4).unwrap_or(0));
                    v.push(i16_at(r, 10 + i * 4).unwrap_or(0));
                }
                // EMR_POLYGON (3) / EMR_POLYLINE (4), with 32-bit points.
                let mut body = Vec::new();
                for x in &v[..4] {
                    body.extend_from_slice(&x.to_le_bytes());
                }
                for x in &v[4..] {
                    body.extend_from_slice(&x.to_le_bytes());
                }
                out.record(if func == 0x0324 { 3 } else { 4 }, &body);
            }
            0x0538 => {
                let n = usize::from(u16_at(r, 6).unwrap_or(0));
                let base = 8usize.checked_add(n.checked_mul(2)?)?;
                if base > r.len() { return None; }
                let counts: Vec<usize> = (0..n).map(|i| usize::from(u16_at(r, 8 + i * 2).unwrap_or(0))).collect();
                let total = counts.iter().try_fold(0usize, |sum, &count| sum.checked_add(count))?;
                if base.checked_add(total.checked_mul(4)?)? > r.len() { return None; }
                let mut v = vec![0, 0, 0, 0, n as i32, total as i32];
                v.extend(counts.iter().map(|&c| c as i32));
                for i in 0..total {
                    v.push(i16_at(r, base + i * 4).unwrap_or(0));
                    v.push(i16_at(r, base + i * 4 + 2).unwrap_or(0));
                }
                out.ints(8, &v);
            }
            0x041B => out.ints(0x2B, &[p(3), p(2), p(1), p(0)]),
            0x0418 => out.ints(0x2A, &[p(3), p(2), p(1), p(0)]),
            0x061C => out.ints(0x2C, &[p(5), p(4), p(3), p(2), p(1), p(0)]),
            0x0817 | 0x0830 | 0x081A => {
                let kind = match func {
                    0x0817 => 0x2D,
                    0x0830 => 0x2E,
                    _ => 0x2F,
                };
                // Left, top, right, bottom, then the start and the end points.
                out.ints(kind, &[p(7), p(6), p(5), p(4), p(3), p(2), p(1), p(0)]);
            }
            0x00F7 | 0x0142 | 0x01F9 | 0x06FF => {
                let handle = take(&mut slots, &mut free_slots);
                out.ints(0x7A, &[handle as i32]);
            }
            0x02FA => {
                // A pen: style, width (x), colour.
                let handle = take(&mut slots, &mut free_slots);
                let style = u32::from(u16_at(r, 6).unwrap_or(0));
                let width = p(1);
                let color = u32_at(r, 12).unwrap_or(0);
                out.ints(0x26, &[handle as i32, style as i32, width, 0, color as i32]);
            }
            0x02FC => {
                let handle = take(&mut slots, &mut free_slots);
                let style = u32::from(u16_at(r, 6).unwrap_or(0));
                let color = u32_at(r, 8).unwrap_or(0);
                out.ints(0x27, &[handle as i32, style as i32, color as i32, u32::from(u16_at(r, 12).unwrap_or(0)) as i32]);
            }
            0x02FB => {
                let handle = take(&mut slots, &mut free_slots);
                let mut body = vec![0u8; 4 + 92];
                body[0..4].copy_from_slice(&handle.to_le_bytes());
                // Height, width, escapement, orientation, weight, italic, underline, strike, charset...
                body[4..8].copy_from_slice(&p(0).to_le_bytes());
                body[8..12].copy_from_slice(&p(1).to_le_bytes());
                body[12..16].copy_from_slice(&p(2).to_le_bytes());
                body[16..20].copy_from_slice(&p(3).to_le_bytes());
                body[20..24].copy_from_slice(&p(4).to_le_bytes());
                body[24] = r.get(16).copied().unwrap_or(0);
                body[25] = r.get(17).copied().unwrap_or(0);
                body[26] = r.get(18).copied().unwrap_or(0);
                let charset = r.get(19).copied().unwrap_or(0);
                body[27] = charset;
                fonts.insert(handle, charset);
                // The face name (in the font's character set) as UTF-16 at 28 bytes into the LogFont.
                let name = r.get(24..).unwrap_or(&[]);
                let name: Vec<u8> = name.iter().copied().take_while(|&b| b != 0).collect();
                for (i, u) in crate::charset::decode(&name, charset).iter().flat_map(|(c, _)| { let mut b = [0u16; 2]; c.encode_utf16(&mut b).to_vec() }).take(31).enumerate() {
                    body[4 + 28 + i * 2..4 + 30 + i * 2].copy_from_slice(&u.to_le_bytes());
                }
                out.record(0x52, &body);
            }
            0x012D => {
                let index = u32::from(u16_at(r, 6).unwrap_or(0));
                if let Some(c) = fonts.get(&(index + 1)) {
                    charset = *c;
                }
                out.ints(0x25, &[index as i32 + 1]);
            }
            0x01F0 => {
                let index = usize::from(u16_at(r, 6).unwrap_or(0));
                if let Some(slot) = slots.get_mut(index) {
                    if *slot {
                        *slot = false;
                        free_slots.push(std::cmp::Reverse(index));
                    }
                }
                out.ints(0x28, &[index as i32 + 1]);
            }
            0x0521 | 0x0A32 => {
                let (x, y, bytes, dx) = if func == 0x0521 {
                    let len = usize::from(u16_at(r, 6).unwrap_or(0));
                    if 8usize.checked_add(len)?.checked_add(len % 2)?.checked_add(4)? > r.len() { return None; }
                    let s = r.get(8..8 + len).unwrap_or(&[]);
                    let after = 8 + len + len % 2;
                    (i16_at(r, after + 2).unwrap_or(0), i16_at(r, after).unwrap_or(0), s.to_vec(), Vec::new())
                } else {
                    let len = usize::from(u16_at(r, 10).unwrap_or(0));
                    let options = u16_at(r, 12).unwrap_or(0);
                    let s = if options & 6 != 0 { 22 } else { 14 };
                    if s + len > r.len() { return None; }
                    let text = r.get(s..s + len).unwrap_or(&[]).to_vec();
                    let after = s + len + len % 2;
                    let dx: Vec<i32> = (0..len).filter_map(|i| i16_at(r, after + i * 2)).collect();
                    (p(1), p(0), text, dx)
                };
                let dxf: Vec<f64> = dx.iter().map(|&v| f64::from(v)).collect();
                let (text, merged) = crate::charset::decode_with_spacing(&bytes, charset, &dxf);
                let merged: Vec<i32> = merged.iter().map(|&v| v as i32).collect();
                out.text(x, y, &text, &merged);
            }
            0x0F43 | 0x0B41 | 0x0940 => {
                // (destination x, y, width, height), (source x, y, width, height) and where the bitmap starts.
                let (dib_at, dest, src) = match func {
                    0x0F43 => (28, (p(10), p(9), p(8), p(7)), (p(6), p(5), p(4), p(3))),
                    0x0B41 => (26, (p(9), p(8), p(7), p(6)), (p(5), p(4), p(3), p(2))),
                    _ => (22, (p(7), p(6), p(5), p(4)), (p(3), p(2), p(5), p(4))),
                };
                if func == 0x0940 && size == (usize::from(func >> 8) + 3) * 2 {
                    at += size;
                    continue;
                }
                if let Some((info, bits)) = r.get(dib_at..).and_then(split_dib) {
                    out.dib(dest, src, info, bits);
                }
            }
            _ => {}
        }
        at += size;
    }
    {
        // Close the stream and play it.
        out.record(14, &[0; 12]);
        let mut bytes = out.bytes;
        let bounds: [i32; 4] = if window.1 != (0, 0) { [0, 0, window.1.0.abs() - 1, window.1.1.abs() - 1] } else if let Some(b) = placeable { [0, 0, (b.2 - b.0 - 1).max(0), (b.3 - b.1 - 1).max(0)] } else { [0, 0, 0, 0] };
        for (i, v) in bounds.iter().enumerate() {
            bytes[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        emf_to_svg(&bytes)
    }
}

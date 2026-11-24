// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Binary records and device metadata shared by the metafile renderers.

fn take_record<'a>(input: &mut &'a [u8], size: usize, minimum: usize) -> Option<&'a [u8]> {
    if size < minimum {
        return None;
    }
    let (record, rest) = input.split_at_checked(size)?;
    *input = rest;
    Some(record)
}

/// Read an EMF record. Its length is in bytes and includes the eight-byte header.
pub fn take_emf_record<'a>(input: &mut &'a [u8]) -> Option<(u32, &'a [u8])> {
    let kind = u32::from_le_bytes(*input.first_chunk()?);
    let size = u32::from_le_bytes(*input.get(4..)?.first_chunk()?) as usize;
    Some((kind, take_record(input, size, 8)?))
}

/// Read a WMF record. Its length is in 16-bit words and includes the three-word header.
pub fn take_wmf_record<'a>(input: &mut &'a [u8]) -> Option<(u16, &'a [u8])> {
    let size = (u32::from_le_bytes(*input.first_chunk()?) as usize).checked_mul(2)?;
    let kind = u16::from_le_bytes(*input.get(4..)?.first_chunk()?);
    Some((kind, take_record(input, size, 6)?))
}

/// Device pixel and millimetre dimensions from an EMF header, in x/y order.
pub fn emf_device_size(header: &[u8]) -> Option<([i32; 2], [i32; 2])> {
    let field = |offset| Some(i32::from_le_bytes(*header.get(offset..)?.first_chunk()?));
    Some(([field(72)?, field(76)?], [field(80)?, field(84)?]))
}

/// Read a DIB's header and pixels using the four consecutive offset/length fields.
pub fn dib_parts(record: &[u8], fields_at: usize) -> Option<(&[u8], &[u8])> {
    let fields = record.get(fields_at..)?.first_chunk::<16>()?;
    let part = |at| {
        let offset = u32::from_le_bytes(fields.get(at..)?.first_chunk::<4>().copied()?) as usize;
        let length =
            u32::from_le_bytes(fields.get(at + 4..)?.first_chunk::<4>().copied()?) as usize;
        record.get(offset..)?.get(..length)
    };
    Some((part(0)?, part(8)?))
}

/// The RGB channels of a GDI COLORREF, whose low byte is red.
pub fn colorref(value: u32) -> [u8; 3] {
    let [r, g, b, _] = value.to_le_bytes();
    [r, g, b]
}

pub use crate::charset::{StrictTextError, decode_text_strict};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metafile_lengths_use_their_own_units_and_leave_the_tail() {
        let mut emf = [1, 0, 0, 0, 8, 0, 0, 0, 42].as_slice();
        assert_eq!(
            take_emf_record(&mut emf),
            Some((1, &[1, 0, 0, 0, 8, 0, 0, 0][..]))
        );
        assert_eq!(emf, &[42]);
        let mut wmf = [3, 0, 0, 0, 0x09, 0x02, 42].as_slice();
        assert_eq!(
            take_wmf_record(&mut wmf),
            Some((0x0209, &[3, 0, 0, 0, 0x09, 0x02][..]))
        );
        assert_eq!(wmf, &[42]);
    }

    #[test]
    fn dib_offsets_and_lengths_have_to_fit_the_record() {
        let mut record = [0u8; 24];
        for (at, value) in [(0, 16u32), (4, 2), (8, 20), (12, 4)] {
            record[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            dib_parts(&record, 0),
            Some((&record[16..18], &record[20..]))
        );
        assert!(dib_parts(&record[..23], 0).is_none());
        record[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(dib_parts(&record, 0).is_none());
    }

    #[test]
    fn truncated_and_undersized_records_do_not_consume_input() {
        for mut bytes in [
            [1, 0, 0, 0, 7, 0, 0, 0].as_slice(),
            &[1, 0, 0, 0, 12, 0, 0, 0],
        ] {
            let original = bytes;
            assert!(take_emf_record(&mut bytes).is_none());
            assert_eq!(bytes, original);
        }
        let mut bytes = [0xff; 8].as_slice();
        assert!(take_wmf_record(&mut bytes).is_none());
        assert_eq!(bytes.len(), 8);
    }
}

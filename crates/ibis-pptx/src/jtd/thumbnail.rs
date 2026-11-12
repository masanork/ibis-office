// SPDX-License-Identifier: AGPL-3.0-only
//! Bounded metadata inspection of one observed thumbnail envelope.
//! Pixel encoding, freshness and association with a page are unknown.
use crate::Error;

const MAX_THUMBNAIL: usize = 16 * 1024 * 1024;

/// Candidate metadata, not a decoded bitmap or a verified page preview.
#[derive(Debug, PartialEq, Eq)]
pub struct ThumbnailCandidate {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u16,
    pub declared_dib_bytes: u32,
    pub stored_dib_bytes: u32,
    pub stored_payload_bytes: usize,
}

/// Read the observed Thumbnail envelope. Other versions are rejected.
pub fn inspect_thumbnail(data: &[u8]) -> Result<Option<ThumbnailCandidate>, Error> {
    let document = super::source_document(data)?;
    let entries = ibis_ooxml::cfb::directory(&document).map_err(|e| Error::new(e.to_string()))?;
    if !entries
        .iter()
        .any(|e| !e.is_storage && e.path == "Thumbnail")
    {
        return Ok(None);
    }
    let bytes = ibis_ooxml::cfb::stream_path(&document, "Thumbnail")
        .map_err(|e| Error::new(e.to_string()))?;
    parse(&bytes).map(Some)
}
fn bad() -> Error {
    Error::new("サムネイルの診断データが破損しているか未対応です")
}
fn u32_at(b: &[u8], at: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    ))
}
fn u16_at(b: &[u8], at: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    ))
}
fn parse(bytes: &[u8]) -> Result<ThumbnailCandidate, Error> {
    if bytes.len() <= 56 || bytes.len() > MAX_THUMBNAIL || bytes[8..16] != [0; 8] {
        return Err(bad());
    }
    let declared = u32_at(bytes, 0)?;
    let stored = u32_at(bytes, 4)?;
    let width = u32_at(bytes, 20)?;
    let height = u32_at(bytes, 24)?;
    if u32_at(bytes, 16)? != 40
        || width == 0
        || height == 0
        || width > 65535
        || height > 65535
        || u16_at(bytes, 28)? != 1
        || u16_at(bytes, 30)? != 24
        || u32_at(bytes, 32)? != 0
        || usize::try_from(stored).ok() != Some(bytes.len() - 16)
    {
        return Err(bad());
    }
    let stride = width
        .checked_mul(3)
        .and_then(|v| v.checked_add(3))
        .map(|v| v & !3)
        .ok_or_else(bad)?;
    let expected = stride
        .checked_mul(height)
        .and_then(|v| v.checked_add(40))
        .ok_or_else(bad)?;
    if expected != declared || expected as usize > MAX_THUMBNAIL {
        return Err(bad());
    }
    Ok(ThumbnailCandidate {
        width,
        height,
        bit_depth: 24,
        declared_dib_bytes: declared,
        stored_dib_bytes: stored,
        stored_payload_bytes: bytes.len() - 56,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibis_ooxml::cfb::{self, Item};
    fn thumbnail() -> Vec<u8> {
        let mut b = vec![0; 57];
        b[0..4].copy_from_slice(&64u32.to_le_bytes()); // 3 pixels, padded to 12 bytes, 2 rows
        b[4..8].copy_from_slice(&41u32.to_le_bytes());
        b[16..20].copy_from_slice(&40u32.to_le_bytes());
        b[20..24].copy_from_slice(&3u32.to_le_bytes());
        b[24..28].copy_from_slice(&2u32.to_le_bytes());
        b[28..30].copy_from_slice(&1u16.to_le_bytes());
        b[30..32].copy_from_slice(&24u16.to_le_bytes());
        b
    }
    #[test]
    fn inspects_only_bounded_observed_envelope() {
        let b = thumbnail();
        let result = parse(&b).unwrap();
        assert_eq!(
            (
                result.width,
                result.height,
                result.declared_dib_bytes,
                result.stored_payload_bytes
            ),
            (3, 2, 64, 1)
        );
        for n in 0..57 {
            assert!(parse(&b[..n]).is_err());
        }
        for offset in [0, 4, 8, 16, 20, 24, 28, 30, 32] {
            let mut wrong = b.clone();
            wrong[offset] ^= 0x80;
            assert!(parse(&wrong).is_err(), "offset {offset}");
        }
        let mut huge = b;
        huge[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&huge).is_err());
    }
    #[test]
    fn inspects_uncompressed_and_wrapped_documents_without_writing() {
        let body = super::super::decode::fixture(&[0x7532]);
        let none = cfb::write(&[Item::stream("DocumentText", body.clone())]);
        assert_eq!(inspect_thumbnail(&none).unwrap(), None);
        let inner = cfb::write(&[
            Item::stream("DocumentText", body),
            Item::stream("Thumbnail", thumbnail()),
        ]);
        let expected = inspect_thumbnail(&inner).unwrap();
        let outer = cfb::write(&[Item::stream(
            "JSCompDocument",
            super::super::compressed::fixture(&inner),
        )]);
        assert_eq!(inspect_thumbnail(&outer).unwrap(), expected);
    }
}

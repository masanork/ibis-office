// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Read-only cluster lookup over a bitmap and contiguous membership arrays.
//! Cluster membership is not evidence of a direct relation or safe replacement.
use std::{
    io::Read,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock, RwLock,
    },
};

const BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/variant-clusters.zst"));
const DIRECTORY: usize = 4352;
static EMBEDDED: OnceLock<Option<Arc<VariantClusters>>> = OnceLock::new();
static ACTIVE: OnceLock<Slot> = OnceLock::new();

/// Whether a dictionary was supplied at build time.
pub fn is_embedded() -> bool {
    !BLOB.is_empty()
}

/// Decompress (zstd) and parse a dictionary file: the bundled one, or one downloaded or placed by the user.
/// `None` when the bytes are not a dictionary.
pub fn decode(zst: &[u8]) -> Option<VariantClusters> {
    if zst.is_empty() {
        return None;
    }
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(zst).ok()?;
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).ok()?;
    VariantClusters::from_bytes(raw)
}

fn embedded_arc() -> Option<Arc<VariantClusters>> {
    EMBEDDED.get_or_init(|| decode(BLOB).map(Arc::new)).clone()
}

/// The dictionary bundled at build time (whatever is installed later).
pub fn embedded() -> Option<&'static VariantClusters> {
    EMBEDDED.get_or_init(|| decode(BLOB).map(Arc::new)).as_deref()
}

/// The dictionary in use and how many times it was replaced.
struct Slot {
    dict: RwLock<Option<Arc<VariantClusters>>>,
    generation: AtomicU64,
}

impl Slot {
    fn new(dict: Option<Arc<VariantClusters>>) -> Self {
        Slot { dict: RwLock::new(dict), generation: AtomicU64::new(0) }
    }
    fn get(&self) -> Option<Arc<VariantClusters>> {
        self.dict.read().ok()?.clone()
    }
    fn set(&self, dict: Option<Arc<VariantClusters>>) -> u64 {
        if let Ok(mut d) = self.dict.write() {
            *d = dict;
        }
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }
}

fn slot() -> &'static Slot {
    ACTIVE.get_or_init(|| Slot::new(embedded_arc()))
}

/// The dictionary in use: the last one [`install`]ed (a larger edition downloaded with セットアップ, or one placed in the data
/// folder), else the bundled one. The name comparison of `IBIS.FIELDMATCH` and the variant review read it each time.
pub fn active() -> Option<Arc<VariantClusters>> {
    slot().get()
}

/// Whether a dictionary is in use (bundled or installed).
pub fn available() -> bool {
    active().is_some()
}

/// Use `dict` from now on, in place of the bundled one (or of one installed before). Returns the new [`generation`].
/// Cluster indexes of the earlier dictionary are not comparable with the new one's: a workbook calculated before
/// shows its old results until it is calculated again.
pub fn install(dict: VariantClusters) -> u64 {
    slot().set(Some(Arc::new(dict)))
}

/// [`decode`] then [`install`]; `None` (and nothing changed) when the bytes are not a dictionary.
pub fn install_zst(zst: &[u8]) -> Option<u64> {
    decode(zst).map(install)
}

/// Go back to the bundled dictionary.
pub fn reset_to_embedded() -> u64 {
    slot().set(embedded_arc())
}

/// How many times the dictionary in use was replaced since the start (0: the bundled one, never replaced).
pub fn generation() -> u64 {
    slot().generation.load(Ordering::SeqCst)
}

#[derive(Debug)]
pub struct VariantClusters {
    raw: Vec<u8>,
    count: usize,
    groups: usize,
    pages: usize,
    common: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    pub representative: char,
    pub members: Vec<char>,
}
fn u16_at(d: &[u8], i: usize) -> usize {
    u16::from_le_bytes([d[i], d[i + 1]]) as usize
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}
impl VariantClusters {
    pub fn from_bytes(raw: Vec<u8>) -> Option<Self> {
        if raw.len() < 16 + DIRECTORY * 2 {
            return None;
        }
        let count = u32_at(&raw, 0) as usize;
        let groups = u32_at(&raw, 4) as usize;
        let pages = u32_at(&raw, 8) as usize;
        if count == 0
            || count > 0x110000
            || groups == 0
            || groups > u16::MAX as usize
            || pages == 0
            || pages > DIRECTORY
            || u32_at(&raw, 12) != 0
        {
            return None;
        }
        let common = (16 + DIRECTORY * 2)
            .checked_add(pages.checked_mul(36)?)?
            .checked_add(count.checked_mul(2)?)?;
        let len = common
            .checked_add((groups + 1).checked_mul(4)?)?
            .checked_add(groups.checked_mul(4)?)?
            .checked_add(count.checked_mul(4)?)?;
        if raw.len() != len {
            return None;
        }
        let mut next = 1;
        let mut rank = 0;
        for p in 0..DIRECTORY {
            let i = u16_at(&raw, 16 + p * 2);
            if i == 0 {
                continue;
            }
            if i != next || i > pages {
                return None;
            }
            let b = 16 + DIRECTORY * 2 + (i - 1) * 36;
            if u32_at(&raw, b) as usize != rank {
                return None;
            }
            for word in 0..8 {
                rank += u32_at(&raw, b + 4 + word * 4).count_ones() as usize;
            }
            next += 1;
        }
        if next != pages + 1 || rank != count {
            return None;
        }
        let values = 16 + DIRECTORY * 2 + pages * 36;
        for i in 0..count {
            if u16_at(&raw, values + i * 2) >= groups {
                return None;
            }
        }
        let mut prev = 0;
        for i in 0..=groups {
            let off = u32_at(&raw, common + i * 4) as usize;
            if off < prev || off > count {
                return None;
            }
            prev = off;
        }
        if u32_at(&raw, common) != 0 || prev != count {
            return None;
        }
        let reps = common + (groups + 1) * 4;
        let members = reps + groups * 4;
        for i in 0..groups {
            if char::from_u32(u32_at(&raw, reps + i * 4)).is_none() {
                return None;
            }
        }
        for i in 0..count {
            if char::from_u32(u32_at(&raw, members + i * 4)).is_none() {
                return None;
            }
        }
        Some(Self {
            raw,
            count,
            groups,
            pages,
            common,
        })
    }
    fn group(&self, c: char) -> Option<usize> {
        let cp = c as usize;
        let page = u16_at(&self.raw, 16 + (cp / 256) * 2);
        if page == 0 {
            return None;
        }
        let b = 16 + DIRECTORY * 2 + (page - 1) * 36;
        let bit = cp % 256;
        let word = bit / 32;
        let w = u32_at(&self.raw, b + 4 + word * 4);
        let mask = 1u32 << (bit % 32);
        if w & mask == 0 {
            return None;
        }
        let mut rank = u32_at(&self.raw, b) as usize;
        for i in 0..word {
            rank += u32_at(&self.raw, b + 4 + i * 4).count_ones() as usize;
        }
        rank += (w & (mask - 1)).count_ones() as usize;
        let values = 16 + DIRECTORY * 2 + self.pages * 36;
        Some(u16_at(&self.raw, values + rank * 2))
    }
    /// Stable within this dictionary snapshot, suitable for comparing cluster membership.
    pub fn cluster_index(&self, c: char) -> Option<u32> {
        self.group(c).map(|group| group as u32)
    }

    pub fn cluster(&self, c: char) -> Option<Cluster> {
        let group = self.group(c)?;
        let c = self.common;
        let start = u32_at(&self.raw, c + group * 4) as usize;
        let end = u32_at(&self.raw, c + (group + 1) * 4) as usize;
        let reps = c + (self.groups + 1) * 4;
        let members = reps + self.groups * 4;
        Some(Cluster {
            representative: char::from_u32(u32_at(&self.raw, reps + group * 4))?,
            members: (start..end)
                .filter_map(|i| char::from_u32(u32_at(&self.raw, members + i * 4)))
                .collect(),
        })
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut v = vec![0; 16 + DIRECTORY * 2];
        v[0..4].copy_from_slice(&2u32.to_le_bytes());
        v[4..8].copy_from_slice(&1u32.to_le_bytes());
        v[8..12].copy_from_slice(&1u32.to_le_bytes());
        v[16..18].copy_from_slice(&1u16.to_le_bytes());
        let mut page = [0u8; 36];
        page[12..16].copy_from_slice(&6u32.to_le_bytes());
        v.extend(page);
        v.extend([0u8; 4]);
        for x in [0u32, 2, 0x41, 0x41, 0x42] {
            v.extend(x.to_le_bytes());
        }
        v
    }
    #[test]
    fn missing_dictionary_is_reported() {
        if BLOB.is_empty() {
            assert!(!is_embedded());
            assert!(embedded().is_none());
        }
    }
    #[test]
    fn bundled_gjs_dictionary_is_used_by_field_match() {
        if option_env!("IBIS_VARIANT_BITMAP").is_some() {
            return;
        }
        let dict = embedded().expect("default gjs dictionary");
        // The GJS basic edition (dict20260718; no share-alike sources). The full edition is downloaded with セットアップ.
        assert_eq!(dict.len(), 22_131);
        let high = dict.cluster_index('高').expect("高 in bundled gjs dictionary");
        assert_eq!(dict.cluster_index('髙'), Some(high));
        let book = crate::Presentation::empty(crate::geometry::EmuSize::WIDESCREEN);
        let view = crate::sheet_calc::BookView::new(&book);
        for (formula, rank) in [
            (r#"IBIS.FIELDMATCH("塚本花子","塚本花子","name")"#, 1.0),
            (r#"IBIS.FIELDMATCH("塚本花子","塚本花子","name")"#, 2.0),
            (r#"IBIS.FIELDMATCH("髙橋花子","高橋花子","name")"#, 1.0),
        ] {
            assert_eq!(
                ibis_calc::recalc::evaluate(&view, (0, 0, 0), formula),
                (ibis_calc::CellValue::Number(rank), false),
            );
        }
    }

    #[test]
    fn a_slot_replaces_and_restores_the_dictionary() {
        // A slot of its own: the global one is read by the other tests of this crate at the same time.
        let bundled = embedded_arc();
        let s = Slot::new(bundled.clone());
        let other = Arc::new(VariantClusters::from_bytes(fixture()).unwrap());
        assert_eq!(s.set(Some(other)), 1);
        assert_eq!(s.get().unwrap().cluster('B').unwrap().representative, 'A');
        assert_eq!(s.set(bundled.clone()), 2);
        assert_eq!(s.get().map(|d| d.len()), bundled.map(|d| d.len()));
        assert!(decode(b"not zstd").is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn lookup_and_reject_invalid_data() {
        let bytes = fixture();
        let d = VariantClusters::from_bytes(bytes.clone()).unwrap();
        assert_eq!(
            d.cluster('A'),
            Some(Cluster {
                representative: 'A',
                members: vec!['A', 'B']
            })
        );
        assert_eq!(d.cluster('B'), d.cluster('A'));
        assert_eq!(d.cluster('C'), None);
        assert!(VariantClusters::from_bytes(bytes[..bytes.len() - 1].to_vec()).is_none());
        let mut corrupt = bytes;
        corrupt[8] = 0;
        assert!(VariantClusters::from_bytes(corrupt).is_none());
    }
    #[test]
    #[ignore = "requires the locally reviewed cluster CSV and embedded bitmap"]
    fn local_snapshot_matches_every_member_and_absent_scalar() {
        let csv = std::fs::read_to_string(std::env::var("IBIS_VARIANT_CSV").unwrap()).unwrap();
        let dict = embedded().expect("build with IBIS_VARIANT_BITMAP");
        let mut expected = std::collections::BTreeMap::<char, (String, char)>::new();
        let mut groups = std::collections::BTreeMap::<String, Vec<char>>::new();
        for line in csv.lines().skip(1) {
            let r: Vec<_> = line.split(',').collect();
            let cp = char::from_u32(u32::from_str_radix(&r[0][2..], 16).unwrap()).unwrap();
            let rep = char::from_u32(u32::from_str_radix(&r[2][2..], 16).unwrap()).unwrap();
            expected.insert(cp, (r[1].to_owned(), rep));
            groups.entry(r[1].to_owned()).or_default().push(cp);
        }
        assert_eq!(dict.len(), expected.len());
        for (&cp, (id, rep)) in &expected {
            let cluster = dict.cluster(cp).unwrap();
            assert_eq!(cluster.representative, *rep);
            assert_eq!(cluster.members, groups[id]);
        }
        for code in 0..=0x10ffff {
            if let Some(c) = char::from_u32(code) {
                assert_eq!(
                    dict.group(c).is_some(),
                    expected.contains_key(&c),
                    "U+{code:04X}"
                );
            }
        }
    }
}

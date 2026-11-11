// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 masanork
//! IVS-aware font subsetting, donated from GJS; see ORIGIN.md.
use wasm_bindgen::prelude::*;
use std::collections::BTreeMap;


// ─── Binary helpers ─────────────────────────────────────

fn u16be(data: &[u8], off: usize) -> u16 {
    ((data[off] as u16) << 8) | (data[off + 1] as u16)
}

fn u32be(data: &[u8], off: usize) -> u32 {
    ((data[off] as u32) << 24)
        | ((data[off + 1] as u32) << 16)
        | ((data[off + 2] as u32) << 8)
        | (data[off + 3] as u32)
}

fn u24be(data: &[u8], off: usize) -> u32 {
    ((data[off] as u32) << 16) | ((data[off + 1] as u32) << 8) | (data[off + 2] as u32)
}

fn is_variation_selector(cp: u32) -> bool {
    (0xFE00..=0xFE0F).contains(&cp) || (0xE0100..=0xE01EF).contains(&cp)
}

// ─── Cmap format 14 (IVS) parsing ───────────────────────

/// IVS map: variation_selector → { base_codepoint → glyph_id }
type IvsMap = BTreeMap<u32, BTreeMap<u32, u16>>;

/// Parse cmap format 14 from raw font binary to extract IVS mappings
fn parse_cmap_format14(data: &[u8]) -> IvsMap {
    let mut map = IvsMap::new();
    if data.len() < 12 { return map; }

    // Find cmap table
    let num_tables = u16be(data, 4) as usize;
    let mut cmap_off = 0usize;
    let mut cmap_len = 0usize;
    for i in 0..num_tables {
        let p = 12 + i * 16;
        if p + 16 > data.len() { return map; }
        if &data[p..p+4] == b"cmap" {
            cmap_off = u32be(data, p + 8) as usize;
            cmap_len = u32be(data, p + 12) as usize;
            break;
        }
    }
    if cmap_off == 0 || cmap_off + cmap_len > data.len() { return map; }

    let cmap = &data[cmap_off..cmap_off + cmap_len];
    if cmap.len() < 4 { return map; }
    let num_subtables = u16be(cmap, 2) as usize;

    // Find format 14 subtable
    let mut f14_off = 0usize;
    for i in 0..num_subtables {
        let p = 4 + i * 8;
        if p + 8 > cmap.len() { break; }
        let sub_off = u32be(cmap, p + 4) as usize;
        if sub_off + 2 > cmap.len() { continue; }
        if u16be(cmap, sub_off) == 14 {
            f14_off = sub_off;
            break;
        }
    }
    if f14_off == 0 { return map; }

    // Parse format 14
    if f14_off + 10 > cmap.len() { return map; }
    let num_var_selector_records = u32be(cmap, f14_off + 6) as usize;

    for i in 0..num_var_selector_records {
        let p = f14_off + 10 + i * 11;
        if p + 11 > cmap.len() { break; }
        let var_selector = u24be(cmap, p);
        let non_default_uvs_offset = u32be(cmap, p + 7) as usize;

        if non_default_uvs_offset != 0 {
            let nd_off = f14_off + non_default_uvs_offset;
            if nd_off + 4 > cmap.len() { continue; }
            let num_mappings = u32be(cmap, nd_off) as usize;
            let vs_map = map.entry(var_selector).or_default();
            for j in 0..num_mappings {
                let mp = nd_off + 4 + j * 5;
                if mp + 5 > cmap.len() { break; }
                let base_cp = u24be(cmap, mp);
                let glyph_id = u16be(cmap, mp + 3);
                vs_map.insert(base_cp, glyph_id);
            }
        }
    }
    map
}

// ─── Cmap format 14 generation ──────────────────────────

/// Generate a cmap format 14 subtable binary from IVS records.
/// `ivs_records`: non-default UVS entries (vs, base, glyph_id)
/// `default_uvs`: default UVS entries (vs, base) — tells browser to use the default glyph
#[cfg(test)]
fn generate_cmap_format14(ivs_records: &[(u32, u32, u16)], default_uvs: &[(u32, u32)]) -> Vec<u8> {
    // Group non-default by variation selector
    let mut nd_map: BTreeMap<u32, Vec<(u32, u16)>> = BTreeMap::new();
    for &(vs, base, gid) in ivs_records {
        nd_map.entry(vs).or_default().push((base, gid));
    }
    for mappings in nd_map.values_mut() {
        mappings.sort_by_key(|&(base, _)| base);
        mappings.dedup_by_key(|m| m.0);
    }

    // Group default UVS by variation selector
    let mut d_map: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &(vs, base) in default_uvs {
        d_map.entry(vs).or_default().push(base);
    }
    for bases in d_map.values_mut() {
        bases.sort();
        bases.dedup();
    }

    // Collect all VS values (union of non-default and default)
    let mut all_vs: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for k in nd_map.keys() { all_vs.insert(*k); }
    for k in d_map.keys() { all_vs.insert(*k); }

    let num_vs = all_vs.len();
    if num_vs == 0 { return Vec::new(); }

    let header_size = 10 + 11 * num_vs;

    // Calculate sizes of each subtable
    // Default UVS: 4 bytes (numRanges) + 4 bytes per range (UInt24 startUnicodeValue + uint8 additionalCount)
    // Each base codepoint becomes a single range (start=base, additionalCount=0)
    struct VsInfo {
        default_uvs_size: usize,
        non_default_uvs_size: usize,
    }
    let mut vs_info: Vec<(u32, VsInfo)> = Vec::new();
    let mut total_size = header_size;

    for &vs in &all_vs {
        let d_count = d_map.get(&vs).map_or(0, |v| v.len());
        let nd_count = nd_map.get(&vs).map_or(0, |v| v.len());
        let d_size = if d_count > 0 { 4 + 4 * d_count } else { 0 };
        let nd_size = if nd_count > 0 { 4 + 5 * nd_count } else { 0 };
        total_size += d_size + nd_size;
        vs_info.push((vs, VsInfo { default_uvs_size: d_size, non_default_uvs_size: nd_size }));
    }

    let mut buf = vec![0u8; total_size];
    // Format
    buf[0] = 0; buf[1] = 14;
    // Length (u32)
    let len = total_size as u32;
    buf[2] = (len >> 24) as u8; buf[3] = (len >> 16) as u8;
    buf[4] = (len >> 8) as u8; buf[5] = len as u8;
    // numVarSelectorRecords (u32)
    let n = num_vs as u32;
    buf[6] = (n >> 24) as u8; buf[7] = (n >> 16) as u8;
    buf[8] = (n >> 8) as u8; buf[9] = n as u8;

    // Write varSelectorRecords with offsets
    let mut sub_table_offset = header_size;
    let mut rec_offset = 10;

    for (vs, info) in &vs_info {
        // varSelector (UInt24)
        buf[rec_offset] = (vs >> 16) as u8;
        buf[rec_offset + 1] = (vs >> 8) as u8;
        buf[rec_offset + 2] = *vs as u8;

        // defaultUVSOffset
        if info.default_uvs_size > 0 {
            let off = sub_table_offset as u32;
            buf[rec_offset + 3] = (off >> 24) as u8;
            buf[rec_offset + 4] = (off >> 16) as u8;
            buf[rec_offset + 5] = (off >> 8) as u8;
            buf[rec_offset + 6] = off as u8;
        }
        // else defaultUVSOffset = 0 (already zeroed)

        sub_table_offset += info.default_uvs_size;

        // nonDefaultUVSOffset
        if info.non_default_uvs_size > 0 {
            let off = sub_table_offset as u32;
            buf[rec_offset + 7] = (off >> 24) as u8;
            buf[rec_offset + 8] = (off >> 16) as u8;
            buf[rec_offset + 9] = (off >> 8) as u8;
            buf[rec_offset + 10] = off as u8;
        }
        // else nonDefaultUVSOffset = 0 (already zeroed)

        sub_table_offset += info.non_default_uvs_size;
        rec_offset += 11;
    }

    // Write subtable data
    let mut write_off = header_size;

    for (vs, _info) in &vs_info {
        // Default UVS table
        if let Some(bases) = d_map.get(vs) {
            let num_ranges = bases.len() as u32;
            buf[write_off] = (num_ranges >> 24) as u8;
            buf[write_off + 1] = (num_ranges >> 16) as u8;
            buf[write_off + 2] = (num_ranges >> 8) as u8;
            buf[write_off + 3] = num_ranges as u8;
            write_off += 4;
            for &base in bases {
                // startUnicodeValue (UInt24)
                buf[write_off] = (base >> 16) as u8;
                buf[write_off + 1] = (base >> 8) as u8;
                buf[write_off + 2] = base as u8;
                // additionalCount (uint8) = 0 (single codepoint range)
                buf[write_off + 3] = 0;
                write_off += 4;
            }
        }

        // Non-default UVS table
        if let Some(mappings) = nd_map.get(vs) {
            let nm = mappings.len() as u32;
            buf[write_off] = (nm >> 24) as u8;
            buf[write_off + 1] = (nm >> 16) as u8;
            buf[write_off + 2] = (nm >> 8) as u8;
            buf[write_off + 3] = nm as u8;
            write_off += 4;
            for &(base, gid) in mappings {
                buf[write_off] = (base >> 16) as u8;
                buf[write_off + 1] = (base >> 8) as u8;
                buf[write_off + 2] = base as u8;
                buf[write_off + 3] = (gid >> 8) as u8;
                buf[write_off + 4] = gid as u8;
                write_off += 5;
            }
        }
    }

    buf
}

// ─── Font table injection ───────────────────────────────

fn checksum(data: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    let n_longs = data.len() / 4;
    for i in 0..n_longs {
        sum = sum.wrapping_add(u32be(data, i * 4));
    }
    let left = data.len() % 4;
    if left > 0 {
        let mut val: u32 = 0;
        for j in 0..left {
            val = (val << 8) + data[n_longs * 4 + j] as u32;
        }
        val <<= 8 * (4 - left);
        sum = sum.wrapping_add(val);
    }
    sum
}

/// Inject or replace a table in an OpenType font binary
fn inject_table(font_data: &[u8], table_tag: &[u8; 4], table_data: &[u8]) -> Vec<u8> {
    if font_data.len() < 12 { return font_data.to_vec(); }
    let num_tables = u16be(font_data, 4) as usize;

    // Collect existing tables
    let mut tables: BTreeMap<[u8; 4], Vec<u8>> = BTreeMap::new();
    for i in 0..num_tables {
        let p = 12 + i * 16;
        if p + 16 > font_data.len() { continue; }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&font_data[p..p+4]);
        let off = u32be(font_data, p + 8) as usize;
        let len = u32be(font_data, p + 12) as usize;
        if off + len <= font_data.len() {
            tables.insert(tag, font_data[off..off+len].to_vec());
        }
    }

    // Inject/replace
    tables.insert(*table_tag, table_data.to_vec());

    // Rebuild font
    let n = tables.len();
    let dir_size = 12 + 16 * n;
    let mut total = dir_size;
    for data in tables.values() {
        total += (data.len() + 3) & !3; // 4-byte aligned
    }

    let mut out = vec![0u8; total];
    // Copy sfnt version
    out[..4].copy_from_slice(&font_data[..4]);
    // numTables
    out[4] = (n >> 8) as u8;
    out[5] = n as u8;
    // searchRange, entrySelector, rangeShift (simplified)
    let mut search_range = 1u16;
    let mut entry_selector = 0u16;
    while (search_range * 2) <= n as u16 { search_range *= 2; entry_selector += 1; }
    search_range *= 16;
    let range_shift = (n as u16 * 16).wrapping_sub(search_range);
    out[6] = (search_range >> 8) as u8; out[7] = search_range as u8;
    out[8] = (entry_selector >> 8) as u8; out[9] = entry_selector as u8;
    out[10] = (range_shift >> 8) as u8; out[11] = range_shift as u8;

    let mut write_ptr = dir_size;
    let mut head_offset = 0usize;

    for (i, (tag, data)) in tables.iter().enumerate() {
        let dir_off = 12 + i * 16;
        // Align
        while write_ptr % 4 != 0 { write_ptr += 1; }

        out[dir_off..dir_off+4].copy_from_slice(tag);
        let cs = checksum(data);
        out[dir_off+4] = (cs >> 24) as u8; out[dir_off+5] = (cs >> 16) as u8;
        out[dir_off+6] = (cs >> 8) as u8; out[dir_off+7] = cs as u8;
        out[dir_off+8] = (write_ptr as u32 >> 24) as u8;
        out[dir_off+9] = (write_ptr as u32 >> 16) as u8;
        out[dir_off+10] = (write_ptr as u32 >> 8) as u8;
        out[dir_off+11] = write_ptr as u8;
        let dlen = data.len() as u32;
        out[dir_off+12] = (dlen >> 24) as u8; out[dir_off+13] = (dlen >> 16) as u8;
        out[dir_off+14] = (dlen >> 8) as u8; out[dir_off+15] = dlen as u8;

        if tag == b"head" { head_offset = write_ptr; }
        out[write_ptr..write_ptr + data.len()].copy_from_slice(data);
        write_ptr += data.len();
    }

    out.truncate(write_ptr);

    // Fix head checkSumAdjustment
    if head_offset > 0 && head_offset + 12 <= out.len() {
        // Zero out checkSumAdjustment first
        out[head_offset+8] = 0; out[head_offset+9] = 0;
        out[head_offset+10] = 0; out[head_offset+11] = 0;
        let whole_cs = checksum(&out);
        let adj = 0xB1B0AFBAu32.wrapping_sub(whole_cs);
        out[head_offset+8] = (adj >> 24) as u8; out[head_offset+9] = (adj >> 16) as u8;
        out[head_offset+10] = (adj >> 8) as u8; out[head_offset+11] = adj as u8;
    }

    out
}

// ─── Name table rewriting (license compliance) ─────────

/// Family name written into subset outputs (Name ID 1 / 16 / 21).
const SUBSET_FAMILY: &str = "IBS Subset";
/// PostScript-name prefix for subset outputs (Name ID 6 / 20 / 25).
const SUBSET_PS_PREFIX: &str = "IBSSubset";
/// Unique-ID prefix for subset outputs (Name ID 3).
const SUBSET_UNIQUE_PREFIX: &str = "IBS-Subset";

/// Name IDs kept verbatim for attribution: copyright (0), trademark (7),
/// manufacturer (8), designer (9), vendor URL (11), designer URL (12),
/// license (13), license URL (14).
const NAME_IDS_KEEP: &[u16] = &[0, 7, 8, 9, 11, 12, 13, 14];
/// Name IDs whose original value *is* a font name. Their values become
/// needles that must not survive anywhere else in the rewritten table.
const NAME_IDS_FONT_NAME: &[u16] = &[1, 4, 6, 16, 18, 20, 21];

struct NameRecord {
    platform_id: u16,
    encoding_id: u16,
    language_id: u16,
    name_id: u16,
    data: Vec<u8>,
}

impl NameRecord {
    /// Unicode (0) and Windows (3) platforms store UTF-16BE.
    fn is_utf16(&self) -> bool {
        self.platform_id == 0 || self.platform_id == 3
    }

    /// Decode to a Rust string when the encoding is unambiguous:
    /// UTF-16BE (platform 0/3) or pure ASCII bytes (any other platform,
    /// e.g. Mac Roman / Mac Japanese, which are ASCII-compatible).
    fn decode(&self) -> Option<String> {
        if self.is_utf16() {
            if self.data.len() % 2 != 0 { return None; }
            let units: Vec<u16> = self.data.chunks(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16(&units).ok()
        } else if self.data.is_ascii() {
            Some(String::from_utf8_lossy(&self.data).into_owned())
        } else {
            None
        }
    }

    /// Encode an ASCII string for this record's platform. ASCII is valid
    /// in UTF-16BE (after widening) and in every Mac script encoding.
    fn encode_ascii(&self, s: &str) -> Vec<u8> {
        debug_assert!(s.is_ascii());
        if self.is_utf16() {
            s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
        } else {
            s.as_bytes().to_vec()
        }
    }

    fn same_key(&self, other: &NameRecord) -> bool {
        self.platform_id == other.platform_id
            && self.encoding_id == other.encoding_id
            && self.language_id == other.language_id
    }
}

/// Rewrite the name table so a subset (a Modified Version under SIL OFL 1.1
/// §3 / a 派生プログラム under the IPA Font License §3-1-iv) never carries the
/// original font name, which may be a Reserved Font Name (e.g. acgjm's
/// "GJMincho" / "GJ明朝" / "追加文字行政事務標準明朝").
///
/// For every platform (Unicode / Mac / Windows) and language:
/// - ID 1, 16, 21 (family / typographic family / WWS family) → "IBS Subset"
/// - ID 2, 17 (subfamily) → kept, unless they contain an original name
///   (then "Regular"); ID 22 (WWS subfamily) likewise kept or dropped
/// - ID 4, 18 (full name) → "IBS Subset " + subfamily (17, else 2)
/// - ID 6, 20 (PostScript / CID findfont name) → "IBSSubset-" + ASCII
///   alphanumerics of the English subfamily
/// - ID 25 (variations PostScript prefix) → "IBSSubset"
/// - ID 3 (unique ID) → "IBS-Subset;" + version number + ";" + PostScript
///   name (no original name)
/// - ID 0, 7, 8, 9, 11, 12, 13, 14 (copyright / trademark / manufacturer /
///   designer / URLs / license) → kept verbatim for attribution
/// - Any other ID (version, description, sample text, ID ≥ 256, …) is kept
///   if it does not contain an original font name. If it does, UTF-16
///   records get the name replaced with "IBS Subset"; records that cannot
///   be decoded safely (non-ASCII Mac script encodings) are dropped.
///
/// If parsing fails, returns the original font data unchanged.
fn rewrite_name_table(font_data: &[u8]) -> Vec<u8> {
    // Find name table in the font directory
    if font_data.len() < 12 { return font_data.to_vec(); }
    let num_tables = u16be(font_data, 4) as usize;
    let mut name_off = 0usize;
    let mut name_len = 0usize;
    for i in 0..num_tables {
        let p = 12 + i * 16;
        if p + 16 > font_data.len() { return font_data.to_vec(); }
        if &font_data[p..p+4] == b"name" {
            name_off = u32be(font_data, p + 8) as usize;
            name_len = u32be(font_data, p + 12) as usize;
            break;
        }
    }
    if name_off == 0 || name_off + name_len > font_data.len() {
        return font_data.to_vec();
    }

    let name_tbl = &font_data[name_off..name_off + name_len];
    if name_tbl.len() < 6 { return font_data.to_vec(); }

    let _format = u16be(name_tbl, 0);
    let count = u16be(name_tbl, 2) as usize;
    let string_offset = u16be(name_tbl, 4) as usize;

    if name_tbl.len() < 6 + count * 12 { return font_data.to_vec(); }

    // Parse all name records
    let mut records: Vec<NameRecord> = Vec::with_capacity(count);
    for i in 0..count {
        let rp = 6 + i * 12;
        let platform_id = u16be(name_tbl, rp);
        let encoding_id = u16be(name_tbl, rp + 2);
        let language_id = u16be(name_tbl, rp + 4);
        let name_id = u16be(name_tbl, rp + 6);
        let length = u16be(name_tbl, rp + 8) as usize;
        let offset = u16be(name_tbl, rp + 10) as usize;

        let abs_off = string_offset + offset;
        if abs_off + length > name_tbl.len() { return font_data.to_vec(); }
        let data = name_tbl[abs_off..abs_off + length].to_vec();

        records.push(NameRecord { platform_id, encoding_id, language_id, name_id, data });
    }

    // Needles: every original font name (IDs 1/4/6/16/18/20/21) as text,
    // plus the same with spaces removed (PostScript-style spelling).
    let mut needles: Vec<String> = Vec::new();
    // Raw byte needles for non-UTF-16 records that cannot be decoded
    // (e.g. Mac Japanese "IPAmj明朝"), keyed by (platform, encoding).
    let mut raw_needles: Vec<(u16, u16, Vec<u8>)> = Vec::new();
    for rec in records.iter().filter(|r| NAME_IDS_FONT_NAME.contains(&r.name_id)) {
        if let Some(s) = rec.decode() {
            let s = s.trim().to_string();
            if s.is_empty() { continue; }
            let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            for n in [s, compact] {
                if !needles.contains(&n) { needles.push(n); }
            }
        } else if !rec.is_utf16() && !rec.data.is_empty() {
            raw_needles.push((rec.platform_id, rec.encoding_id, rec.data.clone()));
        }
    }
    // Longest first so replacement prefers the most specific match.
    needles.sort_by_key(|n| std::cmp::Reverse(n.chars().count()));

    let contains_original = |rec: &NameRecord| -> bool {
        match rec.decode() {
            Some(s) => needles.iter().any(|n| s.contains(n.as_str())),
            None => {
                raw_needles.iter().any(|(p, e, n)| {
                    *p == rec.platform_id && *e == rec.encoding_id && contains_bytes(&rec.data, n)
                }) || needles.iter().any(|n| n.is_ascii() && contains_bytes(&rec.data, n.as_bytes()))
            }
        }
    };

    // English (or first decodable ASCII) value of a name ID, used for the
    // PostScript name and unique ID which must be ASCII on every platform.
    let english = |name_id: u16| -> Option<String> {
        let rank = |r: &NameRecord| match (r.platform_id, r.language_id) {
            (3, 0x0409) => 0,
            (3, _) => 1,
            (0, _) => 2,
            (1, 0) => 3,
            _ => 4,
        };
        let mut cands: Vec<&NameRecord> = records.iter()
            .filter(|r| r.name_id == name_id && !contains_original(r))
            .collect();
        cands.sort_by_key(|r| rank(r));
        cands.iter().filter_map(|r| r.decode()).find(|s| s.is_ascii() && !s.trim().is_empty())
    };

    let english_subfamily = english(17).or_else(|| english(2));
    let ps_style: String = english_subfamily.as_deref()
        .map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Regular".to_string());
    let mut ps_name = format!("{}-{}", SUBSET_PS_PREFIX, ps_style);
    ps_name.truncate(63); // PostScript name limit (ASCII, so byte-safe)
    let version = english(5).and_then(|s| extract_version_number(&s));
    let unique_id = match &version {
        Some(v) => format!("{};{};{}", SUBSET_UNIQUE_PREFIX, v, ps_name),
        None => format!("{};{}", SUBSET_UNIQUE_PREFIX, ps_name),
    };

    // Subfamily bytes (in the record's own encoding) for the full name,
    // looked up per platform/encoding/language. ID 17 wins over ID 2.
    let subfamily_bytes = |rec: &NameRecord| -> Vec<u8> {
        for id in [17u16, 2] {
            if let Some(src) = records.iter().find(|r| {
                r.name_id == id && r.same_key(rec) && !r.data.is_empty() && !contains_original(r)
            }) {
                return src.data.clone();
            }
        }
        rec.encode_ascii(english_subfamily.as_deref().unwrap_or("Regular"))
    };

    let mut new_records: Vec<NameRecord> = Vec::with_capacity(records.len());
    for rec in records.iter() {
        let data: Option<Vec<u8>> = match rec.name_id {
            id if NAME_IDS_KEEP.contains(&id) => Some(rec.data.clone()),
            1 | 16 | 21 => Some(rec.encode_ascii(SUBSET_FAMILY)),
            4 | 18 => {
                let mut d = rec.encode_ascii(&format!("{} ", SUBSET_FAMILY));
                d.extend_from_slice(&subfamily_bytes(rec));
                Some(d)
            }
            6 | 20 => Some(rec.encode_ascii(&ps_name)),
            25 => Some(rec.encode_ascii(SUBSET_PS_PREFIX)),
            3 => Some(rec.encode_ascii(&unique_id)),
            2 | 17 => {
                if contains_original(rec) { Some(rec.encode_ascii("Regular")) } else { Some(rec.data.clone()) }
            }
            22 => {
                if contains_original(rec) { None } else { Some(rec.data.clone()) }
            }
            _ => {
                if !contains_original(rec) {
                    Some(rec.data.clone())
                } else if rec.is_utf16() {
                    rec.decode().map(|mut s| {
                        for n in needles.iter() {
                            s = s.replace(n.as_str(), SUBSET_FAMILY);
                        }
                        s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
                    })
                } else {
                    None
                }
            }
        };
        if let Some(data) = data {
            new_records.push(NameRecord {
                platform_id: rec.platform_id,
                encoding_id: rec.encoding_id,
                language_id: rec.language_id,
                name_id: rec.name_id,
                data,
            });
        }
    }
    let records = new_records;
    // Rebuild the name table
    let header_size = 6 + records.len() * 12;
    let mut string_data: Vec<u8> = Vec::new();
    let mut new_table = vec![0u8; header_size];

    // Format 0
    new_table[0] = 0; new_table[1] = 0;
    // Count
    let cnt = records.len() as u16;
    new_table[2] = (cnt >> 8) as u8; new_table[3] = cnt as u8;
    // String offset = header_size
    let so = header_size as u16;
    new_table[4] = (so >> 8) as u8; new_table[5] = so as u8;

    for (i, rec) in records.iter().enumerate() {
        let rp = 6 + i * 12;
        new_table[rp] = (rec.platform_id >> 8) as u8;
        new_table[rp + 1] = rec.platform_id as u8;
        new_table[rp + 2] = (rec.encoding_id >> 8) as u8;
        new_table[rp + 3] = rec.encoding_id as u8;
        new_table[rp + 4] = (rec.language_id >> 8) as u8;
        new_table[rp + 5] = rec.language_id as u8;
        new_table[rp + 6] = (rec.name_id >> 8) as u8;
        new_table[rp + 7] = rec.name_id as u8;
        let len = rec.data.len() as u16;
        new_table[rp + 8] = (len >> 8) as u8;
        new_table[rp + 9] = len as u8;
        let off = string_data.len() as u16;
        new_table[rp + 10] = (off >> 8) as u8;
        new_table[rp + 11] = off as u8;
        string_data.extend_from_slice(&rec.data);
    }

    new_table.extend_from_slice(&string_data);

    inject_table(font_data, b"name", &new_table)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Pull the first version-like token ("002.000", "2.003", "2025-09-12")
/// out of a Name ID 5 string such as "Version 2.003;hotconv 1.1.1".
fn extract_version_number(s: &str) -> Option<String> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let token: String = s[start..].chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    let token = token.trim_end_matches(['.', '-']).to_string();
    if token.is_empty() { None } else { Some(token) }
}

// ─── Brotli / WOFF2 ────────────────────────────────────

fn brotli_compress(data: &[u8], quality: u32) -> Vec<u8> {
    let mut output = Vec::new();
    let params = brotli::enc::BrotliEncoderParams {
        quality: quality as i32,
        ..Default::default()
    };
    brotli::BrotliCompress(&mut std::io::Cursor::new(data), &mut output, &params).ok();
    output
}

fn write_u32be(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_be_bytes());
}
fn write_u16be(buf: &mut Vec<u8>, val: u16) {
    buf.extend_from_slice(&val.to_be_bytes());
}

/// Encode raw OTF/TTF font data to WOFF2 format.
///
/// WOFF2 spec: <https://www.w3.org/TR/WOFF2/>
/// - Table directory lists each table with origLength and transformLength
/// - All table data is concatenated RAW (no per-table transform), then Brotli-compressed once
/// - transformLength == origLength when no table-specific transform is applied (flag bits 6-7 = 11)
fn encode_woff2(font_data: &[u8]) -> Vec<u8> {
    if font_data.len() < 12 { return Vec::new(); }
    let sfnt_version = u32be(font_data, 0);
    let num_tables = u16be(font_data, 4) as usize;

    struct TableEntry { tag: [u8; 4], orig_offset: usize, orig_length: usize }
    let mut tables = Vec::new();
    for i in 0..num_tables {
        let p = 12 + i * 16;
        if p + 16 > font_data.len() { return Vec::new(); }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&font_data[p..p+4]);
        tables.push(TableEntry { tag, orig_offset: u32be(font_data, p + 8) as usize, orig_length: u32be(font_data, p + 12) as usize });
    }
    tables.sort_by(|a, b| a.tag.cmp(&b.tag));

    const KNOWN_TAGS: &[[u8; 4]] = &[
        *b"cmap", *b"head", *b"hhea", *b"hmtx", *b"maxp", *b"name", *b"OS/2", *b"post",
        *b"cvt ", *b"fpgm", *b"glyf", *b"loca", *b"prep", *b"CFF ", *b"VORG", *b"EBDT",
        *b"EBLC", *b"gasp", *b"hdmx", *b"kern", *b"LTSH", *b"PCLT", *b"VDMX", *b"vhea",
        *b"vmtx", *b"BASE", *b"GDEF", *b"GPOS", *b"GSUB", *b"EBSC", *b"JSTF", *b"MATH",
        *b"CBDT", *b"CBLC", *b"COLR", *b"CPAL", *b"SVG ", *b"sbix", *b"acnt", *b"avar",
        *b"bdat", *b"bloc", *b"bsln", *b"cvar", *b"fdsc", *b"feat", *b"fmtx", *b"fvar",
        *b"gvar", *b"hsty", *b"just", *b"lcar", *b"mort", *b"morx", *b"opbd", *b"prop",
        *b"trak", *b"Zapf", *b"Silf", *b"Glat", *b"Gloc", *b"Feat", *b"Sill",
    ];

    fn encode_uintbase128(buf: &mut Vec<u8>, mut val: u32) {
        let mut temp = Vec::new();
        loop { temp.push((val & 0x7F) as u8); val >>= 7; if val == 0 { break; } }
        temp.reverse();
        for (i, b) in temp.iter().enumerate() {
            buf.push(if i < temp.len() - 1 { b | 0x80 } else { *b });
        }
    }

    // Build table directory and concatenate raw table data (no per-table transform)
    let mut table_dir = Vec::new();
    let mut raw_tables = Vec::new();

    for entry in &tables {
        let known_idx = KNOWN_TAGS.iter().position(|t| t == &entry.tag);
        let is_glyf_or_loca = entry.tag == *b"glyf" || entry.tag == *b"loca";

        // Flag bits 6-7: transform version
        // For glyf/loca: version 3 = null transform (no transform applied)
        // For other tables: version 0 = identity (no transform applied)
        let transform_version: u8 = if is_glyf_or_loca { 3 } else { 0 };

        let flags: u8 = if let Some(idx) = known_idx {
            ((idx as u8) & 0x3F) | (transform_version << 6)
        } else {
            (63 & 0x3F) | (transform_version << 6)
        };
        table_dir.push(flags);
        if known_idx.is_none() { table_dir.extend_from_slice(&entry.tag); }

        let end = entry.orig_offset + entry.orig_length;
        if end > font_data.len() { return Vec::new(); }
        let td = &font_data[entry.orig_offset..end];

        // origLength (always present)
        encode_uintbase128(&mut table_dir, entry.orig_length as u32);

        // transformLength is only present when the table has a non-null transform:
        // - glyf/loca: transform version 0 = transformed, version 3 = null (no transformLength)
        // - other tables: transform version 0 = null (no transformLength)
        // We use version 3 for glyf/loca and version 0 for others = no transforms at all,
        // so transformLength is NEVER written.

        // Concatenate raw table data (no padding in the compressed stream;
        // padding is only applied when reconstructing the sfnt)
        raw_tables.extend_from_slice(td);

    }

    // Single Brotli compression of the concatenated raw table data
    let compressed_data = brotli_compress(&raw_tables, 4);

    // totalSfntSize = sfnt header (12) + table directory (numTables * 16) + sum of 4-byte-padded table sizes
    let sfnt_header_size = (12 + num_tables * 16) as u32;
    let padded_total: u32 = tables.iter()
        .map(|e| (e.orig_length as u32 + 3) & !3)
        .sum();
    let total_sfnt_size = sfnt_header_size + padded_total;

    // WOFF2 header: 48 bytes
    let unpadded_size = 48 + table_dir.len() + compressed_data.len();
    // WOFF2 files must be 4-byte aligned — Chrome OTS rejects unaligned files
    let total_size = (unpadded_size + 3) & !3;
    let mut woff2 = Vec::with_capacity(total_size);
    write_u32be(&mut woff2, 0x774F4632);           // signature: "wOF2"
    write_u32be(&mut woff2, sfnt_version);           // flavor
    write_u32be(&mut woff2, total_size as u32);      // length
    write_u16be(&mut woff2, num_tables as u16);      // numTables
    write_u16be(&mut woff2, 0);                      // reserved
    write_u32be(&mut woff2, total_sfnt_size);        // totalSfntSize (reconstructed OTF size)
    write_u32be(&mut woff2, compressed_data.len() as u32); // totalCompressedSize
    write_u16be(&mut woff2, 1);                      // majorVersion
    write_u16be(&mut woff2, 0);                      // minorVersion
    write_u32be(&mut woff2, 0);                      // metaOffset
    write_u32be(&mut woff2, 0);                      // metaLength
    write_u32be(&mut woff2, 0);                      // metaOrigLength
    write_u32be(&mut woff2, 0);                      // privOffset
    write_u32be(&mut woff2, 0);                      // privLength
    woff2.extend_from_slice(&table_dir);
    woff2.extend_from_slice(&compressed_data);
    // Pad to 4-byte boundary
    while woff2.len() < total_size {
        woff2.push(0);
    }
    woff2
}

// ─── allsorts-based font subsetting ─────────────────────

use allsorts::binary::read::ReadScope;
use allsorts::font_data::FontData;
use allsorts::tables::cmap::{Cmap, CmapSubtable, EncodingId, PlatformId};
use allsorts::tables::FontTableProvider;
use allsorts::tag;

/// Find the best cmap subtable, preferring Format 12 (full Unicode) over Format 4 (BMP only).
/// This is essential for PUA Plane 16 (U+100000+) support.
fn find_best_cmap_subtable<'a>(cmap: &Cmap<'a>, cmap_data: &'a [u8]) -> Option<CmapSubtable<'a>> {
    // Priority 1: Unicode platform, full repertoire (encoding 4) — Format 12
    // Priority 2: Windows platform, full repertoire (encoding 10) — Format 12
    // Priority 3: Unicode platform, BMP (encoding 3) — Format 4
    // Priority 4: Windows platform, BMP (encoding 1) — Format 4
    // Priority 5: Any Unicode/Windows subtable
    let priorities: &[(PlatformId, EncodingId)] = &[
        (PlatformId::UNICODE, EncodingId(4)),   // Unicode full
        (PlatformId::WINDOWS, EncodingId(10)),   // Windows full
        (PlatformId::UNICODE, EncodingId(3)),    // Unicode BMP
        (PlatformId::WINDOWS, EncodingId(1)),    // Windows BMP
    ];

    for &(platform, encoding) in priorities {
        if let Some(record) = cmap.find_subtable(platform, encoding) {
            if let Ok(subtable) = ReadScope::new(cmap_data)
                .offset(record.offset as usize)
                .read::<CmapSubtable<'a>>()
            {
                return Some(subtable);
            }
        }
    }

    // Fallback: any Unicode or Windows subtable
    let record = cmap
        .find_subtable_for_platform(PlatformId::UNICODE)
        .or_else(|| cmap.find_subtable_for_platform(PlatformId::WINDOWS))?;
    ReadScope::new(cmap_data)
        .offset(record.offset as usize)
        .read::<CmapSubtable<'a>>()
        .ok()
}

/// Core subsetting logic shared by subset_font and subset_font_raw.
/// Returns raw OTF data (caller decides whether to WOFF2-compress).
fn subset_with_layout(font_data: &[u8], text: &str, requested_gids: &[u16]) -> Option<(Vec<u8>, BTreeMap<u16, u16>)> {
    use read_fonts::{collections::IntSet, types::GlyphId, FontRef, TableProvider};
    let font = FontRef::new(font_data).ok()?;
    font.maxp().ok()?;
    font.cmap().ok()?;
    // Close over all requested OpenType features, remap glyph IDs, and
    // preserve vertical metrics. The source cmap is not patched for IVS.
    let unicodes: IntSet<u32> = text.chars().map(u32::from).collect();
    let glyphs: IntSet<GlyphId> = requested_gids.iter().map(|&gid| GlyphId::new(gid as u32)).collect();
    let plan = skera::Plan::new(
        &glyphs, &unicodes, &font, skera::SubsetFlags::default(),
        &skera::DEFAULT_DROP_TABLES.iter().copied().collect(),
        &IntSet::all(), &IntSet::all(), &IntSet::all(), &IntSet::all(),
    );
    if plan.unicode_to_old_glyph_mapping().next().is_none() && !requested_gids.iter().any(|&id| id != 0) { return None; }
    let mapping = plan.old_to_new_glyph_mapping().filter_map(|(old, new)|
        Some((u16::try_from(old.to_u32()).ok()?, u16::try_from(new.to_u32()).ok()?))).collect();
    let subset = skera::subset_font(&font, &plan).ok()?;
    Some((rewrite_name_table(&subset), mapping))
}

fn do_subset(font_data: &[u8], text: &str) -> Vec<u8> {
    subset_with_layout(font_data, text, &[]).map_or_else(Vec::new, |(subset, _)| subset)
}

/// Subset a font without replacing base cmap entries with IVS glyphs.
/// The base cmap and the original variation sequences are kept separately.
/// Returned bytes are raw OTF/TTF (no WOFF2 compression).
fn do_subset_no_cmap4_patch(font_data: &[u8], text: &str) -> Vec<u8> {
    do_subset(font_data, text)
}

/// Subset a font without patching cmap4 for IVS overrides.
///
/// Intended for tooling that will post-process the subset (e.g. font
/// merge). Returns raw OTF data; the caller is responsible for any
/// further transformations.
pub fn subset_font_raw_no_cmap4_patch(font_data: &[u8], text: &str) -> Vec<u8> {
    do_subset_no_cmap4_patch(font_data, text)
}

/// Subset output prepared for downstream font-merge tooling.
///
/// `font_data` is raw OTF/TTF; `unicode_records` and `ivs_records` use the
/// **new (subset-local)** glyph IDs, obtained from the subset plan's glyph map.
pub struct SubsetForMerge {
    pub font_data: Vec<u8>,
    /// (codepoint, new_gid) for every base codepoint that mapped to a real glyph.
    pub unicode_records: Vec<(u32, u16)>,
    /// (variation_selector, base_codepoint, new_gid) for every IVS pair found in text.
    pub ivs_records: Vec<(u32, u32, u16)>,
}

/// Additional codepoint exposure for the merger.
/// `ivs_to_pua` maps `(base, variation_selector) → list of PUA codepoints`.
/// The IVS glyph in the input font (looked up via cmap14) will appear in
/// the merged font's cmap under each PUA codepoint. The IVS sequence
/// itself is also preserved (the same glyph is reachable via both).
///
/// Multiple PUA codepoints per (base, vs) are common: 行政事務標準文字
/// often assigns multiple PUA slots to characters that share a glyph form.
///
/// This is how Phase 5a re-exposes Jigmo's `u####-ueXXXX` glyphs as
/// 行政事務標準文字 entries at U+1XXXXX.
pub type IvsToPuaMap = std::collections::BTreeMap<(u32, u32), Vec<u32>>;

/// Companion to [`IvsToPuaMap`]: maps a base codepoint (no VS) to PUA
/// codepoints. Used for `alias_base` rows in `new-gj-mapping.csv` whose
/// GlyphWiki target is a plain `u####` (i.e. the base glyph itself, no IVS
/// variation needed).
pub type BaseToPuaMap = std::collections::BTreeMap<u32, Vec<u32>>;

/// Mirror an IVS GID under additional variation selectors without pulling
/// extra glyphs from the source font.
///
/// Key: `(base, alias_vs)` — the cmap14 slot to populate.
/// Value: `primary_vs` — the variation selector whose `(base, primary_vs)`
/// glyph is reused at `(base, alias_vs)`.
///
/// Intended use: KAGE-equivalent IVS pairs where Moji_Joho (MJ) and
/// Adobe-Japan1 (AJ1) collections happen to share a glyph form. Source fonts
/// like Jigmo register *both* selectors in cmap14 with distinct GIDs even
/// though the rendered glyph is identical, and pulling both GIDs through the
/// subsetter inflates `glyf` by ~1k entries per Jigmo file. This map lets the
/// caller request the alias slot in the *output* cmap14 without dragging the
/// alias GID through the glyph budget.
pub type IvsVsAliasMap = std::collections::BTreeMap<(u32, u32), u32>;

/// Subset a font for merging. Returns the subset bytes along with the
/// post-subset GID mappings needed to rebuild cmap and IVS tables in the
/// merged font.
///
/// Returns `None` if the input has no usable glyphs for the requested text.
pub fn subset_for_merge(font_data: &[u8], text: &str) -> Option<SubsetForMerge> {
    subset_for_merge_with_aliases(font_data, text, &IvsToPuaMap::new(), &BaseToPuaMap::new())
}

/// Like `subset_for_merge`, but additionally exposes specified IVS sequences
/// at PUA codepoints. The IVS glyph is collected during subsetting so it
/// survives in the output font, and the merger uses the PUA mapping to add
/// extra cmap entries.
pub fn subset_for_merge_with_pua(
    font_data: &[u8],
    text: &str,
    ivs_to_pua: &IvsToPuaMap,
) -> Option<SubsetForMerge> {
    subset_for_merge_with_aliases(font_data, text, ivs_to_pua, &BaseToPuaMap::new())
}

/// Back-compat wrapper: forwards to [`subset_for_merge_with_full_aliases`]
/// with no VS aliases.
pub fn subset_for_merge_with_aliases(
    font_data: &[u8],
    text: &str,
    ivs_to_pua: &IvsToPuaMap,
    base_to_pua: &BaseToPuaMap,
) -> Option<SubsetForMerge> {
    subset_for_merge_with_full_aliases(
        font_data,
        text,
        ivs_to_pua,
        base_to_pua,
        &IvsVsAliasMap::new(),
    )
}

/// Full form: expose both `(base, vs) → pua` and `base → pua` aliases, plus
/// VS aliases that mirror existing IVS GIDs under additional variation
/// selectors without pulling new glyphs from the source.
pub fn subset_for_merge_with_full_aliases(
    font_data: &[u8],
    text: &str,
    ivs_to_pua: &IvsToPuaMap,
    base_to_pua: &BaseToPuaMap,
    vs_aliases: &IvsVsAliasMap,
) -> Option<SubsetForMerge> {
    let ivs_map = parse_cmap_format14(font_data);

    let font = ReadScope::new(font_data).read::<FontData<'_>>().ok()?;
    let provider = font.table_provider(0).ok()?;
    let cmap_cow = match provider.table_data(tag::CMAP).ok()? {
        Some(d) => d.into_owned(),
        None => return None,
    };

    // Mirror text_to_glyph_ids logic but also remember (cp, orig_gid) and
    // (vs, cp, orig_gid) so we can map them to new GIDs after subsetting.
    let cmap = ReadScope::new(&cmap_cow).read::<Cmap<'_>>().ok()?;
    let subtable = find_best_cmap_subtable(&cmap, &cmap_cow)?;

    let mut glyph_ids: Vec<u16> = vec![0];
    let mut unicode_orig: Vec<(u32, u16)> = Vec::new();
    let mut ivs_orig: Vec<(u32, u32, u16)> = Vec::new();

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let cp = chars[i] as u32;
        if is_variation_selector(cp) { i += 1; continue; }

        let base_gid = subtable.map_glyph(cp).ok().flatten().unwrap_or(0);
        if base_gid != 0 {
            if !glyph_ids.contains(&base_gid) { glyph_ids.push(base_gid); }
            unicode_orig.push((cp, base_gid));
        }

        if i + 1 < chars.len() {
            let next_cp = chars[i + 1] as u32;
            if is_variation_selector(next_cp) {
                let mut found = false;
                if let Some(vs_map) = ivs_map.get(&next_cp) {
                    if let Some(&ivs_gid) = vs_map.get(&cp) {
                        if ivs_gid != 0 {
                            if !glyph_ids.contains(&ivs_gid) { glyph_ids.push(ivs_gid); }
                            ivs_orig.push((next_cp, cp, ivs_gid));
                            found = true;
                        }
                    }
                }
                // Default IVS (no non-default entry) → fall back to base glyph
                if !found && base_gid != 0 {
                    ivs_orig.push((next_cp, cp, base_gid));
                }
                i += 1; // skip VS
            }
        }
        i += 1;
    }

    // Phase 5a: collect PUA aliases for IVS glyphs the caller explicitly
    // wants exposed at extra codepoints. We pull the same glyph IDs that
    // would have been collected if the (base, vs) pair were in the text.
    let mut pua_orig: Vec<(u32, u16)> = Vec::new();
    let mut requested_total = 0usize;
    let mut pua_diag = (0usize, 0usize, 0usize); // (matched, ivs_missing_used_base, no_glyph)
    for ((base, vs), pua_cps) in ivs_to_pua {
        requested_total += pua_cps.len();
        // Skip if the alias already exists in input text (avoids duplicate work)
        let already = ivs_orig.iter().any(|&(v, b, _)| v == *vs && b == *base);
        let ivs_gid = if let Some(vs_map) = ivs_map.get(vs) {
            vs_map.get(base).copied().unwrap_or(0)
        } else { 0 };
        let target_gid = if ivs_gid != 0 {
            pua_diag.0 += pua_cps.len();
            ivs_gid
        } else {
            // No explicit IVS variant — fall back to base glyph
            let g = subtable.map_glyph(*base).ok().flatten().unwrap_or(0);
            if g != 0 { pua_diag.1 += pua_cps.len(); }
            g
        };
        if target_gid == 0 { pua_diag.2 += pua_cps.len(); continue; }
        if !glyph_ids.contains(&target_gid) { glyph_ids.push(target_gid); }
        if !already {
            // Also record the IVS so the merged font's cmap14 still works
            ivs_orig.push((*vs, *base, target_gid));
        }
        for &pua_cp in pua_cps {
            pua_orig.push((pua_cp, target_gid));
        }
    }
    if !ivs_to_pua.is_empty() {
        eprintln!(
            "    [pua-aliases] requested={} matched_ivs={} fallback_base={} glyph_missing={}",
            requested_total, pua_diag.0, pua_diag.1, pua_diag.2
        );
    }

    // VS aliases: populate additional cmap14 slots without pulling new glyphs
    // from the source font. For each `(base, alias_vs) → primary_vs`:
    //   1. If the source font's own cmap14 already routes `(alias_vs, base)`
    //      to a GID we've already collected (typically the base GID itself,
    //      common when the AJ1 selector renders identically to the base),
    //      reuse it. This preserves any base→IVS aliasing the source
    //      establishes — important for snapshot stability — at zero cost.
    //   2. Otherwise mirror the `(primary_vs, base)` GID we already have.
    //      This is the case that motivates this map: Jigmo registers AJ1
    //      selectors with distinct GIDs even when the rendered form is
    //      KAGE-equivalent to the MJ selector, and pulling those GIDs
    //      bloats `glyf` by ~1k entries per Jigmo file.
    //   3. If neither is available, drop the alias silently.
    let mut vs_alias_diag = (0usize, 0usize, 0usize); // (reused_source, mirrored_primary, dropped)
    if !vs_aliases.is_empty() {
        let mut alias_records: Vec<(u32, u32, u16)> = Vec::with_capacity(vs_aliases.len());
        for (&(base, alias_vs), &primary_vs) in vs_aliases {
            let source_alias_gid = ivs_map
                .get(&alias_vs)
                .and_then(|m| m.get(&base).copied())
                .unwrap_or(0);
            let reuse = source_alias_gid != 0 && glyph_ids.contains(&source_alias_gid);
            if reuse {
                alias_records.push((alias_vs, base, source_alias_gid));
                vs_alias_diag.0 += 1;
                continue;
            }
            let primary_gid = ivs_orig.iter().find_map(|&(v, b, g)| {
                if v == primary_vs && b == base { Some(g) } else { None }
            });
            match primary_gid {
                Some(g) => {
                    alias_records.push((alias_vs, base, g));
                    vs_alias_diag.1 += 1;
                }
                None => vs_alias_diag.2 += 1,
            }
        }
        ivs_orig.extend(alias_records);
        eprintln!(
            "    [vs-aliases] requested={} reused_source={} mirrored_primary={} dropped={}",
            vs_aliases.len(), vs_alias_diag.0, vs_alias_diag.1, vs_alias_diag.2
        );
    }

    // Phase 5a: alias_base rows. Like the IVS path above but the alias target
    // is the bare base glyph (no VS), so we just look up the base and re-expose
    // its glyph at the PUA codepoint(s).
    let mut base_alias_diag = (0usize, 0usize); // (matched, glyph_missing)
    let mut base_requested_total = 0usize;
    for (base, pua_cps) in base_to_pua {
        base_requested_total += pua_cps.len();
        let g = subtable.map_glyph(*base).ok().flatten().unwrap_or(0);
        if g == 0 { base_alias_diag.1 += pua_cps.len(); continue; }
        base_alias_diag.0 += pua_cps.len();
        if !glyph_ids.contains(&g) { glyph_ids.push(g); }
        for &pua_cp in pua_cps {
            pua_orig.push((pua_cp, g));
        }
    }
    if !base_to_pua.is_empty() {
        eprintln!(
            "    [pua-base-aliases] requested={} matched={} glyph_missing={}",
            base_requested_total, base_alias_diag.0, base_alias_diag.1
        );
    }

    if glyph_ids.iter().all(|&id| id == 0) { return None; }

    let (subset_data, gid_map) = subset_with_layout(font_data, text, &glyph_ids)?;

    // Merge the regular unicode mappings with the PUA aliases derived from
    // ivs_to_pua. Both lists go through the same gid_map remapping.
    let mut combined_unicode = unicode_orig;
    combined_unicode.extend(pua_orig);
    let unicode_records: Vec<(u32, u16)> = combined_unicode.into_iter()
        .filter_map(|(cp, orig)| gid_map.get(&orig).map(|&new_gid| (cp, new_gid)))
        .collect();
    let ivs_records: Vec<(u32, u32, u16)> = ivs_orig.into_iter()
        .filter_map(|(vs, cp, orig)| gid_map.get(&orig).map(|&new_gid| (vs, cp, new_gid)))
        .collect();

    Some(SubsetForMerge {
        font_data: subset_data,
        unicode_records,
        ivs_records,
    })
}

/// Subset a font to only include glyphs needed for the given text,
/// with IVS (cmap format 14) support, then compress to WOFF2 format.
#[wasm_bindgen]
pub fn subset_font(font_data: &[u8], text: &str) -> Vec<u8> {
    let subset_data = do_subset(font_data, text);
    if subset_data.is_empty() { return Vec::new(); }
    encode_woff2(&subset_data)
}

/// Subset a font without WOFF2 compression (returns raw OTF).
#[wasm_bindgen]
pub fn subset_font_raw(font_data: &[u8], text: &str) -> Vec<u8> {
    do_subset(font_data, text)
}

/// Encode raw font data to WOFF2 format.
#[wasm_bindgen]
pub fn compress_woff2(font_data: &[u8]) -> Vec<u8> {
    encode_woff2(font_data)
}

/// Check if a font has glyphs for all base characters in the text.
#[wasm_bindgen]
pub fn font_has_glyphs(font_data: &[u8], text: &str) -> bool {
    let font = match ReadScope::new(font_data).read::<FontData<'_>>() {
        Ok(f) => f,
        Err(_) => return false,
    };
    let provider = match font.table_provider(0) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let cmap_cow = match provider.table_data(tag::CMAP) {
        Ok(Some(data)) => data.into_owned(),
        _ => return false,
    };

    let cmap = match ReadScope::new(&cmap_cow).read::<Cmap<'_>>() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let subtable = match find_best_cmap_subtable(&cmap, &cmap_cow) {
        Some(s) => s,
        None => return false,
    };

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let cp = chars[i] as u32;
        if is_variation_selector(cp) { i += 1; continue; }
        match subtable.map_glyph(cp) {
            Ok(Some(gid)) if gid != 0 => {}
            _ => return false,
        }
        if i + 1 < chars.len() && is_variation_selector(chars[i + 1] as u32) { i += 1; }
        i += 1;
    }
    true
}

/// Load a bounded source incrementally, without retaining a second full
/// JavaScript copy while subsetting a large font in a Worker.
#[wasm_bindgen]
pub struct FontSubsetter {
    data: Vec<u8>,
    expected: usize,
}

#[wasm_bindgen]
impl FontSubsetter {
    #[wasm_bindgen(constructor)]
    pub fn new(size: u32) -> Result<FontSubsetter, String> {
        if size == 0 || size > 50 * 1024 * 1024 {
            return Err("Source font must be between 1 byte and 50 MiB".into());
        }
        let mut data = Vec::new();
        data.try_reserve_exact(size as usize).map_err(|_| "Could not allocate source font")?;
        Ok(Self { data, expected: size as usize })
    }

    pub fn append(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > self.expected - self.data.len() {
            return Err("Source font exceeds its declared size".into());
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }

    pub fn has_glyphs(&self, text: &str) -> bool {
        self.data.len() == self.expected && font_has_glyphs(&self.data, text)
    }

    pub fn subset(&self, text: &str, woff2: bool) -> Result<Vec<u8>, String> {
        if self.data.len() != self.expected {
            return Err("Source font is incomplete".into());
        }
        Ok(if woff2 { subset_font(&self.data, text) } else { subset_font_raw(&self.data, text) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_variation_selector() {
        assert!(is_variation_selector(0xFE00));
        assert!(is_variation_selector(0xFE0F));
        assert!(is_variation_selector(0xE0100));
        assert!(is_variation_selector(0xE01EF));
        assert!(!is_variation_selector(0x4E00));
    }

    #[test]
    fn test_empty_font() {
        assert!(!font_has_glyphs(&[], "漢"));
    }

    #[test]
    fn test_woff2_empty() {
        assert!(compress_woff2(&[]).is_empty());
    }

    #[test]
    fn test_generate_cmap_format14() {
        let records = vec![(0xE0100u32, 0x845Bu32, 2u16)];
        let data = generate_cmap_format14(&records, &[]);
        assert_eq!(u16be(&data, 0), 14); // format
        assert!(data.len() > 10);
    }

    /// Build a minimal sfnt with only a name table from
    /// `(platform, encoding, language, name_id, bytes)` records.
    fn build_name_font(recs: &[(u16, u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
        let header = 6 + recs.len() * 12;
        let mut nt = vec![0u8; header];
        nt[2..4].copy_from_slice(&(recs.len() as u16).to_be_bytes());
        nt[4..6].copy_from_slice(&(header as u16).to_be_bytes());
        let mut strings = Vec::new();
        for (i, (p, e, l, n, data)) in recs.iter().enumerate() {
            let rp = 6 + i * 12;
            nt[rp..rp + 2].copy_from_slice(&p.to_be_bytes());
            nt[rp + 2..rp + 4].copy_from_slice(&e.to_be_bytes());
            nt[rp + 4..rp + 6].copy_from_slice(&l.to_be_bytes());
            nt[rp + 6..rp + 8].copy_from_slice(&n.to_be_bytes());
            nt[rp + 8..rp + 10].copy_from_slice(&(data.len() as u16).to_be_bytes());
            nt[rp + 10..rp + 12].copy_from_slice(&(strings.len() as u16).to_be_bytes());
            strings.extend_from_slice(data);
        }
        nt.extend_from_slice(&strings);

        let mut font = vec![0u8; 28];
        font[0..4].copy_from_slice(&[0, 1, 0, 0]);
        font[4..6].copy_from_slice(&1u16.to_be_bytes());
        font[6..8].copy_from_slice(&16u16.to_be_bytes());
        font[12..16].copy_from_slice(b"name");
        font[20..24].copy_from_slice(&28u32.to_be_bytes());
        font[24..28].copy_from_slice(&(nt.len() as u32).to_be_bytes());
        font.extend_from_slice(&nt);
        font
    }

    /// Read back `(platform, encoding, language, name_id, bytes)` records.
    fn read_name_records(font: &[u8]) -> Vec<(u16, u16, u16, u16, Vec<u8>)> {
        let num_tables = u16be(font, 4) as usize;
        for i in 0..num_tables {
            let p = 12 + i * 16;
            if &font[p..p + 4] != b"name" { continue; }
            let off = u32be(font, p + 8) as usize;
            let len = u32be(font, p + 12) as usize;
            let nt = &font[off..off + len];
            let count = u16be(nt, 2) as usize;
            let so = u16be(nt, 4) as usize;
            return (0..count).map(|j| {
                let rp = 6 + j * 12;
                let l = u16be(nt, rp + 8) as usize;
                let o = so + u16be(nt, rp + 10) as usize;
                (u16be(nt, rp), u16be(nt, rp + 2), u16be(nt, rp + 4), u16be(nt, rp + 6), nt[o..o + l].to_vec())
            }).collect();
        }
        panic!("name table not found");
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    }

    fn decode_rec(p: u16, data: &[u8]) -> String {
        if p == 0 || p == 3 {
            let units: Vec<u16> = data.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(data).into_owned()
        }
    }

    fn find_name(recs: &[(u16, u16, u16, u16, Vec<u8>)], p: u16, l: u16, n: u16) -> Option<String> {
        recs.iter()
            .find(|r| r.0 == p && r.2 == l && r.3 == n)
            .map(|r| decode_rec(r.0, &r.4))
    }

    /// acgjm-like name table (SIL OFL 1.1, RFN "GJMincho" / "GJ明朝" /
    /// "追加文字行政事務標準明朝") on Mac Roman, Mac Japanese and Windows.
    fn acgjm_like_records() -> Vec<(u16, u16, u16, u16, Vec<u8>)> {
        let copyright = "Copyright © Digital Agency All Right Reserved.";
        let license = "This Font Software is licensed under the SIL Open Font License, Version 1.1.";
        // "追加文字行政事務標準明朝" in Shift_JIS (Mac Japanese, platform 1 / encoding 1)
        let sjis_family: Vec<u8> = vec![
            0x92, 0xC7, 0x89, 0xC1, 0x95, 0xB6, 0x8E, 0x9A, 0x8D, 0x73, 0x90, 0xAD,
            0x8E, 0x96, 0x96, 0xB1, 0x95, 0x57, 0x8F, 0x80, 0x96, 0xBE, 0x92, 0xA9,
        ];
        vec![
            (1, 0, 0, 0, b"Copyright Digital Agency".to_vec()),
            (1, 0, 0, 1, b"Add Character_GJMincho".to_vec()),
            (1, 0, 0, 2, b"Regular".to_vec()),
            (1, 0, 0, 3, b"Add Character_GJMincho Version 001.000".to_vec()),
            (1, 0, 0, 4, b"Add Character_GJMincho".to_vec()),
            (1, 0, 0, 5, b"Version 002.000".to_vec()),
            (1, 0, 0, 6, b"Add Character_GJMincho".to_vec()),
            (1, 1, 11, 1, sjis_family.clone()),
            (1, 1, 11, 2, b"Regular".to_vec()),
            (1, 1, 11, 4, sjis_family.clone()),
            (1, 1, 11, 10, [b"Description of ".to_vec(), sjis_family].concat()),
            (3, 1, 0x409, 0, utf16(copyright)),
            (3, 1, 0x409, 1, utf16("Add Character_GJMincho")),
            (3, 1, 0x409, 2, utf16("Regular")),
            (3, 1, 0x409, 3, utf16("Add Character_GJMincho Version 001.000")),
            (3, 1, 0x409, 4, utf16("Add Character_GJMincho")),
            (3, 1, 0x409, 5, utf16("Version 002.000;Add Character_GJMincho")),
            (3, 1, 0x409, 6, utf16("Add Character_GJMincho")),
            (3, 1, 0x409, 7, utf16("GJMincho is a trademark")),
            (3, 1, 0x409, 13, utf16(license)),
            (3, 1, 0x409, 14, utf16("https://openfontlicense.org")),
            (3, 1, 0x409, 16, utf16("Add Character_GJMincho")),
            (3, 1, 0x409, 17, utf16("Regular")),
            (3, 1, 0x409, 18, utf16("Add Character_GJMincho")),
            (3, 1, 0x409, 21, utf16("Add Character_GJMincho")),
            (3, 1, 0x409, 25, utf16("AddCharacter_GJMincho")),
            (3, 1, 0x409, 256, utf16("AddCharacter_GJMincho Swash")),
            (3, 1, 0x411, 1, utf16("追加文字行政事務標準明朝")),
            (3, 1, 0x411, 2, utf16("標準")),
            (3, 1, 0x411, 4, utf16("追加文字行政事務標準明朝")),
            (3, 1, 0x411, 10, utf16("追加文字行政事務標準明朝の説明")),
        ]
    }

    #[test]
    fn test_rewrite_name_table_replaces_original_names() {
        let original = acgjm_like_records();
        let result = rewrite_name_table(&build_name_font(&original));
        let recs = read_name_records(&result);

        // Windows English
        assert_eq!(find_name(&recs, 3, 0x409, 1).as_deref(), Some("IBS Subset"));
        assert_eq!(find_name(&recs, 3, 0x409, 2).as_deref(), Some("Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 3).as_deref(), Some("IBS-Subset;002.000;IBSSubset-Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 4).as_deref(), Some("IBS Subset Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 5).as_deref(), Some("Version 002.000;IBS Subset"));
        assert_eq!(find_name(&recs, 3, 0x409, 6).as_deref(), Some("IBSSubset-Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 16).as_deref(), Some("IBS Subset"));
        assert_eq!(find_name(&recs, 3, 0x409, 17).as_deref(), Some("Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 18).as_deref(), Some("IBS Subset Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 21).as_deref(), Some("IBS Subset"));
        assert_eq!(find_name(&recs, 3, 0x409, 25).as_deref(), Some("IBSSubset"));
        assert_eq!(find_name(&recs, 3, 0x409, 256).as_deref(), Some("IBS Subset Swash"));
        // Windows Japanese: localized subfamily is reused for the full name
        assert_eq!(find_name(&recs, 3, 0x411, 1).as_deref(), Some("IBS Subset"));
        assert_eq!(find_name(&recs, 3, 0x411, 4).as_deref(), Some("IBS Subset 標準"));
        assert_eq!(find_name(&recs, 3, 0x411, 10).as_deref(), Some("IBS Subsetの説明"));
        // Mac Roman / Mac Japanese
        assert_eq!(find_name(&recs, 1, 0, 1).as_deref(), Some("IBS Subset"));
        assert_eq!(find_name(&recs, 1, 0, 3).as_deref(), Some("IBS-Subset;002.000;IBSSubset-Regular"));
        assert_eq!(find_name(&recs, 1, 0, 4).as_deref(), Some("IBS Subset Regular"));
        assert_eq!(find_name(&recs, 1, 0, 6).as_deref(), Some("IBSSubset-Regular"));
        assert_eq!(find_name(&recs, 1, 11, 1).as_deref(), Some("IBS Subset"));
        assert_eq!(find_name(&recs, 1, 11, 4).as_deref(), Some("IBS Subset Regular"));
        // Undecodable Mac Japanese description containing the RFN is dropped
        assert!(find_name(&recs, 1, 11, 10).is_none());

        // Attribution records are kept byte-for-byte (even the trademark
        // notice that mentions the RFN).
        for keep_id in [0u16, 7, 13, 14] {
            for o in original.iter().filter(|r| r.3 == keep_id) {
                assert!(recs.iter().any(|r| r == o), "name ID {} must be kept verbatim", keep_id);
            }
        }

        // No original font name / RFN survives outside the kept IDs.
        let forbidden = ["GJMincho", "GJ明朝", "追加文字行政事務標準明朝", "Add Character", "AddCharacter"];
        let sjis_family = &original[7].4;
        for r in recs.iter().filter(|r| !NAME_IDS_KEEP.contains(&r.3)) {
            let s = decode_rec(r.0, &r.4);
            for f in forbidden {
                assert!(!s.contains(f), "name ID {} (platform {}) still contains {:?}: {:?}", r.3, r.0, f, s);
            }
            assert!(!contains_bytes(&r.4, sjis_family), "name ID {} still contains Shift_JIS family", r.3);
        }
    }

    #[test]
    fn test_rewrite_name_table_postscript_from_subfamily() {
        let original = vec![
            (3, 1, 0x409, 1, utf16("Noto Serif CJK JP")),
            (3, 1, 0x409, 2, utf16("Bold")),
            (3, 1, 0x409, 3, utf16("2.003;GOOG;NotoSerifCJKjp-Bold;ADOBE")),
            (3, 1, 0x409, 4, utf16("Noto Serif CJK JP Bold")),
            (3, 1, 0x409, 5, utf16("Version 2.003;hotconv 1.1.1;makeotfexe 2.6.0")),
            (3, 1, 0x409, 6, utf16("NotoSerifCJKjp-Bold")),
            (3, 1, 0x409, 16, utf16("Noto Serif CJK JP")),
            (3, 1, 0x409, 17, utf16("Semi Bold")),
        ];
        let recs = read_name_records(&rewrite_name_table(&build_name_font(&original)));
        assert_eq!(find_name(&recs, 3, 0x409, 2).as_deref(), Some("Bold"));
        assert_eq!(find_name(&recs, 3, 0x409, 4).as_deref(), Some("IBS Subset Semi Bold"));
        assert_eq!(find_name(&recs, 3, 0x409, 6).as_deref(), Some("IBSSubset-SemiBold"));
        assert_eq!(find_name(&recs, 3, 0x409, 3).as_deref(), Some("IBS-Subset;2.003;IBSSubset-SemiBold"));
        assert_eq!(find_name(&recs, 3, 0x409, 5).as_deref(), Some("Version 2.003;hotconv 1.1.1;makeotfexe 2.6.0"));
        assert_eq!(recs.len(), original.len());
    }

    #[test]
    fn test_rewrite_name_table_subfamily_containing_name() {
        // A subfamily that embeds the family name must not leak it.
        let original = vec![
            (3, 1, 0x409, 1, utf16("FooMincho")),
            (3, 1, 0x409, 2, utf16("FooMincho Regular")),
            (3, 1, 0x409, 4, utf16("FooMincho Regular")),
            (3, 1, 0x409, 6, utf16("FooMincho-Regular")),
        ];
        let recs = read_name_records(&rewrite_name_table(&build_name_font(&original)));
        assert_eq!(find_name(&recs, 3, 0x409, 2).as_deref(), Some("Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 4).as_deref(), Some("IBS Subset Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 6).as_deref(), Some("IBSSubset-Regular"));
        assert_eq!(find_name(&recs, 3, 0x409, 3), None);
    }

    #[test]
    fn test_extract_version_number() {
        assert_eq!(extract_version_number("Version 002.000").as_deref(), Some("002.000"));
        assert_eq!(extract_version_number("Version 2.003;hotconv 1.1.1").as_deref(), Some("2.003"));
        assert_eq!(extract_version_number("2025-09-12").as_deref(), Some("2025-09-12"));
        assert_eq!(extract_version_number("Version 1."), Some("1".to_string()));
        assert_eq!(extract_version_number("no digits"), None);
    }

    #[test]
    fn test_rewrite_name_table_empty_font() {
        // Should not panic on empty/invalid input
        let result = rewrite_name_table(&[]);
        assert!(result.is_empty());
        let result2 = rewrite_name_table(&[0; 5]);
        assert_eq!(result2.len(), 5);
    }

    #[test]
    fn test_generate_cmap_format14_all_non_default() {
        // Both VS3 and VS4 should be non-default (explicit glyph references)
        let records = vec![
            (0xE0102u32, 0x9BD6u32, 1u16), // VS3 → base glyph (月鯖)
            (0xE0103u32, 0x9BD6u32, 2u16), // VS4 → alt glyph (円鯖)
        ];
        let data = generate_cmap_format14(&records, &[]);
        assert_eq!(u16be(&data, 0), 14); // format
        // Should have 2 varSelectorRecords (VS3 and VS4)
        assert_eq!(u32be(&data, 6), 2);
    }
}

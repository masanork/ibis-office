// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! A dictionary installed at run time (セットアップ's larger edition, `shell::dataset`) replaces the bundled one for the name
//! comparison of `IBIS.FIELDMATCH`. A test binary of its own: it changes the process's dictionary.

use ibis_doc_model::variant_clusters::{self, VariantClusters};

/// A dictionary of one cluster, 一 and 丁 (not variants: only to see the swap), in the layout `VariantClusters::from_bytes` reads.
fn one_cluster() -> VariantClusters {
    let mut raw = Vec::new();
    for x in [2u32, 1, 1, 0] {
        raw.extend(x.to_le_bytes()); // count, groups, pages, reserved
    }
    let mut directory = vec![0u16; 4352];
    directory[0x4E] = 1; // the page of U+4E00..U+4EFF
    for d in directory {
        raw.extend(d.to_le_bytes());
    }
    raw.extend(0u32.to_le_bytes()); // rank before the page
    raw.extend(0b11u32.to_le_bytes()); // U+4E00 and U+4E01
    for _ in 1..8 {
        raw.extend(0u32.to_le_bytes());
    }
    for g in [0u16, 0] {
        raw.extend(g.to_le_bytes()); // the cluster of each member
    }
    for x in [0u32, 2, 0x4E00, 0x4E00, 0x4E01] {
        raw.extend(x.to_le_bytes()); // offsets, representative, members
    }
    VariantClusters::from_bytes(raw).expect("a valid dictionary")
}

fn field_match(a: &str, b: &str) -> ibis_calc::CellValue {
    let book = ibis_doc_model::Presentation::empty(ibis_doc_model::geometry::EmuSize::WIDESCREEN);
    let view = ibis_doc_model::sheet_calc::BookView::new(&book);
    ibis_calc::recalc::evaluate(&view, (0, 0, 0), &format!(r#"IBIS.FIELDMATCH("{a}","{b}","name")"#)).0
}

#[test]
fn an_installed_dictionary_replaces_the_bundled_one_and_the_bundled_one_comes_back() {
    let before = variant_clusters::generation();
    assert_eq!(field_match("一郎", "丁郎"), ibis_calc::CellValue::Number(0.0), "not variants in the bundled dictionary");

    let generation = variant_clusters::install(one_cluster());
    assert_eq!(generation, before + 1);
    assert_eq!(variant_clusters::active().map(|d| d.len()), Some(2));
    assert_eq!(field_match("一郎", "丁郎"), ibis_calc::CellValue::Number(1.0), "the installed dictionary is used");
    assert_eq!(field_match("髙橋", "高橋"), ibis_calc::CellValue::Number(0.0), "and the bundled one is not");

    assert!(variant_clusters::install_zst(b"not a dictionary").is_none());
    assert_eq!(variant_clusters::generation(), generation, "bytes that are not a dictionary change nothing");

    variant_clusters::reset_to_embedded();
    assert_eq!(field_match("一郎", "丁郎"), ibis_calc::CellValue::Number(0.0));
    if variant_clusters::is_embedded() {
        assert_eq!(field_match("髙橋", "高橋"), ibis_calc::CellValue::Number(1.0));
    }
}

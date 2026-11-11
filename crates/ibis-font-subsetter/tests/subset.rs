// SPDX-License-Identifier: AGPL-3.0-only
use ibis_font_subsetter::{FontSubsetter, font_has_glyphs, subset_font_raw};
use ttf_parser::Face;

const TTF: &[u8] = include_bytes!("fonts/ivs.ttf");
const CFF: &[u8] = include_bytes!("fonts/ivs-cff.otf");

#[test]
fn preserves_bare_base_and_distinct_bmp_and_supplementary_variants() {
    for input in [TTF, CFF] {
        let output = subset_font_raw(input, "AA\u{fe00}A\u{e0100}");
        let original = Face::parse(input, 0).unwrap();
        let subset = Face::parse(&output, 0).unwrap();
        let base = subset.glyph_index('A').unwrap();
        let bmp = subset.glyph_variation_index('A', '\u{fe00}').unwrap();
        let supplementary = subset.glyph_variation_index('A', '\u{e0100}').unwrap();
        assert_ne!(base, bmp);
        assert_ne!(base, supplementary);
        assert_ne!(bmp, supplementary);
        for (new, old) in [(base, original.glyph_index('A').unwrap()),
            (bmp, original.glyph_variation_index('A', '\u{fe00}').unwrap()),
            (supplementary, original.glyph_variation_index('A', '\u{e0100}').unwrap())] {
            assert_eq!(subset.glyph_bounding_box(new), original.glyph_bounding_box(old));
            assert_eq!(subset.glyph_hor_advance(new), original.glyph_hor_advance(old));
        }
    }
}

#[test]
fn retains_composite_dependencies_and_supplementary_codepoints() {
    let output = subset_font_raw(TTF, "D\u{20000}\u{100001}");
    let original = Face::parse(TTF, 0).unwrap();
    let subset = Face::parse(&output, 0).unwrap();
    for cp in ['D', '\u{20000}', '\u{100001}'] {
        let new = subset.glyph_index(cp).unwrap();
        let old = original.glyph_index(cp).unwrap();
        assert_eq!(subset.glyph_bounding_box(new), original.glyph_bounding_box(old));
        assert_eq!(subset.glyph_hor_advance(new), original.glyph_hor_advance(old));
    }
    assert!(subset.number_of_glyphs() >= 5);
}

#[test]
fn streamed_input_agrees_with_direct_subsetting_and_checks_lengths() {
    let mut engine = FontSubsetter::new(TTF.len() as u32).unwrap();
    assert!(engine.subset("A", false).is_err());
    for chunk in TTF.chunks(17) { engine.append(chunk).unwrap(); }
    assert!(engine.append(&[0]).is_err());
    assert!(engine.has_glyphs("Am"));
    assert!(!engine.has_glyphs("Z"));
    assert_eq!(engine.subset("AA\u{fe00}", false).unwrap(), subset_font_raw(TTF, "AA\u{fe00}"));
    assert!(FontSubsetter::new(0).is_err());
    assert!(FontSubsetter::new(50 * 1024 * 1024 + 1).is_err());
}

#[test]
fn invalid_and_unmatched_fonts_do_not_produce_a_subset() {
    for input in [&[][..], b"not a font", &TTF[..12]] {
        assert!(!font_has_glyphs(input, "A"));
        assert!(subset_font_raw(input, "A").is_empty());
    }
    assert!(subset_font_raw(TTF, "Z").is_empty());
    assert!(!font_has_glyphs(TTF, "Z"));
}

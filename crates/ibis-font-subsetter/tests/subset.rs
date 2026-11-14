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

fn shape_glyphs(font_data: &[u8], text: &str, vertical: bool, liga: bool) -> Vec<(u32, i32, i32, i32, i32)> {
    use harfrust::{Direction, Feature, FontRef, ShaperData, Tag, UnicodeBuffer};
    let font = FontRef::from_index(font_data, 0).unwrap();
    let shaper = ShaperData::new(&font);
    let shaper = shaper.shaper(&font).build();
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(if vertical { Direction::TopToBottom } else { Direction::LeftToRight });
    buffer.guess_segment_properties();
    let features = [Feature::new(Tag::new(b"vert"), u32::from(vertical), ..), Feature::new(Tag::new(b"liga"), u32::from(liga), ..)];
    let result = shaper.shape(buffer, &features);
    result.glyph_infos().iter().zip(result.glyph_positions()).map(|(g,p)|
        (g.glyph_id, p.x_advance, p.y_advance, p.x_offset, p.y_offset)).collect()
}

#[test]
fn preserves_vertical_and_ligature_shaping_and_metrics_for_ttf_and_cff() {
    for input in [TTF, CFF] {
        let output = subset_font_raw(input, "AA");
        let original = Face::parse(input, 0).unwrap();
        let subset = Face::parse(&output, 0).unwrap();
        assert!(subset.tables().gsub.is_some(), "GSUB must remain in the subset");
        assert!(subset.tables().gpos.is_some(), "GPOS must remain in the subset");
        for (text, vertical, liga) in [("A", false, false), ("A", true, false), ("AA", false, true), ("AA", false, false)] {
            let before = shape_glyphs(input, text, vertical, liga);
            let after = shape_glyphs(&output, text, vertical, liga);
            assert_eq!(before.len(), after.len(), "{text:?} vertical={vertical} liga={liga}");
            for (old, new) in before.iter().zip(&after) {
                assert_eq!((old.1,old.2,old.3,old.4), (new.1,new.2,new.3,new.4), "position {text:?} vertical={vertical} liga={liga}");
                assert_eq!(original.glyph_bounding_box(ttf_parser::GlyphId(old.0 as u16)), subset.glyph_bounding_box(ttf_parser::GlyphId(new.0 as u16)), "outline {text:?} vertical={vertical} liga={liga}");
                assert_eq!(original.glyph_ver_advance(ttf_parser::GlyphId(old.0 as u16)), subset.glyph_ver_advance(ttf_parser::GlyphId(new.0 as u16)));
                assert_eq!(original.glyph_y_origin(ttf_parser::GlyphId(old.0 as u16)), subset.glyph_y_origin(ttf_parser::GlyphId(new.0 as u16)));
            }
        }
        assert_ne!(shape_glyphs(&output, "A", true, false)[0].0, shape_glyphs(&output, "A", false, false)[0].0);
        assert_eq!(shape_glyphs(&output, "AA", false, true).len(), 1);
    }
}

#[test]
fn real_japanese_fonts_preserve_horizontal_and_vertical_shaping_when_available() {
    let Some(dir) = std::env::var_os("IBIS_TEST_FONT_DIR") else { return };
    for filename in ["ipamjm.ttf", "NotoSansCJKjp-Regular.otf"] {
        let bytes = std::fs::read(std::path::Path::new(&dir).join(filename)).unwrap();
        let text = "、。「」（）ー漢m";
        let output = subset_font_raw(&bytes, text);
        let original = Face::parse(&bytes, 0).unwrap();
        let subset = Face::parse(&output, 0).unwrap();
        for vertical in [false, true] {
            let before = shape_glyphs(&bytes, text, vertical, true);
            let after = shape_glyphs(&output, text, vertical, true);
            assert_eq!(before.len(), after.len(), "{filename} vertical={vertical}");
            for (old, new) in before.iter().zip(&after) {
                let old_id = ttf_parser::GlyphId(old.0 as u16);
                let new_id = ttf_parser::GlyphId(new.0 as u16);
                assert_eq!((old.1,old.2,old.3,old.4), (new.1,new.2,new.3,new.4), "{filename} vertical={vertical}");
                assert_eq!(original.glyph_bounding_box(old_id), subset.glyph_bounding_box(new_id), "{filename} vertical={vertical}");
                assert_eq!(original.glyph_ver_advance(old_id), subset.glyph_ver_advance(new_id), "{filename}");
            }
        }
        assert!(subset.tables().gsub.is_some(), "{filename} has no GSUB");
        // Both deployed Japanese faces contain this non-default variation.
        let ivs_output = subset_font_raw(&bytes, "侮侮\u{fe00}");
        let ivs = Face::parse(&ivs_output, 0).unwrap();
        for (old, new) in [
            (original.glyph_index('侮').unwrap(), ivs.glyph_index('侮').unwrap()),
            (original.glyph_variation_index('侮', '\u{fe00}').unwrap(), ivs.glyph_variation_index('侮', '\u{fe00}').unwrap()),
        ] {
            assert_eq!(original.glyph_bounding_box(old), ivs.glyph_bounding_box(new), "{filename} IVS");
        }
    }
}

#[test]
fn merge_apis_preserve_shaping_and_report_remapped_alias_glyphs() {
    use ibis_font_subsetter::{subset_for_merge_with_full_aliases, subset_font_raw_no_cmap4_patch, BaseToPuaMap, IvsToPuaMap, IvsVsAliasMap};
    for input in [TTF, CFF] {
        let original = Face::parse(input, 0).unwrap();
        let normal = subset_font_raw_no_cmap4_patch(input, "AA\u{fe00}");
        assert!(Face::parse(&normal, 0).unwrap().tables().gsub.is_some());
        let mut ivs_aliases = IvsToPuaMap::new();
        ivs_aliases.insert((u32::from('A'), 0xfe00), vec![0x100010]);
        let mut base_aliases = BaseToPuaMap::new();
        base_aliases.insert(u32::from('C'), vec![0x100011]);
        let result = subset_for_merge_with_full_aliases(input, "AA\u{fe00}", &ivs_aliases, &base_aliases, &IvsVsAliasMap::new()).unwrap();
        let subset = Face::parse(&result.font_data, 0).unwrap();
        assert!(subset.tables().gsub.is_some());
        for (cp, old) in [(0x100010, original.glyph_variation_index('A', '\u{fe00}').unwrap()),
                          (0x100011, original.glyph_index('C').unwrap())] {
            let new = result.unicode_records.iter().find(|(value,_)| *value == cp).unwrap().1;
            assert_eq!(original.glyph_bounding_box(old), subset.glyph_bounding_box(ttf_parser::GlyphId(new)));
        }
        let variant = result.ivs_records.iter().find(|(vs,cp,_)| *vs==0xfe00 && *cp==u32::from('A')).unwrap().2;
        assert_eq!(original.glyph_bounding_box(original.glyph_variation_index('A', '\u{fe00}').unwrap()), subset.glyph_bounding_box(ttf_parser::GlyphId(variant)));
    }
}

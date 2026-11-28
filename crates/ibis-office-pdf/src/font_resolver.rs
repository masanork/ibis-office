// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! System font fallback resolver for non-embedded PDF fonts via GPUI [`TextSystem`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui_kit::{Font, FontStyle, FontWeight, SharedString, TextSystem};
use ibis_pdf::interpret::{CidCollection, FallbackFontRequest, FontResolver};

const JA_SERIF_CANDIDATES: &[&str] = &[
    "Hiragino Mincho ProN",
    "Hiragino Mincho Pro",
    "YuMincho",
    "Yu Mincho",
    "游明朝",
    "MS PMincho",
    "MS Mincho",
    "ＭＳ 明朝",
    "ＭＳ Ｐ明朝",
    "BIZ UDMincho",
    "BIZ UDMincho Medium",
    "Noto Serif CJK JP",
    "Noto Serif JP",
    "IPAexMincho",
    "IPAMincho",
    "TakaoMincho",
    "TakaoPMincho",
    "Shippori Mincho",
    "ACGJMincho",
];

const JA_SANS_CANDIDATES: &[&str] = &[
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Hiragino Kaku Gothic Pro",
    "YuGothic",
    "Yu Gothic",
    "游ゴシック",
    "Meiryo",
    "メイリオ",
    "MS PGothic",
    "MS Gothic",
    "ＭＳ ゴシック",
    "ＭＳ Ｐゴシック",
    "BIZ UDGothic",
    "Noto Sans CJK JP",
    "Noto Sans JP",
    "IPAexGothic",
    "IPAGothic",
    "TakaoPGothic",
    "TakaoGothic",
    "VL PGothic",
];

const SC_CANDIDATES: &[&str] = &[
    "PingFang SC",
    "STSong",
    "STHeiti",
    "SimSun",
    "NSimSun",
    "Microsoft YaHei",
    "SimHei",
    "Noto Sans CJK SC",
    "Noto Serif CJK SC",
    "WenQuanYi Micro Hei",
];

const TC_CANDIDATES: &[&str] = &[
    "PingFang TC",
    "STSong",
    "STHeiti",
    "PMingLiU",
    "MingLiU",
    "Microsoft JhengHei",
    "Noto Sans CJK TC",
    "Noto Serif CJK TC",
];

const KR_CANDIDATES: &[&str] = &[
    "Apple SD Gothic Neo",
    "AppleMyungjo",
    "Malgun Gothic",
    "Batang",
    "Dotum",
    "Gulim",
    "Noto Sans CJK KR",
    "Noto Serif CJK KR",
];

const CMAP_SUFFIXES: &[&str] = &[
    "-UniJIS-UCS2-H",
    "-UniJIS-UCS2-V",
    "-UniJIS-UTF16-H",
    "-UniJIS-UTF16-V",
    "-UniJIS-X0213-H",
    "-90ms-RKSJ-H",
    "-90ms-RKSJ-V",
    "-90msp-RKSJ-H",
    "-90msp-RKSJ-V",
    "-UniGB-UCS2-H",
    "-UniGB-UTF16-H",
    "-UniCNS-UCS2-H",
    "-UniCNS-UTF16-H",
    "-UniKS-UCS2-H",
    "-UniKS-UTF16-H",
    "-Identity-H",
    "-Identity-V",
    "-H",
    "-V",
];

const STYLE_SUFFIXES: &[&str] = &[
    "-Regular",
    "-Bold",
    "-Italic",
    "-BoldItalic",
    "-W3",
    "-W4",
    "-W5",
    "-W6",
    "-W7",
    "-W8",
    ",Bold",
    ",Italic",
    ",BoldItalic",
];

struct ResolverCache {
    generation: u64,
    name_map: HashMap<String, SharedString>,
    resolved: HashMap<FallbackFontRequest, Option<(Arc<Vec<u8>>, u32)>>,
}

/// Build a [`FontResolver`] backed by GPUI's [`TextSystem`], caching resolved
/// font bytes across pages and refreshing when `fonts_generation()` changes.
pub(crate) fn build_font_resolver(text_system: Arc<TextSystem>) -> FontResolver {
    let cache = Mutex::new(ResolverCache {
        generation: u64::MAX,
        name_map: HashMap::new(),
        resolved: HashMap::new(),
    });

    Arc::new(move |req: &FallbackFontRequest| {
        let current_gen = ibis_office_render::shape_view::fonts_generation();
        let name_map = {
            let mut guard = cache.lock().expect("pdf font resolver cache poisoned");
            if guard.generation != current_gen {
                guard.name_map = text_system
                    .all_font_names()
                    .into_iter()
                    .map(|name| (name.to_ascii_lowercase(), SharedString::from(name)))
                    .collect();
                guard.resolved.clear();
                guard.generation = current_gen;
            }
            if let Some(cached) = guard.resolved.get(req) {
                return cached.clone();
            }
            guard.name_map.clone()
        };

        let result = resolve_from_text_system(&text_system, &name_map, req);

        let mut guard = cache.lock().expect("pdf font resolver cache poisoned");
        if guard.generation == current_gen {
            guard.resolved.insert(req.clone(), result.clone());
        }
        result
    })
}

fn resolve_from_text_system(
    text_system: &TextSystem,
    name_map: &HashMap<String, SharedString>,
    req: &FallbackFontRequest,
) -> Option<(Arc<Vec<u8>>, u32)> {
    let weight = FontWeight(req.font_weight.clamp(100, 900) as f32);
    let style = if req.is_italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };

    for family in candidate_families(req, name_map) {
        let font = Font {
            family,
            weight,
            style,
            ..Default::default()
        };
        let font_id = text_system.resolve_font(&font);
        if let Some(data) = text_system.font_data(font_id) {
            return Some(data);
        }
    }
    None
}

fn strip_pdf_font_suffixes(raw: &str) -> Vec<String> {
    let mut base = raw;
    for suffix in CMAP_SUFFIXES {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped;
            break;
        }
    }
    let mut variants = Vec::new();
    let push_variant = |v: &str, out: &mut Vec<String>| {
        if !v.is_empty() && !out.iter().any(|existing| existing == v) {
            out.push(v.to_string());
        }
        let spaced = v.replace('-', " ");
        if !spaced.is_empty() && !out.iter().any(|existing| existing == &spaced) {
            out.push(spaced);
        }
    };
    push_variant(base, &mut variants);
    for suffix in STYLE_SUFFIXES {
        if let Some(stripped) = base.strip_suffix(suffix) {
            push_variant(stripped, &mut variants);
        }
    }
    variants
}

fn looks_japanese(req: &FallbackFontRequest) -> bool {
    if matches!(req.cid_collection, Some(CidCollection::Japan1)) {
        return true;
    }
    for name in [
        req.post_script_name.as_deref(),
        req.font_name.as_deref(),
        req.font_family.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let lower = name.to_ascii_lowercase();
        if lower.contains("heisei")
            || lower.contains("mincho")
            || lower.contains("gothic")
            || lower.contains("ryumin")
            || lower.contains("kozmin")
            || lower.contains("kozgo")
            || lower.contains("hiragino")
            || lower.contains("meiryo")
            || lower.contains("yumin")
            || lower.contains("yugo")
            || lower.contains("unijis")
            || lower.contains("rksj")
            || name.contains("明朝")
            || name.contains("ゴシック")
        {
            return true;
        }
    }
    false
}

fn prefers_serif_ja(req: &FallbackFontRequest) -> bool {
    let mut saw_gothic = false;
    let mut saw_mincho = false;
    for name in [
        req.post_script_name.as_deref(),
        req.font_name.as_deref(),
        req.font_family.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let lower = name.to_ascii_lowercase();
        if lower.contains("gothic")
            || lower.contains("kakugo")
            || lower.contains("kozgo")
            || lower.contains("midashigo")
            || lower.contains("meiryo")
            || lower.contains("sans")
            || name.contains("ゴシック")
        {
            saw_gothic = true;
        }
        if lower.contains("min")
            || lower.contains("ryumin")
            || lower.contains("serif")
            || name.contains("明朝")
        {
            saw_mincho = true;
        }
    }
    if saw_gothic && !saw_mincho {
        false
    } else if saw_mincho {
        true
    } else {
        req.is_serif
    }
}

fn candidate_families(
    req: &FallbackFontRequest,
    name_map: &HashMap<String, SharedString>,
) -> Vec<SharedString> {
    let mut out = Vec::new();
    let mut push_if_present = |name: &str| {
        if let Some(resolved) = name_map.get(&name.to_ascii_lowercase())
            && !out.contains(resolved)
        {
            out.push(resolved.clone());
        }
    };

    for raw in [
        req.font_family.as_deref(),
        req.font_name.as_deref(),
        req.post_script_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        for variant in strip_pdf_font_suffixes(raw) {
            push_if_present(&variant);
        }
    }

    if looks_japanese(req) {
        let (first, second) = if prefers_serif_ja(req) {
            (JA_SERIF_CANDIDATES, JA_SANS_CANDIDATES)
        } else {
            (JA_SANS_CANDIDATES, JA_SERIF_CANDIDATES)
        };
        for name in first.iter().chain(second.iter()) {
            push_if_present(name);
        }
    } else if let Some(cc) = &req.cid_collection {
        let list: &[&str] = match cc {
            CidCollection::GB1 => SC_CANDIDATES,
            CidCollection::CNS1 => TC_CANDIDATES,
            CidCollection::Korea1 => KR_CANDIDATES,
            CidCollection::Japan1 | CidCollection::Identity | CidCollection::Custom { .. } => &[],
        };
        for name in list {
            push_if_present(name);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req_for(name: &str, collection: Option<CidCollection>, is_serif: bool) -> FallbackFontRequest {
        FallbackFontRequest {
            post_script_name: Some(name.to_string()),
            font_name: None,
            font_family: None,
            font_weight: 400,
            is_fixed_pitch: false,
            is_serif,
            is_italic: false,
            is_bold: false,
            cid_collection: collection,
        }
    }

    #[test]
    fn strip_suffixes_handles_cmap_and_style() {
        assert_eq!(
            strip_pdf_font_suffixes("HeiseiMin-W3-UniJIS-UCS2-H"),
            vec!["HeiseiMin-W3", "HeiseiMin W3", "HeiseiMin", "HeiseiMin"]
                .into_iter()
                .fold(Vec::<String>::new(), |mut acc, s| {
                    if !acc.iter().any(|x| x == s) {
                        acc.push(s.to_string());
                    }
                    acc
                })
        );
    }

    #[test]
    fn japan1_mincho_prefers_serif_over_gothic() {
        let name_map: HashMap<String, SharedString> = [
            ("hiragino sans", SharedString::from("Hiragino Sans")),
            ("hiragino mincho pron", SharedString::from("Hiragino Mincho ProN")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        let req = req_for("HeiseiMin-W3-UniJIS-UCS2-H", Some(CidCollection::Japan1), false);
        let candidates = candidate_families(&req, &name_map);
        assert_eq!(
            candidates,
            vec![
                SharedString::from("Hiragino Mincho ProN"),
                SharedString::from("Hiragino Sans"),
            ]
        );
    }

    #[test]
    fn japan1_gothic_prefers_sans_over_mincho() {
        let name_map: HashMap<String, SharedString> = [
            ("biz udmincho", SharedString::from("BIZ UDMincho")),
            ("biz udgothic", SharedString::from("BIZ UDGothic")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        let req = req_for("HeiseiKakuGo-W5-UniJIS-UCS2-H", Some(CidCollection::Japan1), false);
        let candidates = candidate_families(&req, &name_map);
        assert_eq!(
            candidates,
            vec![
                SharedString::from("BIZ UDGothic"),
                SharedString::from("BIZ UDMincho"),
            ]
        );
    }
}

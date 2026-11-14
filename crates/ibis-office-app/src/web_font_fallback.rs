// SPDX-License-Identifier: AGPL-3.0-only
//! Face hints for a self-hosted web font catalog. Explicit aliases take precedence.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Face { Mincho, Gothic }

pub(crate) fn face_hint(name: &str) -> Option<Face> {
    // Normalize full-width ASCII used in Windows font names without changing
    // Japanese letters; keep the original document's font name intact.
    let normalized: String = name.chars().map(|c| match c {
        '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap(),
        '\u{3000}' => ' ',
        _ => c,
    }).collect::<String>().to_lowercase();
    let tokens = normalized.split(|c: char| !c.is_ascii_alphanumeric()).collect::<Vec<_>>();
    // Check sans-serif first: it contains the substring "serif".
    if normalized.contains("ゴシック") || normalized.contains("ｺﾞｼｯｸ") || normalized.contains("gothic")
        || normalized.contains("メイリオ") || normalized.contains("meiryo")
        || normalized.contains("sansserif") || tokens.contains(&"sans") || tokens.contains(&"ui") {
        Some(Face::Gothic)
    } else if normalized.contains("明朝") || normalized.contains("mincho") || tokens.contains(&"serif") {
        Some(Face::Mincho)
    } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_named_faces_and_respects_sans_serif_precedence() {
        for name in ["ＭＳ Ｐ明朝", "游明朝", "Futura Mincho Pro", "Example Serif", "Ｓｅｒｉｆ"] {
            assert_eq!(face_hint(name), Some(Face::Mincho), "{name}");
        }
        for name in ["Meiryo UI", "メイリオ", "ＭＳ ゴシック", "HGPｺﾞｼｯｸE", "Unknown Gothic", "Noto Sans JP", "sans-serif", "sans serif", "Segoe UI"] {
            assert_eq!(face_hint(name), Some(Face::Gothic), "{name}");
        }
        for name in ["Arial", "Sanskrit", "Guild", ""] { assert_eq!(face_hint(name), None, "{name}"); }
    }
}

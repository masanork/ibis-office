// SPDX-License-Identifier: AGPL-3.0-only
//! Face hints for a self-hosted web font catalog. Explicit aliases take precedence.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Face { Mincho, Gothic }

pub(crate) fn face_hint(name: &str) -> Option<Face> {
    // The desktop renderer shares this classification. The document's font name stays as stored.
    match ibis_office_render::shape_view::cjk_face_hint(name) {
        Some(ibis_office_render::shape_view::CjkFace::Mincho) => Some(Face::Mincho),
        Some(ibis_office_render::shape_view::CjkFace::Gothic) => Some(Face::Gothic),
        None => None,
    }
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

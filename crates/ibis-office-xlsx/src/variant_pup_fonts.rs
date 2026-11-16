// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Explicit PUP font selection without changing document text.
use gpui_kit::{App, SharedString, TextRun, TextSystem, font};
use std::{ops::Range, sync::Arc};

struct Face {
    family: SharedString,
    data: Arc<Vec<u8>>,
    index: u32,
}
impl Face {
    fn has(&self, c: char) -> bool {
        ttf_parser::Face::parse(&self.data, self.index)
            .ok()
            .and_then(|f| f.glyph_index(c))
            .is_some_and(|id| id.0 != 0)
    }
}
#[derive(Default)]
pub(crate) struct PupFonts {
    compatibility: Option<Face>,
    administrative: Option<Face>,
}
impl PupFonts {
    pub(crate) fn discover(cx: &App) -> Self {
        Self::discover_from(cx.text_system())
    }
    fn discover_from(ts: &TextSystem) -> Self {
        let find = |candidates: &[&str]| -> Option<Face> {
            for name in candidates {
                let family: SharedString = (*name).into();
                let id = ts.resolve_font(&font(family.clone()));
                let Some((data, index)) = ts.font_data(id) else {
                    continue;
                };
                // `resolve_font` may silently return a different fallback when the
                // named family cannot load. Never treat that as the target font.
                let Ok(face) = ttf_parser::Face::parse(&data, index) else {
                    continue;
                };
                if !face
                    .names()
                    .into_iter()
                    .filter(|n| matches!(n.name_id, 1 | 16))
                    .filter_map(|n| n.to_string())
                    .any(|actual| actual == *name)
                {
                    continue;
                }
                return Some(Face {
                    family,
                    data,
                    index,
                });
            }
            None
        };
        Self {
            compatibility: find(&["Ibis PUP Alias Test"]),
            administrative: find(&["Add Character_GJMincho", "追加文字行政事務標準明朝"]),
        }
    }
    fn family(&self, c: char) -> Option<SharedString> {
        let cp = c as u32;
        if !(0x100000..=0x10fffd).contains(&cp) {
            return None;
        }
        // The compatibility face also holds combining-sequence PUPs outside
        // the Han range. Give it priority only for entries in its actual cmap.
        self.compatibility
            .as_ref()
            .filter(|f| f.has(c))
            .or_else(|| {
                (!(0x101000..=0x104295).contains(&cp))
                    .then(|| self.administrative.as_ref().filter(|f| f.has(c)))
                    .flatten()
            })
            .map(|f| f.family.clone())
    }
    pub(crate) fn display(&self, text: &str) -> (String, Vec<(Range<usize>, SharedString)>) {
        display(
            text,
            |cp| {
                self.compatibility
                    .as_ref()
                    .filter(|f| f.has(cp))
                    .map(|f| f.family.clone())
            },
            |cp| {
                self.administrative
                    .as_ref()
                    .filter(|f| f.has(cp))
                    .map(|f| f.family.clone())
            },
        )
    }
}
/// Local opt-in: Plane 16 PUA has no universal meaning. Restart after changing
/// fonts or this setting, since the sheet caches shaped lines.
pub(crate) fn enabled() -> bool {
    #[cfg(not(target_family = "wasm"))]
    {
        std::env::var_os("IBIS_ADMIN_PUP_MODE").is_some_and(|v| v == "1")
    }
    #[cfg(target_family = "wasm")]
    {
        false
    }
}

pub(crate) fn runs(ts: &TextSystem, text: &str, original: &[TextRun]) -> Vec<TextRun> {
    if !enabled()
        || !text
            .chars()
            .any(|c| (0x100000..=0x10fffd).contains(&(c as u32)))
    {
        return original.to_vec();
    }
    let fonts = PupFonts::discover_from(ts);
    split_runs(text, original, |c| fonts.family(c))
}

/// StyledText needs explicit highlight boundaries before family overrides.
pub(crate) fn styled(ts: &TextSystem, text: String) -> gpui_kit::StyledText {
    let original = TextRun {
        len: text.len(),
        ..Default::default()
    };
    let routed = runs(ts, &text, &[original.clone()]);
    let mut offset = 0;
    let mut overrides = Vec::new();
    for run in routed {
        let end = offset + run.len;
        if run.font.family != original.font.family {
            overrides.push((offset..end, run.font.family));
        }
        offset = end;
    }
    let boundaries = overrides
        .iter()
        .map(|(range, _)| (range.clone(), gpui_kit::HighlightStyle::default()))
        .collect::<Vec<_>>();
    gpui_kit::StyledText::new(text)
        .with_highlights(boundaries)
        .with_font_family_overrides(overrides)
}

fn split_runs(
    text: &str,
    original: &[TextRun],
    family: impl Fn(char) -> Option<SharedString>,
) -> Vec<TextRun> {
    let mut result: Vec<TextRun> = Vec::new();
    let mut offset: usize = 0;
    for run in original {
        let Some(end) = offset.checked_add(run.len) else {
            return original.to_vec();
        };
        let Some(part) = text.get(offset..end) else {
            return original.to_vec();
        };
        for (at, c) in part.char_indices() {
            let followed_by_selector = text[offset + at + c.len_utf8()..]
                .chars()
                .next()
                .is_some_and(|v| matches!(v as u32, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef));
            let mut next = run.clone();
            next.len = c.len_utf8();
            if !followed_by_selector {
                if let Some(name) = family(c) {
                    next.font.family = name;
                }
            }
            if let Some(last) = result.last_mut() {
                let mut comparable = next.clone();
                comparable.len = last.len;
                if *last == comparable {
                    last.len += next.len;
                    continue;
                }
            }
            result.push(next);
        }
        offset = end;
    }
    if offset != text.len() {
        return original.to_vec();
    }
    result
}

fn display(
    text: &str,
    compat: impl Fn(char) -> Option<SharedString>,
    admin: impl Fn(char) -> Option<SharedString>,
) -> (String, Vec<(Range<usize>, SharedString)>) {
    let mut output = String::new();
    let mut out = Vec::new();
    for c in text.chars() {
        let cp = c as u32;
        let pup = (0x100000..=0x10fffd).contains(&cp);
        let family = if (0x101000..=0x104295).contains(&cp) {
            compat(c)
        } else if pup {
            compat(c).or_else(|| admin(c))
        } else {
            None
        };
        if pup && family.is_none() {
            // A PUA codepoint can mean something different in a different font.
            // Do not display an unrelated glyph as if it were administrative.
            output.push_str(&format!("⟦U+{cp:04X}⟧"));
        } else {
            let start = output.len();
            output.push(c);
            if let Some(family) = family {
                out.push((start..output.len(), family));
            }
        }
    }
    (output, out)
}

#[cfg(test)]
mod tests {
    use super::{display, split_runs};
    use gpui_kit::{TextRun, font};
    #[test]
    fn splits_without_changing_utf8_or_decorations() {
        let text = "A\u{101000}高\u{10de92}Z";
        let first = TextRun {
            len: "A\u{101000}".len(),
            font: font("source"),
            ..Default::default()
        };
        let second = TextRun {
            len: "高\u{10de92}Z".len(),
            font: font("other"),
            underline: Some(Default::default()),
            ..Default::default()
        };
        let result = split_runs(text, &[first, second], |c| match c {
            '\u{101000}' => Some("compat".into()),
            '\u{10de92}' => Some("admin".into()),
            _ => None,
        });
        let mut pos = 0;
        let spans: Vec<_> = result
            .iter()
            .map(|r| {
                let start = pos;
                pos += r.len;
                (
                    &text[start..pos],
                    r.font.family.as_ref(),
                    r.underline.is_some(),
                )
            })
            .collect();
        assert_eq!(
            spans,
            vec![
                ("A", "source", false),
                ("\u{101000}", "compat", false),
                ("高", "other", true),
                ("\u{10de92}", "admin", true),
                ("Z", "other", true)
            ]
        );
        assert_eq!(pos, text.len());
    }
    #[test]
    fn leaves_invalid_boundaries_and_variation_sequences_untouched() {
        let text = "\u{101000}\u{e0100}";
        let run = TextRun {
            len: text.len(),
            font: font("original"),
            ..Default::default()
        };
        assert_eq!(
            split_runs(text, &[run.clone()], |c| (c == '\u{101000}')
                .then(|| "target".into())),
            vec![run.clone()]
        );
        let bad = TextRun { len: 1, ..run };
        assert_eq!(
            split_runs(text, &[bad.clone()], |_| Some("target".into())),
            vec![bad]
        );
    }
    #[test]
    fn chooses_only_known_pups_and_keeps_original_offsets() {
        let text = "A\u{101000}高\u{10de92}\u{101001}\u{f0000}";
        let (rendered, spans) = display(
            text,
            |c| (c == '\u{101000}').then(|| "compat".into()),
            |c| (c == '\u{10de92}').then(|| "admin".into()),
        );
        assert_eq!(spans.len(), 2);
        assert_eq!(&rendered[spans[0].0.clone()], "\u{101000}");
        assert_eq!(spans[0].1.as_ref(), "compat");
        assert_eq!(&rendered[spans[1].0.clone()], "\u{10de92}");
        assert_eq!(spans[1].1.as_ref(), "admin");
    }
    #[test]
    fn untrusted_or_missing_pua_is_shown_as_codepoint() {
        let (rendered, spans) = display("x\u{101000}y\u{10de92}", |_| None, |_| None);
        assert_eq!(rendered, "x⟦U+101000⟧y⟦U+10DE92⟧");
        assert!(spans.is_empty());
    }
    #[test]
    fn combining_pup_uses_compatibility_face_by_coverage() {
        let (text, spans) = display(
            "x\u{10022a}\u{10de92}",
            |c| (c == '\u{10022a}').then(|| "compat".into()),
            |c| (c == '\u{10de92}').then(|| "admin".into()),
        );
        assert_eq!(text, "x\u{10022a}\u{10de92}");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].1.as_ref(), "compat");
        assert_eq!(spans[1].1.as_ref(), "admin");
    }
}

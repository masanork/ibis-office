// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Explicit PUP font selection for the read-only matching panel.
use gpui_kit::{App, SharedString, font};
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
        let ts = cx.text_system();
        let names = ts.all_font_names();
        let find = |candidates: &[&str]| -> Option<Face> {
            let name = candidates
                .iter()
                .find(|name| names.iter().any(|n| n.as_str() == **name))?;
            let family: SharedString = (*name).into();
            let id = ts.resolve_font(&font(family.clone()));
            let (data, index) = ts.font_data(id)?;
            // `resolve_font` may silently return a different fallback when the
            // named family cannot load. Never treat that as the target font.
            let face = ttf_parser::Face::parse(&data, index).ok()?;
            if !face
                .names()
                .into_iter()
                .filter(|n| matches!(n.name_id, 1 | 16))
                .filter_map(|n| n.to_string())
                .any(|actual| actual == *name)
            {
                return None;
            }
            Some(Face {
                family,
                data,
                index,
            })
        };
        Self {
            compatibility: find(&["Ibis PUP Alias Test"]),
            administrative: find(&["Add Character_GJMincho", "追加文字行政事務標準明朝"]),
        }
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
            admin(c)
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
    use super::display;
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
}

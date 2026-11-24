// SPDX-License-Identifier: AGPL-3.0-only

use std::{collections::BTreeMap, ops::Range, path::Path};

use gpui_kit::component::{
    Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::*;
use serde_json::value::RawValue;

use super::{Source, View, row_schema};
use crate::i18n;

#[derive(Default)]
pub(super) struct SchemaTools {
    edits: BTreeMap<usize, (usize, String)>,
    draft: Option<Draft>,
    busy: bool,
    status: Option<SharedString>,
    pub review: Option<(String, Result<row_schema::Review, &'static str>)>,
}

struct Draft {
    span: Range<usize>,
    label: String,
    input: Entity<InputState>,
}

fn has_json_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

impl Source {
    pub fn is_schema_document(&self) -> bool {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".schema.json"))
    }

    fn accepts_schema(&self) -> bool {
        self.csv.is_none()
            && self.inventory.is_none()
            && !self.is_schema_document()
            && self
                .path
                .as_ref()
                .is_some_and(|path| self.lines || has_json_extension(path))
    }
}

fn scalar(bytes: &[u8]) -> Option<&str> {
    let raw = serde_json::from_slice::<&RawValue>(bytes).ok()?.get();
    (!raw.starts_with(['{', '['])).then_some(raw)
}

fn edited_bytes(bytes: &[u8], edits: &BTreeMap<usize, (usize, String)>) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    for (&start, (end, replacement)) in edits {
        output.extend_from_slice(&bytes[offset..start]);
        output.extend_from_slice(replacement.as_bytes());
        offset = *end;
    }
    output.extend_from_slice(&bytes[offset..]);
    output
}

fn save_copy(input: &Path, destination: &Path, bytes: &[u8]) -> Result<(), String> {
    if !has_json_extension(destination) || crate::save::same_file(input, destination) {
        return Err(i18n::text("structured.schema-edit-output").to_string());
    }
    serde_json::from_slice::<&RawValue>(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .map_err(|error| error.to_string())?;
    crate::save::write_replacing(destination, bytes).map_err(|error| error.to_string())
}

impl View {
    pub(super) fn edit_schema_value(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self
            .source
            .as_ref()
            .filter(|source| source.is_schema_document())
        else {
            return;
        };
        let span = self.tree.nodes[id].span.clone();
        let Some(original) = scalar(&source.bytes[span.clone()]) else {
            return;
        };
        let value = self
            .schema_tools
            .value(&span)
            .unwrap_or(original)
            .to_owned();
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(value, window, cx);
            input
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.schema_tools.draft = Some(Draft {
            span,
            label: self.tree.breadcrumb(id),
            input,
        });
        self.schema_tools.status = None;
        cx.notify();
    }

    fn apply_schema_value(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.schema_tools.draft else {
            return;
        };
        let value = draft.input.read(cx).value();
        let Some(value) = scalar(value.as_bytes()) else {
            self.schema_tools.status = Some(i18n::text("structured.schema-edit-invalid"));
            cx.notify();
            return;
        };
        let span = draft.span.clone();
        let source = self.source.as_ref().unwrap();
        if value.as_bytes() == &source.bytes[span.clone()] {
            self.schema_tools.edits.remove(&span.start);
        } else {
            self.schema_tools
                .edits
                .insert(span.start, (span.end, value.to_owned()));
        }
        for node in self.tree.nodes.iter_mut().filter(|node| node.span == span) {
            node.detail = super::excerpt(value, 400);
        }
        self.schema_tools.draft = None;
        self.schema_tools.status = None;
        cx.notify();
    }

    fn save_schema_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(input) = source.path.as_ref() else {
            return;
        };
        let Some(directory) = input.parent() else {
            return;
        };
        let stem = input.file_stem().unwrap_or_default().to_string_lossy();
        let name = format!(
            "{}-edited.schema.json",
            stem.strip_suffix(".schema").unwrap_or(&stem)
        );
        let edits = self.schema_tools.edits.clone();
        let picked = cx.prompt_for_new_path(directory, Some(&name));
        self.schema_tools.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            let result = match picked.await {
                Ok(Ok(Some(destination))) => Some(
                    cx.background_executor()
                        .spawn(async move {
                            let bytes = edited_bytes(&source.bytes, &edits);
                            save_copy(source.path.as_ref().unwrap(), &destination, &bytes)
                        })
                        .await,
                ),
                Ok(Err(error)) => Some(Err(error.to_string())),
                _ => None,
            };
            let _ = view.update(cx, |view, cx| {
                view.schema_tools.busy = false;
                view.schema_tools.status = result.map(|result| match result {
                    Ok(()) => i18n::text("structured.schema-edit-saved"),
                    Err(error) => {
                        i18n::format("structured.schema-edit-save-error", &[("error", &error)])
                            .into()
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_schema(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            extensions: vec!["json".into()],
            prompt: Some(i18n::text("structured.schema-choose").into()),
        });
        self.schema_tools.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            let result = if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        (name, row_schema::review(&path, &source))
                    })
                    .await;
                Some(result)
            } else {
                None
            };
            let _ = view.update(cx, |view, cx| {
                view.schema_tools.busy = false;
                if let Some(result) = result {
                    view.schema_tools.review = Some(result);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn schema_actions(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let source = self.source.as_ref()?;
        let tools = &self.schema_tools;
        let panel = v_flex().px_4().py_2().gap_1().text_sm();
        if source.is_schema_document() {
            Some(
                panel
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("schema-save-copy")
                                    .outline()
                                    .xsmall()
                                    .label(i18n::text("structured.schema-edit-save"))
                                    .disabled(
                                        tools.busy
                                            || tools.edits.is_empty()
                                            || tools.draft.is_some(),
                                    )
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.save_schema_copy(window, cx)
                                    })),
                            )
                            .children((!tools.edits.is_empty()).then(|| {
                                i18n::format(
                                    "structured.schema-edit-count",
                                    &[("count", &tools.edits.len())],
                                )
                            })),
                    )
                    .children(tools.draft.as_ref().map(|draft| {
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().max_w(px(300.)).truncate().child(draft.label.clone()))
                            .child(div().w(px(220.)).child(Input::new(&draft.input).small()))
                            .child(
                                Button::new("schema-apply")
                                    .outline()
                                    .xsmall()
                                    .label(i18n::text("structured.schema-edit-apply"))
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.apply_schema_value(cx)),
                                    ),
                            )
                            .child(
                                Button::new("schema-cancel")
                                    .ghost()
                                    .xsmall()
                                    .label(i18n::text("structured.schema-edit-cancel"))
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.schema_tools.draft = None;
                                        view.schema_tools.status = None;
                                        cx.notify();
                                    })),
                            )
                    }))
                    .children(tools.status.clone()),
            )
        } else if source.accepts_schema() {
            Some(
                panel.child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("choose-schema")
                                .outline()
                                .xsmall()
                                .label(i18n::text("structured.schema-choose"))
                                .disabled(tools.busy)
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.choose_schema(window, cx)
                                })),
                        )
                        .children(tools.review.as_ref().map(|(name, _)| name.clone())),
                ),
            )
        } else {
            None
        }
    }

    pub(super) fn edit_schema_button(
        &self,
        id: usize,
        source: &Source,
        cx: &mut Context<Self>,
    ) -> Option<Button> {
        if !source.is_schema_document()
            || scalar(&source.bytes[self.tree.nodes[id].span.clone()]).is_none()
        {
            return None;
        }
        Some(
            Button::new("schema-edit-value")
                .outline()
                .xsmall()
                .label(i18n::text("structured.schema-edit-value"))
                .disabled(self.schema_tools.busy || self.schema_tools.draft.is_some())
                .on_click(
                    cx.listener(move |view, _, window, cx| view.edit_schema_value(id, window, cx)),
                ),
        )
    }
}

impl SchemaTools {
    pub(super) fn value(&self, span: &Range<usize>) -> Option<&str> {
        self.edits
            .get(&span.start)
            .filter(|(end, _)| *end == span.end)
            .map(|(_, value)| value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{Source, edited_bytes, row_schema, save_copy, scalar};
    use std::collections::BTreeMap;

    #[test]
    fn schema_copy_widens_contains_without_losing_other_checks() {
        let original = br#"{
          "$schema":"https://json-schema.org/draft/2020-12/schema",
          "properties":{"team":{"properties":{"contacts":{
            "items":{"properties":{"name":{"minLength":1}}},
            "contains":{"required":["primary"],"properties":{"primary":{"const":true}}},
            "minContains":1,"maxContains":1
          }}}}
        }"#;
        let start = original
            .windows(b"\"maxContains\":1".len())
            .position(|bytes| bytes == b"\"maxContains\":1")
            .unwrap()
            + b"\"maxContains\":".len();
        let edits = BTreeMap::from([(start, (start + 1, scalar(b"2").unwrap().to_owned()))]);
        let changed = edited_bytes(original, &edits);
        assert_eq!(changed[start], b'2');
        assert_eq!(original[start], b'1');
        assert_eq!(changed.len(), original.len());
        assert!(scalar(b"{}").is_none());
        assert!(scalar(b"2e").is_none());

        let dir = std::env::temp_dir();
        let original_path = dir.join(format!(
            "ibis-schema-original-{}.schema.json",
            std::process::id()
        ));
        let edited_path = dir.join(format!(
            "ibis-schema-edited-{}.schema.json",
            std::process::id()
        ));
        std::fs::write(&original_path, original).unwrap();
        assert!(save_copy(&original_path, &original_path, &changed).is_err());
        save_copy(&original_path, &edited_path, &changed).unwrap();
        assert_eq!(std::fs::read(&original_path).unwrap(), original);
        assert_eq!(std::fs::read(&edited_path).unwrap(), changed);

        let data = Source::new(
            br#"{"team":{"contacts":[{"name":"","primary":true},{"name":"B","primary":true}]}}"#
                .to_vec(),
            false,
        );
        let before = row_schema::review(&original_path, &data).unwrap();
        let after = row_schema::review(&edited_path, &data).unwrap();
        std::fs::remove_file(original_path).unwrap();
        std::fs::remove_file(edited_path).unwrap();
        assert_eq!(before.issue_count, 2);
        assert_eq!(after.issue_count, 1);
        assert_eq!(
            row_schema::pointer(&after.issues[0].path),
            "/team/contacts/0/name"
        );
    }
}

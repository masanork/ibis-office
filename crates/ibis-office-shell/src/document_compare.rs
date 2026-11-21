// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use gpui_kit::*;

use crate::{editor::SlideEditor, i18n::text};
use ibis_formats::lawtext::{LawDoc, parse_law_input};

#[cfg(not(target_family = "wasm"))]
fn law_file(path: &std::path::Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    name.ends_with(".xml") || name.ends_with(".lawtext") || name.ends_with(".law.txt")
}

#[cfg(not(target_family = "wasm"))]
fn law_from_file(path: &std::path::Path) -> Result<LawDoc, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_law_input(&source).map_err(|e| e.to_string())
}

#[cfg(not(target_family = "wasm"))]
fn label(name: &str, law: &LawDoc) -> String {
    [name, law.law_title.as_str(), law.law_num.as_str()]
        .into_iter()
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(target_family = "wasm")]
fn label(name: &str, law: &LawDoc) -> String {
    // The browser's GPUI prompt has a fixed width and does not wrap long lines.
    let mut label = String::new();
    for field in [name, law.law_title.as_str(), law.law_num.as_str()] {
        for (index, character) in field.chars().take(100).enumerate() {
            if index != 0 && index % 20 == 0 {
                label.push('\n');
            }
            label.push(character);
        }
        if field.chars().nth(100).is_some() {
            label.push('…');
        }
        label.push('\n');
    }
    label.trim_end().to_owned()
}

impl SlideEditor {
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn has_law_source(&self) -> bool {
        if self.kind() != ibis_formats::Kind::Document || self.loading {
            return false;
        }
        self.file_path().as_deref().is_some_and(|path| {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            name.ends_with(".lawtext") || name.ends_with(".law.txt")
        }) || self.presentation().flow.as_ref().is_some_and(|flow| {
            flow.body.iter().take(8).any(|block| match block {
                ibis_doc_model::flow::Block::Paragraph(p) => {
                    p.text().trim_start().starts_with("法令番号:")
                }
                _ => false,
            })
        })
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn compare_documents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_document || self.loading || self.kind() != ibis_formats::Kind::Document {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            extensions: ["xml", "lawtext", "txt", "md"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            multiple: false,
            prompt: Some(text("compare.pick-baseline")),
        });

        cx.spawn_in(window, async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Err(error)) => {
                    let _ = this.update(cx, |editor, cx| {
                        editor.show_alert(text("compare.failed"), error.to_string().into(), cx);
                    });
                    return;
                }
                _ => return,
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |editor, window, cx| {
                editor.compare_law_with_file(path, window, cx);
            });
        })
        .detach();
    }

    /// The selected file contains a recognized statute input.
    #[cfg(not(target_family = "wasm"))]
    pub fn is_law_comparison_file(path: &std::path::Path) -> bool {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if name.ends_with(".lawtext") || name.ends_with(".law.txt") {
            return true;
        }
        if ![".xml", ".md", ".txt"]
            .iter()
            .any(|ext| name.ends_with(ext))
        {
            return false;
        }
        use std::io::Read;
        let Ok(file) = std::fs::File::open(path) else {
            return false;
        };
        let mut data = Vec::new();
        if file.take(8192).read_to_end(&mut data).is_err() {
            return false;
        }
        // A bounded probe can end in the middle of a UTF-8 character.
        if let Err(error) = std::str::from_utf8(&data) {
            if error.error_len().is_some() {
                return false;
            }
            data.truncate(error.valid_up_to());
        }
        if name.ends_with(".xml") {
            return ibis_formats::lawtext::is_law_xml(&data);
        }
        if name.ends_with(".txt") {
            return ibis_formats::lawtext::is_lawtext(&data);
        }
        std::str::from_utf8(&data).is_ok_and(|head| {
            let head = head.trim_start_matches('\u{feff}').trim_start();
            head.starts_with("---") && head.contains("type: lawtext")
        })
    }

    /// Open a statute comparison from the normal document-comparison command.
    #[cfg(not(target_family = "wasm"))]
    pub fn compare_law_with_file(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_document || self.loading || self.kind() != ibis_formats::Kind::Document {
            return;
        }
        self.commit_edit(window, cx);
        let active = self.presentation().clone();
        let active_name = self
            .file_name
            .clone()
            .unwrap_or_else(|| text("file.new.document"));
        let original = self
            .file_path()
            .filter(|path| !self.is_dirty() && law_file(path));
        let known = self.watch.known.clone();
        let baseline_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let inputs = cx.background_executor().spawn(async move {
            let baseline = law_from_file(&path)?;
            // Prefer the exact original markup while the unedited file still matches its disk stamp.
            let current = original.as_deref().and_then(|path| {
                if known.is_none() || crate::file_watch::DiskStamp::stat(path) != known {
                    return None;
                }
                let law = law_from_file(path).ok()?;
                (crate::file_watch::DiskStamp::stat(path) == known).then_some(law)
            });
            let working = match current {
                Some(law) => law,
                None => crate::ai_assistant::presentation_law(&active)?,
            };
            Ok(Some((baseline_name, baseline, working)))
        });
        self.run_statute_comparison(active_name, inputs, window, cx);
    }

    #[cfg(target_family = "wasm")]
    pub fn compare_law_with_browser_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading
            || crate::embed::is_view_only()
            || !crate::host::options().local_files
            || !crate::host::export_allowed("docx")
        {
            return;
        }
        let Some(active) = self.web_law_source.clone() else {
            self.show_alert(text("compare.failed"), text("compare.open-law-first"), cx);
            return;
        };
        self.commit_edit(window, cx);
        if self.is_dirty() || self.document.can_undo() {
            self.show_alert(text("compare.failed"), text("compare.browser-unedited"), cx);
            return;
        }
        let active_name = self
            .file_name
            .clone()
            .unwrap_or_else(|| text("file.new.document"));
        self.run_statute_comparison(
            active_name,
            async move {
                let Some((name, bytes)) = crate::web::pick_file().await? else {
                    return Ok(None);
                };
                let source = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
                let baseline = parse_law_input(source).map_err(|error| error.to_string())?;
                let source = std::str::from_utf8(&active).map_err(|error| error.to_string())?;
                let revised = parse_law_input(source).map_err(|error| error.to_string())?;
                Ok(Some((name, baseline, revised)))
            },
            window,
            cx,
        );
    }

    fn run_statute_comparison(
        &mut self,
        active_name: SharedString,
        inputs: impl std::future::Future<Output = Result<Option<(String, LawDoc, LawDoc)>, String>>
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let (baseline_name, mut old, mut new) = match inputs.await {
                Ok(Some(laws)) => laws,
                Ok(None) => return,
                Err(error) => {
                    let _ = this.update(cx, |editor, cx| {
                        editor.show_alert(text("compare.failed"), error.into(), cx)
                    });
                    return;
                }
            };
            if !old.law_num.is_empty() && !new.law_num.is_empty() && old.law_num != new.law_num {
                let error = format!("{}\n{} / {}", text("compare.different-laws"), old.law_num, new.law_num);
                let _ = this.update(cx, |editor, cx| {
                    editor.show_alert(text("compare.failed"), error.into(), cx)
                });
                return;
            }
            let mut old_name = baseline_name.as_str();
            let mut new_name = active_name.as_ref();
            #[cfg(target_family = "wasm")]
            let output = "compare.download-result";
            #[cfg(not(target_family = "wasm"))]
            let output = "compare.open-result";
            loop {
                let detail = format!(
                    "{}\n{}\n\n{}\n{}",
                    text("compare.current"),
                    label(old_name, &old),
                    text("compare.amended"),
                    label(new_name, &new),
                );
                let buttons = [
                    PromptButton::ok(text(output)),
                    PromptButton::ok(text("compare.swap")),
                    PromptButton::cancel(text("_shell.cancel")),
                ];
                let Ok(receiver) = this.update_in(cx, |_, window, cx| {
                    window.prompt(
                        PromptLevel::Info,
                        &text("compare.law-preview"),
                        Some(&detail),
                        &buttons,
                        cx,
                    )
                }) else {
                    return;
                };
                match receiver.await {
                    Ok(0) => break,
                    Ok(1) => {
                        std::mem::swap(&mut old, &mut new);
                        std::mem::swap(&mut old_name, &mut new_name);
                    }
                    _ => return,
                }
            }
            let title = if new.law_title.is_empty() {
                &old.law_title
            } else {
                &new.law_title
            };
            let name = format!("{title} {}.docx", text("compare.law-result"));
            #[cfg(target_family = "wasm")]
            {
                let result = ibis_formats::lawtext::comparison_docx(&old, &new);
                let _ = this.update_in(cx, |editor, window, cx| match result {
                    Ok(bytes) => {
                        editor.browser_export(
                            name,
                            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                            bytes,
                            window,
                            cx,
                        );
                    }
                    Err(error) => editor.show_alert(text("compare.failed"), error.to_string().into(), cx),
                });
            }
            #[cfg(not(target_family = "wasm"))]
            {
                let opened = cx
                    .background_executor()
                    .spawn(async move { ibis_formats::lawtext::open_comparison_document(&old, &new) })
                    .await;
                let _ = this.update(cx, |editor, cx| match opened {
                    Ok(document) => crate::app::open_window_with(cx, move |editor, window, cx| {
                        editor.load(document.presentation, name.into(), window, cx);
                        editor.source = Some(std::sync::Arc::new(document.source));
                    }),
                    Err(error) => {
                        editor.show_alert(text("compare.failed"), error.to_string().into(), cx)
                    }
                });
            }
        })
        .detach();
    }
}

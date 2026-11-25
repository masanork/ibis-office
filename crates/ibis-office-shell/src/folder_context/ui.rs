// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Root, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::ai_sheet::{parse_cell_range, resolve_sheet_slide};

use super::{Folder, FolderReference, ReferencePosition, extension, read_text};
use crate::editor::SlideEditor;
use crate::i18n::text;
use crate::with_format;

pub(crate) struct FolderPane {
    pub(crate) folder: Arc<Folder>,
    pub(crate) use_agent_instructions: bool,
    pub(crate) use_agent_memory: bool,
    filter: Entity<InputState>,
    _subscription: Subscription,
    limit: usize,
}

impl SlideEditor {
    fn set_folder_pane(
        &mut self,
        folder: Arc<Folder>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(text("folder.filter")));
        let subscription = cx.subscribe(&filter, |editor, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(pane) = editor.folder_pane.as_mut() {
                    pane.limit = 100;
                }
                cx.notify();
            }
        });
        self.folder_pane = Some(FolderPane {
            folder,
            use_agent_instructions: false,
            use_agent_memory: false,
            filter,
            _subscription: subscription,
            limit: 100,
        });
        cx.notify();
    }

    pub(crate) fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::git_workspace::GitWorkspace;

        enum OpenedFolder {
            Files(Folder),
            Repository(Box<GitWorkspace>),
        }

        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            extensions: Vec::new(),
            prompt: Some(text("file.open-folder")),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    if !super::is_obsidian_vault(&path)
                        && let Some(workspace) = GitWorkspace::for_selected_folder(&path)
                    {
                        Ok(OpenedFolder::Repository(Box::new(workspace)))
                    } else {
                        Folder::scan(&path).map(OpenedFolder::Files)
                    }
                })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| match result {
                Ok(OpenedFolder::Files(folder)) => {
                    editor.reset_ai_folder_context(cx);
                    editor.set_folder_pane(Arc::new(folder), window, cx);
                }
                Ok(OpenedFolder::Repository(workspace)) => crate::app::open_window_with(cx, move |editor, _, cx| {
                    editor.git_workspace = Some(*workspace);
                    cx.notify();
                }),
                Err(error) => editor.show_alert(text("folder.error"), error.into(), cx),
            });
        })
        .detach();
    }

    fn open_folder_git_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::git_workspace::GitWorkspace;

        let Some(pane) = &self.folder_pane else {
            return;
        };
        let root = pane.folder.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let workspace = cx
                .background_executor()
                .spawn(async move { GitWorkspace::for_selected_folder(&root) })
                .await;
            let _ = this.update(cx, |editor, cx| match workspace {
                Some(workspace) => crate::app::open_window_with(cx, move |editor, _, cx| {
                    editor.git_workspace = Some(workspace);
                    cx.notify();
                }),
                None => editor.show_alert(text("folder.error"), text("folder.moved"), cx),
            });
        })
        .detach();
    }

    fn refresh_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = &self.folder_pane else {
            return;
        };
        let previous = pane.folder.clone();
        let root = previous.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { Folder::scan(&root) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(pane) = editor
                    .folder_pane
                    .as_mut()
                    .filter(|pane| Arc::ptr_eq(&pane.folder, &previous))
                else {
                    return;
                };
                match result {
                    Ok(folder) if folder.root == previous.root => {
                        pane.folder = Arc::new(folder);
                        editor.clear_assistant_skill();
                    }
                    Ok(_) => editor.show_alert(text("folder.error"), text("folder.moved"), cx),
                    Err(error) => editor.show_alert(text("folder.error"), error.into(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Follow a Markdown link against the folder that owns the open source file.
    pub fn follow_folder_link(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pane) = &self.folder_pane else { return false };
        let Some(file) = self.file_path() else { return false };
        let Some(target) = pane.folder.link_from_file(&file, url) else { return false };
        match target {
            Ok(reference) => self.open_folder_reference(&reference, window, cx),
            Err(error) => self.show_alert(text("folder.error"), error.into(), cx),
        }
        true
    }

    pub(crate) fn open_folder_file(
        &mut self,
        relative: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_folder_reference(
            &FolderReference {
                path: relative.to_owned(),
                position: None,
            },
            window,
            cx,
        );
    }

    pub(crate) fn open_folder_reference(
        &mut self,
        reference: &FolderReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = &self.folder_pane else {
            return;
        };
        let folder = pane.folder.clone();
        let path = match folder.resolve(&reference.path) {
            Ok(path) => path,
            Err(error) => {
                self.show_alert(text("folder.error"), error.into(), cx);
                return;
            }
        };
        if self.shows_path(&path) {
            self.reveal_folder_position(reference.position.as_ref(), cx);
            return;
        }
        if let Some(handle) = crate::open_guard::find_open_window(&path, Some(cx.entity_id()), cx) {
            crate::open_guard::raise(handle, cx);
            if let Some(root) = handle.downcast::<Root>() {
                let _ = root.update(cx, |root, _, cx| {
                    if let Ok(editor) = root.view().clone().downcast::<SlideEditor>() {
                        editor.update(cx, |editor, cx| {
                            editor.reveal_folder_position(reference.position.as_ref(), cx)
                        });
                    }
                });
            }
            return;
        }
        let position = reference.position.clone();
        let ext = extension(&path);
        if matches!(ext.as_str(), "jsonl" | "ndjson") {
            self.open_structured_data(path, None, window, cx);
            return;
        }
        if matches!(ext.as_str(), "txt" | "yaml" | "yml") {
            self.open_folder_text(path, window, cx);
            return;
        }
        let open =
            move |editor: &mut SlideEditor, window: &mut Window, cx: &mut Context<SlideEditor>| {
                let read_path = path.clone();
                editor.open_read_then(
                    path,
                    move || crate::read_file::read_file(&read_path),
                    None,
                    window,
                    cx,
                    move |editor, cx| editor.reveal_folder_position(position.as_ref(), cx),
                );
            };
        if self.replaceable_by_open() {
            open(self, window, cx);
        } else {
            crate::app::open_window_with(cx, move |editor, window, cx| {
                editor.set_folder_pane(folder, window, cx);
                open(editor, window, cx);
            });
        }
    }

    fn open_folder_text(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let (content, partial) = read_text(&path)?;
                    let mut markdown = String::new();
                    if partial {
                        markdown.push_str(&format!("{}\n\n", text("folder.preview-partial")));
                    }
                    for line in content.lines() {
                        markdown.push_str("    ");
                        markdown.push_str(line);
                        markdown.push('\n');
                    }
                    ibis_formats::markdown::open_markdown(markdown.into_bytes())
                        .map_err(|err| err.to_string())
                })
                .await;
            let _ = this.update(cx, |editor, cx| match result {
                Ok(opened) => crate::app::open_window_with(cx, move |editor, window, cx| {
                    editor.load(opened.presentation, name.into(), window, cx);
                    editor.standard_read_only = true;
                    editor.document.set_read_only(true);
                    editor.set_loaded_source(Arc::new(opened.source));
                }),
                Err(error) => editor.show_alert(text("folder.error"), error.into(), cx),
            });
        })
        .detach();
    }

    fn reveal_folder_position(
        &mut self,
        position: Option<&ReferencePosition>,
        cx: &mut Context<Self>,
    ) {
        match position {
            Some(ReferencePosition::Cells { sheet, range }) => {
                if let Some(workbook) = self.presentation().workbook.as_ref()
                    && let Ok(slide) =
                        resolve_sheet_slide(workbook, Some(sheet), self.current_slide)
                    && let Some(range) = parse_cell_range(range)
                {
                    with_format!(self, set_ai_sheet_selection(self, slide, range));
                }
            }
            Some(ReferencePosition::Slide(number)) if self.pages_are_slides() => {
                if let Some(slide) = number
                    .checked_sub(1)
                    .and_then(|index| self.presentation().slides.get(index))
                {
                    self.set_current_slide(slide.id);
                }
            }
            Some(ReferencePosition::Heading(fragment)) => {
                if let Some(id) = self.presentation().flow.as_deref()
                    .and_then(|doc| ibis_formats::markdown::heading_fragment(doc, fragment))
                {
                    with_format!(self, reveal_ai_paragraph(self, id, cx));
                }
            }
            Some(ReferencePosition::Paragraph(id)) => {
                with_format!(self, reveal_ai_paragraph(self, *id, cx))
            }
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn render_folder_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pane = self.folder_pane.as_ref()?;
        let query = pane.filter.read(cx).value().trim().to_lowercase();
        let mut rows = div()
            .id("folder-files")
            .min_h_0()
            .flex_1()
            .overflow_y_scroll();
        if let Some(relative) = self.file_path().as_deref()
            .and_then(|file| file.strip_prefix(&pane.folder.root).ok())
            .and_then(|path| path.to_str())
            .filter(|path| pane.folder.files.binary_search_by(|file| file.as_str().cmp(path)).is_ok())
        {
            let backlinks = pane.folder.markdown_index.backlinks(relative);
            rows = rows
                .child(div().px_1().text_sm().child(text("folder.backlinks")))
                .when(pane.folder.markdown_index.partial, |rows| {
                    rows.child(div().px_1().text_xs().child(text("folder.backlinks-partial")))
                })
                .when(backlinks.is_empty(), |rows| {
                    rows.child(div().px_1().text_xs().child(text("folder.backlinks-empty")))
                });
            for backlink in backlinks {
                let source = backlink.source.clone();
                rows = rows.child(
                    div()
                        .id(SharedString::from(format!("folder-backlink:{}", source.path)))
                        .cursor_pointer()
                        .px_1()
                        .py_1()
                        .hover(|row| row.bg(cx.theme().accent))
                        .child(div().text_sm().truncate().child(format!("{} · {}", source.path, backlink.line)))
                        .child(div().text_xs().truncate().child(backlink.excerpt.clone()))
                        .on_click(cx.listener(move |editor, _, window, cx| {
                            editor.open_folder_reference(&source, window, cx)
                        })),
                );
            }
            rows = rows.child(div().my_1().h(px(1.)).bg(cx.theme().border));
        }
        let mut matches = pane.folder.files.iter().filter(|path| {
            if query.is_empty() {
                *path != "index.md"
            } else {
                pane.folder.matches(path, &query)
            }
        });
        if let Some(index) = pane
            .folder
            .files
            .iter()
            .find(|path| *path == "index.md")
            .filter(|_| query.is_empty())
        {
            rows = rows.child(folder_row(index, None, cx));
        }
        for path in matches.by_ref().take(pane.limit) {
            rows = rows.child(folder_row(path, pane.folder.concepts.get(path).map(|item| &item.review), cx));
        }
        if matches.next().is_some() {
            rows = rows.child(
                Button::new("folder-more")
                    .ghost()
                    .small()
                    .label(text("folder.more"))
                    .on_click(cx.listener(|editor, _, _, cx| {
                        if let Some(pane) = editor.folder_pane.as_mut() {
                            pane.limit += 100;
                        }
                        cx.notify();
                    })),
            );
        }
        let name = pane.folder.name();
        Some(
            v_flex()
                .w(px(240.))
                .flex_shrink_0()
                .min_h_0()
                .border_r_1()
                .border_color(cx.theme().border)
                .p_2()
                .gap_2()
                .child(div().truncate().text_sm().child(name))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("folder-refresh")
                                .ghost()
                                .xsmall()
                                .label(text("folder.refresh"))
                                .on_click(cx.listener(|editor, _, window, cx| {
                                    editor.refresh_folder(window, cx)
                                })),
                        )
                        .when(pane.folder.has_git, |bar| {
                            bar.child(
                                Button::new("folder-git")
                                    .ghost()
                                    .xsmall()
                                    .label(text("folder.git"))
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.open_folder_git_workspace(window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("folder-close")
                                .ghost()
                                .xsmall()
                                .label(text("folder.close"))
                                .on_click(cx.listener(|editor, _, _, cx| {
                                    editor.reset_ai_folder_context(cx);
                                    editor.folder_pane = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(Input::new(&pane.filter).small())
                .when(pane.folder.truncated || pane.folder.skipped > 0, |pane| {
                    pane.child(div().text_xs().child(text("folder.partial")))
                })
                .child(rows)
                .into_any_element(),
        )
    }
}

fn folder_row(
    path: &str,
    review: Option<&ibis_formats::okf::Review>,
    cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    use ibis_formats::okf::{Freshness, Trust};

    let path = path.to_owned();
    div()
        .id(SharedString::from(path.clone()))
        .cursor_pointer()
        .px_1()
        .py_1()
        .text_sm()
        .hover(|row| row.bg(cx.theme().accent))
        .child(div().truncate().child(path.clone()))
        .when_some(review, |row, review| {
            let trust = text(match review.trust {
                Trust::Unverified => "folder.okf.unverified",
                Trust::MachineConfirmed => "folder.okf.machine-confirmed",
                Trust::HumanReviewed => "folder.okf.human-reviewed",
            });
            let freshness = text(match review.freshness() {
                Freshness::Unspecified => "folder.okf.unspecified",
                Freshness::Fresh => "folder.okf.fresh",
                Freshness::Stale => "folder.okf.stale",
                Freshness::Malformed => "folder.okf.malformed",
            });
            let mut details = format!("{trust} · {freshness}");
            if let Some((by, at)) = &review.verifier {
                details.push_str(&format!("\n{by} · {at}"));
            }
            if let Some(deadline) = review.stale_after.as_ref().and_then(|value| value.as_str()) {
                details.push_str(&format!("\n{} {deadline}", text("folder.okf.deadline")));
            }
            row.tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(details.clone()).build(window, cx)
            })
                .child(div().text_xs().child(format!("{trust} · {freshness}")))
                .when(review.malformed_generation, |row| {
                    row.child(div().text_xs().child(text("folder.okf.malformed-generation")))
                })
                .when_some(review.verifier.as_ref(), |row, (by, at)| {
                    row.child(div().text_xs().truncate().child(format!("{by} · {at}")))
                })
        })
        .on_click(
            cx.listener(move |editor, _, window, cx| editor.open_folder_file(&path, window, cx)),
        )
}

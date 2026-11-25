// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Root, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::ai_sheet::{parse_cell_range, resolve_sheet_slide};

use super::{ContentSearchHit, Folder, FolderReference, ReferencePosition, extension, read_text};
use crate::editor::SlideEditor;
use crate::i18n::text;
use crate::with_format;

const MAX_HISTORY: usize = 64;
const MAX_CONTENT_HITS: usize = 80;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SearchMode {
    #[default]
    Files,
    Content,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TreeRow {
    Dir {
        path: String,
        name: String,
        depth: usize,
        collapsed: bool,
    },
    File {
        path: String,
        name: String,
        depth: usize,
    },
}

#[derive(Default)]
struct DirNode {
    dirs: BTreeMap<String, DirNode>,
    files: Vec<(String, String)>,
}

pub(super) fn build_folder_tree_rows(files: &[String], collapsed: &BTreeSet<String>) -> Vec<TreeRow> {
    let mut root = DirNode::default();
    let mut has_root_index = false;
    for path in files {
        if path == "index.md" {
            has_root_index = true;
            continue;
        }
        let parts: Vec<&str> = path.split('/').collect();
        let Some((file_name, dirs)) = parts.split_last() else {
            continue;
        };
        let mut node = &mut root;
        for dir in dirs {
            node = node.dirs.entry((*dir).to_owned()).or_default();
        }
        node.files.push(((*file_name).to_owned(), path.clone()));
    }

    fn emit(
        node: &DirNode,
        prefix: &str,
        depth: usize,
        collapsed: &BTreeSet<String>,
        out: &mut Vec<TreeRow>,
    ) {
        for (name, child) in &node.dirs {
            let dir_path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let is_collapsed = collapsed.contains(&dir_path);
            out.push(TreeRow::Dir {
                path: dir_path.clone(),
                name: name.clone(),
                depth,
                collapsed: is_collapsed,
            });
            if !is_collapsed {
                emit(child, &dir_path, depth + 1, collapsed, out);
            }
        }
        for (name, full_path) in &node.files {
            out.push(TreeRow::File {
                path: full_path.clone(),
                name: name.clone(),
                depth,
            });
        }
    }

    let mut rows = Vec::new();
    if has_root_index {
        rows.push(TreeRow::File {
            path: "index.md".into(),
            name: "index.md".into(),
            depth: 0,
        });
    }
    emit(&root, "", 0, collapsed, &mut rows);
    rows
}

pub(crate) struct FolderPane {
    pub(crate) folder: Arc<Folder>,
    pub(crate) use_agent_instructions: bool,
    pub(crate) use_agent_memory: bool,
    filter: Entity<InputState>,
    _subscription: Subscription,
    limit: usize,
    search_mode: SearchMode,
    collapsed_dirs: BTreeSet<String>,
    cached_content_query: String,
    cached_content_hits: Vec<ContentSearchHit>,
    history: Vec<String>,
    history_index: Option<usize>,
}

impl FolderPane {
    fn record_visit(&mut self, current: Option<String>, target: &str) {
        if let Some(cur) = current
            && self.history.is_empty()
            && cur != target
        {
            self.history.push(cur);
            self.history_index = Some(0);
        }
        if let Some(idx) = self.history_index {
            if self.history.get(idx).map(String::as_str) == Some(target) {
                return;
            }
            self.history.truncate(idx + 1);
        }
        self.history.push(target.to_owned());
        if self.history.len() > MAX_HISTORY {
            let overflow = self.history.len() - MAX_HISTORY;
            self.history.drain(0..overflow);
        }
        self.history_index = Some(self.history.len() - 1);
    }

    fn can_go_back(&self) -> bool {
        self.history_index.is_some_and(|idx| idx > 0)
    }

    fn can_go_forward(&self) -> bool {
        self.history_index
            .is_some_and(|idx| idx + 1 < self.history.len())
    }
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
                    if pane.search_mode == SearchMode::Content {
                        let query = pane.filter.read(cx).value().trim().to_owned();
                        if query != pane.cached_content_query {
                            pane.cached_content_hits =
                                pane.folder.search_markdown_content(&query, MAX_CONTENT_HITS);
                            pane.cached_content_query = query;
                        }
                    }
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
            search_mode: SearchMode::Files,
            collapsed_dirs: BTreeSet::new(),
            cached_content_query: String::new(),
            cached_content_hits: Vec::new(),
            history: Vec::new(),
            history_index: None,
        });
        cx.notify();
    }

    fn set_folder_search_mode(
        &mut self,
        mode: SearchMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.folder_pane.as_mut() else {
            return;
        };
        if pane.search_mode == mode {
            return;
        }
        pane.search_mode = mode;
        pane.limit = 100;
        let placeholder = match mode {
            SearchMode::Files => text("folder.filter"),
            SearchMode::Content => text("folder.filter-content"),
        };
        pane.filter.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        if mode == SearchMode::Content {
            let query = pane.filter.read(cx).value().trim().to_owned();
            pane.cached_content_hits =
                pane.folder.search_markdown_content(&query, MAX_CONTENT_HITS);
            pane.cached_content_query = query;
        }
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
                Ok(OpenedFolder::Repository(workspace)) => {
                    crate::app::open_window_with(cx, move |editor, _, cx| {
                        editor.git_workspace = Some(*workspace);
                        cx.notify();
                    })
                }
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
                        if pane.search_mode == SearchMode::Content
                            && !pane.cached_content_query.is_empty()
                        {
                            pane.cached_content_hits = pane
                                .folder
                                .search_markdown_content(&pane.cached_content_query, MAX_CONTENT_HITS);
                        }
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

    fn current_folder_relative(&self) -> Option<String> {
        let pane = self.folder_pane.as_ref()?;
        let file = self.file_path()?;
        let rel = file.strip_prefix(&pane.folder.root).ok()?;
        let parts = rel
            .iter()
            .map(|part| part.to_str())
            .collect::<Option<Vec<_>>>()?;
        Some(parts.join("/"))
    }

    fn can_navigate_in_place(&self) -> bool {
        if self.replaceable_by_open() {
            return true;
        }
        !self.loading && !self.is_dirty() && self.current_folder_relative().is_some()
    }

    fn navigate_folder_history(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = {
            let Some(pane) = self.folder_pane.as_mut() else {
                return;
            };
            let Some(current) = pane.history_index else {
                return;
            };
            let next = if delta < 0 {
                current.checked_sub(delta.unsigned_abs())
            } else {
                current.checked_add(delta as usize)
            };
            let Some(next) = next.filter(|&idx| idx < pane.history.len()) else {
                return;
            };
            pane.history_index = Some(next);
            pane.history[next].clone()
        };
        self.open_folder_reference_inner(
            &FolderReference {
                path: target,
                position: None,
            },
            false,
            window,
            cx,
        );
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
            Err(error) => {
                if pane.folder.is_obsidian_vault {
                    match pane.folder.create_wikilink_note(&file, url) {
                        Ok(Some(created_rel)) => {
                            let root = pane.folder.root.clone();
                            if let Ok(scanned) = Folder::scan(&root)
                                && let Some(pane) = self.folder_pane.as_mut()
                            {
                                pane.folder = Arc::new(scanned);
                            }
                            self.open_folder_file(&created_rel, window, cx);
                            return true;
                        }
                        Err(create_err) => {
                            self.show_alert(text("folder.error"), create_err.into(), cx);
                            return true;
                        }
                        Ok(None) => {}
                    }
                }
                self.show_alert(text("folder.error"), error.into(), cx);
            }
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
        self.open_folder_reference_inner(reference, true, window, cx);
    }

    fn open_folder_reference_inner(
        &mut self,
        reference: &FolderReference,
        record_history: bool,
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
            let _ = crate::window_editors::activate_path_in_window(handle, &path, cx);
            crate::open_guard::raise(handle, cx);
            if let Some(root) = handle.downcast::<Root>() {
                let _ = root.update(cx, |root, _, cx| {
                    for editor in crate::window_editors::all_in_root(root, cx) {
                        if editor.read(cx).shows_path(&path) {
                            editor.update(cx, |editor, cx| {
                                editor.reveal_folder_position(reference.position.as_ref(), cx)
                            });
                            break;
                        }
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
        // When embedded inside an outer workspace (e.g. SoraneWorkspace), delegate link
        // navigation to the workspace so it opens cleanly in a tab rather than overwriting
        // the current editor in place or spawning an extra OS window.
        if self.embedded_chrome.is_some() {
            let target_path = path.clone();
            let target_pos = position.clone();
            cx.defer(move |cx| {
                if crate::window_editors::try_open_path_via_factory(&target_path, cx)
                    && let Some(pos) = target_pos
                    && let Some(handle) =
                        crate::open_guard::find_open_window(&target_path, None, cx)
                    && let Some(root) = handle.downcast::<Root>()
                {
                    let _ = root.update(cx, |root, _, cx| {
                        for editor in crate::window_editors::all_in_root(root, cx) {
                            if editor.read(cx).shows_path(&target_path) {
                                editor.update(cx, |ed, cx| {
                                    ed.reveal_folder_position(Some(&pos), cx)
                                });
                                break;
                            }
                        }
                    });
                }
            });
            return;
        }
        let in_place = self.can_navigate_in_place();
        if in_place && record_history {
            let current = self.current_folder_relative();
            if let Some(pane) = self.folder_pane.as_mut() {
                pane.record_visit(current, &reference.path);
            }
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
        if in_place {
            open(self, window, cx);
        } else {
            let target_rel = reference.path.clone();
            crate::app::open_window_with(cx, move |editor, window, cx| {
                editor.set_folder_pane(folder, window, cx);
                if let Some(pane) = editor.folder_pane.as_mut() {
                    pane.record_visit(None, &target_rel);
                }
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
                if let Some(id) = self
                    .presentation()
                    .flow
                    .as_deref()
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

    /// Attach or detach a Vault `Folder` on this embedded editor so the AI Assistant
    /// receives Vault instructions, skills, and folder tools even when the editor's
    /// own left pane is hidden by `EmbeddedChromePolicy`.
    pub fn attach_vault_folder(
        &mut self,
        folder: Option<Arc<Folder>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match folder {
            Some(folder) => {
                let already_same = self
                    .folder_pane
                    .as_ref()
                    .is_some_and(|p| Arc::ptr_eq(&p.folder, &folder));
                if !already_same {
                    self.set_folder_pane(folder, window, cx);
                }
            }
            None => {
                if self.folder_pane.is_some() {
                    self.reset_ai_folder_context(cx);
                    self.folder_pane = None;
                    cx.notify();
                }
            }
        }
    }

    /// Reveal and scroll to body paragraph `id` in a flowing document (`.md` / `.docx` / `.jtd`).
    pub fn reveal_body_paragraph(&mut self, id: u32, cx: &mut Context<Self>) {
        with_format!(self, reveal_ai_paragraph(self, id, cx));
        cx.notify();
    }

    pub(crate) fn render_folder_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pane = self.folder_pane.as_ref()?;
        let query = pane.filter.read(cx).value().trim().to_lowercase();
        let active_relative = self.current_folder_relative();
        let mut rows = div()
            .id("folder-files")
            .min_h_0()
            .flex_1()
            .overflow_y_scroll();

        if let Some(relative) = active_relative
            .as_deref()
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
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .child(format!("{} · {}", source.path, backlink.line)),
                        )
                        .child(div().text_xs().truncate().child(backlink.excerpt.clone()))
                        .on_click(cx.listener(move |editor, _, window, cx| {
                            editor.open_folder_reference(&source, window, cx)
                        })),
                );
            }
            rows = rows.child(div().my_1().h(px(1.)).bg(cx.theme().border));
        }

        match pane.search_mode {
            SearchMode::Content => {
                if query.is_empty() {
                    rows = rows.child(div().px_1().py_1().text_xs().child(text("folder.search-hint")));
                } else if pane.cached_content_hits.is_empty() {
                    rows = rows.child(div().px_1().py_1().text_xs().child(text("folder.search-empty")));
                } else {
                    for (index, hit) in pane.cached_content_hits.iter().take(pane.limit).enumerate() {
                        let path = hit.path.clone();
                        rows = rows.child(
                            div()
                                .id(SharedString::from(format!("folder-hit:{index}:{}", hit.path)))
                                .cursor_pointer()
                                .px_1()
                                .py_1()
                                .hover(|row| row.bg(cx.theme().accent))
                                .child(
                                    div()
                                        .text_sm()
                                        .truncate()
                                        .child(format!("{} · {}", hit.path, hit.line)),
                                )
                                .child(div().text_xs().truncate().child(hit.excerpt.clone()))
                                .on_click(cx.listener(move |editor, _, window, cx| {
                                    editor.open_folder_file(&path, window, cx)
                                })),
                        );
                    }
                }
            }
            SearchMode::Files if query.is_empty() => {
                let tree = build_folder_tree_rows(&pane.folder.files, &pane.collapsed_dirs);
                let total = tree.len();
                for row in tree.into_iter().take(pane.limit) {
                    match row {
                        TreeRow::Dir {
                            path,
                            name,
                            depth,
                            collapsed,
                        } => {
                            let marker = if collapsed { "▸" } else { "▾" };
                            let dir_key = path.clone();
                            rows = rows.child(
                                div()
                                    .id(SharedString::from(format!("folder-dir:{path}")))
                                    .cursor_pointer()
                                    .pl(px(4. + (depth as f32) * 12.))
                                    .pr_1()
                                    .py_1()
                                    .text_sm()
                                    .hover(|row| row.bg(cx.theme().accent))
                                    .child(div().truncate().child(format!("{marker} {name}")))
                                    .on_click(cx.listener(move |editor, _, _, cx| {
                                        if let Some(pane) = editor.folder_pane.as_mut() {
                                            if !pane.collapsed_dirs.remove(&dir_key) {
                                                pane.collapsed_dirs.insert(dir_key.clone());
                                            }
                                            cx.notify();
                                        }
                                    })),
                            );
                        }
                        TreeRow::File { path, name, depth } => {
                            let is_active = active_relative.as_deref() == Some(path.as_str());
                            rows = rows.child(folder_row(
                                &path,
                                &name,
                                depth,
                                is_active,
                                pane.folder.concepts.get(&path).map(|item| &item.review),
                                cx,
                            ));
                        }
                    }
                }
                if total > pane.limit {
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
            }
            SearchMode::Files => {
                let mut matches = pane
                    .folder
                    .files
                    .iter()
                    .filter(|path| pane.folder.matches(path, &query));
                for path in matches.by_ref().take(pane.limit) {
                    let is_active = active_relative.as_deref() == Some(path.as_str());
                    rows = rows.child(folder_row(
                        path,
                        path,
                        0,
                        is_active,
                        pane.folder.concepts.get(path).map(|item| &item.review),
                        cx,
                    ));
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
            }
        }

        let name = pane.folder.name();
        let can_back = pane.can_go_back();
        let can_forward = pane.can_go_forward();
        let mode = pane.search_mode;
        Some(
            v_flex()
                .w(px(240.))
                .flex_shrink_0()
                .min_h_0()
                .border_r_1()
                .border_color(cx.theme().border)
                .p_2()
                .gap_2()
                .child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1().min_w_0().truncate().text_sm().child(name))
                        .child(
                            Button::new("folder-back")
                                .ghost()
                                .xsmall()
                                .disabled(!can_back)
                                .label("←")
                                .tooltip(text("folder.nav-back"))
                                .on_click(cx.listener(|editor, _, window, cx| {
                                    editor.navigate_folder_history(-1, window, cx)
                                })),
                        )
                        .child(
                            Button::new("folder-forward")
                                .ghost()
                                .xsmall()
                                .disabled(!can_forward)
                                .label("→")
                                .tooltip(text("folder.nav-forward"))
                                .on_click(cx.listener(|editor, _, window, cx| {
                                    editor.navigate_folder_history(1, window, cx)
                                })),
                        ),
                )
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
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("folder-mode-files")
                                .xsmall()
                                .when(mode != SearchMode::Files, |btn| btn.ghost())
                                .label(text("folder.mode-files"))
                                .on_click(cx.listener(|editor, _, window, cx| {
                                    editor.set_folder_search_mode(SearchMode::Files, window, cx)
                                })),
                        )
                        .child(
                            Button::new("folder-mode-content")
                                .xsmall()
                                .when(mode != SearchMode::Content, |btn| btn.ghost())
                                .label(text("folder.mode-content"))
                                .on_click(cx.listener(|editor, _, window, cx| {
                                    editor.set_folder_search_mode(SearchMode::Content, window, cx)
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
    label: &str,
    depth: usize,
    active: bool,
    review: Option<&ibis_formats::okf::Review>,
    cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    use ibis_formats::okf::{Freshness, Trust};

    let path = path.to_owned();
    let label = label.to_owned();
    div()
        .id(SharedString::from(path.clone()))
        .cursor_pointer()
        .pl(px(4. + (depth as f32) * 12.))
        .pr_1()
        .py_1()
        .text_sm()
        .when(active, |row| row.bg(cx.theme().secondary))
        .hover(|row| row.bg(cx.theme().accent))
        .child(div().truncate().child(label))
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

// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{editor::SlideEditor, i18n::text};

mod commit_changes;
mod graph;
mod working_tree;
use graph::CommitGraph;
use working_tree::WorkingTree;

const HISTORY_SCAN: usize = 1500;
const HISTORY_PAGE: usize = 24;
const MAX_PREVIEW_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TEXT_DIFF_BYTES: usize = 512 * 1024;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    directory: bool,
}

struct Revision {
    id: gix::ObjectId,
    title: String,
    date: String,
    exists: bool,
}

pub(crate) struct GitWorkspace {
    repo: PathBuf,
    worktree: Option<PathBuf>,
    title: String,
    branch: String,
    head: gix::ObjectId,
    snapshot: gix::ObjectId,
    graph: CommitGraph,
    commit_changes: Option<commit_changes::CommitChanges>,
    tree: BTreeMap<PathBuf, Vec<Entry>>,
    file: Option<PathBuf>,
    revisions: Vec<Revision>,
    older: Option<gix::ObjectId>,
    base: Option<gix::ObjectId>,
    base_input: Option<Entity<InputState>>,
    base_path_input: Option<Entity<InputState>>,
    base_path: Option<PathBuf>,
    revised: Option<gix::ObjectId>,
    busy: bool,
    working: WorkingTree,
}

impl GitWorkspace {
    fn open(path: &Path) -> Result<Self> {
        let repo = gix::discover(path)?;
        let head = repo.head_id()?.detach();
        let worktree = repo.workdir().map(Path::to_path_buf);
        let name = worktree.as_deref().unwrap_or_else(|| repo.path());
        let title = name
            .file_name()
            .unwrap_or(name.as_os_str())
            .to_string_lossy()
            .into_owned();
        let branch = repo
            .head_name()?
            .map(|name| name.shorten().to_string())
            .unwrap_or_else(|| short(head));
        let mut tree = BTreeMap::new();
        tree.insert(PathBuf::new(), entries(&repo, head, Path::new(""))?);
        Ok(Self {
            repo: repo.path().to_path_buf(),
            worktree,
            title,
            branch,
            head,
            snapshot: head,
            graph: CommitGraph::default(),
            commit_changes: None,
            tree,
            file: None,
            revisions: Vec::new(),
            older: None,
            base: None,
            base_input: None,
            base_path_input: None,
            base_path: None,
            revised: None,
            busy: false,
            working: WorkingTree::default(),
        })
    }
}

fn short(id: gix::ObjectId) -> String {
    id.to_string().chars().take(8).collect()
}

fn filename(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStrExt as _;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    };
    #[cfg(not(unix))]
    let name = PathBuf::from(std::str::from_utf8(bytes)?);
    if !matches!(
        name.components().next(),
        Some(std::path::Component::Normal(_))
    ) || name.components().count() != 1
    {
        return Err("invalid Git tree entry".into());
    }
    Ok(name)
}

fn entries(repo: &gix::Repository, commit: gix::ObjectId, dir: &Path) -> Result<Vec<Entry>> {
    let root = repo.find_commit(commit)?.tree()?;
    let tree = if dir.as_os_str().is_empty() {
        root
    } else {
        root.lookup_entry_by_path(dir)?
            .ok_or("Git directory not found")?
            .object()?
            .try_into_tree()?
    };
    let mut rows = Vec::new();
    for row in tree.iter() {
        let row = row?;
        let mode = row.mode();
        if mode.is_tree() || mode.is_blob() {
            rows.push(Entry {
                path: dir.join(filename(row.filename())?),
                directory: mode.is_tree(),
            });
        }
    }
    rows.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(rows)
}

fn history(
    repo: &Path,
    start: gix::ObjectId,
    path: &Path,
) -> Result<(Vec<Revision>, Option<gix::ObjectId>)> {
    let mut repo = gix::open(repo)?;
    repo.object_cache_size(8 * 1024 * 1024);
    let mut revisions = Vec::new();
    let mut next = Some(start);
    let shallow = repo.shallow_commits()?;
    for _ in 0..HISTORY_SCAN {
        if revisions.len() == HISTORY_PAGE {
            break;
        }
        let Some(id) = next else { break };
        let commit = repo.find_commit(id)?;
        next = if shallow
            .as_ref()
            .is_some_and(|boundaries| boundaries.contains(&id))
        {
            None
        } else {
            commit.parent_ids().next().map(|parent| parent.detach())
        };
        let entry = commit
            .tree()?
            .lookup_entry_by_path(path)?
            .filter(|entry| entry.mode().is_blob());
        let before = match next {
            Some(parent) => repo
                .find_commit(parent)?
                .tree()?
                .lookup_entry_by_path(path)?
                .filter(|entry| entry.mode().is_blob()),
            None => None,
        };
        let signature = |entry: &gix::object::tree::Entry<'_>| (entry.object_id(), entry.mode());
        if entry.as_ref().map(signature) != before.as_ref().map(signature) {
            let raw = String::from_utf8_lossy(commit.message_raw_sloppy());
            let title = raw.lines().next().unwrap_or_default().to_string();
            let date = commit
                .time()
                .ok()
                .and_then(|time| time.format(gix::date::time::format::SHORT).ok())
                .unwrap_or_default();
            revisions.push(Revision {
                id,
                title,
                date,
                exists: entry.is_some(),
            });
        }
    }
    Ok((revisions, next))
}

fn blob(repo: &gix::Repository, id: gix::ObjectId, path: &Path) -> Result<Vec<u8>> {
    let tree = repo.find_commit(id)?.tree()?;
    let Some(entry) = tree
        .lookup_entry_by_path(path)?
        .filter(|entry| entry.mode().is_blob())
    else {
        return Err("File is absent in this revision".into());
    };
    if repo.find_header(entry.object_id())?.size() > MAX_PREVIEW_BYTES {
        return Err("Git file is too large to preview".into());
    }
    Ok(entry.object()?.data.clone())
}

fn markdown_block(content: &str, language: &str) -> Vec<u8> {
    let fence = "~".repeat(
        1 + content
            .split(|ch: char| ch != '~')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2),
    );
    format!("{fence}{language}\n{content}\n{fence}\n").into_bytes()
}

fn preview(repo: &Path, path: &Path, id: gix::ObjectId) -> Result<ibis_formats::Opened> {
    let repo = gix::open(repo)?;
    let bytes = blob(&repo, id, path)?;
    if let Some(markdown) = ibis_formats::lawtext::path_input_to_markdown(path, &bytes)? {
        return Ok(ibis_formats::markdown::open_markdown(
            markdown.into_bytes(),
        )?);
    }
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
    {
        return Ok(ibis_formats::markdown::open_markdown(bytes)?);
    }
    if let Ok(plain) = std::str::from_utf8(&bytes)
        && !plain.contains('\0')
    {
        return Ok(ibis_formats::markdown::open_markdown(markdown_block(
            plain, "text",
        ))?);
    }
    Ok(ibis_odf::open(bytes)?)
}

enum RevisionPreview {
    ReadOnly(ibis_formats::Opened),
    LawComparison(ibis_formats::Opened),
}

fn compare(
    repo: &Path,
    old_path: &Path,
    new_path: &Path,
    old: gix::ObjectId,
    new: gix::ObjectId,
) -> Result<RevisionPreview> {
    let repo = gix::open(repo)?;
    let old_bytes = blob(&repo, old, old_path)?;
    let new_bytes = blob(&repo, new, new_path)?;
    if let (Some(_), Some(_)) = (
        ibis_formats::lawtext::path_input_to_markdown(old_path, &old_bytes)?,
        ibis_formats::lawtext::path_input_to_markdown(new_path, &new_bytes)?,
    ) {
        let old_law = ibis_formats::lawtext::parse_law_input(std::str::from_utf8(&old_bytes)?)?;
        let new_law = ibis_formats::lawtext::parse_law_input(std::str::from_utf8(&new_bytes)?)?;
        return Ok(RevisionPreview::LawComparison(
            ibis_formats::lawtext::open_comparison_document(&old_law, &new_law)?,
        ));
    }
    if old_bytes.len().max(new_bytes.len()) > MAX_TEXT_DIFF_BYTES {
        return Err("Text diff exceeds 512 KiB per revision".into());
    }
    let old_text = std::str::from_utf8(&old_bytes)?;
    let new_text = std::str::from_utf8(&new_bytes)?;
    if old_text.contains('\0') || new_text.contains('\0') {
        return Err("Text comparison requires UTF-8 files".into());
    }
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_secs(1))
        .diff_lines(old_text, new_text);
    let patch = diff
        .unified_diff()
        .header(&short(old), &short(new))
        .to_string();
    let markdown = if patch.is_empty() {
        text("git.no-changes").to_string().into_bytes()
    } else {
        markdown_block(&patch, "diff")
    };
    Ok(RevisionPreview::ReadOnly(
        ibis_formats::markdown::open_markdown(markdown)?,
    ))
}

impl SlideEditor {
    pub(crate) fn pick_git_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            extensions: Vec::new(),
            prompt: Some(text("file.open-repository")),
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
                .spawn(async move { GitWorkspace::open(&path).map_err(|err| err.to_string()) })
                .await;
            let _ = this.update(cx, |editor, cx| match result {
                Ok(workspace) => crate::app::open_window_with(cx, move |editor, _, cx| {
                    editor.git_workspace = Some(workspace);
                    cx.notify();
                }),
                Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
            });
        })
        .detach();
    }

    fn git_toggle_dir(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        if state.tree.remove(&path).is_some() {
            cx.notify();
            return;
        }
        let repo = state.repo.clone();
        let head = state.snapshot;
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move {
                        gix::open(repo)
                            .map_err(|err| err.to_string())
                            .and_then(|repo| {
                                entries(&repo, head, &path).map_err(|err| err.to_string())
                            })
                    }
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if let Some(state) = editor.git_workspace.as_mut() {
                    state.busy = false;
                    match result {
                        Ok(rows) => {
                            state.tree.insert(path, rows);
                        }
                        Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn git_select_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        state.file = Some(path);
        state.base_input.get_or_insert_with(|| {
            cx.new(|cx| InputState::new(window, cx).placeholder(text("git.base-by-id")))
        });
        state.base_path_input =
            Some(cx.new(|cx| InputState::new(window, cx).placeholder(text("git.base-path"))));
        state.revisions.clear();
        state.base = None;
        state.base_path = None;
        state.revised = None;
        state.older = Some(state.snapshot);
        self.git_more_history(window, cx);
    }

    fn git_more_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let (Some(path), Some(first)) = (state.file.clone(), state.older) else {
            return;
        };
        let repo = state.repo.clone();
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { history(&repo, first, &path).map_err(|err| err.to_string()) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok((rows, next)) => {
                        state.older = next;
                        state.revisions.extend(rows);
                        let mut present = state.revisions.iter().filter(|row| row.exists);
                        if state.revised.is_none() {
                            state.revised = present.next().map(|row| row.id);
                        } else {
                            let _ = present.next();
                        }
                        if state.base.is_none() {
                            state.base = present.next().map(|row| row.id);
                        }
                    }
                    Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn git_set_base_by_id(
        &mut self,
        prefix: String,
        old_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_workspace.as_mut().filter(|state| !state.busy) else {
            return;
        };
        let Some(path) = state.file.clone() else {
            return;
        };
        if old_path.as_ref().is_some_and(|path| {
            path.components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        }) {
            self.show_alert(text("git.failed"), text("git.base-path-invalid"), cx);
            return;
        }
        let old_path = old_path.filter(|old| old != &path);
        let repo = state.repo.clone();
        let root = state.worktree.clone().unwrap_or_else(|| repo.clone());
        let revised = state.revised.unwrap_or(state.snapshot);
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let read = || -> Result<_> {
                        let repo = gix::open(repo)?;
                        let id = graph::find_commit_prefix(&repo, &root, &prefix)?;
                        if id == revised {
                            return Err(text("git.base-same-as-revised").to_string().into());
                        }
                        let tree = repo.find_commit(id)?.tree()?;
                        if !tree
                            .lookup_entry_by_path(old_path.as_deref().unwrap_or(&path))?
                            .is_some_and(|entry| entry.mode().is_blob())
                        {
                            return Err(text("git.base-file-missing").to_string().into());
                        }
                        Ok((id, old_path))
                    };
                    read().map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok((id, old_path)) => {
                        state.base = Some(id);
                        state.base_path = old_path;
                        state.revised = Some(revised);
                    }
                    Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn git_show_revision(
        &mut self,
        old: Option<gix::ObjectId>,
        new: gix::ObjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let Some(path) = state.file.clone() else {
            return;
        };
        let repo = state.repo.clone();
        let old_path = state.base_path.clone();
        let filename = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let label = old
            .map(|id| format!("{} → {}", short(id), short(new)))
            .unwrap_or_else(|| short(new));
        let name = format!("{filename} · {label}");
        let comparison_name = old.map(|old| {
            let stem = path
                .file_stem()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            format!(
                "{stem} {} {}-{}.docx",
                text("compare.law-result"),
                short(old),
                short(new)
            )
        });
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match old {
                        Some(id) => {
                            compare(&repo, old_path.as_deref().unwrap_or(&path), &path, id, new)
                        }
                        None => preview(&repo, &path, new).map(RevisionPreview::ReadOnly),
                    }
                    .map_err(|err| err.to_string())
                })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok(RevisionPreview::LawComparison(opened)) => {
                        let name = comparison_name.unwrap_or_else(|| format!("{name}.docx"));
                        crate::app::open_window_with(cx, move |editor, window, cx| {
                            editor.load(opened.presentation, name.into(), window, cx);
                            editor.set_loaded_source(std::sync::Arc::new(opened.source));
                        });
                    }
                    Ok(RevisionPreview::ReadOnly(opened)) => {
                        editor.present_git_preview(Ok(opened), name, window, cx);
                    }
                    Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn present_git_preview(
        &mut self,
        result: std::result::Result<ibis_formats::Opened, String>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(opened) if self.has_document && !self.standard_read_only => {
                crate::app::open_window_with(cx, move |editor, window, cx| {
                    editor.load_git_preview(opened, name, window, cx)
                });
            }
            Ok(opened) => self.load_git_preview(opened, name, window, cx),
            Err(error) => self.show_alert(text("git.failed"), error.into(), cx),
        }
    }

    fn load_git_preview(
        &mut self,
        opened: ibis_formats::Opened,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load(opened.presentation, name.into(), window, cx);
        self.standard_read_only = true;
        self.document.set_read_only(true);
        self.sidebar_open = false;
        self.set_loaded_source(std::sync::Arc::new(opened.source));
    }

    fn git_open_working_file(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &self.git_workspace else {
            return;
        };
        if let Some(file) = state.file.clone() {
            self.git_open_worktree_path(&file, cx);
        }
    }

    fn git_open_worktree_path(&mut self, file: &Path, cx: &mut Context<Self>) {
        let Some(root) = self
            .git_workspace
            .as_ref()
            .and_then(|state| state.worktree.as_ref())
        else {
            return;
        };
        let result = root.canonicalize().and_then(|root| {
            let path = root.join(file).canonicalize()?;
            if !path.starts_with(&root) || !path.is_file() {
                return Err(std::io::Error::other(
                    "File is outside the working tree or no longer exists",
                ));
            }
            Ok(path)
        });
        match result {
            Ok(path) => crate::app::open_path_in_app(path, cx),
            Err(error) => self.show_alert(text("git.failed"), error.to_string().into(), cx),
        }
    }

    pub(crate) fn render_git_workspace(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.git_workspace.as_ref()?;
        let border = cx.theme().border;
        let accent = cx.theme().accent;
        let mut tree = div().id("git-files").flex_1().min_h_0().overflow_y_scroll();
        let root = Path::new("");
        let mut stack: Vec<_> = state
            .tree
            .get(root)?
            .iter()
            .rev()
            .map(|entry| (entry, 0))
            .collect();
        let mut index = 0usize;
        while let Some((entry, depth)) = stack.pop() {
            index += 1;
            let path = entry.path.clone();
            let directory = entry.directory;
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned();
            let open = state.tree.contains_key(&path);
            let label = if directory {
                if open {
                    format!("▾ {name}")
                } else {
                    format!("▸ {name}")
                }
            } else {
                name
            };
            tree = tree.child(
                div()
                    .id(("git-file", index))
                    .pl(px(10. + depth as f32 * 14.))
                    .py_1()
                    .cursor_pointer()
                    .truncate()
                    .when(state.file.as_ref() == Some(&path), |row| row.bg(accent))
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if directory {
                            this.git_toggle_dir(path.clone(), window, cx);
                        } else {
                            this.git_select_file(path.clone(), window, cx);
                        }
                    })),
            );
            if let Some(children) = state.tree.get(&entry.path) {
                stack.extend(children.iter().rev().map(|child| (child, depth + 1)));
            }
        }

        let mut revisions = div()
            .id("git-revisions")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for (index, revision) in state.revisions.iter().enumerate() {
            let id = revision.id;
            let row = div().px_2().py_1().border_b_1().border_color(border).child(
                div().truncate().child(format!(
                    "{}  {}  {}",
                    short(id),
                    revision.date,
                    revision.title
                )),
            );
            revisions = revisions.child(row.when(revision.exists, |row| {
                row.child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new(("git-open-revision", index))
                                .xsmall()
                                .label(text("git.open-revision"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.git_show_revision(None, id, window, cx)
                                })),
                        )
                        .child(
                            Button::new(("git-set-base", index))
                                .xsmall()
                                .label(text(
                                    if state.base == Some(id) && state.base_path.is_none() {
                                        "git.base-selected"
                                    } else {
                                        "git.base"
                                    },
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(state) = this.git_workspace.as_mut() {
                                        state.base = Some(id);
                                        state.base_path = None;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("git-set-revised", index))
                                .xsmall()
                                .label(text(if state.revised == Some(id) {
                                    "git.revised-selected"
                                } else {
                                    "git.revised"
                                }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(state) = this.git_workspace.as_mut() {
                                        state.revised = Some(id);
                                    }
                                    cx.notify();
                                })),
                        ),
                )
            }));
        }
        let pair = state
            .base
            .zip(state.revised)
            .filter(|(old, new)| old != new);
        let header = h_flex()
            .gap_2()
            .px_2()
            .py_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(format!("{} · {}", state.title, state.branch)),
            )
            .when(!state.working.is_open(), |bar| {
                bar.child(self.git_graph_toggle(state, cx))
            })
            .when(state.worktree.is_some() && !state.graph.is_open(), |bar| {
                bar.child(
                    Button::new("git-working-mode")
                        .xsmall()
                        .disabled(state.busy)
                        .label(text(if state.working.is_open() {
                            "git.files"
                        } else {
                            "git.changes"
                        }))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.git_show_changes(window, cx)),
                        ),
                )
            })
            .child(
                Button::new("git-close")
                    .xsmall()
                    .label(text("_shell.close"))
                    .disabled(state.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.git_workspace = None;
                        cx.notify();
                    })),
            );
        let bottom =
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .border_t_1()
                .border_color(border)
                .child(
                    div().px_2().py_1().truncate().child(
                        state
                            .file
                            .as_ref()
                            .map(|path| path.to_string_lossy().into_owned())
                            .unwrap_or_else(|| text("git.select-file").to_string()),
                    ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .px_2()
                        .when_some(pair, |actions, (old, new)| {
                            actions.child(
                                Button::new("git-compare")
                                    .xsmall()
                                    .label(text("git.compare"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.git_show_revision(Some(old), new, window, cx)
                                    })),
                            )
                        })
                        .when(
                            state.file.is_some()
                                && state.worktree.is_some()
                                && state.snapshot == state.head,
                            |actions| {
                                actions.child(
                                    Button::new("git-open-working")
                                        .xsmall()
                                        .label(text("git.working-file"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.git_open_working_file(cx)
                                        })),
                                )
                            },
                        )
                        .when(state.older.is_some(), |actions| {
                            actions.child(
                                Button::new("git-older")
                                    .xsmall()
                                    .label(text("git.older"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.git_more_history(window, cx)
                                    })),
                            )
                        }),
                )
                .when_some(
                    state.base_input.as_ref().filter(|_| state.file.is_some()),
                    |pane, input| {
                        let draft = input.clone();
                        let path_draft = state.base_path_input.clone();
                        pane.child(
                            h_flex()
                                .px_2()
                                .py_1()
                                .gap_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(input).small().disabled(state.busy)),
                                )
                                .child(
                                    Button::new("git-set-base-id")
                                        .xsmall()
                                        .disabled(state.busy)
                                        .label(text("git.set-base-id"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let prefix = draft.read(cx).value().trim().to_owned();
                                            let old_path = path_draft.as_ref().and_then(|input| {
                                                let value = input.read(cx).value();
                                                (!value.trim().is_empty())
                                                    .then(|| PathBuf::from(value.to_string()))
                                            });
                                            this.git_set_base_by_id(prefix, old_path, window, cx);
                                        })),
                                ),
                        )
                    },
                )
                .when_some(
                    state
                        .base_path_input
                        .as_ref()
                        .filter(|_| state.file.is_some()),
                    |pane, input| {
                        pane.child(
                            div()
                                .px_2()
                                .pb_1()
                                .child(Input::new(input).small().disabled(state.busy)),
                        )
                    },
                )
                .when_some(state.base_path.as_ref(), |pane, path| {
                    pane.child(div().px_2().truncate().child(format!(
                        "{} {}",
                        text("git.base"),
                        path.display()
                    )))
                })
                .when_some(pair, |pane, (base, revised)| {
                    pane.child(div().px_2().truncate().child(format!(
                        "{} {} → {} {}",
                        text("git.base"),
                        short(base),
                        text("git.revised"),
                        short(revised)
                    )))
                })
                .child(revisions);
        Some(
            div()
                .flex()
                .flex_col()
                .w(px(310.))
                .flex_shrink_0()
                .min_h_0()
                .border_r_1()
                .border_color(border)
                .text_sm()
                .child(header)
                .when(state.busy, |pane| {
                    pane.child(div().px_2().child(text("git.loading")))
                })
                .when_some(self.render_git_changes(state, cx), |pane, changes| {
                    pane.child(changes)
                })
                .when_some(self.render_git_graph(state, cx), |pane, graph| {
                    pane.child(graph)
                })
                .when(!state.working.is_open() && !state.graph.is_open(), |pane| {
                    pane.when(state.snapshot != state.head, |pane| {
                        pane.child(self.git_snapshot_banner(state, cx))
                    })
                    .child(tree)
                    .child(bottom)
                })
                .into_any_element(),
        )
    }
}

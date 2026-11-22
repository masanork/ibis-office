// SPDX-License-Identifier: AGPL-3.0-only

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    GitWorkspace, Result, filename, short,
    working_tree::{git, git_diff_document},
};
use crate::{editor::SlideEditor, i18n::text};

const FILES_PER_PAGE: usize = 100;

pub(super) struct CommitChanges {
    id: gix::ObjectId,
    merge: bool,
    files: Vec<(u8, PathBuf)>,
    visible: usize,
}

const DIFF_TREE: &[&str] = &[
    "-c",
    "core.quotePath=false",
    "diff-tree",
    "--root",
    "--diff-merges=first-parent",
    "--no-commit-id",
    "-r",
    "--no-renames",
    "--no-ext-diff",
    "--no-textconv",
    "--no-color",
    "--submodule=short",
];

fn commit_changes(root: &Path, id: gix::ObjectId) -> Result<CommitChanges> {
    let repo = gix::discover(root)?;
    if repo
        .shallow_commits()?
        .as_ref()
        .is_some_and(|boundaries| boundaries.contains(&id))
    {
        return Err(text("git.commit-parent-unavailable").to_string().into());
    }
    let merge = repo.find_commit(id)?.parent_ids().nth(1).is_some();
    let commit = id.to_string();
    let args: Vec<_> = DIFF_TREE
        .iter()
        .copied()
        .chain(["--name-status", "-z", &commit, "--"])
        .map(OsStr::new)
        .collect();
    let bytes = git(root, &args)?;
    let mut parts = bytes
        .strip_suffix(b"\0")
        .unwrap_or(&bytes)
        .split(|byte| *byte == 0);
    let mut files = Vec::new();
    if !bytes.is_empty() {
        while let Some(status) = parts.next() {
            let [code] = status else {
                return Err("Invalid Git commit status".into());
            };
            let path = parts.next().ok_or("Missing Git change path")?;
            let path = path
                .split(|byte| *byte == b'/')
                .map(filename)
                .collect::<Result<PathBuf>>()?;
            files.push((*code, path));
        }
    }
    Ok(CommitChanges {
        id,
        merge,
        files,
        visible: FILES_PER_PAGE,
    })
}

impl SlideEditor {
    pub(super) fn git_show_commit_changes(
        &mut self,
        id: gix::ObjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let root = state.worktree.clone().unwrap_or_else(|| state.repo.clone());
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { commit_changes(&root, id).map_err(|error| error.to_string()) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok(changes) => state.commit_changes = Some(changes),
                    Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn git_show_commit_diff(
        &mut self,
        id: gix::ObjectId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let root = state.worktree.clone().unwrap_or_else(|| state.repo.clone());
        let name = format!("{} · {}", path.display(), short(id));
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let commit = id.to_string();
                    let mut args: Vec<_> = DIFF_TREE
                        .iter()
                        .copied()
                        .chain(["--patch", &commit, "--"])
                        .map(OsStr::new)
                        .collect();
                    args.push(path.as_os_str());
                    git_diff_document(&root, &args, false).map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                editor.present_git_preview(result, name, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_commit_changes(
        &self,
        state: &GitWorkspace,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let changes = state.commit_changes.as_ref()?;
        let id = changes.id;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let toolbar = h_flex()
            .px_2()
            .pb_1()
            .gap_2()
            .child(
                Button::new("git-back-to-graph")
                    .xsmall()
                    .disabled(state.busy)
                    .label(text("git.back-to-graph"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(state) = this.git_workspace.as_mut() {
                            state.commit_changes = None;
                            cx.notify();
                        }
                    })),
            )
            .child(div().min_w_0().truncate().child(format!(
                "{} · {}",
                short(id),
                text(if changes.merge {
                    "git.first-parent"
                } else {
                    "git.changes"
                })
            )));
        let mut list = div()
            .id("git-commit-files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for (index, (code, path)) in changes.files.iter().take(changes.visible).enumerate() {
            let path = path.clone();
            list = list.child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(border)
                    .child(div().text_color(muted).child(char::from(*code).to_string()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(path.to_string_lossy().into_owned()),
                    )
                    .child(
                        Button::new(("git-commit-file-diff", index))
                            .xsmall()
                            .disabled(state.busy)
                            .label(text("git.diff"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.git_show_commit_diff(id, path.clone(), window, cx)
                            })),
                    ),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(toolbar)
                .child(
                    list.when(changes.files.is_empty(), |list| {
                        list.child(div().px_2().child(text("git.no-changes")))
                    })
                    .when(changes.visible < changes.files.len(), |list| {
                        list.child(
                            Button::new("git-commit-files-more")
                                .xsmall()
                                .label(text("git.more-files"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(changes) = this
                                        .git_workspace
                                        .as_mut()
                                        .and_then(|state| state.commit_changes.as_mut())
                                    {
                                        changes.visible += FILES_PER_PAGE;
                                        cx.notify();
                                    }
                                })),
                        )
                    }),
                )
                .into_any_element(),
        )
    }
}

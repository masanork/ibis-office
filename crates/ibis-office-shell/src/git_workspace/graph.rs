// SPDX-License-Identifier: AGPL-3.0-only

use std::ffi::OsStr;
use std::path::Path;

use gpui_kit::component::button::Button;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{GitWorkspace, Result, entries, short, working_tree::git};
use crate::{editor::SlideEditor, i18n::text};

const PAGE: usize = 40;

struct Row {
    lanes: String,
    commit: Option<Commit>,
}

struct Commit {
    id: gix::ObjectId,
    date: String,
    refs: String,
    subject: String,
}

#[derive(Default)]
pub(super) struct CommitGraph {
    open: bool,
    rows: Vec<Row>,
    limit: usize,
    more: bool,
}

impl CommitGraph {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }
}

fn read_graph(root: &Path, head: gix::ObjectId, limit: usize) -> Result<(Vec<Row>, bool)> {
    let count_arg = format!("--max-count={}", limit + 1);
    let tip = head.to_string();
    let args = [
        "--no-pager",
        "log",
        "--graph",
        "--branches",
        "--remotes",
        "--no-color",
        "--no-show-signature",
        "--no-notes",
        "--decorate=short",
        "--encoding=UTF-8",
        "--format=%x00%H%x00%as%x00%D%x00%s",
        &count_arg,
        &tip,
        "--",
    ];
    let output = git(root, &args.map(OsStr::new))?;
    let output = String::from_utf8_lossy(&output);
    let mut rows = Vec::new();
    let mut count = 0;
    for line in output.lines().filter(|line| !line.is_empty()) {
        let mut fields = line.splitn(5, '\0');
        let lanes = fields.next().unwrap_or_default().replace(' ', "\u{a0}");
        let commit = match fields.next() {
            Some(id) => {
                if count == limit {
                    return Ok((rows, true));
                }
                count += 1;
                Some(Commit {
                    id: gix::ObjectId::from_hex(id.as_bytes())?,
                    date: fields.next().ok_or("Missing Git commit date")?.to_owned(),
                    refs: fields.next().ok_or("Missing Git refs")?.to_owned(),
                    subject: fields.next().ok_or("Missing Git subject")?.to_owned(),
                })
            }
            None => None,
        };
        rows.push(Row { lanes, commit });
    }
    Ok((rows, false))
}

impl SlideEditor {
    fn git_load_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let limit = state.graph.limit + PAGE;
        let root = state.worktree.clone().unwrap_or_else(|| state.repo.clone());
        let head = state.head;
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        read_graph(&root, head, limit).map_err(|error| error.to_string())
                    })
                    .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok((rows, more)) => {
                        state.graph.rows = rows;
                        state.graph.more = more;
                        state.graph.limit = limit;
                    }
                    Err(error) => {
                        state.graph.open = false;
                        editor.show_alert(text("git.failed"), error.into(), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn git_browse_snapshot(
        &mut self,
        id: gix::ObjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let repo = state.repo.clone();
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let read = || -> Result<_> { entries(&gix::open(repo)?, id, Path::new("")) };
                    read().map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok(rows) => {
                        state.snapshot = id;
                        state.tree.clear();
                        state.tree.insert(Default::default(), rows);
                        state.file = None;
                        state.revisions.clear();
                        state.older = None;
                        state.base = None;
                        state.revised = None;
                        state.graph.open = false;
                    }
                    Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn git_graph_toggle(&self, state: &GitWorkspace, cx: &mut Context<Self>) -> Button {
        Button::new("git-graph-toggle")
            .xsmall()
            .disabled(state.busy)
            .label(text(if state.graph.open {
                "git.files"
            } else {
                "git.graph"
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(state) = this.git_workspace.as_mut().filter(|s| !s.busy) {
                    state.graph.open = !state.graph.open;
                    if state.graph.open && state.graph.limit == 0 {
                        this.git_load_graph(window, cx);
                    }
                    cx.notify();
                }
            }))
    }

    pub(super) fn git_snapshot_banner(
        &self,
        state: &GitWorkspace,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let head = state.head;
        h_flex()
            .px_2()
            .gap_2()
            .child(format!(
                "{} {}",
                text("git.snapshot"),
                short(state.snapshot)
            ))
            .child(
                Button::new("git-back-to-head")
                    .xsmall()
                    .disabled(state.busy)
                    .label(text("git.back-to-head"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.git_browse_snapshot(head, window, cx)
                    })),
            )
    }

    pub(super) fn render_git_graph(
        &self,
        state: &GitWorkspace,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !state.graph.open {
            return None;
        }
        let theme = cx.theme();
        let font = Theme::global(cx).mono_font_family.clone();
        let primary = theme.primary;
        let accent = theme.accent;
        let mut body = div().id("git-graph").flex_1().min_h_0().overflow_y_scroll();
        for (index, row) in state.graph.rows.iter().enumerate() {
            let line = h_flex().h(px(22.)).px_2().min_w_0().child(
                div()
                    .flex_shrink_0()
                    .font_family(font.clone())
                    .text_color(primary)
                    .child(row.lanes.clone()),
            );
            body = body.child(match &row.commit {
                Some(commit) => {
                    let id = commit.id;
                    let refs = if commit.refs.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", commit.refs)
                    };
                    let label =
                        format!("{} {}{} · {}", short(id), commit.subject, refs, commit.date);
                    let full_label: SharedString =
                        format!("{} {}{} · {}", id, commit.subject, refs, commit.date).into();
                    line.id(("git-graph-commit", index))
                        .cursor_pointer()
                        .when(id == state.snapshot, |line| line.bg(accent))
                        .child(div().min_w_0().truncate().child(label))
                        .tooltip(move |window, cx| {
                            Tooltip::new(full_label.clone()).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.git_browse_snapshot(id, window, cx)
                        }))
                        .into_any_element()
                }
                None => line.into_any_element(),
            });
        }
        Some(
            body.when(state.graph.more, |body| {
                body.child(
                    Button::new("git-graph-more")
                        .xsmall()
                        .disabled(state.busy)
                        .label(text("git.older"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.git_load_graph(window, cx)),
                        ),
                )
            })
            .into_any_element(),
        )
    }
}

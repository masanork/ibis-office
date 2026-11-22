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
    completed: Option<&'static str>,
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

fn fetch_remotes(root: &Path) -> Result<()> {
    if gix::discover(root)?.remote_names().is_empty() {
        return Err(text("git.no-remotes").to_string().into());
    }
    let args = [
        "fetch",
        "--all",
        "--no-recurse-submodules",
        "--no-prune",
        "--no-prune-tags",
    ];
    git(root, &args.map(OsStr::new))?;
    Ok(())
}

#[derive(PartialEq, Eq)]
struct PushTarget {
    branch: String,
    remote: String,
    reference: String,
    urls: Vec<u8>,
}

impl PushTarget {
    fn read(repo: &gix::Repository) -> Result<Self> {
        let missing = || text("git.push-no-upstream").to_string();
        let branch = repo.head_name()?.ok_or_else(missing)?;
        let direction = gix::remote::Direction::Fetch;
        let remote = repo
            .branch_remote_name(branch.shorten(), direction)
            .ok_or_else(missing)?;
        if remote.as_bstr() == "." || !repo.remote_names().contains(remote.as_bstr()) {
            return Err(missing().into());
        }
        let reference = repo
            .branch_remote_ref_name(branch.as_ref(), direction)
            .ok_or_else(missing)??;
        let reference = std::str::from_utf8(reference.as_bstr())?;
        if !reference.starts_with("refs/heads/") {
            return Err(missing().into());
        }
        let remote = std::str::from_utf8(remote.as_bstr())?;
        let args = ["remote", "get-url", "--push", "--all", "--", remote];
        let urls = git(
            repo.workdir().unwrap_or_else(|| repo.path()),
            &args.map(OsStr::new),
        )?;
        Ok(Self {
            branch: std::str::from_utf8(branch.shorten())?.to_owned(),
            remote: remote.to_owned(),
            reference: reference.to_owned(),
            urls,
        })
    }

    fn push(&self, root: &Path, head: gix::ObjectId) -> Result<()> {
        let repo = gix::discover(root)?;
        if repo.head_id()?.detach() != head || Self::read(&repo)? != *self {
            return Err(text("git.push-target-changed").to_string().into());
        }
        let no_mirror = format!("remote.{}.mirror=false", self.remote);
        let refspec = format!("{head}:{}", self.reference);
        let args = [
            "-c",
            &no_mirror,
            "push",
            "--no-follow-tags",
            "--no-recurse-submodules",
            "--",
            &self.remote,
            &refspec,
        ];
        git(root, &args.map(OsStr::new))?;
        Ok(())
    }
}

fn pull_upstream(root: &Path, head: gix::ObjectId, branch: &str) -> Result<()> {
    let repo = gix::discover(root)?;
    let name = repo
        .head_name()?
        .ok_or_else(|| text("git.pull-no-upstream").to_string())?;
    if repo.head_id()?.detach() != head || name.shorten() != branch {
        return Err(text("git.pull-head-changed").to_string().into());
    }
    let direction = gix::remote::Direction::Fetch;
    let remote = repo.branch_remote_name(name.shorten(), direction);
    let upstream = repo
        .branch_remote_ref_name(name.as_ref(), direction)
        .transpose()?;
    let (Some(remote), Some(upstream)) = (remote, upstream) else {
        return Err(text("git.pull-no-upstream").to_string().into());
    };
    if remote.as_bstr() == "."
        || !repo.remote_names().contains(remote.as_bstr())
        || !upstream.as_bstr().starts_with(b"refs/heads/")
    {
        return Err(text("git.pull-no-upstream").to_string().into());
    }
    let upstream = std::str::from_utf8(upstream.as_bstr())?;
    // Allow fetch to update a tracking ref, while keeping local branches subject to the fast-forward check.
    let tracking = repo
        .branch_remote_tracking_ref_name(name.as_ref(), direction)
        .transpose()?
        .filter(|reference| reference.as_bstr().starts_with(b"refs/remotes/"));
    let refspec = match tracking {
        Some(reference) => format!("+{upstream}:{}", std::str::from_utf8(reference.as_bstr())?),
        None => upstream.to_owned(),
    };
    let args = [
        "pull",
        "--ff-only",
        "--no-rebase",
        "--no-squash",
        "--no-autostash",
        "--no-recurse-submodules",
        "--no-prune",
        "--refmap=",
        "--",
        std::str::from_utf8(remote.as_bstr())?,
        &refspec,
    ];
    git(root, &args.map(OsStr::new))?;
    Ok(())
}

enum SyncAction {
    Fetch,
    Pull,
    Push(PushTarget),
}

impl SyncAction {
    fn failure(&self) -> &'static str {
        match self {
            Self::Fetch => "git.fetch-failed",
            Self::Pull => "git.pull-failed",
            Self::Push(_) => "git.push-failed",
        }
    }

    fn completion(&self) -> &'static str {
        match self {
            Self::Fetch => "git.fetch-complete",
            Self::Pull => "git.pull-complete",
            Self::Push(_) => "git.push-complete",
        }
    }

    fn run(&self, root: &Path, head: gix::ObjectId, branch: &str) -> Result<()> {
        match self {
            Self::Fetch => fetch_remotes(root),
            Self::Pull => pull_upstream(root, head, branch),
            Self::Push(target) => target.push(root, head),
        }
    }
}

impl SlideEditor {
    fn git_load_graph(
        &mut self,
        action: Option<SyncAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let limit = if action.is_some() {
            state.graph.completed = None;
            state.graph.limit.max(PAGE)
        } else {
            state.graph.limit + PAGE
        };
        let root = state.worktree.clone().unwrap_or_else(|| state.repo.clone());
        let head = state.head;
        let branch = state.branch.clone();
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let (result, sync_error, action, updated_workspace) = cx
                .background_executor()
                .spawn(async move {
                    let sync_error = action
                        .as_ref()
                        .and_then(|action| action.run(&root, head, &branch).err())
                        .map(|error| error.to_string());
                    let updated_workspace =
                        if matches!(action, Some(SyncAction::Pull)) && sync_error.is_none() {
                            GitWorkspace::open(&root)
                                .map(|workspace| (workspace.head != head).then_some(workspace))
                                .map_err(|error| error.to_string())
                        } else {
                            Ok(None)
                        };
                    let graph_head = updated_workspace
                        .as_ref()
                        .ok()
                        .and_then(Option::as_ref)
                        .map_or(head, |workspace| workspace.head);
                    let result =
                        read_graph(&root, graph_head, limit).map_err(|error| error.to_string());
                    (result, sync_error, action, updated_workspace)
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                let workspace_error = match updated_workspace {
                    Ok(Some(mut updated)) => {
                        updated.graph = std::mem::take(&mut state.graph);
                        updated.working = std::mem::take(&mut state.working);
                        *state = updated;
                        None
                    }
                    Ok(None) => None,
                    Err(error) => Some(error),
                };
                if action.is_some() && sync_error.is_none() {
                    state.graph.completed = action.as_ref().map(SyncAction::completion);
                }
                let graph_error = match result {
                    Ok((rows, more)) => {
                        state.graph.rows = rows;
                        state.graph.more = more;
                        state.graph.limit = limit;
                        None
                    }
                    Err(error) => {
                        state.graph.open = state.graph.limit != 0;
                        Some(error)
                    }
                };
                if let (Some(error), Some(action)) = (sync_error, action) {
                    editor.show_alert(text(action.failure()), error.into(), cx);
                } else if let Some(error) = workspace_error.or(graph_error) {
                    editor.show_alert(text("git.failed"), error.into(), cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn git_confirm_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_ref().filter(|state| !state.busy) else {
            return;
        };
        let path = state.repo.clone();
        let head = state.head;
        cx.spawn_in(window, async move |this, cx| {
            let repo_path = path.clone();
            let target = cx
                .background_executor()
                .spawn(async move {
                    let read = || -> Result<_> {
                        let repo = gix::open(repo_path)?;
                        if repo.head_id()?.detach() != head {
                            return Err(text("git.push-target-changed").to_string().into());
                        }
                        PushTarget::read(&repo)
                    };
                    read().map_err(|error| error.to_string())
                })
                .await;
            let target = match target {
                Ok(target) => target,
                Err(error) => {
                    let _ = this.update(cx, |editor, cx| {
                        editor.show_alert(text("git.push-failed"), error.into(), cx);
                    });
                    return;
                }
            };
            let destination = target.reference.trim_start_matches("refs/heads/");
            let detail = format!(
                "{} ({}) → {}/{}",
                target.branch,
                short(head),
                target.remote,
                destination
            );
            let buttons = [
                PromptButton::ok(text("git.push")),
                PromptButton::cancel(text("_shell.cancel")),
            ];
            let Ok(response) = this.update_in(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Info,
                    &text("git.push-confirm"),
                    Some(&detail),
                    &buttons,
                    cx,
                )
            }) else {
                return;
            };
            if matches!(response.await, Ok(0)) {
                let _ = this.update_in(cx, |editor, window, cx| {
                    if editor.git_workspace.as_ref().is_some_and(|state| {
                        state.repo == path && state.head == head && state.graph.open
                    }) {
                        editor.git_load_graph(Some(SyncAction::Push(target)), window, cx);
                    }
                });
            }
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
                        this.git_load_graph(None, window, cx);
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
        let muted = theme.muted_foreground;
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
        let toolbar = h_flex()
            .px_2()
            .pb_1()
            .gap_2()
            .child(
                Button::new("git-fetch")
                    .xsmall()
                    .disabled(state.busy)
                    .label(text("git.fetch"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.git_load_graph(Some(SyncAction::Fetch), window, cx)
                    })),
            )
            .when(state.worktree.is_some(), |bar| {
                bar.child(
                    Button::new("git-pull")
                        .xsmall()
                        .disabled(state.busy)
                        .label(text("git.pull"))
                        .tooltip(text("git.pull-ff-only"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.git_load_graph(Some(SyncAction::Pull), window, cx)
                        })),
                )
            })
            .child(
                Button::new("git-push")
                    .xsmall()
                    .disabled(state.busy)
                    .label(text("git.push"))
                    .on_click(cx.listener(|this, _, window, cx| this.git_confirm_push(window, cx))),
            )
            .when_some(state.graph.completed, |bar, completed| {
                bar.child(div().text_color(muted).child(text(completed)))
            });
        let body = body.when(state.graph.more, |body| {
            body.child(
                Button::new("git-graph-more")
                    .xsmall()
                    .disabled(state.busy)
                    .label(text("git.older"))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.git_load_graph(None, window, cx)),
                    ),
            )
        });
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(toolbar)
                .child(body)
                .into_any_element(),
        )
    }
}

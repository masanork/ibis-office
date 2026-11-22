// SPDX-License-Identifier: AGPL-3.0-only

use std::ffi::OsStr;
use std::path::Path;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
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
    upstream: Option<UpstreamStatus>,
    branches: Option<Branches>,
    new_branch: Option<Entity<InputState>>,
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

enum UpstreamStatus {
    NoUpstream,
    Detached,
    Missing,
    Unavailable,
    Counts {
        name: String,
        outgoing: usize,
        incoming: usize,
        shallow: bool,
    },
}

impl UpstreamStatus {
    fn label(&self) -> String {
        match self {
            Self::NoUpstream => text("git.status.no-upstream").to_string(),
            Self::Detached => text("git.status.detached").to_string(),
            Self::Missing => text("git.status.upstream-missing").to_string(),
            Self::Unavailable => text("git.status.upstream-unavailable").to_string(),
            Self::Counts {
                name,
                outgoing,
                incoming,
                shallow,
            } => {
                let counts = format!(
                    "{name} · {} {outgoing} · {} {incoming}",
                    text("git.status.outgoing"),
                    text("git.status.incoming"),
                );
                if *shallow {
                    format!("{counts} · {}", text("git.status.shallow"))
                } else {
                    counts
                }
            }
        }
    }
}

fn upstream_status(root: &Path, head: gix::ObjectId, branch: &str) -> Result<UpstreamStatus> {
    let repo = gix::discover(root)?;
    let Some(local) = repo.head_name()? else {
        return Ok(UpstreamStatus::Detached);
    };
    if repo.head_id()?.detach() != head || local.shorten() != branch {
        return Ok(UpstreamStatus::Unavailable);
    }
    let direction = gix::remote::Direction::Fetch;
    if repo
        .branch_remote_ref_name(local.as_ref(), direction)
        .transpose()?
        .is_none()
    {
        return Ok(UpstreamStatus::NoUpstream);
    }
    let Some(tracking) = repo
        .branch_remote_tracking_ref_name(local.as_ref(), direction)
        .transpose()?
    else {
        return Ok(UpstreamStatus::Missing);
    };
    let Some(mut upstream) = repo.try_find_reference(tracking.as_ref())? else {
        return Ok(UpstreamStatus::Missing);
    };
    let upstream_id = upstream.peel_to_id()?.detach();
    let (outgoing, incoming) = if head == upstream_id {
        (0, 0)
    } else {
        let range = format!("{head}...{upstream_id}");
        let args = ["rev-list", "--left-right", "--count", &range, "--"];
        let output = git(root, &args.map(OsStr::new))?;
        let mut counts = std::str::from_utf8(&output)?.split_whitespace();
        (
            counts.next().ok_or("Missing Git outgoing count")?.parse()?,
            counts.next().ok_or("Missing Git incoming count")?.parse()?,
        )
    };
    Ok(UpstreamStatus::Counts {
        name: tracking.shorten().to_string(),
        outgoing,
        incoming,
        shallow: repo.is_shallow()?,
    })
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
    publish: bool,
}

impl PushTarget {
    fn read(repo: &gix::Repository) -> Result<Self> {
        let missing = || text("git.push-no-upstream").to_string();
        let branch = repo.head_name()?.ok_or_else(missing)?;
        let upstream = repo
            .branch_remote_ref_name(branch.as_ref(), gix::remote::Direction::Fetch)
            .transpose()?;
        let publish = upstream.is_none();
        let (reference, remote) = if let Some(upstream) = upstream {
            let remote = repo
                .branch_remote_name(branch.shorten(), gix::remote::Direction::Fetch)
                .ok_or_else(missing)?;
            (
                std::str::from_utf8(upstream.as_bstr())?.to_owned(),
                remote.as_bstr().to_owned(),
            )
        } else {
            let direction = gix::remote::Direction::Push;
            let remote = repo
                .branch_remote_name(branch.shorten(), direction)
                .map(|remote| remote.as_bstr().to_owned())
                .or_else(|| repo.remote_default_name(direction))
                .ok_or_else(|| text("git.publish-no-remote").to_string())?;
            (std::str::from_utf8(branch.as_bstr())?.to_owned(), remote)
        };
        if remote.as_slice() == b"." || !repo.remote_names().contains(&remote) {
            return Err(text(if publish {
                "git.publish-no-remote"
            } else {
                "git.push-no-upstream"
            })
            .to_string()
            .into());
        }
        if !reference.starts_with("refs/heads/") {
            return Err(missing().into());
        }
        let remote = std::str::from_utf8(remote.as_slice())?;
        let args = ["remote", "get-url", "--push", "--all", "--", remote];
        let urls = git(
            repo.workdir().unwrap_or_else(|| repo.path()),
            &args.map(OsStr::new),
        )?;
        Ok(Self {
            branch: std::str::from_utf8(branch.shorten())?.to_owned(),
            remote: remote.to_owned(),
            reference,
            urls,
            publish,
        })
    }

    fn push(&self, root: &Path, head: gix::ObjectId) -> Result<()> {
        let repo = gix::discover(root)?;
        if repo.head_id()?.detach() != head || Self::read(&repo)? != *self {
            return Err(text("git.push-target-changed").to_string().into());
        }
        let no_mirror = format!("remote.{}.mirror=false", self.remote);
        let mut args = vec![
            "-c",
            &no_mirror,
            "push",
            "--no-follow-tags",
            "--no-recurse-submodules",
        ];
        let refspec;
        let require_absent;
        if self.publish {
            // An empty expected value permits creating the destination but rejects overwriting it.
            require_absent = format!("--force-with-lease={}:", self.reference);
            args.extend(["--set-upstream", &require_absent]);
            refspec = format!("refs/heads/{}:{}", self.branch, self.reference);
        } else {
            refspec = format!("{head}:{}", self.reference);
        }
        args.extend(["--", &self.remote, &refspec]);
        let args: Vec<_> = args.into_iter().map(OsStr::new).collect();
        git(root, &args)?;
        Ok(())
    }
}

fn pull_upstream(root: &Path, head: gix::ObjectId, branch: &str) -> Result<()> {
    let repo = gix::discover(root)?;
    let name = repo
        .head_name()?
        .ok_or_else(|| text("git.pull-no-upstream").to_string())?;
    if repo.head_id()?.detach() != head || name.shorten() != branch {
        return Err(text("git.workspace-head-changed").to_string().into());
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

#[derive(Default)]
struct Branches {
    local: Vec<String>,
    remote: Vec<String>,
}

fn branches(root: &Path) -> Result<Branches> {
    let args = [
        "for-each-ref",
        "--format=%(refname)%00%(symref)",
        "refs/heads/",
        "refs/remotes/",
    ];
    let refs = git(root, &args.map(OsStr::new))?;
    let mut branches = Branches::default();
    for line in refs.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        let Some((name, symbolic_target)) = line.split_once('\0') else {
            continue;
        };
        if !symbolic_target.is_empty() {
            continue;
        }
        if let Some(local) = name.strip_prefix("refs/heads/").filter(|name| *name != "-") {
            branches.local.push(local.to_owned());
        } else if name.starts_with("refs/remotes/") {
            branches.remote.push(name.to_owned());
        }
    }
    Ok(branches)
}

enum BranchStart {
    Local,
    Commit(gix::ObjectId),
    Remote,
}

fn switch_branch(
    root: &Path,
    head: gix::ObjectId,
    branch: &str,
    target: &str,
    start: BranchStart,
) -> Result<()> {
    let repo = gix::discover(root)?;
    let current = repo
        .head_name()?
        .map(|name| name.shorten().to_string())
        .unwrap_or_else(|| short(head));
    if repo.head_id()?.detach() != head || current != branch {
        return Err(text("git.workspace-head-changed").to_string().into());
    }
    let mut args = vec!["switch", "--no-guess", "--no-recurse-submodules"];
    let commit;
    match start {
        BranchStart::Local => args.extend(["--", target]),
        BranchStart::Remote => args.extend(["--track", "--", target]),
        BranchStart::Commit(start) => {
            if target.starts_with('-')
                || gix::refs::FullName::try_from(format!("refs/heads/{target}")).is_err()
            {
                return Err(text("git.branch-name-invalid").to_string().into());
            }
            commit = start.to_string();
            args.extend(["--no-track", "-c", target, "--", &commit]);
        }
    }
    let args: Vec<_> = args.into_iter().map(OsStr::new).collect();
    git(root, &args)?;
    Ok(())
}

enum SyncAction {
    Fetch,
    Pull,
    Push(PushTarget),
    Switch(String),
    TrackBranch(String),
    CreateBranch { name: String, start: gix::ObjectId },
}

impl SyncAction {
    fn failure(&self) -> &'static str {
        match self {
            Self::Fetch => "git.fetch-failed",
            Self::Pull => "git.pull-failed",
            Self::Push(target) if target.publish => "git.publish-failed",
            Self::Push(_) => "git.push-failed",
            Self::Switch(_) => "git.switch-failed",
            Self::TrackBranch(_) => "git.track-failed",
            Self::CreateBranch { .. } => "git.branch-create-failed",
        }
    }

    fn completion(&self) -> &'static str {
        match self {
            Self::Fetch => "git.fetch-complete",
            Self::Pull => "git.pull-complete",
            Self::Push(target) if target.publish => "git.publish-complete",
            Self::Push(_) => "git.push-complete",
            Self::Switch(_) => "git.switch-complete",
            Self::TrackBranch(_) => "git.track-complete",
            Self::CreateBranch { .. } => "git.branch-created",
        }
    }

    fn run(&self, root: &Path, head: gix::ObjectId, branch: &str) -> Result<()> {
        match self {
            Self::Fetch => fetch_remotes(root),
            Self::Pull => pull_upstream(root, head, branch),
            Self::Push(target) => target.push(root, head),
            Self::Switch(target) => switch_branch(root, head, branch, target, BranchStart::Local),
            Self::TrackBranch(target) => {
                switch_branch(root, head, branch, target, BranchStart::Remote)
            }
            Self::CreateBranch { name, start } => {
                switch_branch(root, head, branch, name, BranchStart::Commit(*start))
            }
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
        let check_upstream = state.graph.upstream.is_none() || action.is_some();
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
            let (result, sync_error, action, updated_workspace, upstream) = cx
                .background_executor()
                .spawn(async move {
                    let sync_error = action
                        .as_ref()
                        .and_then(|action| action.run(&root, head, &branch).err())
                        .map(|error| error.to_string());
                    let updated_workspace = if matches!(
                        action,
                        Some(
                            SyncAction::Pull
                                | SyncAction::Switch(_)
                                | SyncAction::TrackBranch(_)
                                | SyncAction::CreateBranch { .. }
                        )
                    ) && sync_error.is_none()
                    {
                        GitWorkspace::open(&root)
                            .map(|workspace| {
                                (workspace.head != head || workspace.branch != branch)
                                    .then_some(workspace)
                            })
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
                    let upstream = check_upstream.then(|| {
                        let graph_branch = updated_workspace
                            .as_ref()
                            .ok()
                            .and_then(Option::as_ref)
                            .map_or(branch.as_str(), |workspace| workspace.branch.as_str());
                        upstream_status(&root, graph_head, graph_branch)
                            .unwrap_or(UpstreamStatus::Unavailable)
                    });
                    (result, sync_error, action, updated_workspace, upstream)
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
                if let Some(upstream) = upstream {
                    state.graph.upstream = Some(upstream);
                }
                if action.is_some() && sync_error.is_none() {
                    state.graph.completed = action.as_ref().map(SyncAction::completion);
                    if matches!(
                        action,
                        Some(
                            SyncAction::Switch(_)
                                | SyncAction::TrackBranch(_)
                                | SyncAction::CreateBranch { .. }
                        )
                    ) {
                        state.graph.branches = None;
                        state.graph.new_branch = None;
                    }
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

    fn git_show_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_mut().filter(|state| !state.busy) else {
            return;
        };
        let Some(root) = state.worktree.clone() else {
            return;
        };
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { branches(&root).map_err(|error| error.to_string()) })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok(branches) => {
                        state.graph.branches = Some(branches);
                        state.graph.new_branch.get_or_insert_with(|| {
                            cx.new(|cx| {
                                InputState::new(window, cx).placeholder(text("git.branch-name"))
                            })
                        });
                    }
                    Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_git_branches(
        &self,
        state: &GitWorkspace,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let branches = state.graph.branches.as_ref()?;
        let input = state.graph.new_branch.as_ref()?;
        let draft = input.clone();
        let start = state.snapshot;
        let start_label = if start == state.head {
            format!("HEAD ({})", short(start))
        } else {
            short(start)
        };
        let new_branch = div()
            .px_2()
            .pb_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().child(format!("{} {start_label}", text("git.branch-start"))))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(input).small().disabled(state.busy)),
                    )
                    .child(
                        Button::new("git-create-branch")
                            .small()
                            .label(text("git.branch-create"))
                            .disabled(state.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let name = draft.read(cx).value().trim().to_string();
                                if name.is_empty() {
                                    this.show_alert(
                                        text("git.branch-create-failed"),
                                        text("git.branch-name-required"),
                                        cx,
                                    );
                                } else {
                                    this.git_load_graph(
                                        Some(SyncAction::CreateBranch { name, start }),
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    ),
            );
        let border = cx.theme().border;
        let mut list = div()
            .id("git-branch-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for (is_remote, names, key) in [
            (false, &branches.local, "git.local-branches"),
            (true, &branches.remote, "git.remote-branches"),
        ] {
            if names.is_empty() {
                continue;
            }
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text(key)),
            );
            for (index, name) in names.iter().enumerate() {
                let name = name.clone();
                let current = !is_remote && name == state.branch;
                let label = if is_remote {
                    name.strip_prefix("refs/remotes/")
                        .unwrap_or(&name)
                        .to_owned()
                } else {
                    name.clone()
                };
                list = list.child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .border_b_1()
                        .border_color(border)
                        .child(div().flex_1().min_w_0().truncate().child(label))
                        .child(
                            Button::new((
                                if is_remote {
                                    "git-track-branch"
                                } else {
                                    "git-switch-branch"
                                },
                                index,
                            ))
                            .xsmall()
                            .disabled(current || state.busy)
                            .label(text(if is_remote {
                                "git.track"
                            } else if current {
                                "git.branch-current"
                            } else {
                                "git.switch"
                            }))
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    let action = if is_remote {
                                        SyncAction::TrackBranch(name.clone())
                                    } else {
                                        SyncAction::Switch(name.clone())
                                    };
                                    this.git_load_graph(Some(action), window, cx)
                                },
                            )),
                        ),
                );
            }
        }
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(
                    h_flex()
                        .px_2()
                        .pb_1()
                        .gap_2()
                        .child(
                            Button::new("git-back-from-branches")
                                .xsmall()
                                .disabled(state.busy)
                                .label(text("git.back-to-graph"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(state) = this.git_workspace.as_mut() {
                                        state.graph.branches = None;
                                        state.graph.new_branch = None;
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(text("git.branches")),
                )
                .child(new_branch)
                .child(list.when(
                    branches.local.is_empty() && branches.remote.is_empty(),
                    |list| list.child(div().px_2().child(text("git.no-branches"))),
                ))
                .into_any_element(),
        )
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
                PromptButton::ok(text(if target.publish {
                    "git.publish"
                } else {
                    "git.push"
                })),
                PromptButton::cancel(text("_shell.cancel")),
            ];
            let Ok(response) = this.update_in(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Info,
                    &text(if target.publish {
                        "git.publish-confirm"
                    } else {
                        "git.push-confirm"
                    }),
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
        if let Some(branches) = self.render_git_branches(state, cx) {
            return Some(branches);
        }
        if let Some(detail) = self.render_commit_changes(state, cx) {
            return Some(detail);
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
                    let commit_link = div()
                        .flex_1()
                        .min_w_0()
                        .id(("git-graph-commit", index))
                        .cursor_pointer()
                        .child(div().min_w_0().truncate().child(label))
                        .tooltip(move |window, cx| {
                            Tooltip::new(full_label.clone()).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.git_browse_snapshot(id, window, cx)
                        }));
                    line.when(id == state.snapshot, |line| line.bg(accent))
                        .gap_1()
                        .child(commit_link)
                        .child(
                            Button::new(("git-commit-changes", index))
                                .xsmall()
                                .disabled(state.busy)
                                .label(text("git.changes"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.git_show_commit_changes(id, window, cx)
                                })),
                        )
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
                    .label(text(
                        if matches!(state.graph.upstream, Some(UpstreamStatus::NoUpstream)) {
                            "git.publish"
                        } else {
                            "git.push"
                        },
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.git_confirm_push(window, cx))),
            )
            .when(state.worktree.is_some(), |bar| {
                bar.child(
                    Button::new("git-branches")
                        .xsmall()
                        .disabled(state.busy)
                        .label(text("git.branches"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.git_show_branches(window, cx)),
                        ),
                )
            })
            .when_some(state.graph.completed, |bar, completed| {
                bar.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(muted)
                        .child(text(completed)),
                )
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
                .when_some(state.graph.upstream.as_ref(), |pane, status| {
                    let label: SharedString = status.label().into();
                    pane.child(
                        div()
                            .id("git-upstream-status")
                            .px_2()
                            .pb_1()
                            .min_w_0()
                            .truncate()
                            .text_color(muted)
                            .child(label.clone())
                            .tooltip(move |window, cx| {
                                Tooltip::new(label.clone()).build(window, cx)
                            }),
                    )
                })
                .child(body)
                .into_any_element(),
        )
    }
}

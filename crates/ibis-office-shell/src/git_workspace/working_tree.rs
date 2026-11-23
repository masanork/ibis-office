// SPDX-License-Identifier: AGPL-3.0-only

use std::ffi::OsStr;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{GitWorkspace, MAX_TEXT_DIFF_BYTES, Result, filename, markdown_block, short};
use crate::{editor::SlideEditor, i18n::text};

pub(super) struct Change {
    pub(super) path: PathBuf,
    pub(super) index: u8,
    pub(super) worktree: u8,
}

impl Change {
    fn conflicted(&self) -> bool {
        matches!(
            (self.index, self.worktree),
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D')
        )
    }

    fn staged(&self) -> bool {
        !self.conflicted() && self.index != b' ' && self.index != b'?'
    }

    fn unstaged(&self) -> bool {
        !self.conflicted() && self.worktree != b' '
    }
}

#[derive(Default)]
pub(super) struct WorkingTree {
    open: bool,
    changes: Vec<Change>,
    stashes: Vec<Stash>,
    merge_heads: Option<Vec<u8>>,
    message: Option<Entity<InputState>>,
}

impl WorkingTree {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }
}

enum Operation {
    Refresh,
    Stage(PathBuf),
    Unstage(PathBuf),
    Commit(String),
    Stash,
    ApplyStash(gix::ObjectId),
    AbortMerge(Vec<u8>),
}

fn git_command(root: &Path, args: &[&OsStr]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["--literal-pathspecs", "-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(name);
    }
    command
}

pub(super) fn git(root: &Path, args: &[&OsStr]) -> Result<Vec<u8>> {
    let output = git_command(root, args)
        .output()
        .map_err(|error| format!("git: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git: {}", error.trim()).into());
    }
    Ok(output.stdout)
}

#[derive(Clone, Copy)]
enum ChangeDiff {
    Conflict,
    Staged,
    Unstaged,
    Untracked,
}

impl ChangeDiff {
    fn label(self) -> &'static str {
        match self {
            Self::Conflict => "git.conflicts",
            Self::Staged => "git.staged",
            Self::Unstaged | Self::Untracked => "git.unstaged",
        }
    }
}

fn change_diff(root: &Path, path: &Path, kind: ChangeDiff) -> Result<ibis_formats::Opened> {
    let mut args = vec![
        "--no-optional-locks",
        "-c",
        "core.quotePath=false",
        "diff",
        "--patch",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--submodule=short",
    ];
    match kind {
        ChangeDiff::Conflict => args.extend(["--cc", "--"]),
        ChangeDiff::Staged => args.extend(["--cached", "--"]),
        ChangeDiff::Unstaged => args.push("--"),
        ChangeDiff::Untracked => {
            let file_type = root.join(path).symlink_metadata()?.file_type();
            if !file_type.is_file() && !file_type.is_symlink() {
                return Err(text("git.diff-file-only").to_string().into());
            }
            args.extend(["--no-index", "--", "/dev/null"]);
        }
    }
    let mut args: Vec<_> = args.into_iter().map(OsStr::new).collect();
    args.push(path.as_os_str());
    git_diff_document(root, &args, matches!(kind, ChangeDiff::Untracked))
}

pub(super) fn git_diff_document(
    root: &Path,
    args: &[&OsStr],
    no_index: bool,
) -> Result<ibis_formats::Opened> {
    let mut child = git_command(root, args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped Git diff output")
        .take((MAX_TEXT_DIFF_BYTES + 1) as u64)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() > MAX_TEXT_DIFF_BYTES {
        let _ = child.kill();
    }
    let status = child.wait()?;
    read?;
    if bytes.len() > MAX_TEXT_DIFF_BYTES {
        return Err(text("git.diff-too-large").to_string().into());
    }
    // Git's no-index diff uses exit code 1 to report differing inputs.
    if !(status.success() || no_index && status.code() == Some(1) && !bytes.is_empty()) {
        return Err(text("git.diff-failed").to_string().into());
    }
    let markdown = if bytes.is_empty() {
        text("git.no-changes").to_string().into_bytes()
    } else {
        markdown_block(&String::from_utf8_lossy(&bytes), "diff")
    };
    Ok(ibis_formats::markdown::open_markdown(markdown)?)
}

pub(super) fn status(root: &Path, scope: Option<&Path>) -> Result<Vec<Change>> {
    let options = [
        "--no-optional-locks",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--no-renames",
    ];
    let mut args = options.map(OsStr::new).to_vec();
    if let Some(scope) = scope {
        args.extend([OsStr::new("--ignore-submodules=all"), OsStr::new("--")]);
        if !scope.as_os_str().is_empty() {
            args.push(scope.as_os_str());
        }
    }
    let bytes = git(root, &args)?;
    let mut changes = Vec::new();
    for row in bytes.split(|byte| *byte == 0).filter(|row| !row.is_empty()) {
        let [index, worktree, b' ', tail @ ..] = row else {
            return Err("Invalid Git status response".into());
        };
        let mut path = PathBuf::new();
        let tail = tail.strip_suffix(b"/").unwrap_or(tail);
        for segment in tail.split(|byte| *byte == b'/') {
            path.push(filename(segment)?);
        }
        changes.push(Change {
            path,
            index: *index,
            worktree: *worktree,
        });
    }
    Ok(changes)
}

struct Stash {
    id: gix::ObjectId,
    title: String,
}

fn list_stashes(root: &Path) -> Result<Vec<Stash>> {
    let args = [
        "stash",
        "list",
        "--encoding=UTF-8",
        "--format=%H%x00%gs",
        "-z",
    ];
    let output = git(root, &args.map(OsStr::new))?;
    if output.is_empty() {
        return Ok(Vec::new());
    }
    let mut fields = output
        .strip_suffix(b"\0")
        .ok_or("Invalid Git stash list")?
        .split(|byte| *byte == 0);
    let mut stashes = Vec::new();
    while let Some(id) = fields.next() {
        let title = fields.next().ok_or("Missing Git stash message")?;
        stashes.push(Stash {
            id: gix::ObjectId::from_hex(id)?,
            title: String::from_utf8_lossy(title).into_owned(),
        });
    }
    Ok(stashes)
}

struct Outcome {
    changes: std::result::Result<Vec<Change>, String>,
    workspace: std::result::Result<Option<GitWorkspace>, String>,
    stashes: std::result::Result<Vec<Stash>, String>,
    operation_error: Option<(&'static str, String)>,
    committed: bool,
    merge_heads: Option<Vec<u8>>,
}

fn perform(
    root: &Path,
    head: gix::ObjectId,
    branch: &str,
    operation: Operation,
) -> Result<Outcome> {
    let current = GitWorkspace::open(root)?;
    let shifted = current.head != head || current.branch != branch;
    if shifted && !matches!(operation, Operation::Refresh) {
        return Err("Git HEAD changed. Refresh the changes list and try again.".into());
    }
    let committed = matches!(operation, Operation::Commit(_));
    let mut operation_error = None;
    let merge_head_path = current.repo.join("MERGE_HEAD");
    match operation {
        Operation::Refresh => {}
        Operation::Stage(path) => {
            git(
                root,
                &[OsStr::new("add"), OsStr::new("--"), path.as_os_str()],
            )?;
        }
        Operation::Unstage(path) => {
            git(
                root,
                &[
                    OsStr::new("restore"),
                    OsStr::new("--staged"),
                    OsStr::new("--"),
                    path.as_os_str(),
                ],
            )?;
        }
        Operation::Commit(message) => {
            git(
                root,
                &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new(&message)],
            )?;
        }
        Operation::Stash => {
            // Git stash uses internal pathspecs when removing saved untracked files.
            let args = [
                "--no-literal-pathspecs",
                "stash",
                "push",
                "--include-untracked",
            ];
            operation_error = git(root, &args.map(OsStr::new))
                .err()
                .map(|error| ("git.stash-failed", error.to_string()));
        }
        Operation::ApplyStash(id) => {
            let id = id.to_string();
            let args = ["--no-literal-pathspecs", "stash", "apply", "--index", &id];
            operation_error = git(root, &args.map(OsStr::new))
                .err()
                .map(|error| ("git.stash-failed", error.to_string()));
        }
        Operation::AbortMerge(expected) => {
            let result = if std::fs::read(&merge_head_path).ok().as_ref() == Some(&expected) {
                let args = ["--no-literal-pathspecs", "merge", "--abort"];
                git(root, &args.map(OsStr::new)).map(|_| ())
            } else {
                Err(text("git.merge-abort-changed").to_string().into())
            };
            operation_error = result
                .err()
                .map(|error| ("git.merge-abort-failed", error.to_string()));
        }
    }
    let merge_heads = std::fs::read(merge_head_path).ok();
    let workspace = if committed {
        GitWorkspace::open(root)
            .map(Some)
            .map_err(|error| error.to_string())
    } else {
        Ok(shifted.then_some(current))
    };
    Ok(Outcome {
        changes: status(root, None).map_err(|error| error.to_string()),
        workspace,
        stashes: list_stashes(root).map_err(|error| error.to_string()),
        operation_error,
        committed,
        merge_heads,
    })
}

impl SlideEditor {
    pub(super) fn git_show_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        state.working.open = !state.working.open;
        if state.working.open {
            state.working.message.get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).placeholder(text("git.commit-message")))
            });
            self.git_operate(Operation::Refresh, window, cx);
        }
        cx.notify();
    }

    fn git_confirm_abort_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_ref().filter(|state| !state.busy) else {
            return;
        };
        let Some(merge_heads) = state.working.merge_heads.clone() else {
            return;
        };
        let repo = state.repo.clone();
        let head = state.head;
        let branch = state.branch.clone();
        let buttons = [
            PromptButton::ok(text("git.merge-abort")),
            PromptButton::cancel(text("_shell.cancel")),
        ];
        let response = window.prompt(
            PromptLevel::Warning,
            &text("git.merge-abort-confirm"),
            Some(&text("git.merge-abort-detail")),
            &buttons,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if matches!(response.await, Ok(0)) {
                let _ = this.update_in(cx, |editor, window, cx| {
                    if editor.git_workspace.as_ref().is_some_and(|state| {
                        state.repo == repo && state.head == head && state.branch == branch
                    }) {
                        editor.git_operate(Operation::AbortMerge(merge_heads), window, cx);
                    }
                });
            }
        })
        .detach();
    }

    fn git_operate(&mut self, operation: Operation, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let Some(root) = state.worktree.clone() else {
            return;
        };
        let head = state.head;
        let branch = state.branch.clone();
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    perform(&root, head, &branch, operation).map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| {
                let Some(state) = editor.git_workspace.as_mut() else {
                    return;
                };
                state.busy = false;
                match result {
                    Ok(outcome) => {
                        state.working.merge_heads = outcome.merge_heads;
                        if outcome.committed
                            && let Some(message) = &state.working.message
                        {
                            message.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        let error = match outcome.changes {
                            Ok(changes) => {
                                state.working.changes = changes;
                                None
                            }
                            Err(error) => {
                                state.working.changes.clear();
                                Some(error)
                            }
                        };
                        let stash_list_error = match outcome.stashes {
                            Ok(stashes) => {
                                state.working.stashes = stashes;
                                None
                            }
                            Err(error) => {
                                state.working.stashes.clear();
                                Some(error)
                            }
                        };
                        let error = match outcome.workspace {
                            Ok(Some(mut updated)) => {
                                updated.working = std::mem::take(&mut state.working);
                                *state = updated;
                                error
                            }
                            Ok(None) => error,
                            Err(error) => Some(error),
                        };
                        if let Some((title, error)) = outcome.operation_error {
                            editor.show_alert(text(title), error.into(), cx);
                        } else if let Some(error) = error.or(stash_list_error) {
                            editor.show_alert(
                                text(if outcome.committed {
                                    "git.commit-refresh-failed"
                                } else {
                                    "git.failed"
                                }),
                                error.into(),
                                cx,
                            );
                        }
                    }
                    Err(error) => editor.show_alert(text("git.failed"), error.into(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn git_show_change_diff(
        &mut self,
        path: PathBuf,
        kind: ChangeDiff,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_workspace.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let Some(root) = state.worktree.clone() else {
            return;
        };
        let name = format!("{} · {}", path.display(), text(kind.label()));
        state.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        change_diff(&root, &path, kind).map_err(|error| error.to_string())
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

    pub(super) fn render_git_changes(
        &self,
        state: &GitWorkspace,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !state.working.open {
            return None;
        }
        let merging = state.working.merge_heads.is_some();
        let theme = cx.theme();
        let message = state.working.message.as_ref()?;
        let staged = state
            .working
            .changes
            .iter()
            .filter(|change| change.staged())
            .count();
        let mut list = div()
            .id("git-changes-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        let has_conflict = state.working.changes.iter().any(Change::conflicted);
        if has_conflict {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text("git.conflicts")),
            );
        }
        for (index, change) in state
            .working
            .changes
            .iter()
            .enumerate()
            .filter(|(_, change)| change.conflicted())
        {
            let path = change.path.clone();
            let diff_path = path.clone();
            let open_path = path.clone();
            let flags = format!(
                "{}{}",
                char::from(change.index),
                char::from(change.worktree)
            );
            list = list.child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().text_color(theme.muted_foreground).child(flags))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(path.to_string_lossy().into_owned()),
                    )
                    .when((change.index, change.worktree) != (b'D', b'D'), |row| {
                        row.child(
                            Button::new(("git-conflict-open", index))
                                .xsmall()
                                .disabled(state.busy)
                                .label(text("git.conflict-open"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.git_open_worktree_path(&open_path, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new(("git-conflict-diff", index))
                            .xsmall()
                            .disabled(state.busy)
                            .label(text("git.diff"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.git_show_change_diff(
                                    diff_path.clone(),
                                    ChangeDiff::Conflict,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(("git-conflict-resolve", index))
                            .xsmall()
                            .disabled(state.busy)
                            .label(text("git.conflict-resolve"))
                            .tooltip(text("git.conflict-resolve-tooltip"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.git_operate(Operation::Stage(path.clone()), window, cx);
                            })),
                    ),
            );
        }
        for (staged_group, title) in [(true, "git.staged"), (false, "git.unstaged")] {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text(title)),
            );
            for (index, change) in state
                .working
                .changes
                .iter()
                .enumerate()
                .filter(|(_, change)| {
                    if staged_group {
                        change.staged()
                    } else {
                        change.unstaged()
                    }
                })
            {
                let path = change.path.clone();
                let status_code = if staged_group {
                    change.index
                } else {
                    change.worktree
                };
                let kind = if staged_group {
                    ChangeDiff::Staged
                } else if status_code == b'?' {
                    ChangeDiff::Untracked
                } else {
                    ChangeDiff::Unstaged
                };
                let diff_path = path.clone();
                let row = h_flex()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(char::from(status_code).to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(path.to_string_lossy().into_owned()),
                    )
                    .child(
                        Button::new((
                            if staged_group {
                                "git-staged-diff"
                            } else {
                                "git-unstaged-diff"
                            },
                            index,
                        ))
                        .xsmall()
                        .disabled(state.busy)
                        .label(text("git.diff"))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.git_show_change_diff(diff_path.clone(), kind, window, cx);
                            },
                        )),
                    )
                    .child(
                        Button::new((
                            if staged_group {
                                "git-unstage-file"
                            } else {
                                "git-stage-file"
                            },
                            index,
                        ))
                        .xsmall()
                        .disabled(state.busy)
                        .label(text(if staged_group {
                            "git.unstage"
                        } else {
                            "git.stage"
                        }))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                let action = if staged_group {
                                    Operation::Unstage(path.clone())
                                } else {
                                    Operation::Stage(path.clone())
                                };
                                this.git_operate(action, window, cx);
                            },
                        )),
                    );
                list = list.child(row);
            }
        }
        if !state.working.stashes.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text("git.stashes")),
            );
        }
        for (index, stash) in state.working.stashes.iter().enumerate() {
            let id = stash.id;
            list = list.child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().min_w_0().truncate().child(format!(
                        "{} {}",
                        short(id),
                        stash.title
                    )))
                    .child(
                        Button::new(("git-apply-stash", index))
                            .xsmall()
                            .disabled(state.busy || has_conflict || merging)
                            .label(text("git.stash-apply"))
                            .tooltip(text("git.stash-apply-tooltip"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.git_operate(Operation::ApplyStash(id), window, cx);
                            })),
                    ),
            );
        }
        let input = message.clone();
        let commit = Button::new("git-commit")
            .small()
            .label(text(if merging {
                "git.commit-merge"
            } else {
                "git.commit-staged"
            }))
            .disabled(has_conflict || (staged == 0 && !merging) || state.busy)
            .on_click(cx.listener(move |this, _, window, cx| {
                let message = input.read(cx).value().trim().to_string();
                if message.is_empty() {
                    this.show_alert(text("git.failed"), text("git.commit-message-required"), cx);
                } else {
                    this.git_operate(Operation::Commit(message), window, cx);
                }
            }));
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(Input::new(message).small().disabled(state.busy))
                        .child(
                            h_flex()
                                .gap_1()
                                .child(commit)
                                .child(
                                    Button::new("git-refresh-status")
                                        .small()
                                        .disabled(state.busy)
                                        .label(text("git.refresh"))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.git_operate(Operation::Refresh, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("git-stash-changes")
                                        .small()
                                        .disabled(
                                            state.busy
                                                || has_conflict
                                                || merging
                                                || state.working.changes.is_empty(),
                                        )
                                        .label(text("git.stash"))
                                        .tooltip(text("git.stash-tooltip"))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.git_operate(Operation::Stash, window, cx);
                                        })),
                                ),
                        ),
                )
                .when(merging, |pane| {
                    pane.child(
                        h_flex()
                            .px_2()
                            .py_1()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(text("git.merge-in-progress")),
                            )
                            .child(
                                Button::new("git-abort-merge")
                                    .xsmall()
                                    .disabled(state.busy)
                                    .label(text("git.merge-abort"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.git_confirm_abort_merge(window, cx)
                                    })),
                            ),
                    )
                })
                .when(state.working.changes.is_empty() && !state.busy, |pane| {
                    pane.child(div().px_2().child(text("git.no-changes")))
                })
                .child(list)
                .into_any_element(),
        )
    }
}

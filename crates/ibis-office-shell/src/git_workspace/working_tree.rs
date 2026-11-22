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

use super::{GitWorkspace, MAX_TEXT_DIFF_BYTES, Result, filename, markdown_block};
use crate::{editor::SlideEditor, i18n::text};

struct Change {
    path: PathBuf,
    index: u8,
    worktree: u8,
}

impl Change {
    fn staged(&self) -> bool {
        self.index != b' ' && self.index != b'?'
    }

    fn unstaged(&self) -> bool {
        self.worktree != b' '
    }
}

#[derive(Default)]
pub(super) struct WorkingTree {
    open: bool,
    changes: Vec<Change>,
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
    Staged,
    Unstaged,
    Untracked,
}

impl ChangeDiff {
    fn label(self) -> &'static str {
        match self {
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
    let mut child = git_command(root, &args)
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
    if !(status.success()
        || matches!(kind, ChangeDiff::Untracked) && status.code() == Some(1) && !bytes.is_empty())
    {
        return Err(text("git.diff-failed").to_string().into());
    }
    let markdown = if bytes.is_empty() {
        text("git.no-changes").to_string().into_bytes()
    } else {
        markdown_block(&String::from_utf8_lossy(&bytes), "diff")
    };
    Ok(ibis_formats::markdown::open_markdown(markdown)?)
}

fn status(root: &Path) -> Result<Vec<Change>> {
    let options = [
        "--no-optional-locks",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--no-renames",
    ];
    let args = options.map(OsStr::new);
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

struct Outcome {
    changes: std::result::Result<Vec<Change>, String>,
    workspace: std::result::Result<Option<GitWorkspace>, String>,
    committed: bool,
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
    }
    let workspace = if committed {
        GitWorkspace::open(root)
            .map(Some)
            .map_err(|error| error.to_string())
    } else {
        Ok(shifted.then_some(current))
    };
    Ok(Outcome {
        changes: status(root).map_err(|error| error.to_string()),
        workspace,
        committed,
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
                        let error = match outcome.workspace {
                            Ok(Some(mut updated)) => {
                                updated.working = std::mem::take(&mut state.working);
                                *state = updated;
                                error
                            }
                            Ok(None) => error,
                            Err(error) => Some(error),
                        };
                        if let Some(error) = error {
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
        let input = message.clone();
        let commit = Button::new("git-commit")
            .small()
            .label(text("git.commit-staged"))
            .disabled(staged == 0 || state.busy)
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
                            h_flex().gap_1().child(commit).child(
                                Button::new("git-refresh-status")
                                    .small()
                                    .disabled(state.busy)
                                    .label(text("git.refresh"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.git_operate(Operation::Refresh, window, cx);
                                    })),
                            ),
                        ),
                )
                .when(state.working.changes.is_empty() && !state.busy, |pane| {
                    pane.child(div().px_2().child(text("git.no-changes")))
                })
                .child(list)
                .into_any_element(),
        )
    }
}

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Obsidian-style Quick Switcher (`Cmd/Ctrl+O`) + Command Palette (`Cmd/Ctrl+P` or `>`).
//!
//! - In File mode (`⌘O`): fuzzy-searches open tabs and Vault files, supports `⌘Enter` to open
//!   in a split pane, and offers instant creation of a new Markdown note when the typed name
//!   doesn't exist yet.
//! - Typing `>` (or invoking `⌘P`) switches seamlessly to Command Palette mode for keyboard-first
//!   workspace control.

use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{LeftPanelKind, RightPanelKind, badge_for_path_and_kind, fuzzy_score};
use crate::session::resolve_new_note_path;
use crate::workspace::SoraneWorkspace;

const MAX_RESULTS: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteMode {
    Files,
    Commands,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceCommand {
    ToggleSplitPane,
    MoveTabToOtherPane,
    TogglePinActiveTab,
    ReopenClosedTab,
    NavigateBack,
    NavigateForward,
    NewMarkdownNote,
    NewMarpDeck,
    NewMarkdownWorkbook,
    OpenDailyNote,
    OpenVaultFolder,
    RefreshVault,
    ToggleLeftSidebar,
    ShowVaultPanel,
    ShowSearchPanel,
    ShowTagsPanel,
    ShowGitPanel,
    ToggleRightSidebar,
    ShowBacklinksPanel,
    ShowOutlinePanel,
    ShowGraphPanel,
    OpenAiAssistant,
    ToggleInspectorPanel,
    ToggleMarkdownSource,
    SaveWorkspaceSession,
}

#[derive(Clone, Debug)]
pub(crate) enum SwitcherAction {
    OpenFile(PathBuf),
    CreateNote { rel_label: String, full_path: PathBuf },
    RunCommand(WorkspaceCommand),
}

#[derive(Clone, Debug)]
pub(crate) struct SwitcherCandidate {
    pub(crate) label: String,
    pub(crate) subtitle: String,
    pub(crate) badge: &'static str,
    pub(crate) shortcut: Option<&'static str>,
    pub(crate) is_open: bool,
    pub(crate) action: SwitcherAction,
}

pub(crate) struct QuickSwitcherState {
    pub(crate) mode: PaletteMode,
    pub(crate) input: Entity<InputState>,
    pub(crate) selected: usize,
    pub(crate) _sub: Subscription,
}

const ALL_COMMANDS: &[(WorkspaceCommand, &str, &str, Option<&str>)] = &[
    (
        WorkspaceCommand::ToggleMarkdownSource,
        "Markdown ソース／分割ビューを切替 (Markdown Split)",
        "文書・Marp スライド・ワークブックのソースと WYSIWYG 分割表示を切替",
        Some("⇧⌘M"),
    ),
    (
        WorkspaceCommand::ToggleSplitPane,
        "左右ペイン分割を切替 (Split Pane)",
        "ペインを左右分割または統合",
        Some("⌘\\"),
    ),
    (
        WorkspaceCommand::MoveTabToOtherPane,
        "アクティブタブを反対ペインへ移動 (Move Tab)",
        "現在開いているタブを隣ペインへ移す",
        None,
    ),
    (
        WorkspaceCommand::TogglePinActiveTab,
        "アクティブタブをピン留め／解除 (Pin Tab)",
        "誤って閉じないようタブを左端に固定",
        None,
    ),
    (
        WorkspaceCommand::ReopenClosedTab,
        "直前に閉じたタブを再オープン (Reopen Closed Tab)",
        "最後に閉じたファイルタブを復元",
        Some("⇧⌘T"),
    ),
    (
        WorkspaceCommand::NavigateBack,
        "ペイン履歴: 前に戻る (Navigate Back)",
        "このペインで直前に見ていたファイルへ戻る",
        Some("⌘["),
    ),
    (
        WorkspaceCommand::NavigateForward,
        "ペイン履歴: 次に進む (Navigate Forward)",
        "このペインの履歴を次へ進む",
        Some("⌘]"),
    ),
    (
        WorkspaceCommand::OpenDailyNote,
        "デイリーノートを開く (Daily Note)",
        "今日の日付の Markdown ノートを作成または開く",
        None,
    ),
    (
        WorkspaceCommand::NewMarkdownNote,
        "新規 Markdown ノートを作成 (New Note)",
        "Vault に新しい .md ファイルを作成",
        Some("⌥⌘N"),
    ),
    (
        WorkspaceCommand::NewMarpDeck,
        "新規 Marp スライドを作成 (New Marp Deck)",
        "Marp プレゼンテーション (.md) を作成して分割ビューで開く",
        None,
    ),
    (
        WorkspaceCommand::NewMarkdownWorkbook,
        "新規ワークブック Markdown を作成 (New Workbook Markdown)",
        "GFM テーブルと数式を含むスプレッドシート (.md) を作成して分割ビューで開く",
        None,
    ),
    (
        WorkspaceCommand::OpenVaultFolder,
        "フォルダ（Vault）を開く (Open Vault)",
        "Obsidian Vault または作業フォルダを選択",
        Some("⇧⌘O"),
    ),
    (
        WorkspaceCommand::RefreshVault,
        "Vault と Git 状態を再読み込み (Refresh Vault)",
        "ファイル一覧・バックリンク・Git 差分を更新",
        None,
    ),
    (
        WorkspaceCommand::ToggleLeftSidebar,
        "左サイドバーを開閉 (Toggle Left Sidebar)",
        "Vault / 検索 / タグ / Git ドックを表示または隠す",
        Some("⌘B"),
    ),
    (
        WorkspaceCommand::ShowVaultPanel,
        "左ドック: Vault ファイルツリーを表示",
        "フォルダツリーへ切替",
        None,
    ),
    (
        WorkspaceCommand::ShowSearchPanel,
        "左ドック: Vault 全文検索を表示",
        "Markdown 全文検索へ切替",
        None,
    ),
    (
        WorkspaceCommand::ShowTagsPanel,
        "左ドック: タグ一覧を表示 (Tags)",
        "#tag / OKF タグからノートを絞り込み",
        None,
    ),
    (
        WorkspaceCommand::ShowGitPanel,
        "左ドック: Git 変更・履歴を表示",
        "ブランチ・変更ファイル・コミット履歴へ切替",
        None,
    ),
    (
        WorkspaceCommand::ToggleRightSidebar,
        "右サイドバーを開閉 (Toggle Right Sidebar)",
        "リンク・アウトライン・ローカルグラフを開閉",
        Some("⇧⌘B"),
    ),
    (
        WorkspaceCommand::ShowBacklinksPanel,
        "右ドック: リンク（被リンク・発リンク）を表示 (Links)",
        "アクティブノートの被リンク・発リンク・タグを表示",
        None,
    ),
    (
        WorkspaceCommand::ShowOutlinePanel,
        "右ドック: アウトラインを表示 (Outline)",
        "見出し・シート・スライド一覧へ切替",
        Some("⇧⌘U"),
    ),
    (
        WorkspaceCommand::ShowGraphPanel,
        "右ドック: ローカルグラフを表示 (Local Graph)",
        "アクティブノート周辺のリンク構造を可視化",
        None,
    ),
    (
        WorkspaceCommand::OpenAiAssistant,
        "AI アシスタントを開閉 (AI Assistant)",
        "Vault コンテキスト付き AI パネルを起動",
        Some("⇧⌘L"),
    ),
    (
        WorkspaceCommand::ToggleInspectorPanel,
        "書式・校閲パネルを開閉 (Office Inspector)",
        "埋め込みエディタの書式サイドバーを切替",
        None,
    ),
    (
        WorkspaceCommand::SaveWorkspaceSession,
        "ワークスペース状態を今すぐ保存 (Save Session)",
        "現在のタブ・分割・ドック配置を保存",
        None,
    ),
];

impl SoraneWorkspace {
    pub(crate) fn open_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteMode::Files, window, cx);
    }

    pub(crate) fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteMode::Commands, window, cx);
    }

    fn open_palette(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(existing) = self.quick_switcher.as_ref()
            && existing.mode == mode
        {
            self.close_quick_switcher(window, cx);
            return;
        }
        let placeholder = match mode {
            PaletteMode::Files => {
                "ファイル名で検索・新規作成… (Enter: 開く / ⌘Enter: 分割 / >: コマンド)"
            }
            PaletteMode::Commands => "コマンドを検索… (Enter: 実行 / Esc: 閉じる)",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let sub = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(qs) = this.quick_switcher.as_mut() {
                    qs.selected = 0;
                }
                cx.notify();
            }
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.quick_switcher = Some(QuickSwitcherState {
            mode,
            input,
            selected: 0,
            _sub: sub,
        });
        cx.notify();
    }

    pub(crate) fn close_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_switcher.take().is_some() {
            self.focus_active_editor(window, cx);
            cx.notify();
        }
    }

    fn effective_palette_mode(&self, base: PaletteMode, query: &str) -> (PaletteMode, String) {
        if base == PaletteMode::Files
            && let Some(rest) = query.trim_start().strip_prefix('>')
        {
            return (PaletteMode::Commands, rest.trim().to_owned());
        }
        (base, query.trim().to_owned())
    }

    pub(crate) fn quick_switcher_candidates(
        &self,
        base_mode: PaletteMode,
        raw_query: &str,
    ) -> Vec<SwitcherCandidate> {
        let (mode, query) = self.effective_palette_mode(base_mode, raw_query);
        if mode == PaletteMode::Commands {
            let mut cmds: Vec<(i32, SwitcherCandidate)> = Vec::new();
            for &(cmd, label, subtitle, shortcut) in ALL_COMMANDS {
                let haystack = format!("{label} {subtitle}");
                let Some(score) = fuzzy_score(&haystack, &query) else {
                    continue;
                };
                cmds.push((
                    score,
                    SwitcherCandidate {
                        label: label.to_owned(),
                        subtitle: subtitle.to_owned(),
                        badge: "CMD",
                        shortcut,
                        is_open: false,
                        action: SwitcherAction::RunCommand(cmd),
                    },
                ));
            }
            cmds.sort_by(|a, b| b.0.cmp(&a.0));
            return cmds.into_iter().take(MAX_RESULTS).map(|(_, c)| c).collect();
        }

        let mut items: Vec<(i32, SwitcherCandidate)> = Vec::new();
        let mut seen_paths: Vec<PathBuf> = Vec::new();
        let mut has_exact_note = false;
        let query_lower = query.to_lowercase();
        let query_md_lower = if query_lower.ends_with(".md") || query_lower.ends_with(".markdown") {
            query_lower.clone()
        } else {
            format!("{query_lower}.md")
        };

        // 1. Currently open file-backed tabs (boosted slightly so jumping between active docs is instant).
        for pane in &self.panes {
            for tab in &pane.tabs {
                let Some(path) = tab.path.as_ref() else { continue };
                if seen_paths.iter().any(|p| p == path) {
                    continue;
                }
                seen_paths.push(path.clone());
                let rel = self
                    .vault
                    .as_ref()
                    .and_then(|v| v.relative_path_for(path))
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                if !query.is_empty()
                    && (rel.to_lowercase() == query_lower || rel.to_lowercase() == query_md_lower)
                {
                    has_exact_note = true;
                }
                let Some(score) = fuzzy_score(&rel, &query) else { continue };
                items.push((
                    score + 200,
                    SwitcherCandidate {
                        label: tab.title.clone(),
                        subtitle: rel,
                        badge: tab.display_badge(),
                        shortcut: None,
                        is_open: true,
                        action: SwitcherAction::OpenFile(path.clone()),
                    },
                ));
            }
        }

        // 2. Vault files from `folder_context::Folder`.
        if let Some(vault) = self.vault.as_ref() {
            for rel in vault.files() {
                let rel_lower = rel.to_lowercase();
                if !query.is_empty() && (rel_lower == query_lower || rel_lower == query_md_lower) {
                    has_exact_note = true;
                }
                let Some(score) = fuzzy_score(rel, &query) else { continue };
                let full = vault.root().join(rel);
                if seen_paths.iter().any(|p| p == &full) {
                    continue;
                }
                seen_paths.push(full.clone());
                let label = rel.rsplit('/').next().unwrap_or(rel).to_owned();
                let kind = full
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(ibis_formats::Kind::from_extension)
                    .unwrap_or(ibis_formats::Kind::Document);
                items.push((
                    score,
                    SwitcherCandidate {
                        label,
                        subtitle: rel.clone(),
                        badge: badge_for_path_and_kind(Some(std::path::Path::new(rel)), kind),
                        shortcut: None,
                        is_open: false,
                        action: SwitcherAction::OpenFile(full),
                    },
                ));
            }
        }

        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.subtitle.cmp(&b.1.subtitle)));
        let mut results: Vec<SwitcherCandidate> =
            items.into_iter().take(MAX_RESULTS).map(|(_, c)| c).collect();

        // 3. Obsidian-style instant note creation when typed name doesn't exist yet in Vault.
        if !query.is_empty()
            && !has_exact_note
            && let Some(vault) = self.vault.as_ref()
            && let Some(full_path) = resolve_new_note_path(vault.root(), &query)
            && !full_path.exists()
        {
            let rel_label = full_path
                .strip_prefix(vault.root())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| query.clone());
            results.push(SwitcherCandidate {
                label: format!("新規ノートを作成: {rel_label}"),
                subtitle: "Vault に新しい Markdown ノートを作成して開く".to_owned(),
                badge: "NEW",
                shortcut: Some("Enter"),
                is_open: false,
                action: SwitcherAction::CreateNote {
                    rel_label,
                    full_path,
                },
            });
        }

        results
    }

    pub(crate) fn execute_workspace_command(
        &mut self,
        cmd: WorkspaceCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match cmd {
            WorkspaceCommand::ToggleSplitPane => self.toggle_split_pane(window, cx),
            WorkspaceCommand::MoveTabToOtherPane => self.move_active_tab_to_other_pane(window, cx),
            WorkspaceCommand::TogglePinActiveTab => self.toggle_pin_active_tab(cx),
            WorkspaceCommand::ReopenClosedTab => self.reopen_last_closed_tab(window, cx),
            WorkspaceCommand::NavigateBack => self.navigate_pane_back(self.active_pane, window, cx),
            WorkspaceCommand::NavigateForward => {
                self.navigate_pane_forward(self.active_pane, window, cx)
            }
            WorkspaceCommand::NewMarkdownNote => self.new_markdown_note(window, cx),
            WorkspaceCommand::NewMarpDeck => self.new_marp_deck(window, cx),
            WorkspaceCommand::NewMarkdownWorkbook => self.new_workbook_markdown(window, cx),
            WorkspaceCommand::OpenDailyNote => self.open_daily_note(window, cx),
            WorkspaceCommand::OpenVaultFolder => self.pick_vault_folder(window, cx),
            WorkspaceCommand::RefreshVault => self.refresh_vault_folder(window, cx),
            WorkspaceCommand::ToggleLeftSidebar => {
                self.left_sidebar_open = !self.left_sidebar_open;
                cx.notify();
            }
            WorkspaceCommand::ShowVaultPanel => {
                self.left_sidebar_open = true;
                self.left_panel = LeftPanelKind::Vault;
                cx.notify();
            }
            WorkspaceCommand::ShowSearchPanel => {
                self.left_sidebar_open = true;
                self.left_panel = LeftPanelKind::Search;
                self.vault_filter.update(cx, |s, cx| s.focus(window, cx));
                cx.notify();
            }
            WorkspaceCommand::ShowTagsPanel => {
                self.left_sidebar_open = true;
                self.left_panel = LeftPanelKind::Tags;
                cx.notify();
            }
            WorkspaceCommand::ShowGitPanel => {
                self.left_sidebar_open = true;
                self.left_panel = LeftPanelKind::Git;
                self.refresh_git_snapshot(window, cx);
                cx.notify();
            }
            WorkspaceCommand::ToggleRightSidebar => {
                self.right_sidebar_open = !self.right_sidebar_open;
                cx.notify();
            }
            WorkspaceCommand::ShowBacklinksPanel => {
                self.right_sidebar_open = true;
                self.right_panel = RightPanelKind::Backlinks;
                cx.notify();
            }
            WorkspaceCommand::ShowOutlinePanel => {
                self.right_sidebar_open = true;
                self.right_panel = RightPanelKind::Outline;
                cx.notify();
            }
            WorkspaceCommand::ShowGraphPanel => {
                self.right_sidebar_open = true;
                self.right_panel = RightPanelKind::Graph;
                cx.notify();
            }
            WorkspaceCommand::OpenAiAssistant => self.open_ai_assistant_for_active_pane(window, cx),
            WorkspaceCommand::ToggleInspectorPanel => self.toggle_inspector_for_active_pane(cx),
            WorkspaceCommand::ToggleMarkdownSource => {
                self.toggle_markdown_source_for_active_pane(window, cx)
            }
            WorkspaceCommand::SaveWorkspaceSession => self.save_workspace_session(),
        }
    }

    fn execute_switcher_action(
        &mut self,
        action: SwitcherAction,
        open_in_split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quick_switcher = None;
        match action {
            SwitcherAction::OpenFile(path) => {
                if open_in_split {
                    self.open_path_in_split_pane(path, window, cx);
                } else {
                    self.open_path_in_active_pane(path, window, cx);
                }
            }
            SwitcherAction::CreateNote {
                rel_label,
                full_path,
            } => {
                self.create_and_open_vault_note(full_path, &rel_label, open_in_split, window, cx);
            }
            SwitcherAction::RunCommand(cmd) => {
                self.execute_workspace_command(cmd, window, cx);
            }
        }
    }

    pub(crate) fn on_quick_switcher_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(qs) = self.quick_switcher.as_ref() else {
            return false;
        };
        let mode = qs.mode;
        let selected = qs.selected;
        let query = qs.input.read(cx).value().to_string();
        let candidates = self.quick_switcher_candidates(mode, &query);
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.close_quick_switcher(window, cx);
                true
            }
            "down" => {
                if !candidates.is_empty()
                    && let Some(qs) = self.quick_switcher.as_mut()
                {
                    qs.selected = (qs.selected + 1) % candidates.len();
                    cx.notify();
                }
                true
            }
            "up" => {
                if !candidates.is_empty()
                    && let Some(qs) = self.quick_switcher.as_mut()
                {
                    qs.selected = (qs.selected + candidates.len() - 1) % candidates.len();
                    cx.notify();
                }
                true
            }
            "enter" => {
                let open_in_split =
                    event.keystroke.modifiers.platform || event.keystroke.modifiers.control;
                if let Some(choice) = candidates.get(selected).cloned() {
                    self.execute_switcher_action(choice.action, open_in_split, window, cx);
                } else {
                    self.close_quick_switcher(window, cx);
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn render_quick_switcher(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let qs = self.quick_switcher.as_ref()?;
        let theme = cx.theme().clone();
        let query = qs.input.read(cx).value().to_string();
        let candidates = self.quick_switcher_candidates(qs.mode, &query);
        let selected = qs.selected.min(candidates.len().saturating_sub(1));

        let mut list = v_flex().id("quick-switcher-list").max_h(px(360.)).overflow_y_scroll();
        if candidates.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py_3()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(match qs.mode {
                        PaletteMode::Commands => "一致するコマンドがありません",
                        PaletteMode::Files if self.vault.is_none() => {
                            "フォルダ（Vault）を開くとファイルを検索できます（> でコマンド検索）"
                        }
                        PaletteMode::Files => "一致するファイルがありません",
                    }),
            );
        } else {
            for (idx, item) in candidates.into_iter().enumerate() {
                let is_sel = idx == selected;
                let primary_action = item.action.clone();
                let split_action = item.action.clone();
                let can_split = !matches!(item.action, SwitcherAction::RunCommand(_));
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("qs-row-{idx}")))
                        .px_3()
                        .py(px(6.))
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .when(is_sel, |r| r.bg(theme.accent))
                        .when(!is_sel, |r| r.hover(|h| h.bg(theme.secondary)))
                        .child(
                            div()
                                .px(px(5.))
                                .py(px(1.))
                                .rounded(px(3.))
                                .bg(theme.muted)
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.muted_foreground)
                                .child(item.badge),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.foreground)
                                        .truncate()
                                        .child(if item.is_open {
                                            format!("{}  •", item.label)
                                        } else {
                                            item.label
                                        }),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .truncate()
                                        .child(item.subtitle),
                                ),
                        )
                        .when_some(item.shortcut, |row, sc| {
                            row.child(
                                div()
                                    .px(px(5.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(theme.muted)
                                    .text_size(px(10.5))
                                    .text_color(theme.muted_foreground)
                                    .child(sc),
                            )
                        })
                        .when(can_split, |row| {
                            row.child(
                                div()
                                    .id(SharedString::from(format!("qs-split-{idx}")))
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .hover(|h| h.bg(theme.background).text_color(theme.foreground))
                                    .child("◫")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.execute_switcher_action(
                                            split_action.clone(),
                                            true,
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.execute_switcher_action(primary_action.clone(), false, window, cx);
                        })),
                );
            }
        }

        Some(
            div()
                .id("quick-switcher-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_start()
                .justify_center()
                .pt(px(72.))
                .bg(theme.foreground.opacity(0.22))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.close_quick_switcher(window, cx);
                    }),
                )
                .child(
                    v_flex()
                        .id("quick-switcher-dialog")
                        .w(px(580.))
                        .max_w(px(640.))
                        .rounded(px(10.))
                        .bg(theme.background)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(
                            div()
                                .p_2()
                                .border_b_1()
                                .border_color(theme.border)
                                .child(Input::new(&qs.input).small()),
                        )
                        .child(list),
                )
                .into_any_element(),
        )
    }
}

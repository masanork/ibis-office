// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! `SoraneWorkspace`: Obsidian-style desktop workspace shell hosting one or more
//! embedded [`SlideEditor`] engines across tabs and split panes.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_formats::Kind;
use ibis_office_shell::editor::SlideEditor;
#[cfg(not(target_family = "wasm"))]
use ibis_office_shell::folder_context::{ContentSearchHit, Folder};
#[cfg(not(target_family = "wasm"))]
use ibis_office_shell::git_workspace::{GitSnapshot, inspect_git_snapshot};
#[cfg(not(target_family = "wasm"))]
use ibis_office_shell::open_guard::same_file as same_workspace_file;
use ibis_office_shell::window_editors::{WindowSetup, WorkspaceEditorHost};

#[cfg(target_family = "wasm")]
mod web_platform {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use gpui_kit::{Context, Window};
    use ibis_office_shell::editor::SlideEditor;

    #[derive(Clone, Debug)]
    pub struct ContentSearchHit {
        pub path: String,
        pub line: usize,
        pub excerpt: String,
    }

    #[derive(Clone, Debug)]
    pub struct BacklinkHit {
        pub path: String,
        pub line: usize,
        pub excerpt: String,
    }

    #[derive(Clone, Debug)]
    pub struct GitChange {
        pub path: String,
        pub status: String,
        pub staged: bool,
    }

    #[derive(Clone, Debug)]
    pub struct GitCommit {
        pub short_id: String,
        pub title: String,
        pub date: String,
    }

    #[derive(Clone, Debug)]
    pub struct GitSnapshot {
        pub branch: String,
        pub head_short: String,
        pub changes: Vec<GitChange>,
        pub commits: Vec<GitCommit>,
    }

    pub fn inspect_git_snapshot(_root: &Path, _limit: usize) -> Option<GitSnapshot> {
        None
    }

    pub fn same_workspace_file(a: &Path, b: &Path) -> bool {
        a == b
    }

    #[derive(Clone, Debug)]
    pub struct Folder {
        root: PathBuf,
        display_name: String,
        files: Vec<String>,
        contents: std::collections::BTreeMap<String, String>,
        tags_by_file: std::collections::BTreeMap<String, Vec<String>>,
        outgoing_by_file: std::collections::BTreeMap<String, Vec<String>>,
        backlinks_by_file: std::collections::BTreeMap<String, Vec<BacklinkHit>>,
        all_tags_cache: Vec<(String, usize)>,
    }

    fn extract_tags(content: &str) -> Vec<String> {
        let mut tags = std::collections::BTreeSet::new();
        for line in content.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') && trimmed.chars().nth(1).is_some_and(char::is_whitespace) {
                continue;
            }
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'#' {
                    let prev_ok = i == 0 || bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b'(';
                    if prev_ok {
                        let rest = &line[i + 1..];
                        let tag: String = rest
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '/')
                            .collect();
                        if !tag.is_empty() && !tag.chars().all(|c| c.is_ascii_digit()) {
                            tags.insert(tag);
                        }
                    }
                }
                i += 1;
            }
        }
        tags.into_iter().collect()
    }

    fn resolve_target(target_raw: &str, files: &[String]) -> Option<String> {
        let base = target_raw
            .split('#')
            .next()
            .unwrap_or(target_raw)
            .split('|')
            .next()
            .unwrap_or(target_raw)
            .trim()
            .trim_start_matches("./")
            .trim_start_matches('/');
        if base.is_empty() {
            return None;
        }
        let lower = base.to_ascii_lowercase();
        let with_md = if lower.ends_with(".md") || lower.ends_with(".lawtext") {
            lower.clone()
        } else {
            format!("{lower}.md")
        };
        if let Some(exact) = files.iter().find(|f| {
            let fl = f.to_ascii_lowercase();
            fl == lower || fl == with_md
        }) {
            return Some(exact.clone());
        }
        files
            .iter()
            .find(|f| {
                let name = Path::new(f)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let stem = Path::new(f)
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                name == lower || name == with_md || stem == lower
            })
            .cloned()
    }

    fn extract_links(content: &str, files: &[String]) -> Vec<(String, usize, String)> {
        let mut out = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            let line_no = idx + 1;
            let excerpt = line.trim().chars().take(120).collect::<String>();
            let mut rest = line;
            while let Some(start) = rest.find("[[") {
                let after = &rest[start + 2..];
                if let Some(end) = after.find("]]") {
                    let raw = &after[..end];
                    if let Some(resolved) = resolve_target(raw, files) {
                        out.push((resolved, line_no, excerpt.clone()));
                    }
                    rest = &after[end + 2..];
                } else {
                    break;
                }
            }
            let mut mrest = line;
            while let Some(mid) = mrest.find("](") {
                let after = &mrest[mid + 2..];
                if let Some(end) = after.find(')') {
                    let raw = after[..end].trim();
                    if !raw.starts_with("http://") && !raw.starts_with("https://") && !raw.starts_with('#')
                        && let Some(resolved) = resolve_target(raw, files)
                    {
                        out.push((resolved, line_no, excerpt.clone()));
                    }
                    mrest = &after[end + 1..];
                } else {
                    break;
                }
            }
        }
        out
    }

    impl Folder {
        pub fn scan_dir(root: &Path) -> Result<Self, String> {
            Ok(Self {
                display_name: root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("soraneDocs")
                    .to_string(),
                root: root.to_path_buf(),
                files: Vec::new(),
                contents: std::collections::BTreeMap::new(),
                tags_by_file: std::collections::BTreeMap::new(),
                outgoing_by_file: std::collections::BTreeMap::new(),
                backlinks_by_file: std::collections::BTreeMap::new(),
                all_tags_cache: Vec::new(),
            })
        }

        pub fn from_web_files(
            root: PathBuf,
            display_name: String,
            mut files: Vec<String>,
            contents: std::collections::BTreeMap<String, String>,
        ) -> Self {
            files.sort();
            files.dedup();
            let mut folder = Self {
                root,
                display_name,
                files,
                contents,
                tags_by_file: std::collections::BTreeMap::new(),
                outgoing_by_file: std::collections::BTreeMap::new(),
                backlinks_by_file: std::collections::BTreeMap::new(),
                all_tags_cache: Vec::new(),
            };
            folder.rebuild_index();
            folder
        }

        pub fn with_hydrated_file(&self, rel_path: String, content: String) -> Self {
            let mut files = self.files.clone();
            if !files.contains(&rel_path) {
                files.push(rel_path.clone());
            }
            let mut contents = self.contents.clone();
            contents.insert(rel_path, content);
            Self::from_web_files(
                self.root.clone(),
                self.display_name.clone(),
                files,
                contents,
            )
        }

        pub fn content_for(&self, rel: &str) -> Option<&str> {
            self.contents.get(rel).map(String::as_str)
        }

        pub fn cached_contents(&self) -> &std::collections::BTreeMap<String, String> {
            &self.contents
        }

        fn rebuild_index(&mut self) {
            let mut tag_counts = std::collections::BTreeMap::<String, usize>::new();
            self.tags_by_file.clear();
            self.outgoing_by_file.clear();
            self.backlinks_by_file.clear();

            for (rel, content) in &self.contents {
                let tags = extract_tags(content);
                for t in &tags {
                    *tag_counts.entry(t.clone()).or_default() += 1;
                }
                self.tags_by_file.insert(rel.clone(), tags);

                let links = extract_links(content, &self.files);
                let mut outgoing = Vec::new();
                for (target, line, excerpt) in links {
                    if target != *rel && !outgoing.contains(&target) {
                        outgoing.push(target.clone());
                    }
                    self.backlinks_by_file.entry(target).or_default().push(BacklinkHit {
                        path: rel.clone(),
                        line,
                        excerpt,
                    });
                }
                self.outgoing_by_file.insert(rel.clone(), outgoing);
            }

            let mut all_tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
            all_tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            self.all_tags_cache = all_tags;
        }

        pub fn root(&self) -> &Path {
            &self.root
        }

        pub fn display_name(&self) -> String {
            self.display_name.clone()
        }

        pub fn files(&self) -> &[String] {
            &self.files
        }

        pub fn has_git(&self) -> bool {
            false
        }

        pub fn relative_path_for(&self, path: &Path) -> Option<String> {
            if let Ok(stripped) = path.strip_prefix(&self.root) {
                let rel = stripped.to_string_lossy().replace('\\', "/");
                if !rel.is_empty() {
                    return Some(rel);
                }
            }
            let s = path.to_string_lossy().replace('\\', "/");
            if self.files.iter().any(|f| f == &s) {
                return Some(s);
            }
            None
        }

        pub fn matches_query(&self, rel: &str, query: &str) -> bool {
            rel.to_lowercase().contains(query)
        }

        pub fn search_content(&self, query: &str, limit: usize) -> Vec<ContentSearchHit> {
            let needle = query.trim().to_lowercase();
            if needle.is_empty() || limit == 0 {
                return Vec::new();
            }
            let mut hits = Vec::new();
            for (rel, content) in &self.contents {
                for (idx, line) in content.lines().enumerate() {
                    if line.to_lowercase().contains(&needle) {
                        hits.push(ContentSearchHit {
                            path: rel.clone(),
                            line: idx + 1,
                            excerpt: line.trim().chars().take(120).collect(),
                        });
                        if hits.len() >= limit {
                            return hits;
                        }
                    }
                }
            }
            hits
        }

        pub fn files_with_tag(&self, tag: &str) -> Vec<String> {
            self.tags_by_file
                .iter()
                .filter(|(_, tags)| tags.iter().any(|t| t == tag))
                .map(|(rel, _)| rel.clone())
                .collect()
        }

        pub fn tags_for(&self, rel: &str) -> Vec<String> {
            self.tags_by_file.get(rel).cloned().unwrap_or_default()
        }

        pub fn all_tags(&self) -> Vec<(String, usize)> {
            self.all_tags_cache.clone()
        }

        pub fn backlinks_for(&self, rel: &str) -> Vec<BacklinkHit> {
            self.backlinks_by_file.get(rel).cloned().unwrap_or_default()
        }

        pub fn outgoing_links_for(&self, rel: &str) -> Vec<String> {
            self.outgoing_by_file.get(rel).cloned().unwrap_or_default()
        }
    }

    pub trait SlideEditorWebExt {
        fn has_git_workspace(&self) -> bool;
        fn attach_vault_folder(
            &mut self,
            _folder: Option<Arc<Folder>>,
            _window: &mut Window,
            _cx: &mut Context<SlideEditor>,
        );
        fn open_path(
            &mut self,
            _path: PathBuf,
            _line: Option<usize>,
            _window: &mut Window,
            _cx: &mut Context<SlideEditor>,
        );
        fn open_git_workspace_for_root(
            &mut self,
            _root: PathBuf,
            _window: &mut Window,
            _cx: &mut Context<SlideEditor>,
        );
        fn shows_path(&self, _path: &Path) -> bool;
    }

    impl SlideEditorWebExt for SlideEditor {
        fn has_git_workspace(&self) -> bool {
            false
        }

        fn attach_vault_folder(
            &mut self,
            _folder: Option<Arc<Folder>>,
            _window: &mut Window,
            _cx: &mut Context<SlideEditor>,
        ) {
        }

        fn open_path(
            &mut self,
            _path: PathBuf,
            _line: Option<usize>,
            _window: &mut Window,
            _cx: &mut Context<SlideEditor>,
        ) {
        }

        fn open_git_workspace_for_root(
            &mut self,
            _root: PathBuf,
            _window: &mut Window,
            _cx: &mut Context<SlideEditor>,
        ) {
        }

        fn shows_path(&self, _path: &Path) -> bool {
            false
        }
    }
}

#[cfg(target_family = "wasm")]
use web_platform::{ContentSearchHit, Folder, GitSnapshot, SlideEditorWebExt as _, inspect_git_snapshot, same_workspace_file};

use sorane_github::{
    DeviceCodeResponse, DeviceTokenPoll, GITHUB_APP_INSTALL_URL, GitHubRepo, RepoCatalog,
};

use crate::actions::*;
use crate::github::{self, GitHubSession};
use crate::local_graph::{LinkDirection, compute_local_graph_layout};
use crate::model::*;
use crate::outline::{OutlineTarget, extract_outline};
use crate::quick_switcher::QuickSwitcherState;
use crate::session::{
    PersistedPane, PersistedTab, WorkspaceSession, load_last_session, load_vault_session,
    save_session, today_iso_date,
};

const WORKSPACE_CONTEXT: &str = "SoraneWorkspace";
const RIBBON_WIDTH: f32 = 40.0;
const DIVIDER_WIDTH: f32 = 5.0;
const SEARCH_HIT_LIMIT: usize = 80;
const TREE_ROW_LIMIT: usize = 200;
const MAX_CLOSED_TABS: usize = 20;

pub struct SoraneWorkspace {
    pub(crate) panes: Vec<PaneGroup<Entity<SlideEditor>>>,
    pub(crate) layout: SplitLayout,
    pub(crate) active_pane: PaneId,
    next_pane_id: usize,
    next_tab_id: usize,
    seq: u64,
    closed_tabs: Vec<ClosedTabRecord>,

    // Left dock (Activity Ribbon + collapsible/resizable Vault / Search / Tags / Git sidebar)
    pub(crate) left_sidebar_open: bool,
    pub(crate) left_panel: LeftPanelKind,
    pub(crate) left_sidebar_width: f32,
    left_sidebar_drag: Option<(f32, f32)>,

    // Right dock (collapsible/resizable Backlinks / Outline / Local Graph panel)
    pub(crate) right_sidebar_open: bool,
    pub(crate) right_panel: RightPanelKind,
    pub(crate) right_sidebar_width: f32,
    right_sidebar_drag: Option<(f32, f32)>,

    // Split pane drag state
    split_drag: Option<(f32, f32)>,
    split_container_width: Rc<Cell<f32>>,

    // Vault state (reuses `ibis_office_shell::folder_context::Folder`)
    pub(crate) vault: Option<Arc<Folder>>,
    pub(crate) git_snapshot: Option<GitSnapshot>,
    pub(crate) vault_filter: Entity<InputState>,
    _vault_filter_sub: Subscription,
    collapsed_dirs: BTreeSet<String>,
    selected_tag: Option<String>,
    cached_search_query: String,
    cached_search_hits: Vec<ContentSearchHit>,
    vault_status: Option<SharedString>,

    // Quick Switcher / Command Palette overlay (`Cmd/Ctrl+O` / `Cmd/Ctrl+P`)
    pub(crate) quick_switcher: Option<QuickSwitcherState>,

    // GitHub App Vault integration (Device Flow + GIT_ASKPASS sync)
    github_session: Option<GitHubSession>,
    github_catalog: Option<RepoCatalog>,
    github_device_flow: Option<DeviceCodeResponse>,
    github_busy: bool,
    github_status: Option<(String, bool)>,
    github_auth_generation: u64,

    // Last OS window title applied
    last_window_title: String,
}

impl SoraneWorkspace {
    pub fn new(setup: WindowSetup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_editor = cx.new(|cx| {
            let mut editor = SlideEditor::new(ibis_doc_model::Presentation::new(), window, cx);
            setup(&mut editor, window, cx);
            let policy = Self::policy_for_editor(&editor, None, None);
            editor.embedded_chrome = Some(policy);
            editor
        });

        let launched_empty = initial_editor.read(cx).replaceable_by_open();
        let (path, title, kind) = Self::editor_metadata(&initial_editor, cx);
        let initial_tab = TabEntry {
            id: TabId(0),
            path,
            title,
            kind,
            pinned: false,
            last_used_seq: 1,
            toolbar_override: None,
            right_sidebar_override: None,
            state: TabState::Resident(initial_editor.clone()),
        };

        let initial_pane = PaneGroup::new(PaneId(0), initial_tab);
        let focus = initial_editor.read(cx).focus_handle.clone();
        focus.focus(window, cx);

        let vault_filter = cx.new(|cx| InputState::new(window, cx).placeholder("フィルタ / 検索…"));
        let vault_filter_sub = cx.subscribe(&vault_filter, |this, ed, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if this.left_panel == LeftPanelKind::Search {
                    let query = ed.read(cx).value().trim().to_owned();
                    this.refresh_content_search(&query);
                }
                cx.notify();
            }
        });

        let saved_github_session = github::load_session();

        let mut ws = Self {
            panes: vec![initial_pane],
            layout: SplitLayout::Single(PaneId(0)),
            active_pane: PaneId(0),
            next_pane_id: 1,
            next_tab_id: 1,
            seq: 1,
            closed_tabs: Vec::new(),
            // Single-file startup keeps left sidebar collapsed (per approved design decision).
            left_sidebar_open: false,
            left_panel: LeftPanelKind::Vault,
            left_sidebar_width: DEFAULT_LEFT_SIDEBAR_WIDTH,
            left_sidebar_drag: None,
            right_sidebar_open: false,
            right_panel: RightPanelKind::Backlinks,
            right_sidebar_width: DEFAULT_RIGHT_SIDEBAR_WIDTH,
            right_sidebar_drag: None,
            split_drag: None,
            split_container_width: Rc::new(Cell::new(0.0)),
            vault: None,
            git_snapshot: None,
            vault_filter,
            _vault_filter_sub: vault_filter_sub,
            collapsed_dirs: BTreeSet::new(),
            selected_tag: None,
            cached_search_query: String::new(),
            cached_search_hits: Vec::new(),
            vault_status: None,
            quick_switcher: None,
            github_session: saved_github_session,
            github_catalog: None,
            github_device_flow: None,
            github_busy: false,
            github_status: None,
            github_auth_generation: 0,
            last_window_title: String::new(),
        };

        // Restore the last workspace session automatically only when launched without an explicit file.
        if launched_empty && let Some(session) = load_last_session() {
            ws.restore_session(session, window, cx);
        }

        if ws.github_session.is_some() {
            ws.refresh_github_catalog(window, cx);
        }

        ws
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn alloc_tab_id(&mut self) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        id
    }

    fn alloc_pane_id(&mut self) -> PaneId {
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        id
    }

    fn policy_for_editor(
        editor: &SlideEditor,
        toolbar_override: Option<bool>,
        right_sidebar_override: Option<bool>,
    ) -> ibis_office_shell::window_editors::EmbeddedChromePolicy {
        let path = editor.file_path();
        let file_name = editor.file_name.as_deref();
        let kind = editor.format.kind();
        let read_only = editor.read_only || editor.is_read_only();
        let mut policy = chrome_policy_for(
            path.as_deref(),
            file_name,
            kind,
            read_only,
            toolbar_override,
            right_sidebar_override,
        );
        if editor.has_git_workspace() {
            policy.show_left_pane = true;
        }
        if editor.is_ai_assistant_open() {
            policy.show_right_sidebar = true;
        }
        policy
    }

    fn editor_metadata(editor: &Entity<SlideEditor>, cx: &App) -> (Option<PathBuf>, String, Kind) {
        let e = editor.read(cx);
        let path = e.file_path();
        let kind = e.format.kind();
        let title = e
            .file_name
            .as_ref()
            .map(|s| s.to_string())
            .or_else(|| {
                path.as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "無題".to_owned());
        (path, title, kind)
    }

    fn pane_mut(&mut self, id: PaneId) -> Option<&mut PaneGroup<Entity<SlideEditor>>> {
        self.panes.iter_mut().find(|p| p.id == id)
    }

    fn pane(&self, id: PaneId) -> Option<&PaneGroup<Entity<SlideEditor>>> {
        self.panes.iter().find(|p| p.id == id)
    }

    pub(crate) fn focus_active_editor(&self, window: &mut Window, cx: &mut App) {
        if let Some(editor) = self.active_editor(cx) {
            let focus = editor.read(cx).focus_handle.clone();
            focus.focus(window, cx);
        }
    }

    fn sync_tab_metadata_and_chrome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                if let TabState::Resident(editor) = &tab.state {
                    let (path, title, kind) = Self::editor_metadata(editor, cx);
                    if path.is_some() {
                        tab.path = path;
                    }
                    tab.title = title;
                    tab.kind = kind;
                    let desired = Self::policy_for_editor(
                        editor.read(cx),
                        tab.toolbar_override,
                        tab.right_sidebar_override,
                    );
                    if editor.read(cx).embedded_chrome != Some(desired) {
                        editor.update(cx, |e, cx| e.set_embedded_chrome(Some(desired), cx));
                    }
                }
            }
        }

        // Keep OS window title and macOS menu bar synced with the active pane's editor.
        if let Some(active) = self.active_editor(cx) {
            let e = active.read(cx);
            let title = if e.has_document {
                let base = e.display_title_formatted();
                if e.is_dirty() {
                    format!("{base} ● — Docs")
                } else {
                    format!("{base} — Docs")
                }
            } else if let Some(vault) = &self.vault {
                format!("{} — Docs", vault.display_name())
            } else {
                "Docs".to_owned()
            };
            if title != self.last_window_title {
                self.last_window_title = title.clone();
                window.set_window_title(&title);
            }
            #[cfg(target_os = "macos")]
            {
                active.update(cx, |editor, cx| {
                    ibis_office_shell::macos_menu::sync(editor, window, cx);
                });
            }
        }
    }

    fn sync_vault_to_all_editors(&self, window: &mut Window, cx: &mut Context<Self>) {
        let vault = self.vault.clone();
        for editor in self.all_editors(cx) {
            let folder = vault.clone();
            editor.update(cx, |e, cx| e.attach_vault_folder(folder, window, cx));
        }
    }

    pub(crate) fn refresh_git_snapshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self
            .vault
            .as_ref()
            .filter(|v| v.has_git())
            .map(|v| v.root().to_path_buf())
        else {
            self.git_snapshot = None;
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move { inspect_git_snapshot(&root, 15) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.git_snapshot = snapshot;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn open_ai_assistant_for_active_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor(cx) else {
            return;
        };
        let vault = self.vault.clone();
        let currently_open = editor.read(cx).is_ai_assistant_open();
        if currently_open {
            editor.update(cx, |e, cx| e.close_ai_assistant_panel(cx));
            if let Some(tab) = self.pane_mut(self.active_pane).and_then(|p| p.active_mut()) {
                tab.right_sidebar_override = None;
            }
        } else {
            if let Some(tab) = self.pane_mut(self.active_pane).and_then(|p| p.active_mut()) {
                tab.right_sidebar_override = Some(true);
            }
            editor.update(cx, |e, cx| {
                e.attach_vault_folder(vault, window, cx);
                e.open_ai_assistant_panel(window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn toggle_markdown_source_for_active_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor(cx) else {
            return;
        };
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        window.dispatch_action(
            Box::new(ibis_office_shell::actions::ToggleMarkdownSource),
            cx,
        );
        cx.notify();
    }

    pub(crate) fn toggle_inspector_for_active_pane(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor(cx) else {
            return;
        };
        let currently_shown = {
            let e = editor.read(cx);
            e.embedded_chrome.is_none_or(|p| p.show_right_sidebar) && e.sidebar_open
        };
        let next = !currently_shown;
        if let Some(tab) = self.pane_mut(self.active_pane).and_then(|p| p.active_mut()) {
            tab.right_sidebar_override = Some(next);
        }
        editor.update(cx, |e, cx| {
            e.sidebar_open = next;
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn toggle_outline_panel(&mut self, cx: &mut Context<Self>) {
        if self.right_sidebar_open && self.right_panel == RightPanelKind::Outline {
            self.right_sidebar_open = false;
        } else {
            self.right_sidebar_open = true;
            self.right_panel = RightPanelKind::Outline;
        }
        cx.notify();
    }

    pub(crate) fn jump_to_outline_target(
        &mut self,
        target: OutlineTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor(cx) else {
            return;
        };
        match target {
            OutlineTarget::Paragraph(id) => {
                editor.update(cx, |e, cx| e.reveal_body_paragraph(id, cx));
            }
            OutlineTarget::Slide(slide_id) => {
                editor.update(cx, |e, cx| {
                    e.set_current_slide(slide_id);
                    cx.notify();
                });
            }
        }
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    fn enforce_tab_budget(&mut self, cx: &App) {
        enforce_resident_tab_budget(&mut self.panes, MAX_CLEAN_RESIDENT_TABS, |editor| {
            editor.read(cx).is_dirty()
        });
    }

    fn ensure_tab_resident(
        &mut self,
        pane_id: PaneId,
        tab_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<SlideEditor>> {
        let (needs_spawn, path, toolbar_override, right_sidebar_override) = {
            let tab = self.pane(pane_id)?.tabs.get(tab_idx)?;
            if let TabState::Resident(ed) = &tab.state {
                return Some(ed.clone());
            }
            (true, tab.path.clone(), tab.toolbar_override, tab.right_sidebar_override)
        };
        if !needs_spawn {
            return None;
        }
        let vault = self.vault.clone();
        let editor = cx.new(|cx| {
            let mut e = SlideEditor::new(ibis_doc_model::Presentation::new(), window, cx);
            if let Some(path) = path {
                e.open_path(path, None, window, cx);
            }
            e.attach_vault_folder(vault, window, cx);
            e.embedded_chrome = Some(Self::policy_for_editor(
                &e,
                toolbar_override,
                right_sidebar_override,
            ));
            e
        });
        if let Some(tab) = self.pane_mut(pane_id).and_then(|p| p.tabs.get_mut(tab_idx)) {
            tab.state = TabState::Resident(editor.clone());
        }
        self.enforce_tab_budget(cx);
        Some(editor)
    }

    pub(crate) fn activate_tab(
        &mut self,
        pane_id: PaneId,
        tab_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let seq = self.next_seq();
        self.active_pane = pane_id;
        if let Some(pane) = self.pane_mut(pane_id) {
            pane.activate_index(tab_idx, seq);
        }
        let _ = self.ensure_tab_resident(pane_id, tab_idx, window, cx);
        self.enforce_tab_budget(cx);
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    pub fn open_path_in_active_pane(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_id = self.active_pane;
        self.open_path_in_pane(pane_id, path, window, cx);
    }

    pub fn open_path_in_split_pane(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target_pane = match self.layout.other_pane(self.active_pane) {
            Some(other) => other,
            None => self.split_pane(window, cx),
        };
        self.open_path_in_pane(target_pane, path, window, cx);
    }

    pub fn open_path_in_pane(
        &mut self,
        pane_id: PaneId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_path_in_pane_inner(pane_id, path, true, window, cx);
    }

    fn open_path_in_pane_inner(
        &mut self,
        pane_id: PaneId,
        path: PathBuf,
        record_history: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Deduplicate: if any tab in this workspace already shows `path`, activate it.
        if self.activate_editor_by_path(&path, window, cx) {
            if record_history && let Some(pane) = self.pane_mut(self.active_pane) {
                pane.record_history(path);
            }
            return;
        }

        #[cfg(target_family = "wasm")]
        {
            if let Some(vault) = self.vault.clone()
                && let Some(rel) = vault.relative_path_for(&path)
            {
                let file_name = std::path::Path::new(&rel)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&rel)
                    .to_string();
                if let Some(content) = vault.content_for(&rel) {
                    let bytes = content.as_bytes().to_vec();
                    ibis_office_shell::web::notify_github_file_opened(&rel, &file_name);
                    self.open_web_bytes_tab_with_path(Some(path), file_name, bytes, window, cx);
                    return;
                }
                let encoded: String = rel
                    .as_bytes()
                    .iter()
                    .flat_map(|&b| {
                        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                            vec![b as char]
                        } else {
                            format!("%{b:02X}").chars().collect()
                        }
                    })
                    .collect();
                let url = format!("/api/github/file?path={encoded}");
                let rel_clone = rel.clone();
                let path_clone = path.clone();
                cx.spawn_in(window, async move |this, cx| {
                    if let Ok((_fetched_name, bytes)) = ibis_office_shell::web::fetch_url(&url).await {
                        if let Ok(text) = std::str::from_utf8(&bytes) {
                            let text_owned = text.to_string();
                            let _ = this.update_in(cx, |ws, _, cx| {
                                ws.hydrate_web_vault_file(rel_clone.clone(), text_owned, cx);
                            });
                        }
                        ibis_office_shell::web::notify_github_file_opened(&rel_clone, &file_name);
                        let _ = this.update_in(cx, |ws, window, cx| {
                            ws.open_web_bytes_tab_with_path(
                                Some(path_clone),
                                file_name,
                                bytes,
                                window,
                                cx,
                            );
                        });
                    }
                })
                .detach();
                return;
            }
        }

        // Reuse the active tab if it's an empty replaceable start screen.
        let reuse_active = self.pane(pane_id).and_then(|p| {
            let tab = p.active()?;
            let editor = tab.state.resident()?;
            editor.read(cx).replaceable_by_open().then_some((p.active_tab, editor.clone()))
        });
        if let Some((tab_idx, editor)) = reuse_active {
            let vault = self.vault.clone();
            editor.update(cx, |e, cx| {
                e.open_path(path.clone(), None, window, cx);
                e.attach_vault_folder(vault, window, cx);
                e.embedded_chrome = Some(Self::policy_for_editor(e, None, None));
            });
            let (p, title, kind) = Self::editor_metadata(&editor, cx);
            let resolved_path = p.unwrap_or(path);
            let seq = self.next_seq();
            if let Some(pane) = self.pane_mut(pane_id) {
                if let Some(tab) = pane.tabs.get_mut(tab_idx) {
                    tab.path = Some(resolved_path.clone());
                    tab.title = title;
                    tab.kind = kind;
                    tab.last_used_seq = seq;
                    tab.toolbar_override = None;
                    tab.right_sidebar_override = None;
                }
                if record_history {
                    pane.record_history(resolved_path);
                }
            }
            self.active_pane = pane_id;
            self.focus_active_editor(window, cx);
            cx.notify();
            return;
        }

        // Otherwise create a new tab in `pane_id`.
        let tab_id = self.alloc_tab_id();
        let seq = self.next_seq();
        let path_for_open = path.clone();
        let vault = self.vault.clone();
        let editor = cx.new(|cx| {
            let mut e = SlideEditor::new(ibis_doc_model::Presentation::new(), window, cx);
            e.open_path(path_for_open, None, window, cx);
            e.attach_vault_folder(vault, window, cx);
            e.embedded_chrome = Some(Self::policy_for_editor(&e, None, None));
            e
        });
        let (p, title, kind) = Self::editor_metadata(&editor, cx);
        let resolved_path = p.unwrap_or(path);
        let tab = TabEntry {
            id: tab_id,
            path: Some(resolved_path.clone()),
            title,
            kind,
            pinned: false,
            last_used_seq: seq,
            toolbar_override: None,
            right_sidebar_override: None,
            state: TabState::Resident(editor),
        };
        if let Some(pane) = self.pane_mut(pane_id) {
            pane.tabs.push(tab);
            pane.active_tab = pane.tabs.len() - 1;
            if record_history {
                pane.record_history(resolved_path);
            }
        }
        self.active_pane = pane_id;
        self.enforce_tab_budget(cx);
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    pub(crate) fn navigate_pane_back(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.pane_mut(pane_id).and_then(|p| p.go_back()) else {
            return;
        };
        self.open_path_in_pane_inner(pane_id, target, false, window, cx);
    }

    pub(crate) fn navigate_pane_forward(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.pane_mut(pane_id).and_then(|p| p.go_forward()) else {
            return;
        };
        self.open_path_in_pane_inner(pane_id, target, false, window, cx);
    }

    pub(crate) fn toggle_pin_tab(
        &mut self,
        pane_id: PaneId,
        tab_idx: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self.pane_mut(pane_id) {
            let _ = pane.toggle_pin(tab_idx);
            self.save_workspace_session();
            cx.notify();
        }
    }

    pub(crate) fn toggle_pin_active_tab(&mut self, cx: &mut Context<Self>) {
        let pane_id = self.active_pane;
        if let Some(idx) = self.pane(pane_id).map(|p| p.active_tab) {
            self.toggle_pin_tab(pane_id, idx, cx);
        }
    }

    pub(crate) fn move_active_tab_to_other_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let src_id = self.active_pane;
        let tab_count = self.pane(src_id).map_or(0, |p| p.tabs.len());
        if tab_count <= 1 && !self.layout.is_split() {
            return;
        }
        let dst_id = match self.layout.other_pane(src_id) {
            Some(other) => other,
            None => {
                // Split and then move the original tab into the new pane, dropping its empty placeholder.
                let new_pane = self.split_pane(window, cx);
                self.active_pane = src_id;
                new_pane
            }
        };
        let Some(src_idx) = self.pane(src_id).map(|p| p.active_tab) else {
            return;
        };
        let Some(tab) = self.pane_mut(src_id).and_then(|p| p.remove_tab(src_idx)) else {
            return;
        };
        let src_empty = self.pane(src_id).is_some_and(|p| p.tabs.is_empty());
        let seq = self.next_seq();
        if let Some(dst) = self.pane_mut(dst_id) {
            // Drop replaceable start placeholder in destination pane if it's the sole tab.
            if dst.tabs.len() == 1
                && dst.tabs[0].path.is_none()
                && dst.tabs[0]
                    .state
                    .resident()
                    .is_some_and(|e| e.read(cx).replaceable_by_open())
            {
                dst.tabs.clear();
            }
            let mut moved = tab;
            moved.last_used_seq = seq;
            dst.tabs.push(moved);
            dst.active_tab = dst.tabs.len() - 1;
            if let Some(path) = dst.active().and_then(|t| t.path.clone()) {
                dst.record_history(path);
            }
        }
        if src_empty {
            self.panes.retain(|p| p.id != src_id);
            self.layout = SplitLayout::Single(dst_id);
        }
        self.active_pane = dst_id;
        self.focus_active_editor(window, cx);
        self.save_workspace_session();
        cx.notify();
    }

    pub(crate) fn reopen_last_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(record) = self.closed_tabs.pop() {
            if !record.path.is_file() {
                continue;
            }
            let target_pane = if self.pane(record.pane_id).is_some() {
                record.pane_id
            } else {
                self.active_pane
            };
            self.open_path_in_pane(target_pane, record.path, window, cx);
            if record.pinned
                && let Some(idx) = self.pane(target_pane).map(|p| p.active_tab)
            {
                self.toggle_pin_tab(target_pane, idx, cx);
            }
            return;
        }
    }

    pub(crate) fn create_and_open_vault_note(
        &mut self,
        full_path: PathBuf,
        rel_label: &str,
        open_in_split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !full_path.exists() {
            if let Some(parent) = full_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let stem = full_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(rel_label);
            if let Err(err) = std::fs::write(&full_path, format!("# {stem}\n\n")) {
                self.vault_status = Some(format!("ノートを作成できません: {err}").into());
                cx.notify();
                return;
            }
            self.refresh_vault_folder(window, cx);
        }
        if open_in_split {
            self.open_path_in_split_pane(full_path, window, cx);
        } else {
            self.open_path_in_active_pane(full_path, window, cx);
        }
    }

    pub(crate) fn open_daily_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            self.pick_vault_folder(window, cx);
            return;
        };
        let today = today_iso_date();
        let rel = format!("{today}.md");
        let full_path = vault.root().join(&rel);
        self.create_and_open_vault_note(full_path, &today, false, window, cx);
    }

    pub(crate) fn snapshot_session(&self) -> WorkspaceSession {
        let split_ratio = match self.layout {
            SplitLayout::Single(_) => None,
            SplitLayout::Split { ratio, .. } => Some(ratio),
        };
        let mut panes = Vec::new();
        let mut active_pane_index = 0usize;
        for pane in &self.panes {
            let mut tabs = Vec::new();
            let mut mapped_active = 0usize;
            for (idx, tab) in pane.tabs.iter().enumerate() {
                if let Some(path) = &tab.path {
                    if idx == pane.active_tab {
                        mapped_active = tabs.len();
                    }
                    tabs.push(PersistedTab {
                        path: path.clone(),
                        pinned: tab.pinned,
                    });
                }
            }
            if !tabs.is_empty() {
                if pane.id == self.active_pane {
                    active_pane_index = panes.len();
                }
                panes.push(PersistedPane {
                    tabs,
                    active_tab: mapped_active,
                });
            }
        }
        WorkspaceSession {
            vault_root: self.vault.as_ref().map(|v| v.root().to_path_buf()),
            left_sidebar_open: self.left_sidebar_open,
            left_panel: self.left_panel,
            left_sidebar_width: self.left_sidebar_width,
            right_sidebar_open: self.right_sidebar_open,
            right_panel: self.right_panel,
            right_sidebar_width: self.right_sidebar_width,
            split_ratio: (panes.len() >= 2).then_some(split_ratio).flatten(),
            active_pane_index,
            panes,
        }
    }

    pub(crate) fn save_workspace_session(&self) {
        let snap = self.snapshot_session();
        if snap.vault_root.is_some() || !snap.panes.is_empty() {
            save_session(&snap);
        }
    }

    fn restore_session(
        &mut self,
        session: WorkspaceSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.left_sidebar_open = session.left_sidebar_open;
        self.left_panel = session.left_panel;
        self.left_sidebar_width = clamp_left_sidebar_width(session.left_sidebar_width);
        self.right_sidebar_open = session.right_sidebar_open;
        self.right_panel = session.right_panel;
        self.right_sidebar_width = clamp_right_sidebar_width(session.right_sidebar_width);

        if let Some(root) = session.vault_root {
            if let Ok(folder) = Folder::scan_dir(&root) {
                self.vault = Some(Arc::new(folder));
                self.sync_vault_to_all_editors(window, cx);
                self.refresh_git_snapshot(window, cx);
            }
        }

        if session.panes.is_empty() {
            return;
        }

        // Populate pane 0 (and pane 1 if split was saved).
        let first_pane_id = self.active_pane;
        if let Some(p0) = session.panes.first() {
            for t in &p0.tabs {
                self.open_path_in_pane(first_pane_id, t.path.clone(), window, cx);
                if t.pinned
                    && let Some(idx) = self.pane(first_pane_id).map(|p| p.active_tab)
                {
                    if let Some(tab) = self
                        .pane_mut(first_pane_id)
                        .and_then(|p| p.tabs.get_mut(idx))
                    {
                        tab.pinned = true;
                    }
                }
            }
            let seq = self.next_seq();
            if let Some(pane) = self.pane_mut(first_pane_id) {
                let target = p0.active_tab.min(pane.tabs.len().saturating_sub(1));
                pane.activate_index(target, seq);
            }
        }

        if session.panes.len() >= 2
            && let Some(p1) = session.panes.get(1)
            && !p1.tabs.is_empty()
        {
            let second_pane_id = self.split_pane(window, cx);
            if let Some(ratio) = session.split_ratio {
                self.layout.set_ratio(ratio);
            }
            for t in &p1.tabs {
                self.open_path_in_pane(second_pane_id, t.path.clone(), window, cx);
                if t.pinned
                    && let Some(idx) = self.pane(second_pane_id).map(|p| p.active_tab)
                {
                    if let Some(tab) = self
                        .pane_mut(second_pane_id)
                        .and_then(|p| p.tabs.get_mut(idx))
                    {
                        tab.pinned = true;
                    }
                }
            }
            let seq = self.next_seq();
            if let Some(pane) = self.pane_mut(second_pane_id) {
                let target = p1.active_tab.min(pane.tabs.len().saturating_sub(1));
                pane.activate_index(target, seq);
            }
            self.active_pane = if session.active_pane_index == 1 {
                second_pane_id
            } else {
                first_pane_id
            };
        }

        if let Some(active_idx) = self.pane(self.active_pane).map(|p| p.active_tab) {
            let _ = self.ensure_tab_resident(self.active_pane, active_idx, window, cx);
        }
        self.focus_active_editor(window, cx);
    }

    pub(crate) fn new_tab_in_pane(
        &mut self,
        pane_id: PaneId,
        kind: Option<Kind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_id = self.alloc_tab_id();
        let seq = self.next_seq();
        let vault = self.vault.clone();
        let editor = cx.new(|cx| {
            let mut e = SlideEditor::new(ibis_doc_model::Presentation::new(), window, cx);
            if let Some(k) = kind {
                e.new_file(k, window, cx);
            }
            e.attach_vault_folder(vault, window, cx);
            e.embedded_chrome = Some(Self::policy_for_editor(&e, None, None));
            e
        });
        let (path, title, resolved_kind) = Self::editor_metadata(&editor, cx);
        let tab = TabEntry {
            id: tab_id,
            path,
            title,
            kind: resolved_kind,
            pinned: false,
            last_used_seq: seq,
            toolbar_override: None,
            right_sidebar_override: None,
            state: TabState::Resident(editor),
        };
        if let Some(pane) = self.pane_mut(pane_id) {
            pane.tabs.push(tab);
            pane.active_tab = pane.tabs.len() - 1;
        }
        self.active_pane = pane_id;
        self.enforce_tab_budget(cx);
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    #[cfg(target_family = "wasm")]
    pub fn set_web_vault_tree(
        &mut self,
        label: String,
        files: Vec<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = PathBuf::from(format!("sorane-web://{label}"));
        let existing_contents = self
            .vault
            .as_ref()
            .map(|v| v.cached_contents().clone())
            .unwrap_or_default();
        self.vault = Some(Arc::new(Folder::from_web_files(
            root,
            label,
            files,
            existing_contents,
        )));
        self.collapsed_dirs.clear();
        self.selected_tag = None;
        self.cached_search_hits.clear();
        self.cached_search_query.clear();
        self.left_sidebar_open = true;
        self.left_panel = LeftPanelKind::Vault;
        self.save_workspace_session();
        cx.notify();
    }

    #[cfg(target_family = "wasm")]
    pub fn hydrate_web_vault_file(
        &mut self,
        rel_path: String,
        content: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(vault) = self.vault.as_ref() {
            self.vault = Some(Arc::new(vault.with_hydrated_file(rel_path, content)));
            cx.notify();
        }
    }

    #[cfg(target_family = "wasm")]
    pub fn open_web_bytes_tab(
        &mut self,
        name: String,
        data: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let virtual_path = self.vault.as_ref().and_then(|v| {
            v.relative_path_for(Path::new(&name))
                .map(|rel| v.root().join(rel))
        });
        self.open_web_bytes_tab_with_path(virtual_path, name, data, window, cx);
    }

    #[cfg(target_family = "wasm")]
    pub fn open_web_bytes_tab_with_path(
        &mut self,
        virtual_path: Option<PathBuf>,
        name: String,
        data: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ref vp) = virtual_path
            && self.activate_editor_by_path(vp, window, cx)
        {
            return;
        }
        let reuse_active = self
            .active_editor(cx)
            .is_some_and(|ed| !ed.read(cx).has_document);
        let editor = if reuse_active {
            let Some(ed) = self.active_editor(cx) else {
                return;
            };
            ed.update(cx, |e, cx| {
                e.open_web_bytes(name.clone(), data, false, window, cx);
                e.embedded_chrome = Some(Self::policy_for_editor(e, None, None));
            });
            ed
        } else {
            let tab_id = self.alloc_tab_id();
            let seq = self.next_seq();
            let editor = cx.new(|cx| {
                let mut e = SlideEditor::new(ibis_doc_model::Presentation::new(), window, cx);
                e.open_web_bytes(name.clone(), data, false, window, cx);
                e.embedded_chrome = Some(Self::policy_for_editor(&e, None, None));
                e
            });
            let (path, title, resolved_kind) = Self::editor_metadata(&editor, cx);
            let tab = TabEntry {
                id: tab_id,
                path: virtual_path.clone().or(path),
                title,
                kind: resolved_kind,
                pinned: false,
                last_used_seq: seq,
                toolbar_override: None,
                right_sidebar_override: None,
                state: TabState::Resident(editor.clone()),
            };
            if let Some(pane) = self.pane_mut(self.active_pane) {
                pane.tabs.push(tab);
                pane.active_tab = pane.tabs.len() - 1;
            }
            editor
        };
        let (path, title, resolved_kind) = Self::editor_metadata(&editor, cx);
        if let Some(pane) = self.pane_mut(self.active_pane)
            && let Some(tab) = pane.tabs.get_mut(pane.active_tab)
        {
            tab.path = virtual_path.or(path);
            tab.title = title;
            tab.kind = resolved_kind;
        }
        self.enforce_tab_budget(cx);
        self.focus_active_editor(window, cx);
        self.save_workspace_session();
        cx.notify();
    }

    pub(crate) fn new_markdown_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_family = "wasm")]
        {
            self.open_web_bytes_tab(
                "Untitled.md".to_string(),
                b"# Untitled\n\n".to_vec(),
                window,
                cx,
            );
            return;
        }
        #[cfg(not(target_family = "wasm"))]
        if let Some(vault) = self.vault.clone() {
            let note_path = crate::session::next_numbered_markdown_path(vault.root(), "Untitled");
            if std::fs::write(&note_path, "# Untitled\n\n").is_ok() {
                self.refresh_vault_folder(window, cx);
                self.open_path_in_active_pane(note_path, window, cx);
                return;
            }
        }
        #[cfg(not(target_family = "wasm"))]
        self.new_tab_in_pane(self.active_pane, Some(Kind::Document), window, cx);
    }

    pub(crate) fn new_marp_deck(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_family = "wasm")]
        {
            self.open_web_bytes_tab(
                "Slides.marp.md".to_string(),
                crate::session::DEFAULT_MARP_TEMPLATE.as_bytes().to_vec(),
                window,
                cx,
            );
            return;
        }
        #[cfg(not(target_family = "wasm"))]
        if let Some(vault) = self.vault.clone() {
            let deck_path = crate::session::next_numbered_markdown_path(vault.root(), "Slides");
            if std::fs::write(&deck_path, crate::session::DEFAULT_MARP_TEMPLATE).is_ok() {
                self.refresh_vault_folder(window, cx);
                self.open_path_in_active_pane(deck_path, window, cx);
                return;
            }
        }
        #[cfg(not(target_family = "wasm"))]
        self.new_tab_in_pane(self.active_pane, Some(Kind::Presentation), window, cx);
    }

    pub(crate) fn new_workbook_markdown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_family = "wasm")]
        {
            self.open_web_bytes_tab(
                "Workbook.sheet.md".to_string(),
                crate::session::DEFAULT_WORKBOOK_MARKDOWN_TEMPLATE
                    .as_bytes()
                    .to_vec(),
                window,
                cx,
            );
            return;
        }
        #[cfg(not(target_family = "wasm"))]
        if let Some(vault) = self.vault.clone() {
            let sheet_path = crate::session::next_numbered_markdown_path(vault.root(), "Workbook");
            if std::fs::write(
                &sheet_path,
                crate::session::DEFAULT_WORKBOOK_MARKDOWN_TEMPLATE,
            )
            .is_ok()
            {
                self.refresh_vault_folder(window, cx);
                self.open_path_in_active_pane(sheet_path, window, cx);
                return;
            }
        }
        #[cfg(not(target_family = "wasm"))]
        self.new_tab_in_pane(self.active_pane, Some(Kind::Workbook), window, cx);
    }

    pub(crate) fn new_lawtext_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_family = "wasm")]
        {
            self.open_web_bytes_tab(
                "法令案.lawtext".to_string(),
                crate::session::DEFAULT_LAWTEXT_TEMPLATE.as_bytes().to_vec(),
                window,
                cx,
            );
            return;
        }
        #[cfg(not(target_family = "wasm"))]
        if let Some(vault) = self.vault.clone() {
            let law_path =
                crate::session::next_numbered_file_path(vault.root(), "法令案", "lawtext");
            if std::fs::write(&law_path, crate::session::DEFAULT_LAWTEXT_TEMPLATE).is_ok() {
                self.refresh_vault_folder(window, cx);
                self.open_path_in_active_pane(law_path, window, cx);
                return;
            }
        }
        #[cfg(not(target_family = "wasm"))]
        self.new_tab_in_pane(self.active_pane, Some(Kind::Document), window, cx);
    }

    pub(crate) fn close_tab(
        &mut self,
        pane_id: PaneId,
        tab_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Protect pinned tabs from accidental closure (Obsidian behavior: unpin first).
        if self
            .pane(pane_id)
            .and_then(|p| p.tabs.get(tab_idx))
            .is_some_and(|t| t.pinned)
        {
            return;
        }

        // Ask the editor before closing if it has unsaved edits.
        if let Some(editor) = self
            .pane(pane_id)
            .and_then(|p| p.tabs.get(tab_idx))
            .and_then(|t| t.state.resident())
            .cloned()
        {
            let may_close = editor.update(cx, |e, cx| e.window_should_close(window, cx));
            if !may_close {
                self.activate_tab(pane_id, tab_idx, window, cx);
                return;
            }
        }

        let removed = self
            .pane_mut(pane_id)
            .and_then(|pane| pane.remove_tab(tab_idx));
        if let Some(tab) = removed
            && let Some(path) = tab.path
        {
            self.closed_tabs.push(ClosedTabRecord {
                path,
                pane_id,
                pinned: tab.pinned,
            });
            if self.closed_tabs.len() > MAX_CLOSED_TABS {
                let excess = self.closed_tabs.len() - MAX_CLOSED_TABS;
                self.closed_tabs.drain(0..excess);
            }
        }

        let pane_became_empty = self
            .pane(pane_id)
            .is_some_and(|pane| pane.tabs.is_empty());

        if pane_became_empty {
            if let Some(other) = self.layout.other_pane(pane_id) {
                self.panes.retain(|p| p.id != pane_id);
                self.layout = SplitLayout::Single(other);
                self.active_pane = other;
            } else {
                // Closing the last tab in the sole pane closes the window cleanly.
                self.save_workspace_session();
                window.remove_window();
                return;
            }
        }

        if let Some(active_idx) = self.pane(self.active_pane).map(|p| p.active_tab) {
            let _ = self.ensure_tab_resident(self.active_pane, active_idx, window, cx);
        }
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    pub(crate) fn split_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> PaneId {
        if let Some(other) = self.layout.other_pane(self.active_pane) {
            self.active_pane = other;
            self.focus_active_editor(window, cx);
            cx.notify();
            return other;
        }
        let new_pane_id = self.alloc_pane_id();
        let tab_id = self.alloc_tab_id();
        let seq = self.next_seq();
        let vault = self.vault.clone();
        let editor = cx.new(|cx| {
            let mut e = SlideEditor::new(ibis_doc_model::Presentation::new(), window, cx);
            e.attach_vault_folder(vault, window, cx);
            e.embedded_chrome = Some(Self::policy_for_editor(&e, None, None));
            e
        });
        let (path, title, kind) = Self::editor_metadata(&editor, cx);
        let tab = TabEntry {
            id: tab_id,
            path,
            title,
            kind,
            pinned: false,
            last_used_seq: seq,
            toolbar_override: None,
            right_sidebar_override: None,
            state: TabState::Resident(editor),
        };
        let first = self.active_pane;
        self.panes.push(PaneGroup::new(new_pane_id, tab));
        self.layout = SplitLayout::Split {
            axis: SplitAxis::Horizontal,
            ratio: DEFAULT_PANE_SPLIT_RATIO,
            first,
            second: new_pane_id,
        };
        self.active_pane = new_pane_id;
        self.focus_active_editor(window, cx);
        cx.notify();
        new_pane_id
    }

    pub(crate) fn toggle_split_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(other) = self.layout.other_pane(self.active_pane) {
            // Merge `other`'s tabs into `active_pane` and collapse to Single.
            if let Some(pos) = self.panes.iter().position(|p| p.id == other) {
                let removed = self.panes.remove(pos);
                let active_id = self.active_pane;
                if let Some(target) = self.pane_mut(active_id) {
                    for tab in removed.tabs {
                        let is_empty_start = tab.path.is_none()
                            && tab
                                .state
                                .resident()
                                .is_some_and(|e| e.read(cx).replaceable_by_open());
                        if !is_empty_start {
                            target.tabs.push(tab);
                        }
                    }
                }
            }
            self.layout = SplitLayout::Single(self.active_pane);
            self.enforce_tab_budget(cx);
            self.focus_active_editor(window, cx);
            cx.notify();
        } else {
            let _ = self.split_pane(window, cx);
        }
    }

    pub(crate) fn open_vault_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let scanned = cx
                .background_executor()
                .spawn(async move { Folder::scan_dir(&path) })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| match scanned {
                Ok(folder) => {
                    ws.save_workspace_session();
                    let new_root = folder.root().to_path_buf();
                    ws.vault = Some(Arc::new(folder));
                    ws.collapsed_dirs.clear();
                    ws.selected_tag = None;
                    ws.cached_search_hits.clear();
                    ws.cached_search_query.clear();
                    ws.left_sidebar_open = true;
                    ws.left_panel = LeftPanelKind::Vault;
                    ws.vault_status = None;
                    if let Some(prev) = load_vault_session(&new_root) {
                        ws.left_sidebar_width = clamp_left_sidebar_width(prev.left_sidebar_width);
                        ws.right_sidebar_width = clamp_right_sidebar_width(prev.right_sidebar_width);
                        ws.right_sidebar_open = prev.right_sidebar_open;
                        ws.right_panel = prev.right_panel;
                    }
                    ws.sync_vault_to_all_editors(window, cx);
                    ws.refresh_git_snapshot(window, cx);
                    let index_path = new_root.join("index.md");
                    if index_path.is_file()
                        && ws
                            .active_editor(cx)
                            .is_some_and(|e| e.read(cx).replaceable_by_open())
                    {
                        ws.open_path_in_active_pane(index_path, window, cx);
                    }
                    ws.save_workspace_session();
                    cx.notify();
                }
                Err(err) => {
                    ws.vault_status = Some(err.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn pick_vault_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_family = "wasm")]
        {
            cx.spawn_in(window, async move |this, cx| {
                if let Ok(Some((name, bytes))) = ibis_office_shell::web::pick_file().await {
                    let _ = this.update_in(cx, |ws, window, cx| {
                        ws.open_web_bytes_tab(name, bytes, window, cx);
                    });
                }
            })
            .detach();
            return;
        }
        #[cfg(not(target_family = "wasm"))]
        {
            let picked = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                extensions: Vec::new(),
                prompt: Some(ibis_office_shell::i18n::text("file.open-folder")),
            });
            cx.spawn_in(window, async move |this, cx| {
                let Ok(Ok(Some(paths))) = picked.await else { return };
                let Some(path) = paths.into_iter().next() else { return };
                let _ = this.update_in(cx, |ws, window, cx| {
                    ws.open_vault_path(path, window, cx);
                });
            })
            .detach();
        }
    }

    pub(crate) fn start_github_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_family = "wasm")]
        {
            let _ = (window, cx);
            ibis_office_shell::web::trigger_start_action("github-vault");
            return;
        }
        #[cfg(not(target_family = "wasm"))]
        {
        if self.github_busy {
            return;
        }
        self.github_auth_generation = self.github_auth_generation.wrapping_add(1);
        let auth_gen = self.github_auth_generation;
        self.github_busy = true;
        self.github_status = Some(("GitHub 認証コードを取得しています…".to_string(), false));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let flow_res = cx
                .background_executor()
                .spawn(async move { github::start_device_flow() })
                .await;
            let flow = match flow_res {
                Ok(f) => f,
                Err(err) => {
                    let _ = this.update_in(cx, |ws, _, cx| {
                        ws.github_busy = false;
                        ws.github_status = Some((err, true));
                        cx.notify();
                    });
                    return;
                }
            };

            let verify_url = flow.verification_uri.clone();
            let device_code = flow.device_code.clone();
            let mut interval_secs = flow.interval.max(5);
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_secs(flow.expires_in.min(900));

            let _ = this.update_in(cx, |ws, _, cx| {
                if ws.github_auth_generation != auth_gen {
                    return;
                }
                ws.github_device_flow = Some(flow);
                ws.github_busy = false;
                ws.github_status = Some((
                    "ブラウザでコードを入力して認証を完了してください。".to_string(),
                    false,
                ));
                ibis_office_shell::external::open_url(&verify_url, cx);
                cx.notify();
            });

            loop {
                if std::time::Instant::now() >= deadline {
                    let _ = this.update_in(cx, |ws, _, cx| {
                        if ws.github_auth_generation == auth_gen {
                            ws.github_device_flow = None;
                            ws.github_status = Some((
                                "認証コードの有効期限が切れました。もう一度お試しください。".to_string(),
                                true,
                            ));
                            cx.notify();
                        }
                    });
                    return;
                }

                let code_clone = device_code.clone();
                let wait = std::time::Duration::from_secs(interval_secs);
                let poll_res = cx
                    .background_executor()
                    .spawn(async move {
                        std::thread::sleep(wait);
                        github::poll_device_token(&code_clone)
                    })
                    .await;

                let still_active = this
                    .update_in(cx, |ws, _, _| ws.github_auth_generation == auth_gen)
                    .unwrap_or(false);
                if !still_active {
                    return;
                }

                match poll_res {
                    Ok(DeviceTokenPoll::Pending) => continue,
                    Ok(DeviceTokenPoll::SlowDown) => {
                        interval_secs = (interval_secs + 5).min(30);
                        continue;
                    }
                    Ok(DeviceTokenPoll::Authorized { access_token }) => {
                        let finish_res = cx
                            .background_executor()
                            .spawn(async move {
                                let user = github::fetch_user(&access_token)?;
                                let session = GitHubSession {
                                    access_token: access_token.clone(),
                                    login: user.login.clone(),
                                    name: user.name.clone(),
                                };
                                github::save_session(&session)?;
                                let catalog =
                                    github::fetch_repo_catalog(&access_token, &user.login)?;
                                Ok::<_, String>((session, catalog))
                            })
                            .await;

                        let _ = this.update_in(cx, |ws, _, cx| {
                            if ws.github_auth_generation != auth_gen {
                                return;
                            }
                            ws.github_device_flow = None;
                            match finish_res {
                                Ok((session, catalog)) => {
                                    ws.github_status = Some((
                                        format!("@{} でログインしました。", session.login),
                                        false,
                                    ));
                                    ws.github_session = Some(session);
                                    ws.github_catalog = Some(catalog);
                                }
                                Err(err) => {
                                    ws.github_status = Some((err, true));
                                }
                            }
                            cx.notify();
                        });
                        return;
                    }
                    Ok(DeviceTokenPoll::Expired) => {
                        let _ = this.update_in(cx, |ws, _, cx| {
                            ws.github_device_flow = None;
                            ws.github_status = Some((
                                "認証コードの有効期限が切れました。".to_string(),
                                true,
                            ));
                            cx.notify();
                        });
                        return;
                    }
                    Ok(DeviceTokenPoll::AccessDenied) => {
                        let _ = this.update_in(cx, |ws, _, cx| {
                            ws.github_device_flow = None;
                            ws.github_status = Some(("認証がキャンセルされました。".to_string(), true));
                            cx.notify();
                        });
                        return;
                    }
                    Ok(DeviceTokenPoll::Error(err)) | Err(err) => {
                        let _ = this.update_in(cx, |ws, _, cx| {
                            ws.github_device_flow = None;
                            ws.github_status = Some((err, true));
                            cx.notify();
                        });
                        return;
                    }
                }
            }
        })
        .detach();
        }
    }

    pub(crate) fn cancel_github_login(&mut self, cx: &mut Context<Self>) {
        self.github_auth_generation = self.github_auth_generation.wrapping_add(1);
        self.github_device_flow = None;
        self.github_busy = false;
        self.github_status = None;
        cx.notify();
    }

    pub(crate) fn logout_github(&mut self, cx: &mut Context<Self>) {
        self.github_auth_generation = self.github_auth_generation.wrapping_add(1);
        github::clear_session();
        self.github_session = None;
        self.github_catalog = None;
        self.github_device_flow = None;
        self.github_busy = false;
        self.github_status = None;
        cx.notify();
    }

    pub(crate) fn refresh_github_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.github_session.clone() else {
            return;
        };
        self.github_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    github::fetch_repo_catalog(&session.access_token, &session.login)
                })
                .await;
            let _ = this.update_in(cx, |ws, _, cx| {
                ws.github_busy = false;
                match res {
                    Ok(catalog) => {
                        ws.github_catalog = Some(catalog);
                    }
                    Err(err) => {
                        ws.github_status = Some((err, true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn create_github_soranedocs_repo(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.github_session.clone() else {
            return;
        };
        self.github_busy = true;
        self.github_status = Some((
            format!("Private リポジトリ {}/soraneDocs を作成しています…", session.login),
            false,
        ));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let repo = github::create_private_vault_repo(
                        &session.access_token,
                        &session.login,
                        sorane_github::DEFAULT_VAULT_REPO_NAME,
                    )?;
                    let target = github::default_clone_target(&repo.repo);
                    let cloned =
                        github::clone_or_open_repo(&session.access_token, &repo, &target)?;
                    let catalog =
                        github::fetch_repo_catalog(&session.access_token, &session.login).ok();
                    Ok::<_, String>((cloned, catalog))
                })
                .await;

            let _ = this.update_in(cx, |ws, window, cx| {
                ws.github_busy = false;
                match res {
                    Ok((path, catalog)) => {
                        if let Some(c) = catalog {
                            ws.github_catalog = Some(c);
                        }
                        ws.github_status = Some((
                            format!("{} を開きました。", path.display()),
                            false,
                        ));
                        ws.open_vault_path(path, window, cx);
                    }
                    Err(err) => {
                        ws.github_status = Some((err, true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn connect_github_repo(
        &mut self,
        repo: GitHubRepo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.github_session.clone() else {
            return;
        };
        let target = github::default_clone_target(&repo.repo);
        self.github_busy = true;
        self.github_status = Some((
            format!("{} を {} へ同期しています…", repo.full_name, target.display()),
            false,
        ));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    github::clone_or_open_repo(&session.access_token, &repo, &target)
                })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| {
                ws.github_busy = false;
                match res {
                    Ok(path) => {
                        ws.github_status = Some((
                            format!("{} を開きました。", path.display()),
                            false,
                        ));
                        ws.open_vault_path(path, window, cx);
                    }
                    Err(err) => {
                        ws.github_status = Some((err, true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn github_pull_current_vault(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(session), Some(root)) = (
            self.github_session.clone(),
            self.vault.as_ref().map(|v| v.root().to_path_buf()),
        ) else {
            return;
        };
        self.github_busy = true;
        self.github_status = Some(("GitHub から Pull しています…".to_string(), false));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { github::git_pull(&session.access_token, &root) })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| {
                ws.github_busy = false;
                match res {
                    Ok(msg) => {
                        let summary = if msg.is_empty() {
                            "最新の状態です。".to_string()
                        } else {
                            msg
                        };
                        ws.github_status = Some((summary, false));
                        ws.refresh_vault_folder(window, cx);
                        ws.refresh_git_snapshot(window, cx);
                    }
                    Err(err) => {
                        ws.github_status = Some((err, true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn github_push_current_vault(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(session), Some(root)) = (
            self.github_session.clone(),
            self.vault.as_ref().map(|v| v.root().to_path_buf()),
        ) else {
            return;
        };
        self.github_busy = true;
        self.github_status = Some(("変更をコミットして GitHub へ Push しています…".to_string(), false));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    github::git_commit_and_push(
                        &session.access_token,
                        &root,
                        &session.login,
                        "Update documents via soraneDocs Desktop",
                    )
                })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| {
                ws.github_busy = false;
                match res {
                    Ok(msg) => {
                        ws.github_status = Some((msg, false));
                        ws.refresh_git_snapshot(window, cx);
                    }
                    Err(err) => {
                        ws.github_status = Some((err, true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_github_panel(&self, has_open_git_vault: bool, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let mut card = v_flex()
            .p_2()
            .gap_2()
            .rounded(px(6.))
            .bg(theme.background)
            .border_1()
            .border_color(theme.border);

        let header_label = match &self.github_session {
            Some(s) => format!("GitHub (@{})", s.login),
            None => "GitHub 連携".to_string(),
        };
        card = card.child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .child(header_label),
        );

        if let Some((msg, is_err)) = &self.github_status {
            card = card.child(
                div()
                    .text_size(px(11.))
                    .text_color(if *is_err {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(msg.clone()),
            );
        }

        if let Some(flow) = &self.github_device_flow {
            let verify_url = flow.verification_uri.clone();
            card = card
                .child(
                    div()
                        .p_2()
                        .rounded(px(4.))
                        .bg(theme.secondary)
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child(format!("認証コード: {}", flow.user_code)),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("ws-gh-open-verify")
                                .xsmall()
                                .primary()
                                .label("ブラウザを開く")
                                .on_click(move |_, _, cx| {
                                    ibis_office_shell::external::open_url(&verify_url, cx);
                                }),
                        )
                        .child(
                            Button::new("ws-gh-cancel-login")
                                .xsmall()
                                .label("キャンセル")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_github_login(cx);
                                })),
                        ),
                );
            return card;
        }

        let Some(_session) = &self.github_session else {
            return card.child(
                Button::new("ws-gh-login")
                    .small()
                    .primary()
                    .disabled(self.github_busy)
                    .label("GitHub でログイン")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_github_login(window, cx);
                    })),
            );
        };

        if has_open_git_vault {
            card = card.child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("ws-gh-pull")
                            .xsmall()
                            .disabled(self.github_busy)
                            .label("Pull")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.github_pull_current_vault(window, cx);
                            })),
                    )
                    .child(
                        Button::new("ws-gh-push")
                            .xsmall()
                            .primary()
                            .disabled(self.github_busy)
                            .label("Commit & Push")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.github_push_current_vault(window, cx);
                            })),
                    ),
            );
        }

        if let Some(catalog) = &self.github_catalog {
            if !catalog.installed {
                card = card.child(
                    Button::new("ws-gh-install-app")
                        .xsmall()
                        .label("GitHub App をインストール ↗")
                        .on_click(|_, _, cx| {
                            ibis_office_shell::external::open_url(GITHUB_APP_INSTALL_URL, cx);
                        }),
                );
            } else if !catalog.can_create_repo && !catalog.has_sorane_docs_repo {
                let settings_url = catalog
                    .install_settings_url
                    .clone()
                    .unwrap_or_else(|| "https://github.com/settings/installations".to_string());
                card = card.child(
                    Button::new("ws-gh-app-settings")
                        .xsmall()
                        .label("アプリ設定 (All repos) ↗")
                        .on_click(move |_, _, cx| {
                            ibis_office_shell::external::open_url(&settings_url, cx);
                        }),
                );
            }

            if !has_open_git_vault {
                if !catalog.has_sorane_docs_repo {
                    card = card.child(
                        Button::new("ws-gh-create-soranedocs")
                            .small()
                            .primary()
                            .disabled(self.github_busy)
                            .label("soraneDocs (Private) を作成して開く")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_github_soranedocs_repo(window, cx);
                            })),
                    );
                }

                if !catalog.repos.is_empty() {
                    card = card.child(
                        div()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("接続するリポジトリを選択"),
                    );
                    for (idx, repo) in catalog.repos.iter().take(8).enumerate() {
                        let repo_clone = repo.clone();
                        let label = format!(
                            "{}{}",
                            repo.full_name,
                            if repo.private { " (Private)" } else { "" }
                        );
                        card = card.child(
                            div()
                                .id(SharedString::from(format!("ws-gh-repo:{idx}")))
                                .cursor_pointer()
                                .rounded(px(4.))
                                .px_2()
                                .py(px(4.))
                                .text_xs()
                                .truncate()
                                .hover(|h| h.bg(theme.secondary))
                                .child(label)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if !this.github_busy {
                                        this.connect_github_repo(repo_clone.clone(), window, cx);
                                    }
                                })),
                        );
                    }
                }
            }
        }

        card.child(
            h_flex()
                .gap_1()
                .child(
                    Button::new("ws-gh-refresh")
                        .xsmall()
                        .disabled(self.github_busy)
                        .label("更新")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.refresh_github_catalog(window, cx);
                        })),
                )
                .child(
                    Button::new("ws-gh-logout")
                        .xsmall()
                        .label("ログアウト")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.logout_github(cx);
                        })),
                ),
        )
    }

    pub(crate) fn refresh_vault_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.vault.as_ref().map(|v| v.root().to_path_buf()) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let scanned = cx
                .background_executor()
                .spawn(async move { Folder::scan_dir(&root) })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| {
                if let Ok(folder) = scanned {
                    ws.vault = Some(Arc::new(folder));
                    if ws.left_panel == LeftPanelKind::Search {
                        let q = ws.cached_search_query.clone();
                        ws.refresh_content_search(&q);
                    }
                    ws.sync_vault_to_all_editors(window, cx);
                    ws.refresh_git_snapshot(window, cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn refresh_content_search(&mut self, query: &str) {
        self.cached_search_query = query.to_owned();
        if query.is_empty() {
            self.cached_search_hits.clear();
            return;
        }
        if let Some(vault) = self.vault.as_ref() {
            self.cached_search_hits = vault.search_content(query, SEARCH_HIT_LIMIT);
        } else {
            self.cached_search_hits.clear();
        }
    }

    /// Window close guard: verify every resident editor across all tabs/panes.
    pub fn window_should_close(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let mut to_check: Vec<(PaneId, usize, Entity<SlideEditor>)> = Vec::new();
        for pane in &self.panes {
            for (idx, tab) in pane.tabs.iter().enumerate() {
                if let TabState::Resident(ed) = &tab.state {
                    to_check.push((pane.id, idx, ed.clone()));
                }
            }
        }
        for (pane_id, tab_idx, editor) in to_check {
            let may_close = editor.update(cx, |e, cx| e.window_should_close(window, cx));
            if !may_close {
                let seq = self.next_seq();
                self.active_pane = pane_id;
                if let Some(pane) = self.pane_mut(pane_id) {
                    pane.activate_index(tab_idx, seq);
                }
                cx.notify();
                return false;
            }
        }
        self.save_workspace_session();
        true
    }

    pub fn has_any_document(&self, cx: &App) -> bool {
        self.all_editors(cx).iter().any(|e| e.read(cx).has_document)
    }

    fn active_vault_relative(&self, cx: &App) -> Option<String> {
        let vault = self.vault.as_ref()?;
        let active = self.active_editor(cx)?;
        let path = active
            .read(cx)
            .file_path()
            .or_else(|| self.pane(self.active_pane)?.active()?.path.clone())?;
        vault.relative_path_for(&path)
    }

    fn render_activity_ribbon(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let top_pad = if cfg!(target_os = "macos") { px(32.) } else { px(8.) };
        let ai_active = self
            .active_editor(cx)
            .is_some_and(|e| e.read(cx).is_ai_assistant_open());

        let ribbon_btn = |id: &'static str, label: &'static str, tip: &'static str, active: bool| {
            div()
                .id(id)
                .w(px(30.))
                .h(px(30.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_size(px(14.))
                .when(active, |d| {
                    d.bg(theme.accent)
                        .text_color(theme.accent_foreground)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!active, |d| {
                    d.text_color(theme.muted_foreground)
                        .hover(|h| h.bg(theme.secondary).text_color(theme.foreground))
                })
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx))
                .child(label)
        };

        v_flex()
            .id("sorane-activity-ribbon")
            .w(px(RIBBON_WIDTH))
            .flex_shrink_0()
            .h_full()
            .pt(top_pad)
            .pb_2()
            .items_center()
            .justify_between()
            .bg(theme.title_bar)
            .border_r_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        ribbon_btn(
                            "ribbon-vault",
                            "📁",
                            "Vault ファイルツリー (⌘B)",
                            self.left_sidebar_open && self.left_panel == LeftPanelKind::Vault,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.vault.is_none() {
                                this.pick_vault_folder(window, cx);
                            } else if this.left_sidebar_open && this.left_panel == LeftPanelKind::Vault {
                                this.left_sidebar_open = false;
                            } else {
                                this.left_sidebar_open = true;
                                this.left_panel = LeftPanelKind::Vault;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-search",
                            "🔍",
                            "Vault 全文検索",
                            self.left_sidebar_open && self.left_panel == LeftPanelKind::Search,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.left_sidebar_open && this.left_panel == LeftPanelKind::Search {
                                this.left_sidebar_open = false;
                            } else {
                                this.left_sidebar_open = true;
                                this.left_panel = LeftPanelKind::Search;
                                this.vault_filter.update(cx, |s, cx| s.focus(window, cx));
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-tags",
                            "#",
                            "タグブラウザ",
                            self.left_sidebar_open && self.left_panel == LeftPanelKind::Tags,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.left_sidebar_open && this.left_panel == LeftPanelKind::Tags {
                                this.left_sidebar_open = false;
                            } else {
                                this.left_sidebar_open = true;
                                this.left_panel = LeftPanelKind::Tags;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-git",
                            "🌿",
                            "Git 変更・履歴",
                            self.left_sidebar_open && self.left_panel == LeftPanelKind::Git,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.left_sidebar_open && this.left_panel == LeftPanelKind::Git {
                                this.left_sidebar_open = false;
                            } else {
                                this.left_sidebar_open = true;
                                this.left_panel = LeftPanelKind::Git;
                                this.refresh_git_snapshot(window, cx);
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-quick-switcher",
                            "⌘",
                            "クイックスイッチャー (⌘O)",
                            self.quick_switcher.is_some(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_quick_switcher(window, cx);
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-split",
                            "◫",
                            "左右ペイン分割 (⌘\\)",
                            self.layout.is_split(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_split_pane(window, cx);
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-new-md",
                            "✎",
                            "新規 Markdown ノート (⌥⌘N)",
                            false,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.new_markdown_note(window, cx);
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-daily-note",
                            "📅",
                            "デイリーノートを開く",
                            false,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_daily_note(window, cx);
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-command-palette",
                            "⚡",
                            "コマンドパレット (⌘P)",
                            false,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_command_palette(window, cx);
                        })),
                    ),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        ribbon_btn(
                            "ribbon-open-folder",
                            "📂",
                            "フォルダ（Vault）を開く (⇧⌘O)",
                            false,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.pick_vault_folder(window, cx);
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-outline",
                            "☰",
                            "アウトライン (⇧⌘U)",
                            self.right_sidebar_open && self.right_panel == RightPanelKind::Outline,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_outline_panel(cx);
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-backlinks",
                            "⇄",
                            "リンク: 被リンク・発リンク (⇧⌘B)",
                            self.right_sidebar_open && self.right_panel == RightPanelKind::Backlinks,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.right_sidebar_open && this.right_panel == RightPanelKind::Backlinks {
                                this.right_sidebar_open = false;
                            } else {
                                this.right_sidebar_open = true;
                                this.right_panel = RightPanelKind::Backlinks;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-graph",
                            "◎",
                            "ローカルリンクグラフ",
                            self.right_sidebar_open && self.right_panel == RightPanelKind::Graph,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.right_sidebar_open && this.right_panel == RightPanelKind::Graph {
                                this.right_sidebar_open = false;
                            } else {
                                this.right_sidebar_open = true;
                                this.right_panel = RightPanelKind::Graph;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        ribbon_btn(
                            "ribbon-ai",
                            "✨",
                            "AI アシスタント (⇧⌘L)",
                            ai_active,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_ai_assistant_for_active_pane(window, cx);
                        })),
                    ),
            )
    }

    fn render_left_sidebar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.left_sidebar_open {
            return None;
        }
        let theme = cx.theme().clone();
        let width = clamp_left_sidebar_width(self.left_sidebar_width);
        let active_rel = self.active_vault_relative(cx);
        let query = self.vault_filter.read(cx).value().trim().to_lowercase();

        let header_title = match self.left_panel {
            LeftPanelKind::Vault => self
                .vault
                .as_ref()
                .map(|v| v.display_name())
                .unwrap_or_else(|| "Vault".to_owned()),
            LeftPanelKind::Search => "全文検索".to_owned(),
            LeftPanelKind::Tags => "タグ".to_owned(),
            LeftPanelKind::Git => "Git".to_owned(),
        };

        let mut list = v_flex()
            .id("sorane-left-sidebar-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_1()
            .gap(px(1.));

        match (&self.vault, self.left_panel) {
            (None, _) => {
                list = list.child(
                    v_flex()
                        .p_3()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("フォルダまたは Obsidian Vault を開くと、ツリー・全文検索・バックリンクが使えます。"),
                        )
                        .child(
                            Button::new("vault-empty-open")
                                .small()
                                .label("フォルダを開く…")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pick_vault_folder(window, cx);
                                })),
                        )
                        .child(self.render_github_panel(false, cx)),
                );
            }
            (Some(vault), LeftPanelKind::Vault) => {
                if query.is_empty() {
                    let rows = build_vault_tree_rows(vault.files(), &self.collapsed_dirs);
                    for row in rows.into_iter().take(TREE_ROW_LIMIT) {
                        match row {
                            VaultTreeRow::Dir {
                                path,
                                name,
                                depth,
                                collapsed,
                            } => {
                                let marker = if collapsed { "▸" } else { "▾" };
                                let key = path.clone();
                                list = list.child(
                                    div()
                                        .id(SharedString::from(format!("ws-dir:{path}")))
                                        .cursor_pointer()
                                        .rounded(px(4.))
                                        .pl(px(6. + (depth as f32) * 12.))
                                        .pr_2()
                                        .py(px(3.))
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .hover(|h| h.bg(theme.secondary).text_color(theme.foreground))
                                        .child(div().truncate().child(format!("{marker} {name}")))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if !this.collapsed_dirs.remove(&key) {
                                                this.collapsed_dirs.insert(key.clone());
                                            }
                                            cx.notify();
                                        })),
                                );
                            }
                            VaultTreeRow::File { path, name, depth } => {
                                let is_active = active_rel.as_deref() == Some(path.as_str());
                                let full_path = vault.root().join(&path);
                                let split_path = full_path.clone();
                                list = list.child(
                                    h_flex()
                                        .id(SharedString::from(format!("ws-file:{path}")))
                                        .cursor_pointer()
                                        .rounded(px(4.))
                                        .pl(px(14. + (depth as f32) * 12.))
                                        .pr_1()
                                        .py(px(3.))
                                        .gap_1()
                                        .items_center()
                                        .when(is_active, |r| {
                                            r.bg(theme.accent)
                                                .text_color(theme.accent_foreground)
                                                .font_weight(FontWeight::MEDIUM)
                                        })
                                        .when(!is_active, |r| {
                                            r.text_color(theme.foreground)
                                                .hover(|h| h.bg(theme.secondary))
                                        })
                                        .child(div().flex_1().min_w_0().text_xs().truncate().child(name))
                                        .child(
                                            div()
                                                .id(SharedString::from(format!("ws-file-split:{path}")))
                                                .px(px(4.))
                                                .rounded(px(3.))
                                                .text_size(px(10.))
                                                .text_color(theme.muted_foreground)
                                                .hover(|h| h.bg(theme.background).text_color(theme.foreground))
                                                .child("◫")
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    cx.stop_propagation();
                                                    this.open_path_in_split_pane(split_path.clone(), window, cx);
                                                })),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_path_in_active_pane(full_path.clone(), window, cx);
                                        })),
                                );
                            }
                        }
                    }
                } else {
                    for rel in vault
                        .files()
                        .iter()
                        .filter(|p| vault.matches_query(p, &query))
                        .take(TREE_ROW_LIMIT)
                    {
                        let is_active = active_rel.as_deref() == Some(rel.as_str());
                        let full_path = vault.root().join(rel);
                        list = list.child(
                            div()
                                .id(SharedString::from(format!("ws-match:{rel}")))
                                .cursor_pointer()
                                .rounded(px(4.))
                                .px_2()
                                .py(px(4.))
                                .text_xs()
                                .when(is_active, |r| r.bg(theme.accent).text_color(theme.accent_foreground))
                                .when(!is_active, |r| r.hover(|h| h.bg(theme.secondary)))
                                .child(div().truncate().child(rel.clone()))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_path_in_active_pane(full_path.clone(), window, cx);
                                })),
                        );
                    }
                }
            }
            (Some(vault), LeftPanelKind::Search) => {
                if query.is_empty() {
                    list = list.child(
                        div()
                            .p_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("キーワードを入力して Vault 内の Markdown を検索…"),
                    );
                } else if self.cached_search_hits.is_empty() {
                    list = list.child(
                        div()
                            .p_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("一致する箇所がありません"),
                    );
                } else {
                    for (idx, hit) in self.cached_search_hits.iter().enumerate() {
                        let full_path = vault.root().join(&hit.path);
                        list = list.child(
                            v_flex()
                                .id(SharedString::from(format!("ws-hit:{idx}:{}", hit.path)))
                                .cursor_pointer()
                                .rounded(px(4.))
                                .px_2()
                                .py(px(4.))
                                .hover(|h| h.bg(theme.secondary))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .truncate()
                                        .child(format!("{} · L{}", hit.path, hit.line)),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(theme.muted_foreground)
                                        .truncate()
                                        .child(hit.excerpt.clone()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_path_in_active_pane(full_path.clone(), window, cx);
                                })),
                        );
                    }
                }
            }
            (Some(vault), LeftPanelKind::Tags) => {
                if let Some(selected) = self.selected_tag.clone() {
                    let matching = vault.files_with_tag(&selected);
                    list = list.child(
                        h_flex()
                            .px_2()
                            .py(px(4.))
                            .gap_1()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(theme.accent)
                                    .text_color(theme.accent_foreground)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(format!("#{selected}")),
                            )
                            .child(
                                Button::new("ws-tag-clear")
                                    .ghost()
                                    .xsmall()
                                    .label("すべて")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.selected_tag = None;
                                        cx.notify();
                                    })),
                            ),
                    );
                    let filtered: Vec<String> = matching
                        .into_iter()
                        .filter(|p| query.is_empty() || p.to_lowercase().contains(&query))
                        .collect();
                    if filtered.is_empty() {
                        list = list.child(
                            div()
                                .p_2()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("該当するノートがありません"),
                        );
                    } else {
                        for (idx, rel) in filtered.into_iter().take(TREE_ROW_LIMIT).enumerate() {
                            let is_active = active_rel.as_deref() == Some(rel.as_str());
                            let full_path = vault.root().join(&rel);
                            let split_path = full_path.clone();
                            list = list.child(
                                h_flex()
                                    .id(SharedString::from(format!("ws-tag-file:{idx}:{rel}")))
                                    .cursor_pointer()
                                    .rounded(px(4.))
                                    .px_2()
                                    .py(px(4.))
                                    .gap_1()
                                    .items_center()
                                    .when(is_active, |r| {
                                        r.bg(theme.accent)
                                            .text_color(theme.accent_foreground)
                                            .font_weight(FontWeight::MEDIUM)
                                    })
                                    .when(!is_active, |r| {
                                        r.text_color(theme.foreground)
                                            .hover(|h| h.bg(theme.secondary))
                                    })
                                    .child(div().flex_1().min_w_0().text_xs().truncate().child(rel))
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("ws-tag-file-split:{idx}")))
                                            .px(px(4.))
                                            .rounded(px(3.))
                                            .text_size(px(10.))
                                            .text_color(theme.muted_foreground)
                                            .hover(|h| h.bg(theme.background).text_color(theme.foreground))
                                            .child("◫")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.open_path_in_split_pane(
                                                    split_path.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_path_in_active_pane(
                                            full_path.clone(),
                                            window,
                                            cx,
                                        );
                                    })),
                            );
                        }
                    }
                } else {
                    let tags: Vec<(String, usize)> = vault
                        .all_tags()
                        .into_iter()
                        .filter(|(t, _)| query.is_empty() || t.to_lowercase().contains(&query))
                        .collect();
                    if tags.is_empty() {
                        list = list.child(
                            div()
                                .p_3()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("タグ（#tag または Frontmatter の tags）が見つかりません。"),
                        );
                    } else {
                        for (idx, (tag, count)) in tags.into_iter().take(TREE_ROW_LIMIT).enumerate() {
                            let chosen = tag.clone();
                            list = list.child(
                                h_flex()
                                    .id(SharedString::from(format!("ws-tag:{idx}:{tag}")))
                                    .cursor_pointer()
                                    .rounded(px(4.))
                                    .px_2()
                                    .py(px(4.))
                                    .gap_2()
                                    .items_center()
                                    .hover(|h| h.bg(theme.secondary))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_xs()
                                            .truncate()
                                            .child(format!("#{tag}")),
                                    )
                                    .child(
                                        div()
                                            .px(px(5.))
                                            .py(px(1.))
                                            .rounded(px(8.))
                                            .bg(theme.muted)
                                            .text_size(px(10.))
                                            .text_color(theme.muted_foreground)
                                            .child(count.to_string()),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.selected_tag = Some(chosen.clone());
                                        cx.notify();
                                    })),
                            );
                        }
                    }
                }
            }
            (Some(vault), LeftPanelKind::Git) => {
                if !vault.has_git() {
                    list = list.child(
                        v_flex()
                            .p_3()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("このフォルダは Git リポジトリではありません。"),
                            )
                            .child(self.render_github_panel(false, cx)),
                    );
                } else {
                    let root_for_full_view = vault.root().to_path_buf();
                    let mut git_col = v_flex()
                        .p_2()
                        .gap_2()
                        .child(self.render_github_panel(true, cx));

                    if let Some(snap) = &self.git_snapshot {
                        git_col = git_col
                            .child(
                                v_flex()
                                    .p_2()
                                    .gap_1()
                                    .rounded(px(6.))
                                    .bg(theme.background)
                                    .border_1()
                                    .border_color(theme.border)
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(format!("🌿 {} @ {}", snap.branch, snap.head_short)),
                                    )
                                    .child(
                                        Button::new("ws-git-open-full")
                                            .small()
                                            .label("履歴・差分ビューを開く")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if let Some(editor) = this.active_editor(cx) {
                                                    let root = root_for_full_view.clone();
                                                    editor.update(cx, |e, cx| {
                                                        e.open_git_workspace_for_root(root, window, cx);
                                                    });
                                                }
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .px_1()
                                    .pt_1()
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.muted_foreground)
                                    .child(format!("変更されたファイル ({})", snap.changes.len())),
                            );

                        if snap.changes.is_empty() {
                            git_col = git_col.child(
                                div()
                                    .px_1()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("作業ツリーはクリーンです"),
                            );
                        } else {
                            for (idx, change) in snap.changes.iter().enumerate() {
                                let full_path = vault.root().join(&change.path);
                                let split_path = full_path.clone();
                                let badge_color = if change.staged {
                                    theme.accent
                                } else {
                                    theme.muted_foreground
                                };
                                git_col = git_col.child(
                                    h_flex()
                                        .id(SharedString::from(format!("ws-git-chg:{idx}:{}", change.path)))
                                        .cursor_pointer()
                                        .rounded(px(4.))
                                        .px_1()
                                        .py(px(3.))
                                        .gap_1()
                                        .items_center()
                                        .hover(|h| h.bg(theme.secondary))
                                        .child(
                                            div()
                                                .w(px(20.))
                                                .flex_shrink_0()
                                                .text_size(px(10.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(badge_color)
                                                .child(change.status.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_xs()
                                                .truncate()
                                                .child(change.path.clone()),
                                        )
                                        .child(
                                            div()
                                                .id(SharedString::from(format!("ws-git-chg-split:{idx}")))
                                                .px(px(4.))
                                                .rounded(px(3.))
                                                .text_size(px(10.))
                                                .text_color(theme.muted_foreground)
                                                .hover(|h| h.bg(theme.background).text_color(theme.foreground))
                                                .child("◫")
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    cx.stop_propagation();
                                                    this.open_path_in_split_pane(split_path.clone(), window, cx);
                                                })),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_path_in_active_pane(full_path.clone(), window, cx);
                                        })),
                                );
                            }
                        }

                        git_col = git_col.child(
                            div()
                                .px_1()
                                .pt_2()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.muted_foreground)
                                .child("最近のコミット"),
                        );

                        if snap.commits.is_empty() {
                            git_col = git_col.child(
                                div()
                                    .px_1()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("コミットがありません"),
                            );
                        } else {
                            for (idx, commit) in snap.commits.iter().enumerate() {
                                git_col = git_col.child(
                                    v_flex()
                                        .id(SharedString::from(format!("ws-git-commit:{idx}:{}", commit.short_id)))
                                        .rounded(px(4.))
                                        .px_1()
                                        .py(px(3.))
                                        .child(
                                            div()
                                                .text_xs()
                                                .truncate()
                                                .child(commit.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.5))
                                                .text_color(theme.muted_foreground)
                                                .truncate()
                                                .child(format!("{} · {}", commit.short_id, commit.date)),
                                        ),
                                );
                            }
                        }
                    } else {
                        git_col = git_col.child(
                            div()
                                .p_2()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("Git 状態を読み込み中…"),
                        );
                    }

                    list = list.child(git_col);
                }
            }
        }

        let dragging = self.left_sidebar_drag.is_some();
        let weak = cx.entity().downgrade();
        let resize_handle = div()
            .id("sorane-left-resize")
            .relative()
            .w(px(DIVIDER_WIDTH))
            .flex_shrink_0()
            .h_full()
            .cursor_col_resize()
            .bg(theme.border)
            .hover(|h| h.bg(theme.accent))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.left_sidebar_drag = Some((f32::from(event.position.x), this.left_sidebar_width));
                    cx.notify();
                }),
            )
            .when(dragging, |el| {
                let tracker = weak.clone();
                el.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let moving = tracker.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = moving.update(cx, |this, cx| {
                                        if let Some((x0, w0)) = this.left_sidebar_drag {
                                            this.left_sidebar_width = clamp_left_sidebar_width(
                                                w0 + (f32::from(event.position.x) - x0),
                                            );
                                            cx.notify();
                                        }
                                    });
                                }
                            });
                            let released = tracker.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = released.update(cx, |this, cx| {
                                        if this.left_sidebar_drag.take().is_some() {
                                            cx.notify();
                                        }
                                    });
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            });

        Some(
            h_flex()
                .flex_shrink_0()
                .h_full()
                .child(
                    v_flex()
                        .w(px(width))
                        .h_full()
                        .min_h_0()
                        .bg(theme.sidebar)
                        .child(
                            h_flex()
                                .px_2()
                                .py(px(6.))
                                .gap_1()
                                .items_center()
                                .border_b_1()
                                .border_color(theme.border)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .truncate()
                                        .child(header_title),
                                )
                                .child(
                                    Button::new("ws-vault-refresh")
                                        .ghost()
                                        .xsmall()
                                        .label("↻")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if this.left_panel == LeftPanelKind::Git {
                                                this.refresh_git_snapshot(window, cx);
                                            } else {
                                                this.refresh_vault_folder(window, cx);
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("ws-vault-close")
                                        .ghost()
                                        .xsmall()
                                        .label("×")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.left_sidebar_open = false;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .when(self.left_panel != LeftPanelKind::Git, |col| {
                            col.child(div().p_2().child(Input::new(&self.vault_filter).small()))
                        })
                        .when(
                            self.left_panel == LeftPanelKind::Vault && self.vault.is_some(),
                            |col| {
                                col.child(
                                    h_flex()
                                        .px_2()
                                        .pb(px(6.))
                                        .gap_1()
                                        .child(
                                            Button::new("ws-vault-new-note")
                                                .ghost()
                                                .xsmall()
                                                .label("+ ノート")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.new_markdown_note(window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("ws-vault-new-marp")
                                                .ghost()
                                                .xsmall()
                                                .label("+ Marp")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.new_marp_deck(window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("ws-vault-new-sheet")
                                                .ghost()
                                                .xsmall()
                                                .label("+ 表")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.new_workbook_markdown(window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("ws-vault-new-law")
                                                .ghost()
                                                .xsmall()
                                                .label("+ 法令")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.new_lawtext_draft(window, cx);
                                                })),
                                        ),
                                )
                            },
                        )
                        .when_some(self.vault_status.clone(), |col, msg| {
                            col.child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(theme.danger)
                                    .child(msg),
                            )
                        })
                        .child(list),
                )
                .child(resize_handle)
                .into_any_element(),
        )
    }

    fn render_right_sidebar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.right_sidebar_open {
            return None;
        }
        let theme = cx.theme().clone();
        let width = clamp_right_sidebar_width(self.right_sidebar_width);
        let active_rel = self.active_vault_relative(cx);
        let active_editor = self.active_editor(cx);
        let ai_active = active_editor
            .as_ref()
            .is_some_and(|e| e.read(cx).is_ai_assistant_open());
        let inspector_active = active_editor.as_ref().is_some_and(|e| {
            let ed = e.read(cx);
            !ed.is_ai_assistant_open()
                && ed.embedded_chrome.is_none_or(|p| p.show_right_sidebar)
                && ed.sidebar_open
        });

        let mut content = v_flex()
            .id("sorane-right-sidebar-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .gap_1();

        match self.right_panel {
            RightPanelKind::Backlinks => match (&self.vault, active_rel.as_deref()) {
                (Some(vault), Some(rel)) => {
                    let note_tags = vault.tags_for(rel);
                    if !note_tags.is_empty() {
                        let mut tag_row = div().flex().flex_wrap().gap(px(4.)).pb_2();
                        for (idx, tag) in note_tags.into_iter().enumerate() {
                            let tag_for_click = tag.clone();
                            tag_row = tag_row.child(
                                div()
                                    .id(SharedString::from(format!("ws-note-tag:{idx}:{tag}")))
                                    .cursor_pointer()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(theme.muted)
                                    .text_size(px(10.5))
                                    .text_color(theme.muted_foreground)
                                    .hover(|h| h.bg(theme.accent).text_color(theme.accent_foreground))
                                    .child(format!("#{tag}"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.left_sidebar_open = true;
                                        this.left_panel = LeftPanelKind::Tags;
                                        this.selected_tag = Some(tag_for_click.clone());
                                        cx.notify();
                                    })),
                            );
                        }
                        content = content.child(tag_row);
                    }

                    let backlinks = vault.backlinks_for(rel);
                    content = content.child(
                        div()
                            .px_1()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(format!("被リンク ({})", backlinks.len())),
                    );
                    if backlinks.is_empty() {
                        content = content.child(
                            div()
                                .px_1()
                                .pb_2()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("このファイルへの被リンクはまだありません。"),
                        );
                    } else {
                        for (idx, bl) in backlinks.into_iter().enumerate() {
                            let full = vault.root().join(&bl.path);
                            let split_path = full.clone();
                            content = content.child(
                                v_flex()
                                    .id(SharedString::from(format!("ws-bl:{idx}:{}", bl.path)))
                                    .cursor_pointer()
                                    .rounded(px(4.))
                                    .p(px(6.))
                                    .hover(|h| h.bg(theme.secondary))
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .truncate()
                                                    .child(format!("{} · L{}", bl.path, bl.line)),
                                            )
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!("ws-bl-split:{idx}")))
                                                    .px(px(4.))
                                                    .rounded(px(3.))
                                                    .text_size(px(10.))
                                                    .text_color(theme.muted_foreground)
                                                    .hover(|h| h.bg(theme.background).text_color(theme.foreground))
                                                    .child("◫")
                                                    .on_click(cx.listener(move |this, _, window, cx| {
                                                        cx.stop_propagation();
                                                        this.open_path_in_split_pane(split_path.clone(), window, cx);
                                                    })),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme.muted_foreground)
                                            .truncate()
                                            .child(bl.excerpt),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_path_in_active_pane(full.clone(), window, cx);
                                    })),
                            );
                        }
                    }

                    let outgoing = vault.outgoing_links_for(rel);
                    content = content.child(
                        div()
                            .px_1()
                            .pt_2()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(format!("発リンク ({})", outgoing.len())),
                    );
                    if outgoing.is_empty() {
                        content = content.child(
                            div()
                                .px_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("このファイルからの発リンクはありません。"),
                        );
                    } else {
                        for (idx, target_rel) in outgoing.into_iter().enumerate() {
                            let full = vault.root().join(&target_rel);
                            let split_path = full.clone();
                            content = content.child(
                                h_flex()
                                    .id(SharedString::from(format!("ws-out:{idx}:{target_rel}")))
                                    .cursor_pointer()
                                    .rounded(px(4.))
                                    .px(px(6.))
                                    .py(px(4.))
                                    .gap_1()
                                    .items_center()
                                    .hover(|h| h.bg(theme.secondary))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_xs()
                                            .truncate()
                                            .child(format!("→ {target_rel}")),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("ws-out-split:{idx}")))
                                            .px(px(4.))
                                            .rounded(px(3.))
                                            .text_size(px(10.))
                                            .text_color(theme.muted_foreground)
                                            .hover(|h| h.bg(theme.background).text_color(theme.foreground))
                                            .child("◫")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.open_path_in_split_pane(split_path.clone(), window, cx);
                                            })),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_path_in_active_pane(full.clone(), window, cx);
                                    })),
                            );
                        }
                    }
                }
                (None, _) => {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Vault フォルダを開くと、アクティブなノートのリンクとタグが表示されます。"),
                    );
                }
                (Some(_), None) => {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Vault 内のファイルを開くと被リンク・発リンクが表示されます。"),
                    );
                }
            },
            RightPanelKind::Outline => {
                let items = active_editor.as_ref().and_then(|e| {
                    let ed = e.read(cx);
                    ed.has_document.then(|| extract_outline(ed.presentation()))
                });
                match items {
                    None => {
                        content = content.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("ドキュメントを開くと見出し・シート・スライドのアウトラインが表示されます。"),
                        );
                    }
                    Some(items) if items.is_empty() => {
                        content = content.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("見出し・シート・スライドが見つかりません。"),
                        );
                    }
                    Some(items) => {
                        for (idx, item) in items.into_iter().enumerate() {
                            let indent = (item.level.saturating_sub(1) as f32) * 12.0;
                            let target = item.target.clone();
                            let level_badge = match item.target {
                                OutlineTarget::Paragraph(_) => format!("H{}", item.level),
                                OutlineTarget::Slide(_) => "•".to_owned(),
                            };
                            content = content.child(
                                h_flex()
                                    .id(SharedString::from(format!("ws-outline:{idx}")))
                                    .cursor_pointer()
                                    .rounded(px(4.))
                                    .pl(px(6. + indent))
                                    .pr_2()
                                    .py(px(4.))
                                    .gap(px(6.))
                                    .items_center()
                                    .hover(|h| h.bg(theme.secondary))
                                    .child(
                                        div()
                                            .text_size(px(9.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.muted_foreground)
                                            .child(level_badge),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_xs()
                                            .when(item.level == 1, |d| d.font_weight(FontWeight::MEDIUM))
                                            .truncate()
                                            .child(item.label),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.jump_to_outline_target(target.clone(), window, cx);
                                    })),
                            );
                        }
                    }
                }
            }
            RightPanelKind::Graph => match (&self.vault, active_rel.as_deref()) {
                (Some(vault), Some(rel)) => {
                    let incoming: Vec<String> = vault
                        .backlinks_for(rel)
                        .into_iter()
                        .map(|b| b.path)
                        .collect();
                    let outgoing = vault.outgoing_links_for(rel);
                    let graph_w = (width - 16.0).max(180.0);
                    let graph_h = 280.0_f32;
                    let layout = compute_local_graph_layout(rel, &incoming, &outgoing, graph_w, graph_h);

                    let edges = layout.edges.clone();
                    let out_color = theme.accent.opacity(0.65);
                    let in_color = theme.muted_foreground.opacity(0.45);
                    let edge_canvas = canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            for edge in &edges {
                                let mut builder = PathBuilder::stroke(px(1.5));
                                builder.move_to(point(
                                    bounds.origin.x + px(edge.from_x),
                                    bounds.origin.y + px(edge.from_y),
                                ));
                                builder.line_to(point(
                                    bounds.origin.x + px(edge.to_x),
                                    bounds.origin.y + px(edge.to_y),
                                ));
                                if let Ok(path) = builder.build() {
                                    let color = match edge.direction {
                                        LinkDirection::Incoming => in_color,
                                        LinkDirection::Outgoing | LinkDirection::Bidirectional => {
                                            out_color
                                        }
                                    };
                                    window.paint_path(path, color);
                                }
                            }
                        },
                    )
                    .absolute()
                    .inset_0();

                    let mut graph_box = div()
                        .id("sorane-local-graph-box")
                        .relative()
                        .w(px(layout.width))
                        .h(px(layout.height))
                        .rounded(px(8.))
                        .bg(theme.background)
                        .border_1()
                        .border_color(theme.border)
                        .overflow_hidden()
                        .child(edge_canvas);

                    for (idx, node) in layout.nodes.iter().enumerate() {
                        let pill_w = if node.is_center { 104.0 } else { 90.0 };
                        let pill_h = 22.0;
                        let left = (node.x - pill_w * 0.5).clamp(4.0, (layout.width - pill_w - 4.0).max(4.0));
                        let top = (node.y - pill_h * 0.5).clamp(4.0, (layout.height - pill_h - 4.0).max(4.0));
                        let full = vault.root().join(&node.path);
                        let prefix = match node.direction {
                            None => "● ",
                            Some(LinkDirection::Outgoing) => "→ ",
                            Some(LinkDirection::Incoming) => "← ",
                            Some(LinkDirection::Bidirectional) => "⇄ ",
                        };
                        let is_center = node.is_center;
                        graph_box = graph_box.child(
                            div()
                                .id(SharedString::from(format!("ws-graph-node:{idx}")))
                                .absolute()
                                .left(px(left))
                                .top(px(top))
                                .w(px(pill_w))
                                .h(px(pill_h))
                                .px(px(6.))
                                .rounded(px(11.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(10.5))
                                .border_1()
                                .when(is_center, |d| {
                                    d.bg(theme.accent)
                                        .border_color(theme.accent)
                                        .text_color(theme.accent_foreground)
                                        .font_weight(FontWeight::SEMIBOLD)
                                })
                                .when(!is_center, |d| {
                                    d.cursor_pointer()
                                        .bg(theme.sidebar)
                                        .border_color(theme.border)
                                        .text_color(theme.foreground)
                                        .hover(|h| h.bg(theme.secondary).border_color(theme.accent))
                                })
                                .child(div().truncate().child(format!("{prefix}{}", node.label)))
                                .when(!is_center, |d| {
                                    d.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                        if event.modifiers().platform || event.modifiers().control {
                                            this.open_path_in_split_pane(full.clone(), window, cx);
                                        } else {
                                            this.open_path_in_active_pane(full.clone(), window, cx);
                                        }
                                    }))
                                }),
                        );
                    }

                    content = content
                        .child(graph_box)
                        .child(
                            div()
                                .pt_1()
                                .px_1()
                                .text_size(px(10.5))
                                .text_color(theme.muted_foreground)
                                .child(if layout.edges.is_empty() {
                                    "接続されたノートがありません。[[ノート名]] でリンクを作るとここに表示されます。".to_owned()
                                } else if layout.truncated > 0 {
                                    format!("● 現在 · → 発 · ← 被 · ⇄ 双方向（他 {} 件 · ⌘クリックで分割）", layout.truncated)
                                } else {
                                    "● 現在 · → 発リンク · ← 被リンク · ⇄ 双方向（⌘クリックで分割）".to_owned()
                                }),
                        );
                }
                (None, _) => {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Vault フォルダを開くとローカルリンクグラフが表示されます。"),
                    );
                }
                (Some(_), None) => {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Vault 内のノートを開くと周囲のリンク関係が表示されます。"),
                    );
                }
            },
        }

        let dragging = self.right_sidebar_drag.is_some();
        let weak = cx.entity().downgrade();
        let resize_handle = div()
            .id("sorane-right-resize")
            .relative()
            .w(px(DIVIDER_WIDTH))
            .flex_shrink_0()
            .h_full()
            .cursor_col_resize()
            .bg(theme.border)
            .hover(|h| h.bg(theme.accent))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.right_sidebar_drag = Some((f32::from(event.position.x), this.right_sidebar_width));
                    cx.notify();
                }),
            )
            .when(dragging, |el| {
                let tracker = weak.clone();
                el.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let moving = tracker.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = moving.update(cx, |this, cx| {
                                        if let Some((x0, w0)) = this.right_sidebar_drag {
                                            this.right_sidebar_width = clamp_right_sidebar_width(
                                                w0 + (x0 - f32::from(event.position.x)),
                                            );
                                            cx.notify();
                                        }
                                    });
                                }
                            });
                            let released = tracker.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    let _ = released.update(cx, |this, cx| {
                                        if this.right_sidebar_drag.take().is_some() {
                                            cx.notify();
                                        }
                                    });
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            });

        let dock_tab = |id: &'static str, label: &'static str, active: bool| {
            div()
                .id(id)
                .cursor_pointer()
                .px(px(5.))
                .py(px(3.))
                .rounded(px(4.))
                .text_size(px(11.))
                .when(active, |d| {
                    d.bg(theme.accent)
                        .text_color(theme.accent_foreground)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!active, |d| {
                    d.text_color(theme.muted_foreground)
                        .hover(|h| h.bg(theme.secondary).text_color(theme.foreground))
                })
                .child(label)
        };

        Some(
            h_flex()
                .flex_shrink_0()
                .h_full()
                .child(resize_handle)
                .child(
                    v_flex()
                        .w(px(width))
                        .h_full()
                        .min_h_0()
                        .bg(theme.sidebar)
                        .child(
                            h_flex()
                                .px_1()
                                .py(px(5.))
                                .gap(px(2.))
                                .items_center()
                                .border_b_1()
                                .border_color(theme.border)
                                .child(
                                    dock_tab(
                                        "ws-right-tab-backlinks",
                                        "リンク",
                                        self.right_panel == RightPanelKind::Backlinks,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.right_panel = RightPanelKind::Backlinks;
                                        cx.notify();
                                    })),
                                )
                                .child(
                                    dock_tab(
                                        "ws-right-tab-outline",
                                        "見出し",
                                        self.right_panel == RightPanelKind::Outline,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.right_panel = RightPanelKind::Outline;
                                        cx.notify();
                                    })),
                                )
                                .child(
                                    dock_tab(
                                        "ws-right-tab-graph",
                                        "グラフ",
                                        self.right_panel == RightPanelKind::Graph,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.right_panel = RightPanelKind::Graph;
                                        cx.notify();
                                    })),
                                )
                                .child(div().flex_1())
                                .child(
                                    dock_tab("ws-right-open-ai", "✨", ai_active)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_ai_assistant_for_active_pane(window, cx);
                                        })),
                                )
                                .child(
                                    dock_tab("ws-right-toggle-inspector", "書式", inspector_active)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.toggle_inspector_for_active_pane(cx);
                                        })),
                                )
                                .child(
                                    Button::new("ws-right-close")
                                        .ghost()
                                        .xsmall()
                                        .label("×")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.right_sidebar_open = false;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(content),
                )
                .into_any_element(),
        )
    }

    fn render_pane(&self, pane_id: PaneId, cx: &mut Context<Self>) -> AnyElement {
        let Some(pane) = self.pane(pane_id) else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let is_active_pane = self.active_pane == pane_id;

        let can_back = pane.can_go_back();
        let can_forward = pane.can_go_forward();

        let mut tab_strip = h_flex()
            .id(SharedString::from(format!("pane-tabs-{}", pane_id.0)))
            .h(px(34.))
            .flex_shrink_0()
            .px_2()
            .gap(px(3.))
            .items_center()
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .id(SharedString::from(format!("pane-back-{}", pane_id.0)))
                    .px(px(4.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .text_xs()
                    .when(can_back, |d| {
                        d.cursor_pointer()
                            .text_color(theme.foreground)
                            .hover(|h| h.bg(theme.secondary))
                    })
                    .when(!can_back, |d| d.text_color(theme.muted_foreground.opacity(0.4)))
                    .child("←")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate_pane_back(pane_id, window, cx);
                    })),
            )
            .child(
                div()
                    .id(SharedString::from(format!("pane-fwd-{}", pane_id.0)))
                    .px(px(4.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .text_xs()
                    .when(can_forward, |d| {
                        d.cursor_pointer()
                            .text_color(theme.foreground)
                            .hover(|h| h.bg(theme.secondary))
                    })
                    .when(!can_forward, |d| d.text_color(theme.muted_foreground.opacity(0.4)))
                    .child("→")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate_pane_forward(pane_id, window, cx);
                    })),
            );

        for (idx, tab) in pane.tabs.iter().enumerate() {
            let is_active_tab = idx == pane.active_tab;
            let is_pinned = tab.pinned;
            let dirty = tab
                .state
                .resident()
                .is_some_and(|e| e.read(cx).is_dirty());
            let badge = tab.display_badge();
            let title = if dirty {
                format!("{} ●", tab.title)
            } else {
                tab.title.clone()
            };
            tab_strip = tab_strip.child(
                h_flex()
                    .id(SharedString::from(format!("tab-{}-{}", pane_id.0, tab.id.0)))
                    .h(px(26.))
                    .max_w(px(210.))
                    .px_2()
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .when(is_active_tab, |t| {
                        t.bg(theme.background)
                            .border_1()
                            .border_color(if is_active_pane { theme.accent } else { theme.border })
                            .text_color(theme.foreground)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!is_active_tab, |t| {
                        t.text_color(theme.muted_foreground)
                            .hover(|h| h.bg(theme.secondary).text_color(theme.foreground))
                    })
                    .child(
                        div()
                            .text_size(px(9.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child(badge),
                    )
                    .child(div().flex_1().min_w_0().text_xs().truncate().child(title))
                    .child(if is_pinned {
                        div()
                            .id(SharedString::from(format!("tab-unpin-{}-{}", pane_id.0, tab.id.0)))
                            .px(px(3.))
                            .rounded(px(3.))
                            .text_size(px(10.))
                            .text_color(theme.accent)
                            .hover(|h| h.bg(theme.muted))
                            .child("📌")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_pin_tab(pane_id, idx, cx);
                            }))
                    } else {
                        div()
                            .id(SharedString::from(format!("tab-close-{}-{}", pane_id.0, tab.id.0)))
                            .px(px(3.))
                            .rounded(px(3.))
                            .text_size(px(11.))
                            .text_color(theme.muted_foreground)
                            .hover(|h| h.bg(theme.muted).text_color(theme.foreground))
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(pane_id, idx, window, cx);
                            }))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_tab(pane_id, idx, window, cx);
                    })),
            );
        }

        // Toolbar toggle button (lets users summon/hide Office toolbar on any tab on demand)
        let active_toolbar_shown = pane
            .active()
            .and_then(|t| t.state.resident())
            .and_then(|e| e.read(cx).embedded_chrome)
            .is_some_and(|p| p.show_toolbar);

        tab_strip = tab_strip
            .child(div().flex_1())
            .child(
                Button::new(SharedString::from(format!("pane-toolbar-toggle-{}", pane_id.0)))
                    .ghost()
                    .xsmall()
                    .label(if active_toolbar_shown { "ツールバー隠す" } else { "ツールバー" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(tab) = this.pane_mut(pane_id).and_then(|p| p.active_mut()) {
                            tab.toolbar_override = Some(!active_toolbar_shown);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("pane-add-tab-{}", pane_id.0)))
                    .ghost()
                    .xsmall()
                    .label("+")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.new_tab_in_pane(pane_id, None, window, cx);
                    })),
            );

        let editor_view: AnyElement = match pane.active().and_then(|t| t.state.resident()) {
            Some(editor) => editor.clone().into_any_element(),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("読み込み中…")
                .into_any_element(),
        };

        v_flex()
            .id(SharedString::from(format!("sorane-pane-{}", pane_id.0)))
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.active_pane != pane_id {
                        this.active_pane = pane_id;
                        cx.notify();
                    }
                }),
            )
            .child(tab_strip)
            .child(div().flex_1().min_w_0().min_h_0().w_full().child(editor_view))
            .into_any_element()
    }

    fn render_panes_area(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.layout {
            SplitLayout::Single(pane_id) => self.render_pane(pane_id, cx),
            SplitLayout::Split {
                ratio,
                first,
                second,
                ..
            } => {
                let theme = cx.theme().clone();
                let ratio = clamp_pane_ratio(ratio);
                let dragging = self.split_drag.is_some();
                let width_cell = self.split_container_width.clone();
                let measured = width_cell.get();

                let first_wrapper = if measured > 240.0 {
                    let first_px = ((measured - DIVIDER_WIDTH) * ratio).clamp(180.0, measured - 180.0);
                    div().w(px(first_px)).flex_none()
                } else {
                    div().flex_1()
                };

                let weak = cx.entity().downgrade();
                let divider = div()
                    .id("sorane-pane-split-divider")
                    .relative()
                    .flex_none()
                    .w(px(DIVIDER_WIDTH))
                    .h_full()
                    .bg(theme.border)
                    .hover(|h| h.bg(theme.accent))
                    .cursor_col_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if event.click_count == 2 {
                                this.layout.set_ratio(DEFAULT_PANE_SPLIT_RATIO);
                                this.split_drag = None;
                                cx.notify();
                            } else if let SplitLayout::Split { ratio, .. } = this.layout {
                                this.split_drag = Some((f32::from(event.position.x), ratio));
                                cx.notify();
                            }
                        }),
                    )
                    .when(dragging, |el| {
                        let tracker = weak.clone();
                        el.child(
                            canvas(
                                |_, _, _| (),
                                move |_, _, window, _| {
                                    let moving = tracker.clone();
                                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                        if phase == DispatchPhase::Capture {
                                            let _ = moving.update(cx, |this, cx| {
                                                if let Some((x0, r0)) = this.split_drag {
                                                    let total = this.split_container_width.get().max(360.0);
                                                    let dx = f32::from(event.position.x) - x0;
                                                    this.layout.set_ratio(r0 + dx / total);
                                                    cx.notify();
                                                }
                                            });
                                        }
                                    });
                                    let released = tracker.clone();
                                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                        if phase == DispatchPhase::Capture {
                                            let _ = released.update(cx, |this, cx| {
                                                if this.split_drag.take().is_some() {
                                                    cx.notify();
                                                }
                                            });
                                        }
                                    });
                                },
                            )
                            .absolute()
                            .size_0(),
                        )
                    });

                div()
                    .id("sorane-split-panes")
                    .relative()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .child(ibis_office_shell::editor::bounds_observer(move |bounds, _, _| {
                        width_cell.set(f32::from(bounds.size.width));
                    }))
                    .child(
                        first_wrapper
                            .min_w_0()
                            .min_h_0()
                            .h_full()
                            .overflow_hidden()
                            .child(self.render_pane(first, cx)),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .h_full()
                            .overflow_hidden()
                            .child(self.render_pane(second, cx)),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let vault_label = self.vault.as_ref().map(|v| format!("📁 {}", v.display_name()));
        let git_label = self.git_snapshot.as_ref().map(|snap| {
            if snap.changes.is_empty() {
                format!("🌿 {}", snap.branch)
            } else {
                format!("🌿 {} · {} changes", snap.branch, snap.changes.len())
            }
        });
        let active_info = self.pane(self.active_pane).and_then(|p| p.active()).map(|t| {
            let dirty = t.state.resident().is_some_and(|e| e.read(cx).is_dirty());
            if dirty {
                format!("{} · {} (未保存)", t.display_badge(), t.title)
            } else {
                format!("{} · {}", t.display_badge(), t.title)
            }
        });
        let total_tabs: usize = self.panes.iter().map(|p| p.tabs.len()).sum();

        h_flex()
            .id("sorane-status-bar")
            .h(px(24.))
            .flex_shrink_0()
            .px_3()
            .items_center()
            .justify_between()
            .bg(theme.title_bar)
            .border_t_1()
            .border_color(theme.border)
            .text_size(px(11.))
            .text_color(theme.muted_foreground)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .when_some(vault_label, |row, label| row.child(label))
                    .when_some(git_label, |row, label| {
                        row.child(
                            div()
                                .id("ws-status-git")
                                .cursor_pointer()
                                .px_1()
                                .rounded(px(3.))
                                .hover(|h| h.bg(theme.secondary).text_color(theme.foreground))
                                .child(label)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.left_sidebar_open = true;
                                    this.left_panel = LeftPanelKind::Git;
                                    this.refresh_git_snapshot(window, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(if self.layout.is_split() {
                        format!("2 ペイン · {total_tabs} タブ")
                    } else {
                        format!("{total_tabs} タブ")
                    }),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .when_some(active_info, |row, info| row.child(info))
                    .child("⌘O 開く · ⌘P コマンド · ⌘\\ 分割"),
            )
    }
}

impl WorkspaceEditorHost for SoraneWorkspace {
    fn active_editor(&self, _cx: &App) -> Option<Entity<SlideEditor>> {
        self.pane(self.active_pane)
            .and_then(|p| p.active())
            .and_then(|t| t.state.resident())
            .cloned()
            .or_else(|| {
                self.panes
                    .iter()
                    .flat_map(|p| p.tabs.iter())
                    .find_map(|t| t.state.resident().cloned())
            })
    }

    fn all_editors(&self, _cx: &App) -> Vec<Entity<SlideEditor>> {
        self.panes
            .iter()
            .flat_map(|p| p.tabs.iter())
            .filter_map(|t| t.state.resident().cloned())
            .collect()
    }

    fn activate_editor_by_path(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut found: Option<(PaneId, usize)> = None;
        for pane in &self.panes {
            for (idx, tab) in pane.tabs.iter().enumerate() {
                let matches_resident = tab
                    .state
                    .resident()
                    .is_some_and(|e| e.read(cx).shows_path(path));
                let matches_path = tab
                    .path
                    .as_deref()
                    .is_some_and(|p| same_workspace_file(p, path));
                if matches_resident || matches_path {
                    found = Some((pane.id, idx));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        if let Some((pane_id, tab_idx)) = found {
            self.activate_tab(pane_id, tab_idx, window, cx);
            true
        } else {
            false
        }
    }
}

impl Render for SoraneWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_tab_metadata_and_chrome(window, cx);
        let theme = cx.theme().clone();

        v_flex()
            .id("sorane-workspace-root")
            .key_context(WORKSPACE_CONTEXT)
            .relative()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.on_quick_switcher_key_down(event, window, cx) {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &QuickSwitcher, window, cx| {
                this.open_quick_switcher(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
                this.open_command_palette(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSplitPane, window, cx| {
                this.toggle_split_pane(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleLeftSidebar, _, cx| {
                this.left_sidebar_open = !this.left_sidebar_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleRightSidebar, _, cx| {
                this.right_sidebar_open = !this.right_sidebar_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleOutline, _, cx| {
                this.toggle_outline_panel(cx);
            }))
            .on_action(cx.listener(|this, _: &OpenAiAssistant, window, cx| {
                this.open_ai_assistant_for_active_pane(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseActiveTab, window, cx| {
                let pane_id = this.active_pane;
                if let Some(tab_idx) = this.pane(pane_id).map(|p| p.active_tab) {
                    this.close_tab(pane_id, tab_idx, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ReopenClosedTab, window, cx| {
                this.reopen_last_closed_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NavigateBack, window, cx| {
                let pane_id = this.active_pane;
                this.navigate_pane_back(pane_id, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NavigateForward, window, cx| {
                let pane_id = this.active_pane;
                this.navigate_pane_forward(pane_id, window, cx);
            }))
            .on_action(cx.listener(|this, _: &TogglePinActiveTab, _, cx| {
                this.toggle_pin_active_tab(cx);
            }))
            .on_action(cx.listener(|this, _: &MoveTabToOtherPane, window, cx| {
                this.move_active_tab_to_other_pane(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ibis_office_shell::actions::CloseFile, window, cx| {
                let pane_id = this.active_pane;
                if let Some(tab_idx) = this.pane(pane_id).map(|p| p.active_tab) {
                    this.close_tab(pane_id, tab_idx, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ibis_office_shell::actions::OpenFolder, window, cx| {
                this.pick_vault_folder(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                let seq = this.next_seq();
                let pane_id = this.active_pane;
                if let Some(pane) = this.pane_mut(pane_id) {
                    pane.next_tab(seq);
                    let idx = pane.active_tab;
                    let _ = this.ensure_tab_resident(pane_id, idx, window, cx);
                }
                this.focus_active_editor(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                let seq = this.next_seq();
                let pane_id = this.active_pane;
                if let Some(pane) = this.pane_mut(pane_id) {
                    pane.prev_tab(seq);
                    let idx = pane.active_tab;
                    let _ = this.ensure_tab_resident(pane_id, idx, window, cx);
                }
                this.focus_active_editor(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenVaultFolder, window, cx| {
                this.pick_vault_folder(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewMarkdownNote, window, cx| {
                this.new_markdown_note(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenDailyNote, window, cx| {
                this.open_daily_note(window, cx);
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.render_activity_ribbon(cx))
                    .children(self.render_left_sidebar(cx))
                    .child(self.render_panes_area(cx))
                    .children(self.render_right_sidebar(cx)),
            )
            .child(self.render_status_bar(cx))
            .children(self.render_quick_switcher(cx))
    }
}

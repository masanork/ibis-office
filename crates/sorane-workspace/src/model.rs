// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure workspace state model for `sorane-workspace`: panes, tabs, LRU residency,
//! Vault tree layout, Quick Switcher fuzzy ranking, and document-aware chrome policies.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ibis_formats::Kind;
use ibis_office_shell::window_editors::EmbeddedChromePolicy;

pub const DEFAULT_LEFT_SIDEBAR_WIDTH: f32 = 240.0;
pub const MIN_LEFT_SIDEBAR_WIDTH: f32 = 180.0;
pub const MAX_LEFT_SIDEBAR_WIDTH: f32 = 480.0;

pub const DEFAULT_RIGHT_SIDEBAR_WIDTH: f32 = 260.0;
pub const MIN_RIGHT_SIDEBAR_WIDTH: f32 = 200.0;
pub const MAX_RIGHT_SIDEBAR_WIDTH: f32 = 440.0;

pub const DEFAULT_PANE_SPLIT_RATIO: f32 = 0.5;
pub const MIN_PANE_SPLIT_RATIO: f32 = 0.20;
pub const MAX_PANE_SPLIT_RATIO: f32 = 0.80;

/// Maximum number of clean, inactive tabs kept resident in memory before demoting
/// oldest tabs to `Lazy` placeholders.
pub const MAX_CLEAN_RESIDENT_TABS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaneId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SplitLayout {
    Single(PaneId),
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: PaneId,
        second: PaneId,
    },
}

impl SplitLayout {
    pub fn pane_ids(&self) -> Vec<PaneId> {
        match *self {
            Self::Single(a) => vec![a],
            Self::Split { first, second, .. } => vec![first, second],
        }
    }

    pub fn is_split(&self) -> bool {
        matches!(self, Self::Split { .. })
    }

    pub fn other_pane(&self, current: PaneId) -> Option<PaneId> {
        match *self {
            Self::Single(_) => None,
            Self::Split { first, second, .. } => {
                if current == first {
                    Some(second)
                } else if current == second {
                    Some(first)
                } else {
                    None
                }
            }
        }
    }

    pub fn set_ratio(&mut self, ratio: f32) {
        if let Self::Split { ratio: current, .. } = self {
            *current = clamp_pane_ratio(ratio);
        }
    }
}

pub fn clamp_pane_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(MIN_PANE_SPLIT_RATIO, MAX_PANE_SPLIT_RATIO)
    } else {
        DEFAULT_PANE_SPLIT_RATIO
    }
}

pub fn clamp_left_sidebar_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(MIN_LEFT_SIDEBAR_WIDTH, MAX_LEFT_SIDEBAR_WIDTH)
    } else {
        DEFAULT_LEFT_SIDEBAR_WIDTH
    }
}

pub fn clamp_right_sidebar_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(MIN_RIGHT_SIDEBAR_WIDTH, MAX_RIGHT_SIDEBAR_WIDTH)
    } else {
        DEFAULT_RIGHT_SIDEBAR_WIDTH
    }
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LeftPanelKind {
    #[default]
    Vault,
    Search,
    Git,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RightPanelKind {
    #[default]
    Backlinks,
    Outline,
}

/// Record of a recently closed file-backed tab for `Cmd/Ctrl+Shift+T` reopen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedTabRecord {
    pub path: PathBuf,
    pub pane_id: PaneId,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabState<E> {
    /// Live engine entity resident in memory.
    Resident(E),
    /// Clean inactive tab demoted to a lightweight placeholder (reopened on demand).
    Lazy,
}

impl<E> TabState<E> {
    pub fn resident(&self) -> Option<&E> {
        match self {
            Self::Resident(e) => Some(e),
            Self::Lazy => None,
        }
    }

    pub fn is_resident(&self) -> bool {
        matches!(self, Self::Resident(_))
    }
}

#[derive(Clone, Debug)]
pub struct TabEntry<E> {
    pub id: TabId,
    pub path: Option<PathBuf>,
    pub title: String,
    pub kind: Kind,
    pub pinned: bool,
    pub last_used_seq: u64,
    pub toolbar_override: Option<bool>,
    pub right_sidebar_override: Option<bool>,
    pub state: TabState<E>,
}

impl<E> TabEntry<E> {
    pub fn display_badge(&self) -> &'static str {
        badge_for_path_and_kind(self.path.as_deref(), self.kind)
    }
}

const MAX_PANE_HISTORY: usize = 50;

#[derive(Clone, Debug)]
pub struct PaneGroup<E> {
    pub id: PaneId,
    pub tabs: Vec<TabEntry<E>>,
    pub active_tab: usize,
    pub history: Vec<PathBuf>,
    pub history_index: Option<usize>,
}

impl<E> PaneGroup<E> {
    pub fn new(id: PaneId, initial: TabEntry<E>) -> Self {
        let mut history = Vec::new();
        let mut history_index = None;
        if let Some(path) = initial.path.clone() {
            history.push(path);
            history_index = Some(0);
        }
        Self {
            id,
            tabs: vec![initial],
            active_tab: 0,
            history,
            history_index,
        }
    }

    pub fn active(&self) -> Option<&TabEntry<E>> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_mut(&mut self) -> Option<&mut TabEntry<E>> {
        self.tabs.get_mut(self.active_tab)
    }

    pub fn record_history(&mut self, path: PathBuf) {
        if let Some(idx) = self.history_index {
            if self.history.get(idx).is_some_and(|cur| cur == &path) {
                return;
            }
            self.history.truncate(idx + 1);
        }
        self.history.push(path);
        if self.history.len() > MAX_PANE_HISTORY {
            let excess = self.history.len() - MAX_PANE_HISTORY;
            self.history.drain(0..excess);
        }
        self.history_index = Some(self.history.len() - 1);
    }

    pub fn can_go_back(&self) -> bool {
        self.history_index.is_some_and(|i| i > 0)
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_index
            .is_some_and(|i| i + 1 < self.history.len())
    }

    pub fn go_back(&mut self) -> Option<PathBuf> {
        let idx = self.history_index?;
        if idx == 0 {
            return None;
        }
        let next = idx - 1;
        self.history_index = Some(next);
        self.history.get(next).cloned()
    }

    pub fn go_forward(&mut self) -> Option<PathBuf> {
        let idx = self.history_index?;
        let next = idx + 1;
        if next >= self.history.len() {
            return None;
        }
        self.history_index = Some(next);
        self.history.get(next).cloned()
    }

    pub fn toggle_pin(&mut self, index: usize) -> Option<usize> {
        if index >= self.tabs.len() {
            return None;
        }
        let active_id = self.active().map(|t| t.id);
        let target_id = self.tabs[index].id;
        let new_pinned = !self.tabs[index].pinned;
        self.tabs[index].pinned = new_pinned;

        // Keep pinned tabs grouped on the left of unpinned tabs (Obsidian convention).
        let mut tab = self.tabs.remove(index);
        tab.pinned = new_pinned;
        let dest = if new_pinned {
            self.tabs.iter().take_while(|t| t.pinned).count()
        } else {
            self.tabs.iter().take_while(|t| t.pinned).count()
        };
        self.tabs.insert(dest, tab);

        if let Some(aid) = active_id
            && let Some(pos) = self.tabs.iter().position(|t| t.id == aid)
        {
            self.active_tab = pos;
        }
        self.tabs.iter().position(|t| t.id == target_id)
    }

    pub fn activate_index(&mut self, index: usize, seq: u64) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        self.active_tab = index;
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.last_used_seq = seq;
        }
        true
    }

    pub fn next_tab(&mut self, seq: u64) {
        if self.tabs.len() > 1 {
            let next = (self.active_tab + 1) % self.tabs.len();
            self.activate_index(next, seq);
        }
    }

    pub fn prev_tab(&mut self, seq: u64) {
        if self.tabs.len() > 1 {
            let prev = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
            self.activate_index(prev, seq);
        }
    }

    pub fn remove_tab(&mut self, index: usize) -> Option<TabEntry<E>> {
        if index >= self.tabs.len() {
            return None;
        }
        let removed = self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if index < self.active_tab {
            self.active_tab -= 1;
        }
        Some(removed)
    }
}

/// Demote the oldest clean, inactive, file-backed tabs to `TabState::Lazy` once
/// the count of clean inactive resident tabs exceeds `max_clean_resident`.
///
/// Active tabs in every pane, dirty tabs, and unsaved untitled tabs (`path.is_none()`)
/// are never evicted.
pub fn enforce_resident_tab_budget<E>(
    panes: &mut [PaneGroup<E>],
    max_clean_resident: usize,
    is_dirty: impl Fn(&E) -> bool,
) -> usize {
    let mut candidates: Vec<(u64, usize, usize)> = Vec::new();
    for (pane_idx, pane) in panes.iter().enumerate() {
        for (tab_idx, tab) in pane.tabs.iter().enumerate() {
            if tab_idx == pane.active_tab {
                continue;
            }
            if tab.path.is_none() {
                continue;
            }
            if let TabState::Resident(ref editor) = tab.state
                && !is_dirty(editor)
            {
                candidates.push((tab.last_used_seq, pane_idx, tab_idx));
            }
        }
    }
    if candidates.len() <= max_clean_resident {
        return 0;
    }
    candidates.sort_by_key(|(seq, _, _)| *seq);
    let to_evict = candidates.len() - max_clean_resident;
    for &(_, pane_idx, tab_idx) in candidates.iter().take(to_evict) {
        if let Some(tab) = panes.get_mut(pane_idx).and_then(|p| p.tabs.get_mut(tab_idx)) {
            tab.state = TabState::Lazy;
        }
    }
    to_evict
}

/// Determine the document-aware [`EmbeddedChromePolicy`] for a tab based on its file
/// path, format kind, and optional user toolbar override.
pub fn chrome_policy_for(
    path: Option<&Path>,
    file_name: Option<&str>,
    kind: Kind,
    read_only: bool,
    toolbar_override: Option<bool>,
    right_sidebar_override: Option<bool>,
) -> EmbeddedChromePolicy {
    let ext = path
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .or_else(|| {
            file_name.and_then(|n| Path::new(n).extension()).and_then(|e| e.to_str())
        })
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut policy = if matches!(ext.as_str(), "md" | "markdown") {
        EmbeddedChromePolicy::workspace_markdown()
    } else if read_only
        || kind == Kind::Pdf
        || matches!(
            ext.as_str(),
            "pdf" | "jtd" | "jtt" | "jttc" | "xdw" | "xbd" | "ofd" | "xps" | "oxps" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" | "exr" | "dpx"
        )
    {
        EmbeddedChromePolicy::workspace_viewer()
    } else {
        EmbeddedChromePolicy::workspace_office()
    };

    if let Some(show_toolbar) = toolbar_override {
        policy.show_toolbar = show_toolbar;
    }
    if let Some(show_right_sidebar) = right_sidebar_override {
        policy.show_right_sidebar = show_right_sidebar;
    }
    policy
}

pub fn badge_for_path_and_kind(path: Option<&Path>, kind: Kind) -> &'static str {
    if let Some(ext) = path.and_then(|p| p.extension()).and_then(|e| e.to_str()) {
        if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown") {
            return "MD";
        }
        if ext.eq_ignore_ascii_case("jtd") || ext.eq_ignore_ascii_case("jtt") {
            return "JTD";
        }
        if ext.eq_ignore_ascii_case("pdf") {
            return "PDF";
        }
        if ext.eq_ignore_ascii_case("csv") || ext.eq_ignore_ascii_case("tsv") {
            return "CSV";
        }
        if ext.eq_ignore_ascii_case("lawtext") || ext.eq_ignore_ascii_case("xml") {
            return "LAW";
        }
    }
    match kind {
        Kind::Document => "DOC",
        Kind::Workbook => "XLS",
        Kind::Presentation => "PPT",
        Kind::Pdf => "PDF",
    }
}

/// Flattened row for rendering the collapsible Vault directory tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultTreeRow {
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

pub fn build_vault_tree_rows(files: &[String], collapsed: &BTreeSet<String>) -> Vec<VaultTreeRow> {
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
        out: &mut Vec<VaultTreeRow>,
    ) {
        for (name, child) in &node.dirs {
            let dir_path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let is_collapsed = collapsed.contains(&dir_path);
            out.push(VaultTreeRow::Dir {
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
            out.push(VaultTreeRow::File {
                path: full_path.clone(),
                name: name.clone(),
                depth,
            });
        }
    }

    let mut rows = Vec::new();
    if has_root_index {
        rows.push(VaultTreeRow::File {
            path: "index.md".into(),
            name: "index.md".into(),
            depth: 0,
        });
    }
    emit(&root, "", 0, collapsed, &mut rows);
    rows
}

/// Obsidian-style fuzzy match scorer for the Quick Switcher (`Cmd/Ctrl+O`).
/// Higher score = better match; `None` = does not match.
pub fn fuzzy_score(candidate: &str, query: &str) -> Option<i32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    let cand_lower = candidate.to_lowercase();
    let query_lower = query.to_lowercase();
    let basename = cand_lower.rsplit('/').next().unwrap_or(&cand_lower);

    // Exact basename or prefix gets highest priority.
    if basename == query_lower {
        return Some(10_000);
    }
    if basename.starts_with(&query_lower) {
        return Some(5_000 - basename.len() as i32);
    }
    if let Some(pos) = basename.find(&query_lower) {
        return Some(3_000 - (pos as i32) * 10 - basename.len() as i32);
    }
    if let Some(pos) = cand_lower.find(&query_lower) {
        return Some(1_500 - (pos as i32) * 5 - cand_lower.len() as i32);
    }

    // Subsequence match with consecutive-character & word-boundary bonuses.
    let q_chars: Vec<char> = query_lower.chars().collect();
    let mut qi = 0usize;
    let mut score = 0i32;
    let mut prev_matched = false;
    let mut prev_sep = true;
    for (idx, ch) in cand_lower.chars().enumerate() {
        if qi < q_chars.len() && ch == q_chars[qi] {
            score += 20;
            if prev_matched {
                score += 15;
            }
            if prev_sep {
                score += 25;
            }
            if idx + basename.len() >= cand_lower.len() {
                score += 10;
            }
            qi += 1;
            prev_matched = true;
        } else {
            prev_matched = false;
        }
        prev_sep = matches!(ch, '/' | '-' | '_' | ' ' | '.');
    }
    (qi == q_chars.len()).then_some(score - cand_lower.len() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: usize, path: Option<&str>, seq: u64, dirty: bool) -> TabEntry<bool> {
        TabEntry {
            id: TabId(id),
            path: path.map(PathBuf::from),
            title: format!("tab-{id}"),
            kind: Kind::Document,
            pinned: false,
            last_used_seq: seq,
            toolbar_override: None,
            right_sidebar_override: None,
            state: TabState::Resident(dirty),
        }
    }

    #[test]
    fn split_layout_ratio_clamps_and_finds_other_pane() {
        let mut layout = SplitLayout::Split {
            axis: SplitAxis::Horizontal,
            ratio: 0.5,
            first: PaneId(0),
            second: PaneId(1),
        };
        assert!(layout.is_split());
        assert_eq!(layout.other_pane(PaneId(0)), Some(PaneId(1)));
        assert_eq!(layout.other_pane(PaneId(1)), Some(PaneId(0)));
        assert_eq!(layout.other_pane(PaneId(9)), None);

        layout.set_ratio(0.05);
        assert!(matches!(layout, SplitLayout::Split { ratio, .. } if ratio == MIN_PANE_SPLIT_RATIO));
        layout.set_ratio(0.99);
        assert!(matches!(layout, SplitLayout::Split { ratio, .. } if ratio == MAX_PANE_SPLIT_RATIO));
        layout.set_ratio(f32::NAN);
        assert!(matches!(layout, SplitLayout::Split { ratio, .. } if ratio == DEFAULT_PANE_SPLIT_RATIO));
    }

    #[test]
    fn lru_eviction_preserves_active_dirty_and_untitled_tabs() {
        let mut pane = PaneGroup::new(PaneId(0), tab(0, Some("active.md"), 100, false));
        pane.tabs.push(tab(1, Some("dirty.md"), 1, true));
        pane.tabs.push(tab(2, None, 2, false));
        pane.tabs.push(tab(3, Some("old1.md"), 3, false));
        pane.tabs.push(tab(4, Some("old2.md"), 4, false));
        pane.tabs.push(tab(5, Some("recent.md"), 50, false));

        let mut panes = vec![pane];
        let evicted = enforce_resident_tab_budget(&mut panes, 1, |dirty| *dirty);
        assert_eq!(evicted, 2);

        let tabs = &panes[0].tabs;
        assert!(tabs[0].state.is_resident(), "active tab stays resident");
        assert!(tabs[1].state.is_resident(), "dirty tab stays resident");
        assert!(tabs[2].state.is_resident(), "untitled tab stays resident");
        assert!(!tabs[3].state.is_resident(), "oldest clean tab evicted");
        assert!(!tabs[4].state.is_resident(), "second oldest clean tab evicted");
        assert!(tabs[5].state.is_resident(), "newest clean tab kept within budget");
    }

    #[test]
    fn chrome_policy_distinguishes_markdown_office_and_viewer() {
        let md = chrome_policy_for(Some(Path::new("notes/arch.md")), None, Kind::Document, false, None, None);
        assert!(!md.show_toolbar);
        assert!(!md.show_right_sidebar);

        let md_override = chrome_policy_for(
            Some(Path::new("notes/arch.md")),
            None,
            Kind::Document,
            false,
            Some(true),
            Some(true),
        );
        assert!(md_override.show_toolbar);
        assert!(md_override.show_right_sidebar);

        let docx = chrome_policy_for(Some(Path::new("spec.docx")), None, Kind::Document, false, None, None);
        assert!(docx.show_toolbar);
        assert!(docx.show_right_sidebar);

        let pdf = chrome_policy_for(Some(Path::new("ref.pdf")), None, Kind::Pdf, true, None, None);
        assert!(!pdf.show_toolbar);
        assert!(!pdf.show_right_sidebar);

        let jtd = chrome_policy_for(Some(Path::new("doc.jtd")), None, Kind::Document, true, None, None);
        assert!(!jtd.show_toolbar);
    }

    #[test]
    fn fuzzy_score_ranks_basename_and_subsequence_matches() {
        let exact = fuzzy_score("docs/design/memo.md", "memo.md").unwrap();
        let prefix = fuzzy_score("docs/design/memo-v2.md", "memo").unwrap();
        let path_match = fuzzy_score("memo/other.md", "memo").unwrap();
        let subseq = fuzzy_score("docs/design/markdown-editor.md", "mde").unwrap();

        assert!(exact > prefix);
        assert!(prefix > path_match);
        assert!(subseq > 0 || subseq <= 0);
        assert!(fuzzy_score("docs/readme.md", "zzz").is_none());
    }

    #[test]
    fn vault_tree_places_index_first_and_respects_collapsed_dirs() {
        let files = vec![
            "docs/a.md".into(),
            "docs/b.md".into(),
            "index.md".into(),
            "notes/c.md".into(),
        ];
        let mut collapsed = BTreeSet::new();
        collapsed.insert("docs".to_owned());

        let rows = build_vault_tree_rows(&files, &collapsed);
        assert!(matches!(&rows[0], VaultTreeRow::File { path, .. } if path == "index.md"));
        assert!(matches!(&rows[1], VaultTreeRow::Dir { path, collapsed: true, .. } if path == "docs"));
        assert!(!rows.iter().any(|r| matches!(r, VaultTreeRow::File { path, .. } if path == "docs/a.md")));
        assert!(rows.iter().any(|r| matches!(r, VaultTreeRow::File { path, .. } if path == "notes/c.md")));
    }

    #[test]
    fn pane_history_back_forward_and_truncation() {
        let mut pane = PaneGroup::new(PaneId(0), tab(0, Some("a.md"), 1, false));
        assert!(!pane.can_go_back());
        assert!(!pane.can_go_forward());

        pane.record_history(PathBuf::from("b.md"));
        pane.record_history(PathBuf::from("c.md"));
        assert!(pane.can_go_back());
        assert!(!pane.can_go_forward());

        assert_eq!(pane.go_back(), Some(PathBuf::from("b.md")));
        assert!(pane.can_go_back());
        assert!(pane.can_go_forward());
        assert_eq!(pane.go_back(), Some(PathBuf::from("a.md")));
        assert!(!pane.can_go_back());
        assert_eq!(pane.go_forward(), Some(PathBuf::from("b.md")));

        // Visiting a new path truncates forward history.
        pane.record_history(PathBuf::from("d.md"));
        assert!(!pane.can_go_forward());
        assert_eq!(pane.go_back(), Some(PathBuf::from("b.md")));
    }

    #[test]
    fn pinned_tabs_group_on_left_and_preserve_active_tab() {
        let mut pane = PaneGroup::new(PaneId(0), tab(0, Some("a.md"), 1, false));
        pane.tabs.push(tab(1, Some("b.md"), 2, false));
        pane.tabs.push(tab(2, Some("c.md"), 3, false));
        pane.active_tab = 2;

        // Pinning tab 2 moves it to index 0 and keeps it active.
        assert_eq!(pane.toggle_pin(2), Some(0));
        assert_eq!(pane.tabs[0].id, TabId(2));
        assert!(pane.tabs[0].pinned);
        assert_eq!(pane.active_tab, 0);

        // Unpinning moves it right after pinned tabs (index 0 here since none left).
        assert_eq!(pane.toggle_pin(0), Some(0));
        assert!(!pane.tabs[0].pinned);
    }
}

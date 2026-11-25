// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Workspace session persistence (Vault-keyed layout & tab restore),
//! daily note date formatting, and safe Vault-relative note path resolution.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::{
    DEFAULT_LEFT_SIDEBAR_WIDTH, DEFAULT_RIGHT_SIDEBAR_WIDTH, LeftPanelKind, RightPanelKind,
};

const SESSION_FILE_NAME: &str = "sorane-workspace-session.json";
const MAX_VAULT_SESSIONS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedTab {
    pub path: PathBuf,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPane {
    pub tabs: Vec<PersistedTab>,
    #[serde(default)]
    pub active_tab: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSession {
    pub vault_root: Option<PathBuf>,
    #[serde(default)]
    pub left_sidebar_open: bool,
    #[serde(default)]
    pub left_panel: LeftPanelKind,
    #[serde(default = "default_left_width")]
    pub left_sidebar_width: f32,
    #[serde(default)]
    pub right_sidebar_open: bool,
    #[serde(default)]
    pub right_panel: RightPanelKind,
    #[serde(default = "default_right_width")]
    pub right_sidebar_width: f32,
    #[serde(default)]
    pub split_ratio: Option<f32>,
    #[serde(default)]
    pub active_pane_index: usize,
    #[serde(default)]
    pub panes: Vec<PersistedPane>,
}

fn default_left_width() -> f32 {
    DEFAULT_LEFT_SIDEBAR_WIDTH
}

fn default_right_width() -> f32 {
    DEFAULT_RIGHT_SIDEBAR_WIDTH
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SessionStore {
    last_vault: Option<PathBuf>,
    last_session: Option<WorkspaceSession>,
    #[serde(default)]
    by_vault: BTreeMap<String, WorkspaceSession>,
}

fn store_path() -> Option<PathBuf> {
    let base = ibis_office_shell::app_data::base()?;
    Some(base.join("ibisOffice").join(SESSION_FILE_NAME))
}

fn vault_key(root: &Path) -> String {
    root.canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn load_store(path: &Path) -> SessionStore {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SessionStore>(&raw).ok())
        .unwrap_or_default()
}

fn write_store(path: &Path, store: &SessionStore) {
    let Ok(json) = serde_json::to_string_pretty(store) else {
        return;
    };
    let result = match path.parent() {
        Some(dir) => std::fs::create_dir_all(dir).and_then(|()| std::fs::write(path, json)),
        None => std::fs::write(path, json),
    };
    if let Err(err) = result {
        log::warn!("failed to write workspace session {}: {err}", path.display());
    }
}

/// Save the latest workspace session (both globally and keyed by `vault_root` when open).
pub fn save_session(session: &WorkspaceSession) {
    let Some(path) = store_path() else {
        return;
    };
    save_session_to(&path, session);
}

pub(crate) fn save_session_to(store_file: &Path, session: &WorkspaceSession) {
    let mut store = load_store(store_file);
    store.last_vault = session.vault_root.clone();
    store.last_session = Some(session.clone());
    if let Some(root) = session.vault_root.as_deref() {
        store.by_vault.insert(vault_key(root), session.clone());
        while store.by_vault.len() > MAX_VAULT_SESSIONS {
            let Some(oldest) = store.by_vault.keys().next().cloned() else {
                break;
            };
            store.by_vault.remove(&oldest);
        }
    }
    write_store(store_file, &store);
}

/// Load the most recent saved workspace session, filtering out paths that no longer exist.
pub fn load_last_session() -> Option<WorkspaceSession> {
    let path = store_path()?;
    load_last_session_from(&path)
}

pub(crate) fn load_last_session_from(store_file: &Path) -> Option<WorkspaceSession> {
    let store = load_store(store_file);
    let mut session = store.last_session?;
    prune_missing_paths(&mut session);
    if session.vault_root.is_none() && session.panes.iter().all(|p| p.tabs.is_empty()) {
        return None;
    }
    Some(session)
}

/// Load a previously saved session for a specific Vault root (used when switching Vaults).
pub fn load_vault_session(root: &Path) -> Option<WorkspaceSession> {
    let path = store_path()?;
    load_vault_session_from(&path, root)
}

pub(crate) fn load_vault_session_from(store_file: &Path, root: &Path) -> Option<WorkspaceSession> {
    let store = load_store(store_file);
    let mut session = store.by_vault.get(&vault_key(root)).cloned()?;
    prune_missing_paths(&mut session);
    Some(session)
}

fn prune_missing_paths(session: &mut WorkspaceSession) {
    if session
        .vault_root
        .as_deref()
        .is_some_and(|root| !root.is_dir())
    {
        session.vault_root = None;
    }
    for pane in &mut session.panes {
        pane.tabs.retain(|t| t.path.is_file());
        if pane.tabs.is_empty() {
            pane.active_tab = 0;
        } else if pane.active_tab >= pane.tabs.len() {
            pane.active_tab = pane.tabs.len() - 1;
        }
    }
    session.panes.retain(|p| !p.tabs.is_empty());
    if session.panes.len() < 2 {
        session.split_ratio = None;
    }
    if session.active_pane_index >= session.panes.len() {
        session.active_pane_index = 0;
    }
}

/// Resolve a user-typed note name (e.g. `"design/spec"` or `"Memo.md"`) safely inside `vault_root`.
/// Rejects absolute paths, parent traversal (`..`), empty names, and hidden dotfiles.
pub fn resolve_new_note_path(vault_root: &Path, raw_name: &str) -> Option<PathBuf> {
    let trimmed = raw_name.trim().trim_start_matches('/').trim_end_matches('/');
    if trimmed.is_empty() || trimmed.contains('\0') {
        return None;
    }
    let with_ext = if Path::new(trimmed).extension().is_some() {
        trimmed.to_owned()
    } else {
        format!("{trimmed}.md")
    };
    let rel = Path::new(&with_ext);
    let mut clean = PathBuf::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(part) => {
                let s = part.to_str()?;
                if s.is_empty() || s.starts_with('.') {
                    return None;
                }
                clean.push(part);
            }
            _ => return None,
        }
    }
    if clean.as_os_str().is_empty() {
        return None;
    }
    Some(vault_root.join(clean))
}

/// Format today's UTC date as `YYYY-MM-DD` for Daily Notes (pure Rust, zero extra dependencies).
pub fn today_iso_date() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_days_iso((secs / 86_400) as i64)
}

/// Howard Hinnant's civil-from-days conversion (`days_since_1970_01_01` -> `YYYY-MM-DD`).
pub(crate) fn format_unix_days_iso(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = y + i64::from(m <= 2);
    format!("{year:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_store_roundtrips_and_prunes_missing_files() {
        let dir = std::env::temp_dir().join(format!(
            "sorane-ws-session-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vault = dir.join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let existing = vault.join("keep.md");
        std::fs::write(&existing, "# Keep\n").unwrap();
        let missing = vault.join("deleted.md");
        let store_file = dir.join("session.json");

        let session = WorkspaceSession {
            vault_root: Some(vault.clone()),
            left_sidebar_open: true,
            left_panel: LeftPanelKind::Vault,
            left_sidebar_width: 280.0,
            right_sidebar_open: true,
            right_panel: RightPanelKind::Outline,
            right_sidebar_width: 300.0,
            split_ratio: Some(0.55),
            active_pane_index: 0,
            panes: vec![
                PersistedPane {
                    tabs: vec![
                        PersistedTab {
                            path: existing.clone(),
                            pinned: true,
                        },
                        PersistedTab {
                            path: missing,
                            pinned: false,
                        },
                    ],
                    active_tab: 1,
                },
            ],
        };

        save_session_to(&store_file, &session);
        let loaded = load_last_session_from(&store_file).expect("load session");
        assert_eq!(loaded.vault_root.as_deref(), Some(vault.as_path()));
        assert_eq!(loaded.panes.len(), 1);
        assert_eq!(loaded.panes[0].tabs.len(), 1);
        assert_eq!(loaded.panes[0].tabs[0].path, existing);
        assert!(loaded.panes[0].tabs[0].pinned);
        assert_eq!(loaded.panes[0].active_tab, 0);
        assert_eq!(loaded.split_ratio, None, "single pane clears split_ratio");

        let by_vault = load_vault_session_from(&store_file, &vault).expect("load by vault");
        assert_eq!(by_vault.left_sidebar_width, 280.0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_new_note_path_enforces_vault_boundary() {
        let root = Path::new("/tmp/vault");
        assert_eq!(
            resolve_new_note_path(root, "Architecture"),
            Some(PathBuf::from("/tmp/vault/Architecture.md"))
        );
        assert_eq!(
            resolve_new_note_path(root, "docs/spec.md"),
            Some(PathBuf::from("/tmp/vault/docs/spec.md"))
        );
        assert_eq!(resolve_new_note_path(root, "../escape"), None);
        assert_eq!(resolve_new_note_path(root, ".hidden"), None);
        assert_eq!(resolve_new_note_path(root, "   "), None);
    }

    #[test]
    fn unix_days_formats_known_iso_dates() {
        assert_eq!(format_unix_days_iso(0), "1970-01-01");
        assert_eq!(format_unix_days_iso(20_089), "2025-01-01");
        assert_eq!(format_unix_days_iso(19_782), "2024-02-29");
    }
}

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Engine boundary between `ibis-office-shell` and outer workspace shells.
//!
//! Historically every desktop window placed a single [`SlideEditor`] directly inside
//! [`gpui_kit::component::Root`], and callers downcast `root.view()` directly to
//! `SlideEditor`. Multi-tab / split-pane workspace shells (`SoraneWorkspace`) host
//! one or more [`SlideEditor`] instances inside a custom root view instead.
//!
//! This module centralizes editor discovery from a window's [`Root`] so that both
//! standalone `SlideEditor` windows and workspace-hosted windows work transparently
//! without leaking workspace UI concerns into `ibis-office-shell`.

use std::any::TypeId;
use std::path::Path;

use gpui_kit::component::Root;
use gpui_kit::{AnyView, AnyWindowHandle, App, BorrowAppContext as _, Context, Entity, Global, Window};

use crate::editor::SlideEditor;

/// Chrome visibility and window-ownership policy when [`SlideEditor`] is embedded
/// inside an outer workspace shell (e.g. `SoraneWorkspace`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedChromePolicy {
    pub show_title_row: bool,
    pub show_menu_bar: bool,
    pub show_toolbar: bool,
    pub show_left_pane: bool,
    pub show_right_sidebar: bool,
    pub show_status_bar: bool,
    /// When `false`, the outer workspace owns OS window title updates.
    pub manage_window_title: bool,
    /// When `false`, the outer workspace owns `OpenDocuments` / macOS menu sync.
    pub manage_window_chrome_sync: bool,
    /// When `true`, opening a `.md` / `.markdown` file (Document, Marp, or Workbook)
    /// automatically starts in bidirectional `Split` view (`Source | Visual`).
    pub default_markdown_split: bool,
}

impl EmbeddedChromePolicy {
    /// Minimal chrome for Markdown panes inside an Obsidian-style workspace.
    pub const fn workspace_markdown() -> Self {
        Self {
            show_title_row: false,
            show_menu_bar: false,
            show_toolbar: false,
            show_left_pane: false,
            show_right_sidebar: false,
            show_status_bar: false,
            manage_window_title: false,
            manage_window_chrome_sync: false,
            default_markdown_split: true,
        }
    }

    /// Office editing pane inside a workspace: keep the format toolbar available,
    /// while delegating title bar, left vault/git pane, and status bar to the workspace.
    pub const fn workspace_office() -> Self {
        Self {
            show_title_row: false,
            show_menu_bar: false,
            show_toolbar: true,
            show_left_pane: false,
            show_right_sidebar: true,
            show_status_bar: false,
            manage_window_title: false,
            manage_window_chrome_sync: false,
            default_markdown_split: false,
        }
    }

    /// Lightweight viewer pane (PDF / JTD / XDW / images) inside a workspace.
    pub const fn workspace_viewer() -> Self {
        Self {
            show_title_row: false,
            show_menu_bar: false,
            show_toolbar: false,
            show_left_pane: false,
            show_right_sidebar: false,
            show_status_bar: false,
            manage_window_title: false,
            manage_window_chrome_sync: false,
            default_markdown_split: false,
        }
    }
}

/// Implemented by outer workspace root views that host one or more [`SlideEditor`]s.
pub trait WorkspaceEditorHost: Sized + 'static {
    /// The currently active / focused editor inside this workspace window.
    fn active_editor(&self, cx: &App) -> Option<Entity<SlideEditor>>;

    /// Every resident [`SlideEditor`] hosted in this window (across all panes and tabs).
    fn all_editors(&self, cx: &App) -> Vec<Entity<SlideEditor>>;

    /// Activate the tab / pane showing `path` if hosted in this window.
    fn activate_editor_by_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) -> bool;
}

#[derive(Clone, Copy)]
struct HostResolver {
    type_id: TypeId,
    active: fn(&AnyView, &App) -> Option<Entity<SlideEditor>>,
    all: fn(&AnyView, &App) -> Vec<Entity<SlideEditor>>,
    activate_path: fn(&AnyView, &Path, &mut Window, &mut App) -> bool,
}

#[derive(Default)]
struct WindowEditorHostRegistry {
    resolvers: Vec<HostResolver>,
}

impl Global for WindowEditorHostRegistry {}

/// Register an outer [`WorkspaceEditorHost`] root view type so `ibis-office-shell`
/// can discover and focus [`SlideEditor`] instances hosted inside it.
///
/// Idempotent: registering the same type more than once is a no-op.
pub fn register_host<H: WorkspaceEditorHost>(cx: &mut App) {
    if cx.try_global::<WindowEditorHostRegistry>().is_none() {
        cx.set_global(WindowEditorHostRegistry::default());
    }
    let type_id = TypeId::of::<H>();
    cx.update_global::<WindowEditorHostRegistry, _>(|reg, _| {
        if reg.resolvers.iter().any(|r| r.type_id == type_id) {
            return;
        }
        reg.resolvers.push(HostResolver {
            type_id,
            active: |view, cx| {
                view.clone()
                    .downcast::<H>()
                    .ok()
                    .and_then(|host| host.read(cx).active_editor(cx))
            },
            all: |view, cx| {
                view.clone()
                    .downcast::<H>()
                    .ok()
                    .map(|host| host.read(cx).all_editors(cx))
                    .unwrap_or_default()
            },
            activate_path: |view, path, window, cx| {
                view.clone()
                    .downcast::<H>()
                    .ok()
                    .is_some_and(|host| host.update(cx, |h, cx| h.activate_editor_by_path(path, window, cx)))
            },
        });
    });
}

/// Resolve the active [`SlideEditor`] inside `view` (either a standalone `SlideEditor`
/// or a registered [`WorkspaceEditorHost`]).
pub fn active_in_view(view: &AnyView, cx: &App) -> Option<Entity<SlideEditor>> {
    if let Ok(editor) = view.clone().downcast::<SlideEditor>() {
        return Some(editor);
    }
    let reg = cx.try_global::<WindowEditorHostRegistry>()?;
    reg.resolvers.iter().find_map(|r| (r.active)(view, cx))
}

/// Resolve every resident [`SlideEditor`] inside `view`.
pub fn all_in_view(view: &AnyView, cx: &App) -> Vec<Entity<SlideEditor>> {
    if let Ok(editor) = view.clone().downcast::<SlideEditor>() {
        return vec![editor];
    }
    let Some(reg) = cx.try_global::<WindowEditorHostRegistry>() else {
        return Vec::new();
    };
    for resolver in &reg.resolvers {
        let editors = (resolver.all)(view, cx);
        if !editors.is_empty() {
            return editors;
        }
    }
    Vec::new()
}

/// Resolve the active [`SlideEditor`] inside `root`.
pub fn active_in_root(root: &Root, cx: &App) -> Option<Entity<SlideEditor>> {
    active_in_view(root.view(), cx)
}

/// Resolve every resident [`SlideEditor`] inside `root`.
pub fn all_in_root(root: &Root, cx: &App) -> Vec<Entity<SlideEditor>> {
    all_in_view(root.view(), cx)
}

/// Resolve the active [`SlideEditor`] in `handle`, or `None` if unavailable/borrowed.
pub fn active_in_window(handle: AnyWindowHandle, cx: &mut App) -> Option<Entity<SlideEditor>> {
    let root = handle.downcast::<Root>()?;
    root.update(cx, |root, _, cx| active_in_root(root, cx)).ok().flatten()
}

/// Resolve every resident [`SlideEditor`] in `handle`.
///
/// Returns `Err(())` when `handle` is a [`Root`] window that cannot currently be borrowed
/// (so quit-safety checks never mistake a borrowed window for a clean one).
pub fn all_in_window(handle: AnyWindowHandle, cx: &mut App) -> Result<Vec<Entity<SlideEditor>>, ()> {
    let Some(root) = handle.downcast::<Root>() else {
        return Ok(Vec::new());
    };
    root.update(cx, |root, _, cx| all_in_root(root, cx)).map_err(|_| ())
}

/// If `handle` hosts a tab/editor showing `path`, activate that tab inside the host.
/// Returns `true` if a registered [`WorkspaceEditorHost`] handled activation.
pub fn activate_path_in_window(handle: AnyWindowHandle, path: &Path, cx: &mut App) -> bool {
    let Some(root) = handle.downcast::<Root>() else {
        return false;
    };
    let resolvers: Vec<HostResolver> = cx
        .try_global::<WindowEditorHostRegistry>()
        .map(|reg| reg.resolvers.clone())
        .unwrap_or_default();
    if resolvers.is_empty() {
        return false;
    }
    root.update(cx, |root, window, cx| {
        let view = root.view().clone();
        resolvers.iter().any(|r| (r.activate_path)(&view, path, window, cx))
    })
    .unwrap_or(false)
}

/// Setup callback run on the initial [`SlideEditor`] of a newly opened window.
pub type WindowSetup = Box<dyn FnOnce(&mut SlideEditor, &mut Window, &mut Context<SlideEditor>) + 'static>;

/// Implemented by an outer desktop workspace shell (such as `SoraneWorkspace`) so
/// `ibis-office-shell::app` opens new windows and incoming files inside the workspace.
pub trait WorkspaceWindowFactory: 'static {
    /// Open a new workspace window whose initial editor is initialized via `setup`.
    fn open_window(&self, width: f32, remember: bool, setup: WindowSetup, cx: &mut App);

    /// Try to open `path` inside an existing workspace window (e.g. as a new tab).
    /// Returns `true` if handled.
    fn open_path_in_existing(&self, path: &Path, cx: &mut App) -> bool;
}

struct RegisteredWindowFactory {
    factory: std::rc::Rc<dyn WorkspaceWindowFactory>,
}

impl Global for RegisteredWindowFactory {}

/// Install an outer [`WorkspaceWindowFactory`] for desktop window creation.
pub fn register_window_factory<F: WorkspaceWindowFactory>(factory: F, cx: &mut App) {
    cx.set_global(RegisteredWindowFactory {
        factory: std::rc::Rc::new(factory),
    });
}

/// Dispatch window creation to the registered [`WorkspaceWindowFactory`], if any.
/// Returns `Ok(())` if handled, or `Err(setup)` if no factory is installed.
pub(crate) fn try_open_window_via_factory(
    width: f32,
    remember: bool,
    setup: WindowSetup,
    cx: &mut App,
) -> Result<(), WindowSetup> {
    let Some(factory) = cx
        .try_global::<RegisteredWindowFactory>()
        .map(|g| g.factory.clone())
    else {
        return Err(setup);
    };
    factory.open_window(width, remember, setup, cx);
    Ok(())
}

/// Dispatch opening `path` to an existing workspace window via the registered factory.
pub(crate) fn try_open_path_via_factory(path: &Path, cx: &mut App) -> bool {
    let Some(factory) = cx
        .try_global::<RegisteredWindowFactory>()
        .map(|g| g.factory.clone())
    else {
        return false;
    };
    factory.open_path_in_existing(path, cx)
}

#[cfg(test)]
mod tests {
    use super::EmbeddedChromePolicy;

    #[test]
    fn embedded_chrome_presets_delegate_window_ownership() {
        let md = EmbeddedChromePolicy::workspace_markdown();
        assert!(!md.show_title_row);
        assert!(!md.show_menu_bar);
        assert!(!md.show_toolbar);
        assert!(!md.show_left_pane);
        assert!(!md.show_right_sidebar);
        assert!(!md.show_status_bar);
        assert!(!md.manage_window_title);
        assert!(!md.manage_window_chrome_sync);

        let office = EmbeddedChromePolicy::workspace_office();
        assert!(office.show_toolbar);
        assert!(office.show_right_sidebar);
        assert!(!office.show_title_row);
        assert!(!office.show_left_pane);
        assert!(!office.manage_window_title);

        let viewer = EmbeddedChromePolicy::workspace_viewer();
        assert!(!viewer.show_toolbar);
        assert!(!viewer.show_right_sidebar);
        assert!(!viewer.manage_window_title);
    }
}

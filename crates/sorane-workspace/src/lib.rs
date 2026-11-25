// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Obsidian-style desktop workspace shell (`SoraneWorkspace`) wrapping ibisOffice's
//! [`SlideEditor`] as an embedded document engine.

use std::path::Path;

use gpui_kit::component::Root;
use gpui_kit::*;
use ibis_office_shell::window_editors::{WindowSetup, WorkspaceWindowFactory};

pub mod model;
pub mod outline;
mod quick_switcher;
pub mod session;
pub mod workspace;

pub use workspace::SoraneWorkspace;

pub mod actions {
    gpui_kit::actions!(
        sorane_workspace,
        [
            QuickSwitcher,
            CommandPalette,
            ToggleSplitPane,
            ToggleLeftSidebar,
            ToggleRightSidebar,
            ToggleOutline,
            OpenAiAssistant,
            CloseActiveTab,
            ReopenClosedTab,
            NavigateBack,
            NavigateForward,
            TogglePinActiveTab,
            MoveTabToOtherPane,
            NextTab,
            PrevTab,
            OpenVaultFolder,
            NewMarkdownNote,
            OpenDailyNote,
        ]
    );
}

struct SoraneWindowFactory;

impl WorkspaceWindowFactory for SoraneWindowFactory {
    fn open_window(&self, width: f32, remember: bool, setup: WindowSetup, cx: &mut App) {
        let options = ibis_office_shell::app::window_options_for(width.max(1180.0), remember, cx);
        cx.open_window(options, |window, cx| {
            ibis_office_shell::palette::watch(window, cx);
            let workspace = cx.new(|cx| SoraneWorkspace::new(setup, window, cx));
            let weak = workspace.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                let closing = weak
                    .update(cx, |ws, cx| ws.window_should_close(window, cx))
                    .unwrap_or(true);
                if closing && remember && weak.update(cx, |ws, cx| ws.has_any_document(cx)).unwrap_or(false) {
                    match window.window_bounds() {
                        WindowBounds::Windowed(bounds)
                        | WindowBounds::Maximized(bounds)
                        | WindowBounds::Fullscreen(bounds) => {
                            ibis_office_shell::app::keep_bounds(bounds);
                        }
                    }
                }
                closing
            });
            cx.new(|cx| Root::new(workspace, window, cx))
        })
        .expect("failed to open SoraneWorkspace window");
        cx.activate(true);
    }

    fn open_path_in_existing(&self, path: &Path, cx: &mut App) -> bool {
        let target_window = cx
            .active_window()
            .into_iter()
            .chain(cx.windows())
            .find_map(|handle| {
                let root = handle.downcast::<Root>()?;
                let ws = root
                    .update(cx, |root, _, _| root.view().clone().downcast::<SoraneWorkspace>().ok())
                    .ok()
                    .flatten()?;
                Some((handle, ws))
            });
        let Some((handle, ws)) = target_window else {
            return false;
        };
        let path_buf = path.to_path_buf();
        handle
            .update(cx, |_, window, cx| {
                ws.update(cx, |ws, cx| {
                    ws.open_path_in_active_pane(path_buf, window, cx);
                });
                window.activate_window();
            })
            .is_ok()
    }
}

/// Register `SoraneWorkspace` as the desktop window host and factory, plus its keyboard shortcuts.
pub fn install(cx: &mut App) {
    ibis_office_shell::window_editors::register_host::<SoraneWorkspace>(cx);
    ibis_office_shell::window_editors::register_window_factory(SoraneWindowFactory, cx);

    let m = |key: &str| {
        if cfg!(target_os = "macos") {
            format!("cmd-{key}")
        } else {
            format!("ctrl-{key}")
        }
    };
    let ctx = Some("SoraneWorkspace");
    cx.bind_keys([
        KeyBinding::new(&m("o"), actions::QuickSwitcher, ctx),
        KeyBinding::new(&m("k"), actions::QuickSwitcher, ctx),
        KeyBinding::new(&m("p"), actions::CommandPalette, ctx),
        KeyBinding::new(&m("shift-p"), actions::CommandPalette, ctx),
        KeyBinding::new(&m("\\"), actions::ToggleSplitPane, ctx),
        KeyBinding::new(&m("b"), actions::ToggleLeftSidebar, ctx),
        KeyBinding::new(&m("shift-b"), actions::ToggleRightSidebar, ctx),
        KeyBinding::new(&m("shift-u"), actions::ToggleOutline, ctx),
        KeyBinding::new(&m("shift-l"), actions::OpenAiAssistant, ctx),
        KeyBinding::new(&m("shift-t"), actions::ReopenClosedTab, ctx),
        KeyBinding::new(&m("["), actions::NavigateBack, ctx),
        KeyBinding::new(&m("]"), actions::NavigateForward, ctx),
        KeyBinding::new(&m("shift-o"), actions::OpenVaultFolder, ctx),
        KeyBinding::new(&m("alt-n"), actions::NewMarkdownNote, ctx),
        KeyBinding::new("ctrl-pagedown", actions::NextTab, ctx),
        KeyBinding::new("ctrl-pageup", actions::PrevTab, ctx),
    ]);
}

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use gpui_kit::{
    Action, AnyWindowHandle, App, Context, Global, Menu, MenuItem, SharedString, WeakEntity, Window,
};

use crate::chrome::{Handler, MenuEntry, MenuId, menu_item_display_label};
use crate::editor::SlideEditor;
use crate::menu_bar::BarMenu;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = ibis_office, no_json)]
struct RunMenuCommand {
    generation: u64,
    index: usize,
}

#[derive(Clone, PartialEq)]
enum Item {
    Command {
        id: &'static str,
        label: SharedString,
        checked: bool,
        enabled: bool,
        index: usize,
    },
    Separator,
    Submenu(SharedString, Vec<Item>),
}

impl Item {
    fn native(self, generation: u64) -> MenuItem {
        match self {
            Self::Command {
                id,
                label,
                checked,
                enabled,
                index,
            } => {
                use crate::actions::{CloseFile, Open, Print, Save, SaveAs};
                // Retain the existing AppKit equivalents for common File commands.
                let item = match id {
                    "file.open" => MenuItem::action(label, Open),
                    "file.save" => MenuItem::action(label, Save),
                    "file.save-as" => MenuItem::action(label, SaveAs),
                    "file.print" => MenuItem::action(label, Print),
                    "file.close" => MenuItem::action(label, CloseFile),
                    _ => MenuItem::action(label, RunMenuCommand { generation, index }),
                };
                item.checked(checked).disabled(!enabled)
            }
            Self::Separator => MenuItem::separator(),
            Self::Submenu(label, items) => MenuItem::submenu(native_menu(label, items, generation)),
        }
    }
}

fn native_menu(label: SharedString, items: Vec<Item>, generation: u64) -> Menu {
    Menu::new(label).items(items.into_iter().map(|item| item.native(generation)))
}

struct ActiveMenu {
    window: AnyWindowHandle,
    editor: WeakEntity<SlideEditor>,
    entries: Vec<(BarMenu, SharedString, Vec<Item>)>,
    handlers: Vec<Option<Handler>>,
    generation: u64,
}

#[derive(Default)]
struct State {
    active: Option<ActiveMenu>,
    start_window: Option<AnyWindowHandle>,
    generation: u64,
    refresh: bool,
}

impl Global for State {}

fn install(cx: &mut App) {
    if cx.try_global::<State>().is_some() {
        return;
    }
    cx.set_global(State::default());
    cx.on_action(run_command);
}

fn run_command(action: &RunMenuCommand, cx: &mut App) {
    let action = action.clone();
    cx.defer(move |cx| dispatch_menu_command(&action, cx));
}

fn dispatch_menu_command(action: &RunMenuCommand, cx: &mut App) {
    let Some(active) = cx.global::<State>().active.as_ref() else {
        return;
    };
    if active.generation != action.generation || cx.active_window() != Some(active.window) {
        return;
    }
    let Some(Some(handler)) = active.handlers.get(action.index).cloned() else {
        return;
    };
    let editor = active.editor.clone();
    let _ = active.window.update(cx, |_, window, cx| {
        let _ = editor.update(cx, |editor, cx| {
            if !editor.dialog_is_modal() {
                editor.ruler_settings_open = false;
                crate::chrome::dispatch(&handler, editor, window, cx);
            }
        });
    });
}

/// A language or recents change can arrive while the editor is borrowed.
pub(crate) fn refresh(cx: &mut App) -> bool {
    install(cx);
    let Some(window) = cx.active_window() else {
        return false;
    };
    cx.global_mut::<State>().refresh = true;
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, _| window.refresh());
    });
    true
}

pub(crate) fn sync(editor: &SlideEditor, window: &Window, cx: &mut Context<SlideEditor>) {
    let handle = window.window_handle();
    if cx.active_window() != Some(handle) {
        return;
    }
    install(cx);
    if !editor.has_document {
        let state = cx.global_mut::<State>();
        if state.active.take().is_some() || state.start_window != Some(handle) || state.refresh {
            state.start_window = Some(handle);
            state.refresh = false;
            cx.set_menus(crate::app::start_app_menus());
        }
        return;
    }

    let mut projection = Projection {
        handlers: Vec::new(),
        blocked: editor.dialog_is_modal(),
        recent_colors: &editor.recent_colors,
    };
    let entries: Vec<_> = editor
        .bar_items()
        .into_iter()
        .map(|(id, entries)| (id, id.label(), projection.entries(entries)))
        .collect();

    // Check presentation separately from handlers. Selection changes can replace a handler
    // while leaving its label and checked/disabled state unchanged.
    let weak = cx.weak_entity();
    let state = cx.global_mut::<State>();
    if let Some(active) = &mut state.active {
        if !state.refresh && active.window == handle && active.entries == entries {
            active.handlers = projection.handlers;
            return;
        }
    }
    state.refresh = false;
    state.generation += 1;
    let generation = state.generation;
    let mut menus = vec![crate::app::application_menu()];
    for (id, label, items) in &entries {
        let mut menu = Menu::new(label.clone());
        if matches!(id, BarMenu::Static(MenuId::Window)) && !projection.blocked {
            menu.items.push(MenuItem::action(
                crate::i18n::text("window.minimize"),
                crate::actions::MinimizeWindow,
            ));
            menu.items.push(MenuItem::separator());
        }
        menu.items
            .extend(items.iter().map(|item| item.clone().native(generation)));
        menus.push(menu);
    }
    state.active = Some(ActiveMenu {
        window: handle,
        editor: weak,
        entries,
        handlers: projection.handlers,
        generation,
    });
    cx.set_menus(menus);
}

struct Projection<'a> {
    handlers: Vec<Option<Handler>>,
    blocked: bool,
    recent_colors: &'a [ibis_doc_model::Rgb],
}

impl Projection<'_> {
    fn command(&mut self, item: crate::chrome::MenuItem) -> Item {
        let index = self.handlers.len();
        let enabled = !self.blocked && item.enabled();
        let label = menu_item_display_label(&item);
        self.handlers.push(item.handler.filter(|_| enabled));
        Item::Command {
            id: item.id,
            label,
            checked: item.checked,
            enabled,
            index,
        }
    }

    fn entries(&mut self, entries: Vec<MenuEntry>) -> Vec<Item> {
        let mut items = Vec::new();
        for entry in entries {
            match entry {
                MenuEntry::Item(item) => items.push(self.command(item)),
                MenuEntry::Separator => items.push(Item::Separator),
                MenuEntry::Submenu(sub) => {
                    items.push(Item::Submenu(
                        crate::i18n::text(&sub.label),
                        self.entries(sub.items),
                    ));
                }
                MenuEntry::Tiles(tiles) => {
                    items.extend(tiles.into_iter().map(|item| self.command(item)))
                }
                MenuEntry::Palette(palette) => self.palette(palette, &mut items),
            }
        }
        items
    }

    fn palette(&mut self, palette: crate::color_palette::Palette, items: &mut Vec<Item>) {
        use crate::color_palette::Swatch;
        use crate::i18n::text;

        items.extend(palette.none.into_iter().map(|item| self.command(item)));
        let current = palette
            .current
            .or_else(|| {
                palette
                    .theme
                    .iter()
                    .flatten()
                    .chain(&palette.standard)
                    .find(|swatch| swatch.item.checked)
                    .map(|swatch| swatch.rgb)
            })
            .unwrap_or(ibis_doc_model::Rgb(0));

        for (label, swatches) in [
            (
                "color.palette.theme",
                palette.theme.into_iter().flatten().collect(),
            ),
            ("color.palette.standard", palette.standard),
            (
                "color.palette.recent",
                palette
                    .pick
                    .as_ref()
                    .map(|pick| {
                        self.recent_colors
                            .iter()
                            .map(|&rgb| {
                                Swatch::new(rgb, format!("#{:06X}", rgb.0), rgb == current, pick)
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
        ] {
            if !swatches.is_empty() {
                let colors = swatches
                    .into_iter()
                    .map(|swatch| {
                        let mut item = swatch.item;
                        if let Some(handler) = item.handler.take() {
                            let rgb = swatch.rgb;
                            item = item.on(move |editor, window, cx| {
                                editor.note_recent_color(rgb);
                                handler(editor, window, cx);
                            });
                        }
                        self.command(item)
                    })
                    .collect();
                items.push(Item::Submenu(text(label), colors));
            }
        }
        let more = palette
            .pick
            .map(|pick| {
                crate::chrome::MenuItem::tr("color.palette.more").on(move |editor, window, cx| {
                    editor.open_color_picker_dialog(current, pick.clone(), window, cx);
                })
            })
            .or(palette.more);
        if let Some(more) = more {
            items.push(Item::Separator);
            items.push(self.command(more));
        }
    }
}

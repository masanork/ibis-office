// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The start view (スタート画面), shown while no file is open (at launch,
//! and after ファイル > 閉じる): 新規作成 (an empty document, workbook or
//! presentation), 選択 (開く…) and, on the desktop, the recently opened
//! files.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::palette::rgb;
use std::cell::RefCell;

use crate::actions::Open;
use crate::editor::SlideEditor;
use crate::icons::Material;
use gpui_kit::component::Icon;

/// Custom item displayed in the Web start view list (clicking fetches and opens in-app).
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartViewItem {
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub badge: Option<String>,
}

/// Custom link displayed in the Web start view right-hand panel.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartViewLink {
    pub label: String,
    pub url: String,
}

/// Custom feature highlight card in the Web start view right-hand panel.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartViewFeature {
    pub title: String,
    #[serde(default)]
    pub description: String,
}

/// Optional customization of the right-hand info panel on the start view.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartViewPanel {
    #[serde(default)]
    pub heading: Option<String>,
    #[serde(default)]
    pub body: Vec<String>,
    #[serde(default)]
    pub features: Vec<StartViewFeature>,
    #[serde(default)]
    pub links: Vec<StartViewLink>,
    #[serde(default)]
    pub note: Option<String>,
}

/// Host-supplied configuration for the Web start view (`set_start_view_config`).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartViewConfig {
    #[serde(default)]
    pub list_title: Option<String>,
    #[serde(default)]
    pub items: Vec<StartViewItem>,
    #[serde(default)]
    pub panel: Option<StartViewPanel>,
}

thread_local! {
    static START_VIEW_CONFIG: RefCell<Option<StartViewConfig>> = const { RefCell::new(None) };
}

fn is_safe_same_origin_url(url: &str) -> bool {
    (url.starts_with('/') && !url.starts_with("//")) || url.starts_with("./")
}

fn is_safe_link_url(url: &str) -> bool {
    is_safe_same_origin_url(url) || url.starts_with("https://") || url.starts_with("http://")
}

/// Parse and install a Web start view configuration from JSON.
pub fn set_start_view_config(json: &str) -> Result<(), String> {
    if json.trim().is_empty() {
        START_VIEW_CONFIG.with(|slot| *slot.borrow_mut() = None);
        return Ok(());
    }
    let mut config: StartViewConfig = serde_json::from_str(json).map_err(|err| format!("invalid start view config: {err}"))?;
    config.items.retain(|item| {
        !item.title.trim().is_empty()
            && !item.name.trim().is_empty()
            && is_safe_same_origin_url(&item.url)
    });
    config.items.truncate(100);
    if let Some(panel) = &mut config.panel {
        panel.body.retain(|p| !p.trim().is_empty());
        panel.body.truncate(8);
        panel.features.retain(|f| !f.title.trim().is_empty());
        panel.features.truncate(6);
        panel.links.retain(|link| !link.label.trim().is_empty() && is_safe_link_url(&link.url));
        panel.links.truncate(12);
    }
    START_VIEW_CONFIG.with(|slot| *slot.borrow_mut() = Some(config));
    Ok(())
}

/// Current host-supplied Web start view configuration, if any.
pub fn start_view_config() -> Option<StartViewConfig> {
    START_VIEW_CONFIG.with(|slot| slot.borrow().clone())
}

#[cfg(target_family = "wasm")]
fn badge_color(badge: &str) -> u32 {
    match badge.to_ascii_uppercase().as_str() {
        "DOCX" | "DOC" => 0x185abd,
        "XLSX" | "XLS" => 0x107c41,
        "PPTX" | "PPT" => 0xc43e1c,
        "JTD" => 0x6d28d9,
        "MD" | "MARKDOWN" => 0x334155,
        "XDW" => 0x0f766e,
        "HWPX" | "HWP" => 0x0284c7,
        "VSDX" => 0x2563eb,
        "CSV" => 0x15803d,
        "OFD" => 0xb91c1c,
        "ODS" | "ODF" => 0x047857,
        "JSON" | "SBOM" | "CBOM" => 0x0e7490,
        _ => 0x475569,
    }
}

/// The colour of the links (開く…), as Office's start screen.
const LINK_COLOR: u32 = 0x0f6cbd;
/// Width of the column of the start view, in pixels.
const COLUMN_WIDTH: f32 = 560.0;
/// Height of one row of the recent files list, in pixels.
#[cfg(not(target_family = "wasm"))]
const RECENT_ROW_HEIGHT: f32 = 44.0;
#[cfg(target_family = "wasm")]
const WEB_ITEM_ROW_HEIGHT: f32 = 54.0;
/// Space left under the recent files list, to the bottom of the window, in
/// pixels; also the space above each section's title.
const BOTTOM_MARGIN: f32 = 24.0;
/// The padding around a page of 新規作成 (where its hover shows), in pixels.
const TILE_PADDING: f32 = 8.0;
/// Space between a section's title and what it shows (開く…, the list), in
/// pixels: the pages' padding, which is all the space between 新規作成 and
/// its pages (their hover reaches the title), so that every title is as far
/// from what it shows.
const TITLE_GAP: f32 = TILE_PADDING;
/// The empty edge of a 20-pixel Material icon (開く…'s folder), in pixels.
const ICON_EDGE: f32 = 3.0;

/// A section's title (新規作成, 選択, 最近開いたファイル), `above` pixels
/// below what is over it and `below` pixels above what it shows; its line as tall as its letters, so that the
/// spaces are those of the letters.
fn section_title(key: &str, above: f32, below: f32) -> Div {
    div()
        .mt(px(above))
        .mb(px(below))
        .text_sm()
        .line_height(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(crate::i18n::text(key))
}
/// A page of 新規作成, in pixels.
const PAGE_WIDTH: f32 = 52.0;
const PAGE_HEIGHT: f32 = 66.0;
/// How far a page's badge reaches out left of the page, in pixels.
const BADGE_OVERHANG: f32 = 6.0;
/// The column's padding on each side (`px_8`), in pixels: also the space
/// between the recent files list and the 協力のお願い panel, and right of it.
const COLUMN_PADDING: f32 = 32.0;
/// The 協力のお願い panel right of the column: as wide as the recent files
/// list, and as narrow as this when the window is (its text wraps), in pixels.
const REQUEST_WIDTH: f32 = COLUMN_WIDTH - 2.0 * COLUMN_PADDING;
const REQUEST_MIN_WIDTH: f32 = 220.0;
/// The narrowest the column gets beside the panel, in pixels.
const COLUMN_MIN_WIDTH: f32 = 320.0;
/// The size of the panel's letters, in pixels: 1.2 times `text_sm`'s.
const REQUEST_TEXT_SIZE: f32 = 14.0 * 1.2;
/// The width a start window opens at: the column, the panel and the space right of it.
pub(crate) const START_WINDOW_WIDTH: f32 = COLUMN_WIDTH + REQUEST_WIDTH + COLUMN_PADDING;
/// Where 協力のお願い asks for reports.
const ISSUES_URL: &str = "https://github.com/masanork/ibis-office/issues";

/// The window's title while no file is open: the app and its version.
pub fn start_title() -> String {
    format!("Docs {}", env!("IBIS_VERSION"))
}

/// Custom right-hand landing panel supplied by a Web host (`StartViewPanel`).
fn custom_panel(panel: &StartViewPanel, muted: Hsla) -> Div {
    let mut card = v_flex()
        .w(px(REQUEST_WIDTH))
        .min_w(px(REQUEST_MIN_WIDTH))
        .flex_shrink(1.0)
        .mt(px(BOTTOM_MARGIN))
        .mr(px(COLUMN_PADDING))
        .p_4()
        .gap_2()
        .rounded(px(8.))
        .bg(rgb(0xf0f4f9))
        .text_size(px(14.0));
    if let Some(heading) = &panel.heading {
        if !heading.trim().is_empty() {
            card = card.child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child(heading.clone()));
        }
    }
    for paragraph in &panel.body {
        card = card.child(div().text_color(muted).child(paragraph.clone()));
    }
    if !panel.features.is_empty() {
        let mut list = v_flex().mt_1().gap_2();
        for feature in &panel.features {
            let mut item = v_flex()
                .px_3()
                .py_2()
                .gap_1()
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(0xe2e8f0))
                .bg(rgb(0xffffff))
                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(feature.title.clone()));
            if !feature.description.trim().is_empty() {
                item = item.child(div().text_xs().text_color(muted).child(feature.description.clone()));
            }
            list = list.child(item);
        }
        card = card.child(list);
    }
    if !panel.links.is_empty() {
        let mut links_row = div().mt_2().flex().flex_wrap().gap_2();
        for (idx, link) in panel.links.iter().enumerate() {
            let url = link.url.clone();
            links_row = links_row.child(
                div()
                    .id(("start-panel-link", idx))
                    .px_3()
                    .py_1()
                    .rounded(px(999.))
                    .border_1()
                    .border_color(rgb(0xd0d7e2))
                    .bg(rgb(0xffffff))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(LINK_COLOR))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xe8f0fe)))
                    .child(link.label.clone())
                    .on_click(move |_, _, cx| crate::external::open_url(&url, cx)),
            );
        }
        card = card.child(links_row);
    }
    if let Some(note) = &panel.note {
        if !note.trim().is_empty() {
            card = card.child(div().mt_1().text_xs().text_color(muted).child(note.clone()));
        }
    }
    card
}

/// 協力のお願い, right of the column: what ibisOffice aims at, and a link to
/// GitHub's issues (opened in the default browser) for reports.
fn request_panel(muted: Hsla) -> Div {
    // The link's words go where the text has {link}.
    let report = crate::i18n::text("start.request-report");
    let link = crate::i18n::text("start.request-link");
    let (before, after) = report.split_once("{link}").unwrap_or((&report, ""));
    let range = before.len()..before.len() + link.len();
    let link_style = HighlightStyle {
        color: Some(rgb(LINK_COLOR).into()),
        underline: Some(UnderlineStyle { thickness: px(1.), color: Some(rgb(LINK_COLOR).into()), wavy: false }),
        ..Default::default()
    };
    let report = InteractiveText::new("start-request-report", StyledText::new(format!("{before}{link}{after}")).with_highlights([(range.clone(), link_style)]))
        .on_click(vec![range], |_, _, cx| crate::external::open_url(ISSUES_URL, cx));
    v_flex()
        .w(px(REQUEST_WIDTH))
        .min_w(px(REQUEST_MIN_WIDTH))
        .flex_shrink(1.0)
        .mt(px(BOTTOM_MARGIN))
        .mr(px(COLUMN_PADDING))
        .p_4()
        .gap_2()
        .rounded(px(8.))
        .bg(rgb(0xf0f4f9))
        .text_size(px(REQUEST_TEXT_SIZE))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(crate::i18n::text("start.request")))
        .child(div().text_color(muted).child(crate::i18n::text("start.request-body")))
        .child(div().text_color(muted).child(report))
}

impl SlideEditor {
    /// The white page under the title bar while no file is open.
    pub(crate) fn render_start_view(&self, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let open = div()
            .id("start-open")
            .ml_4()
            .mt(px(-ICON_EDGE))
            .flex()
            .items_center()
            .gap_2()
            .text_color(rgb(LINK_COLOR))
            .text_base()
            .line_height(px(20.))
            .cursor_pointer()
            .hover(|s| s.underline())
            .child(Icon::new(Material("folder_open")).size(px(20.)).text_color(rgb(LINK_COLOR)))
            .child(crate::i18n::text("start.open"))
            .on_click(cx.listener(|this, _, window, cx| this.open_file(&Open, window, cx)));
        // The first badge's left edge under 新規作成's.
        let new_files = div()
            .flex()
            .gap_4()
            .when(narrow, |row| row.flex_wrap())
            .ml(px(BADGE_OVERHANG - TILE_PADDING))
            .children(crate::format::creatable_infos().into_iter().map(|(kind, info)| self.new_file_tile(kind, info.extension, info.new_badge_color, cx)))
            .child(self.new_markdown_tile(cx));

        div()
            .id("start-view")
            .key_context(crate::editor::WORKSPACE_CONTEXT)
            .track_focus(&self.focus_handle)
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(rgb(0xffffff))
            .overflow_hidden()
            .flex()
            .items_start()
            .child(
                v_flex()
                    .w(px(COLUMN_WIDTH))
                    .min_w(px(if narrow { 0. } else { COLUMN_MIN_WIDTH }))
                    .flex_shrink(1.0)
                    .h_full()
                    .when(narrow, |column| column.px_4())
                    .when(!narrow, |column| column.px_8())
                    .pb(px(BOTTOM_MARGIN))
                    .child(section_title("start.new", BOTTOM_MARGIN, 0.0))
                    .child(new_files)
                    // The pages' padding (their hover) is part of the space above 選択.
                    .child(section_title("start.select", BOTTOM_MARGIN - TILE_PADDING, TITLE_GAP))
                    .child(div().flex().child(open))
                    .map(|d| self.recent_files_list(d, cx))
                    .when_some(self.status.clone(), |d, status| d.child(div().mt_3().text_sm().text_color(muted).child(status))),
            )
            .when(!narrow, |view| view.child(v_flex().w(px(REQUEST_WIDTH)).min_w(px(REQUEST_MIN_WIDTH)).flex_shrink(1.).mr(px(COLUMN_PADDING)).mt(px(BOTTOM_MARGIN)).gap_4()
                .children(Self::update_button())
                .child(crate::chrome::brand_logo("start-app-logo",REQUEST_WIDTH / crate::chrome::LOGO_ASPECT,true))
                .child(Self::version_line(muted))
                .child(
                    match start_view_config().and_then(|c| c.panel) {
                        Some(panel) => custom_panel(&panel, muted).w_full().mt_0().mr_0(),
                        None => request_panel(muted).w_full().mt_0().mr_0(),
                    }
                )))
    }

    /// 「Powered by ibisOffice」 and 「ver.0.8.x」 directly under the logo.
    fn version_line(muted: Hsla) -> impl IntoElement {
        div()
            .id("start-version")
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .text_xs()
            .text_color(muted)
            .mt(px(-8.))
            .child(div().font_weight(FontWeight::MEDIUM).child("Powered by ibisOffice"))
            .child(crate::i18n::format("start.version", &[("version", &env!("IBIS_VERSION"))]))
    }

    /// 「ibisOffice を更新する」 over the logo, right-aligned, when a newer version is known (never in the browser, which has no
    /// updater): a pill, white with a blue border and label whatever the theme. A click asks the update question, as the About
    /// window's button does.
    #[cfg(not(target_family = "wasm"))]
    fn update_button() -> Option<impl IntoElement> {
        crate::update::available_version()?;
        Some(
            div().w_full().flex().justify_end().child(
                div()
                    .id("start-update")
                    .px_4()
                    .py_1()
                    .rounded(px(999.))
                    .border_1()
                    // White with a blue frame and label in the dark theme too (the request's colours, not theme tokens: `rgb` would invert them).
                    .border_color(crate::palette::doc(LINK_COLOR))
                    .bg(crate::palette::doc(0xffffff))
                    .text_color(crate::palette::doc(LINK_COLOR))
                    .text_sm()
                    .cursor_pointer()
                    .child(crate::i18n::text("start.update"))
                    .on_click(|_, _, cx| crate::update::offer_again(cx)),
            ),
        )
    }
    #[cfg(target_family = "wasm")]
    fn update_button() -> Option<Div> {
        None
    }

    /// One kind of 新規作成: a blank page with the format's badge on its
    /// lower left; a click makes an empty file of it.
    fn new_file_tile(&self, kind: ibis_formats::Kind, label: &'static str, color: u32, cx: &mut Context<Self>) -> AnyElement {
        self.render_new_tile(label, color, move |this, window, cx| this.new_file(kind, window, cx), cx)
    }

    /// 新規作成 tile for a blank Markdown document (`.md`).
    fn new_markdown_tile(&self, cx: &mut Context<Self>) -> AnyElement {
        self.render_new_tile("md", 0x334155, |this, window, cx| this.new_markdown_file(window, cx), cx)
    }

    fn render_new_tile(
        &self,
        label: &'static str,
        color: u32,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .w(px(PAGE_WIDTH))
            .h(px(PAGE_HEIGHT))
            .bg(rgb(0xffffff))
            .border_1()
            .border_color(rgb(0xc4c7c5))
            .rounded(px(3.));
        let badge = div()
            .absolute()
            .left(px(-BADGE_OVERHANG))
            .bottom(px(8.))
            .px(px(5.))
            .py(px(1.))
            .rounded(px(3.))
            .bg(rgb(color))
            .text_color(crate::palette::doc(0xffffff))
            .text_size(px(12.))
            .font_weight(FontWeight::BOLD)
            .child(label);
        div()
            .id(SharedString::from(format!("start-new-{label}")))
            .p(px(TILE_PADDING))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0xf0f4f9)))
            .child(div().relative().child(page).child(badge))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .into_any_element()
    }

    #[cfg(not(target_family="wasm"))]
    pub(crate) fn recent_file_actions(&self,path:std::path::PathBuf)->Vec<crate::chrome::MenuEntry>{
        vec![crate::chrome::MenuItem::tr("start.recent.remove").icon("delete").on(move|e,_,cx|{
            e.recent_files=crate::recent_files::remove(&path);crate::app::set_app_menus(cx);cx.notify();
        }).into()]
    }

    /// Look up, once and off the UI thread, which files of the recent list exist (`recent_exists`).
    #[cfg(not(target_family = "wasm"))]
    fn check_recent_exists(&self, cx: &mut Context<Self>) {
        if self.recent_checking.get() {
            return;
        }
        let unknown: Vec<std::path::PathBuf> = self.recent_files.iter().filter(|p| !self.recent_exists.contains_key(*p)).cloned().collect();
        if unknown.is_empty() {
            return;
        }
        self.recent_checking.set(true);
        cx.spawn(async move |this, cx| {
            let found = cx.background_executor().spawn(async move { unknown.into_iter().map(|p| { let e = p.is_file(); (p, e) }).collect::<Vec<_>>() }).await;
            this.update(cx, |this, cx| {
                this.recent_checking.set(false);
                this.recent_exists.extend(found);
                cx.notify();
            }).ok();
        })
        .detach();
    }

    /// 最近開いたファイル and the list of them; a file that is gone is greyed.
    #[cfg(not(target_family = "wasm"))]
    fn recent_files_list(&self, column: Div, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (foreground, muted, border, hover, background) =
            (theme.foreground, theme.muted_foreground, theme.border, theme.muted, theme.background);
        self.check_recent_exists(cx);
        let rows: Vec<AnyElement> = self
            .recent_files
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let name: SharedString = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned())
                    .into();
                let dir: SharedString = path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default().into();
                // Not yet checked: drawn as existing (the check is on a background thread).
                let exists = self.recent_exists.get(path).copied().unwrap_or(true);
                let path = path.clone();
                let entries=self.recent_file_actions(path.clone());
                let context_entries=entries.clone();
                div()
                    .id(("recent-file", index))
                    .flex_shrink_0()
                    .h(px(RECENT_ROW_HEIGHT))
                    .px_3()
                    .flex()
                    .group("row-actions")
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(border)
                    .text_color(if exists { foreground } else { muted })
                    .on_mouse_down(MouseButton::Right,cx.listener(move|e,event:&MouseDownEvent,w,cx|{
                        cx.stop_propagation();e.open_hyperlink_context(event.position,context_entries.clone(),w,cx);
                    }))
                    // A gone file is greyed but still clickable: the click tells why it cannot be opened (a message box).
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_path_or_raise(path.clone(), window, cx)))
                    .child(div().flex_1().min_w_0().child(div().text_sm().truncate().child(name)).child(div().text_xs().text_color(muted).truncate().child(dir)))
                    .child(crate::chrome::hover_row_actions(index,entries,&cx.weak_entity()))
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        // The box reaches down to the bottom margin whatever the number of files,
        // and scrolls when the window is too short for them.
        column.child(section_title("start.recent", BOTTOM_MARGIN - ICON_EDGE, TITLE_GAP)).child(
            div()
                .relative()
                .w_full()
                .flex_1()
                .min_h_0()
                .border_1()
                .border_color(border)
                .rounded_sm()
                .bg(background)
                .overflow_hidden()
                .child(
                    div()
                        .id("recent-files-scroll")
                        .size_full()
                        .flex()
                        .flex_col()
                        .overflow_y_scroll()
                        .track_scroll(&self.recent_scroll)
                        .when(empty, |d| {
                            d.child(
                                div()
                                    .h(px(RECENT_ROW_HEIGHT))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(crate::i18n::text("start.recent-none")),
                            )
                        })
                        .children(rows),
                )
                .vertical_scrollbar(&self.recent_scroll),
        )
    }

    /// Host-supplied items (samples or shortcuts) shown in the Web start view.
    /// When no items are configured, the web build shows no history section.
    #[cfg(target_family = "wasm")]
    fn recent_files_list(&self, column: Div, cx: &mut Context<Self>) -> Div {
        let Some(config) = start_view_config() else {
            return column;
        };
        if config.items.is_empty() {
            return column;
        }
        let theme = cx.theme();
        let (foreground, muted, border, hover, background) =
            (theme.foreground, theme.muted_foreground, theme.border, theme.muted, theme.background);
        let title_text: SharedString = config
            .list_title
            .filter(|t| !t.trim().is_empty())
            .map(SharedString::from)
            .unwrap_or_else(|| crate::i18n::text("start.recent").into());
        let rows: Vec<AnyElement> = config
            .items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let badge_label = item.badge.unwrap_or_else(|| {
                    item.name
                        .rsplit_once('.')
                        .map(|(_, ext)| ext.to_ascii_uppercase())
                        .unwrap_or_else(|| "FILE".to_string())
                });
                let badge_bg = badge_color(&badge_label);
                let title: SharedString = item.title.into();
                let subtitle: SharedString = item.subtitle.into();
                let has_subtitle = !subtitle.is_empty();
                let url = item.url;
                let name = item.name;
                div()
                    .id(("web-start-item", index))
                    .flex_shrink_0()
                    .min_h(px(WEB_ITEM_ROW_HEIGHT))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(border)
                    .text_color(foreground)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_web_url(url.clone(), name.clone(), window, cx);
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(44.))
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgb(badge_bg))
                            .text_color(crate::palette::doc(0xffffff))
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .flex()
                            .justify_center()
                            .child(badge_label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).truncate().child(title))
                            .when(has_subtitle, |d| {
                                d.child(div().text_xs().text_color(muted).truncate().child(subtitle))
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        column
            .child(
                div()
                    .mt(px(BOTTOM_MARGIN - ICON_EDGE))
                    .mb(px(TITLE_GAP))
                    .text_sm()
                    .line_height(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title_text),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(border)
                    .rounded_sm()
                    .bg(background)
                    .overflow_hidden()
                    .child(
                        div()
                            .id("recent-files-scroll")
                            .size_full()
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .track_scroll(&self.recent_scroll)
                            .children(rows),
                    )
                    .vertical_scrollbar(&self.recent_scroll),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[core::prelude::v1::test]
    fn start_view_config_filters_unsafe_urls_and_resets() {
        let json = r#"{
            "listTitle": "サンプル",
            "items": [
                { "title": "Valid", "subtitle": "ok", "name": "a.docx", "url": "/samples/a.docx", "badge": "DOCX" },
                { "title": "Relative", "name": "b.md", "url": "./samples/b.md" },
                { "title": "CrossOrigin", "name": "c.docx", "url": "https://evil.example/c.docx" },
                { "title": "ProtoRelative", "name": "d.docx", "url": "//evil.example/d.docx" },
                { "title": "JsUrl", "name": "e.docx", "url": "javascript:alert(1)" }
            ],
            "panel": {
                "heading": "Docs",
                "body": ["Hello", ""],
                "links": [
                    { "label": "Files", "url": "/files" },
                    { "label": "GitHub", "url": "https://github.com/masanork/ibis-office" },
                    { "label": "Bad", "url": "javascript:void(0)" }
                ]
            }
        }"#;
        set_start_view_config(json).expect("valid config");
        let cfg = start_view_config().expect("stored");
        assert_eq!(cfg.list_title.as_deref(), Some("サンプル"));
        assert_eq!(cfg.items.len(), 2);
        assert_eq!(cfg.items[0].name, "a.docx");
        assert_eq!(cfg.items[1].name, "b.md");
        let panel = cfg.panel.expect("panel");
        assert_eq!(panel.body, vec!["Hello".to_string()]);
        assert_eq!(panel.links.len(), 2);

        set_start_view_config("").expect("clear");
        assert!(start_view_config().is_none());
    }
}

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
use std::cell::{Cell, RefCell};

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

/// Optional secondary action shown beside 「開く…」 in the Web start view (e.g. GitHub login / repository picker).
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartViewSelectAction {
    pub label: String,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// One row in the Web recent-files history (`recents` in the start-view config).
/// `action` is `open-recent:<id>`; clicking it is handled in `www/main.ts`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartViewRecent {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    pub name: String,
    #[serde(default)]
    pub badge: Option<String>,
    pub action: String,
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
    pub recents: Vec<StartViewRecent>,
    #[serde(default)]
    pub select_action: Option<StartViewSelectAction>,
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

const WEB_RECENTS_SHOW_LIMIT: usize = 5;

/// `open-recent:<id>` as supplied by the Web recent-files store.
pub fn is_open_recent_action(action: &str) -> bool {
    recent_action_id(action, "open-recent").is_some()
}

fn recent_action_id<'a>(action: &'a str, kind: &str) -> Option<&'a str> {
    let (found, id) = action.split_once(':')?;
    if found != kind || id.is_empty() || id.len() > 512 {
        return None;
    }
    if id.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '\\')) {
        return None;
    }
    Some(id)
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
    if let Some(sa) = &config.select_action {
        let valid_action = sa.action.as_deref().is_some_and(|a| !a.trim().is_empty());
        let valid_url = sa.url.as_deref().is_some_and(is_safe_link_url);
        if sa.label.trim().is_empty() || (!valid_action && !valid_url) {
            config.select_action = None;
        }
    }
    if let Some(panel) = &mut config.panel {
        panel.body.retain(|p| !p.trim().is_empty());
        panel.body.truncate(8);
        panel.features.retain(|f| !f.title.trim().is_empty());
        panel.features.truncate(6);
        panel.links.retain(|link| !link.label.trim().is_empty() && is_safe_link_url(&link.url));
        panel.links.truncate(12);
    }
    config.recents.retain(|item| {
        !item.id.trim().is_empty()
            && !item.title.trim().is_empty()
            && !item.name.trim().is_empty()
            && is_open_recent_action(&item.action)
    });
    config.recents.truncate(WEB_RECENTS_SHOW_LIMIT);
    START_VIEW_CONFIG.with(|slot| *slot.borrow_mut() = Some(config));
    Ok(())
}

/// Current host-supplied Web start view configuration, if any.
pub fn start_view_config() -> Option<StartViewConfig> {
    START_VIEW_CONFIG.with(|slot| slot.borrow().clone())
}

thread_local! {
    // Phone start-view UI state (new-document sheet and sample list). Kept off the
    // editor struct so a layout-only change does not touch its field list.
    static MOBILE_NEW_SHEET_OPEN: Cell<bool> = const { Cell::new(false) };
    static MOBILE_SAMPLES_OPEN: Cell<Option<bool>> = const { Cell::new(None) };
}

fn mobile_new_sheet_open() -> bool {
    MOBILE_NEW_SHEET_OPEN.with(|slot| slot.get())
}

fn set_mobile_new_sheet_open(open: bool) {
    MOBILE_NEW_SHEET_OPEN.with(|slot| slot.set(open));
}

fn mobile_samples_user_override() -> Option<bool> {
    MOBILE_SAMPLES_OPEN.with(|slot| slot.get())
}

fn set_mobile_samples_user_override(open: Option<bool>) {
    MOBILE_SAMPLES_OPEN.with(|slot| slot.set(open));
}

/// Bottom inset reserved for the home indicator (GPUI has no CSS env() yet).
const MOBILE_SAFE_BOTTOM: f32 = 24.0;
/// Minimum height of a mobile start-view action, in pixels.
const MOBILE_TOUCH: f32 = 56.0;

/// A start-view item fetched from the GitHub vault API (not a bundled sample).
pub fn is_github_vault_url(url: &str) -> bool {
    url.starts_with("/api/github/")
}

/// A bundled sample under `/samples` or `./samples`.
pub fn is_sample_item_url(url: &str) -> bool {
    let path = url.split_once('?').map(|(p, _)| p).unwrap_or(url);
    path.starts_with("/samples/") || path.starts_with("./samples/") || path == "/samples" || path == "./samples"
}

/// Whether the mobile sample list starts expanded. The user override wins.
/// Auto-expand only when this platform has a recents source and that list is
/// empty (first run, including the Web IndexedDB history). A connected
/// GitHub repo list also stays collapsed.
pub fn mobile_samples_expanded(
    recents_empty: bool,
    has_recent_source: bool,
    has_github_items: bool,
    user_override: Option<bool>,
) -> bool {
    if let Some(open) = user_override {
        return open;
    }
    if has_github_items {
        return false;
    }
    has_recent_source && recents_empty
}

/// Labels of the mobile new-document sheet, in display order.
pub fn mobile_new_document_kinds() -> [&'static str; 4] {
    ["memo", "word", "excel", "powerpoint"]
}


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
    format!("soraneDocs {}", env!("IBIS_VERSION"))
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
        if narrow {
            return self.render_mobile_start_view(cx);
        }
        // Leaving phone mode must not leave the sheet open for the next visit.
        set_mobile_new_sheet_open(false);
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
        let select_action = start_view_config().and_then(|c| c.select_action).map(|sa| {
            let action = sa.action.clone();
            let url = sa.url.clone();
            div()
                .id("start-select-action")
                .mt(px(-ICON_EDGE))
                .px_3()
                .py_1()
                .rounded(px(999.))
                .border_1()
                .border_color(rgb(0xd0d7e2))
                .bg(rgb(0xf8fafc))
                .flex()
                .items_center()
                .gap_1()
                .text_color(rgb(LINK_COLOR))
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0xe8f0fe)).border_color(rgb(LINK_COLOR)))
                .child(Icon::new(Material("cloud")).size(px(16.)).text_color(rgb(LINK_COLOR)))
                .child(sa.label)
                .on_click(move |_, _, cx| {
                    if let Some(act) = &action {
                        #[cfg(target_family = "wasm")]
                        crate::web::trigger_start_action(act);
                        let _ = act;
                    } else if let Some(target_url) = &url {
                        crate::external::open_url(target_url, cx);
                    }
                })
        });
        // The first badge's left edge under 新規作成's.
        let new_files = div()
            .flex()
            .gap_4()
            .flex_wrap()
            .ml(px(BADGE_OVERHANG - TILE_PADDING))
            .child(self.new_markdown_tile(cx))
            .children(crate::format::creatable_infos().into_iter().map(|(kind, info)| self.new_file_tile(kind, info.extension, info.new_badge_color, cx)));
        let new_files = new_files.when(!crate::host::is_connected(), |row| {
            row.child(self.render_new_tile("vsdx", 0x2563a8, |this, window, cx| this.new_visio_draft(window, cx), cx))
        });

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
                    .children(self.render_crash_report_banner(cx))
                    .child(section_title("start.new", BOTTOM_MARGIN, 0.0))
                    .child(new_files)
                    // The pages' padding (their hover) is part of the space above 選択.
                    .child(section_title("start.select", BOTTOM_MARGIN - TILE_PADDING, TITLE_GAP))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_4()
                            .child(open)
                            .when_some(select_action, |row, btn| row.child(btn)),
                    )
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
        .into_any_element()
    }

    fn render_crash_report_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        #[cfg(target_family = "wasm")]
        {
            let _ = cx;
            None
        }
        #[cfg(not(target_family = "wasm"))]
        {
            if !crate::crash_report::has_pending_report() {
                return None;
            }
            let theme = cx.theme();
            Some(
                div().flex().flex_row()
                    .id("start-crash-report-banner")
                    .w_full()
                    .px_3()
                    .py_2()
                    .mb_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(0xf59e0b))
                    .bg(rgb(0xfffbeb))
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div().flex().flex_row()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(Icon::new(Material("warning")).size(px(16.)).text_color(rgb(0xb45309)))
                            .child(div().text_xs().text_color(theme.foreground).child(crate::i18n::text("crash-report.banner"))),
                    )
                    .child(
                        div().flex().flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("start-crash-report-share")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.))
                                    .bg(rgb(0xb45309))
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(0xffffff))
                                    .cursor_pointer()
                                    .child(crate::i18n::text("crash-report.share"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(msg) = crate::crash_report::share_report(cx) {
                                            this.set_status(msg, cx);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("start-crash-report-dismiss")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .cursor_pointer()
                                    .child(crate::i18n::text("crash-report.dismiss"))
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        crate::crash_report::dismiss_report();
                                        cx.notify();
                                    })),
                            ),
                    )
                    .into_any_element(),
            )
        }
    }

    /// Phone-sized start view (`narrow_phone_editor`: phone editor flag and width under 600 px).
    fn render_mobile_start_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (foreground, muted, border, hover, background) =
            (theme.foreground, theme.muted_foreground, theme.border, theme.muted, theme.background);
        let config = start_view_config();
        let select_action = config.as_ref().and_then(|c| c.select_action.clone());
        let all_items = config.as_ref().map(|c| c.items.as_slice()).unwrap_or(&[]);
        let github_items: Vec<StartViewItem> = all_items
            .iter()
            .filter(|item| is_github_vault_url(&item.url))
            .cloned()
            .collect();
        let sample_items: Vec<StartViewItem> = all_items
            .iter()
            .filter(|item| is_sample_item_url(&item.url))
            .cloned()
            .collect();
        let has_github_items = !github_items.is_empty();

        #[cfg(not(target_family = "wasm"))]
        let (recent_paths, has_recent_source) = (self.recent_files.clone(), true);
        #[cfg(not(target_family = "wasm"))]
        let recents_empty = recent_paths.is_empty();
        #[cfg(target_family = "wasm")]
        let web_recents: Vec<StartViewRecent> =
            config.as_ref().map(|c| c.recents.clone()).unwrap_or_default();
        #[cfg(target_family = "wasm")]
        let has_recent_source = true;
        #[cfg(target_family = "wasm")]
        let recents_empty = web_recents.is_empty();
        let samples_open = mobile_samples_expanded(
            recents_empty,
            has_recent_source,
            has_github_items,
            mobile_samples_user_override(),
        );

        let mut body = v_flex()
            .id("start-mobile-scroll")
            .flex_1()
            .min_h_0()
            .w_full()
            .px_4()
            .pt_3()
            .pb(px(MOBILE_TOUCH + 16.0 + MOBILE_SAFE_BOTTOM + 16.0))
            .gap_3()
            .overflow_y_scroll()
            .track_scroll(&self.recent_scroll)
            .children(self.render_crash_report_banner(cx));

        #[cfg(not(target_family = "wasm"))]
        if !recents_empty {
            body = body
                .child(section_title("start.recent", 0.0, TITLE_GAP))
                .child(self.mobile_recent_rows(&recent_paths, foreground, muted, border, hover, cx));
        }
        #[cfg(target_family = "wasm")]
        if !web_recents.is_empty() {
            body = body
                .child(section_title("start.recent", 0.0, TITLE_GAP))
                .child(self.web_recent_rows(&web_recents, foreground, muted, border, hover, true, cx))
                .child(self.web_recents_clear_link(muted, cx));
        }

        body = body.child(self.mobile_action_button(
            "start-open",
            "folder_open",
            crate::i18n::text("start.open-file"),
            Some(SharedString::from("Word・Excel・PowerPoint・PDF・一太郎・DocuWorks・Markdown など")),
            foreground,
            muted,
            border,
            hover,
            background,
            |this, _, window, cx| this.open_file(&Open, window, cx),
            cx,
        ));

        if let Some(sa) = select_action {
            let action = sa.action.clone();
            let url = sa.url.clone();
            let default_label = crate::i18n::text("start.github-open");
            let title = default_label.clone();
            let subtitle = if sa.label.trim() != default_label.trim() && !sa.label.trim().is_empty() {
                Some(SharedString::from(sa.label))
            } else {
                None
            };
            body = body.child(self.mobile_action_button(
                "start-select-action",
                "cloud",
                title,
                subtitle,
                foreground,
                muted,
                border,
                hover,
                background,
                move |_, _, _, cx| {
                    if let Some(act) = &action {
                        #[cfg(target_family = "wasm")]
                        crate::web::trigger_start_action(act);
                        let _ = act;
                    } else if let Some(target_url) = &url {
                        crate::external::open_url(target_url, cx);
                    }
                },
                cx,
            ));
        }

        if has_github_items {
            let list_title: SharedString = config
                .as_ref()
                .and_then(|c| c.list_title.clone())
                .filter(|t| !t.trim().is_empty())
                .map(SharedString::from)
                .unwrap_or_else(|| crate::i18n::text("start.github-open").into());
            body = body
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(foreground)
                        .child(list_title),
                )
                .child(self.mobile_web_item_rows(github_items, foreground, muted, border, hover, cx));
        }

        if !sample_items.is_empty() {
            let toggle_key = if samples_open { "start.samples-hide" } else { "start.samples-open" };
            body = body.child(
                div()
                    .id("start-samples-toggle")
                    .mt_2()
                    .py_2()
                    .flex()
                    .justify_center()
                    .cursor_pointer()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(LINK_COLOR))
                            .child(crate::i18n::text(toggle_key)),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        set_mobile_samples_user_override(Some(!samples_open));
                        cx.notify();
                    })),
            );
            if samples_open {
                body = body.child(self.mobile_web_item_rows(sample_items, foreground, muted, border, hover, cx));
            }
        }

        body = body.when_some(self.status.clone(), |d, status| {
            d.child(div().mt_2().text_sm().text_color(muted).child(status))
        });

        let fab = div()
            .id("start-new-fab")
            .absolute()
            .right(px(16.))
            .bottom(px(16. + MOBILE_SAFE_BOTTOM))
            .w(px(MOBILE_TOUCH))
            .h(px(MOBILE_TOUCH))
            .rounded(px(MOBILE_TOUCH / 2.))
            .bg(rgb(LINK_COLOR))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .shadow_lg()
            .child(Icon::new(Material("add")).size(px(28.)).text_color(crate::palette::doc(0xffffff)))
            .on_click(cx.listener(|_, _, _, cx| {
                set_mobile_new_sheet_open(true);
                cx.notify();
            }));

        let sheet = mobile_new_sheet_open().then(|| self.render_mobile_new_sheet(cx));

        div()
            .id("start-view")
            .key_context(crate::editor::WORKSPACE_CONTEXT)
            .track_focus(&self.focus_handle)
            .flex_1()
            .min_h_0()
            .w_full()
            .relative()
            .bg(background)
            .overflow_hidden()
            .child(body.vertical_scrollbar(&self.recent_scroll))
            .child(fab)
            .when_some(sheet, |d, s| d.child(s))
            .into_any_element()
    }

    fn mobile_action_button(
        &self,
        id: &'static str,
        icon: &'static str,
        title: impl Into<SharedString>,
        subtitle: Option<SharedString>,
        foreground: Hsla,
        muted: Hsla,
        border: Hsla,
        hover: Hsla,
        background: Hsla,
        on_click: impl Fn(&mut Self, &gpui_kit::ClickEvent, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = title.into();
        let has_sub = subtitle.as_ref().is_some_and(|s| !s.is_empty());
        div()
            .id(id)
            .w_full()
            .min_h(px(MOBILE_TOUCH))
            .px_4()
            .py_3()
            .rounded(px(12.))
            .border_1()
            .border_color(border)
            .bg(background)
            .flex()
            .items_center()
            .gap_3()
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(Icon::new(Material(icon)).size(px(24.)).text_color(rgb(LINK_COLOR)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_base().font_weight(FontWeight::MEDIUM).text_color(foreground).child(title))
                    .when(has_sub, |d| {
                        d.child(div().text_xs().text_color(muted).truncate().child(subtitle.clone().unwrap_or_default()))
                    }),
            )
            .on_click(cx.listener(on_click))
    }


    #[cfg(target_family = "wasm")]
    fn web_recents_clear_link(&self, muted: Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("start-recents-clear")
            .mt_1()
            .py_1()
            .flex()
            .justify_center()
            .cursor_pointer()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(crate::i18n::text("start.recent.clear")),
            )
            .on_click(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                #[cfg(target_family = "wasm")]
                crate::web::trigger_start_action("clear-recents");
            }))
    }

    #[cfg(target_family = "wasm")]
    fn web_recent_rows(
        &self,
        recents: &[StartViewRecent],
        foreground: Hsla,
        muted: Hsla,
        border: Hsla,
        hover: Hsla,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let rows = recents.iter().cloned().enumerate().map(|(index, item)| {
            let badge_label = item.badge.clone().unwrap_or_else(|| {
                item.name
                    .rsplit_once('.')
                    .map(|(_, ext)| ext.to_ascii_uppercase())
                    .unwrap_or_else(|| "FILE".to_string())
            });
            let badge_bg = badge_color(&badge_label);
            let title: SharedString = item.title.into();
            let subtitle: SharedString = item.subtitle.clone().into();
            let has_subtitle = !item.subtitle.is_empty();
            let open_action = item.action.clone();
            let remove_action = format!("remove-recent:{}", item.id);
            div()
                .id(("start-web-recent", index))
                .w_full()
                .min_h(px(if compact { MOBILE_TOUCH - 8.0 } else { WEB_ITEM_ROW_HEIGHT }))
                .px_3()
                .py_1()
                .flex()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(border)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |_, _, _, _cx| {
                    #[cfg(target_family = "wasm")]
                    crate::web::trigger_start_action(&open_action);
                    #[cfg(not(target_family = "wasm"))]
                    let _ = &open_action;
                }))
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(40.))
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
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(foreground).truncate().child(title))
                        .when(has_subtitle, |d| {
                            d.child(div().text_xs().text_color(muted).truncate().child(subtitle))
                        }),
                )
                .child(
                    div()
                        .id(("start-web-recent-remove", index))
                        .flex_shrink_0()
                        .w(px(32.))
                        .h(px(32.))
                        .rounded(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .child(Icon::new(Material("close")).size(px(16.)).text_color(muted))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            #[cfg(target_family = "wasm")]
                            crate::web::trigger_start_action(&remove_action);
                            #[cfg(not(target_family = "wasm"))]
                            let _ = &remove_action;
                        })),
                )
        });
        v_flex()
            .w_full()
            .rounded(px(12.))
            .border_1()
            .border_color(border)
            .overflow_hidden()
            .children(rows)
    }

    fn mobile_web_item_rows(
        &self,
        items: Vec<StartViewItem>,
        foreground: Hsla,
        muted: Hsla,
        border: Hsla,
        hover: Hsla,
        cx: &mut Context<Self>,
    ) -> Div {
        let rows = items.into_iter().enumerate().map(|(index, item)| {
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
                .id(("start-mobile-item", index))
                .w_full()
                .min_h(px(MOBILE_TOUCH))
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(border)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    #[cfg(target_family = "wasm")]
                    this.open_web_url(url.clone(), name.clone(), window, cx);
                    #[cfg(not(target_family = "wasm"))]
                    let _ = (this, &url, &name, window, cx);
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
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(foreground).truncate().child(title))
                        .when(has_subtitle, |d| d.child(div().text_xs().text_color(muted).truncate().child(subtitle))),
                )
        });
        v_flex()
            .w_full()
            .rounded(px(12.))
            .border_1()
            .border_color(border)
            .overflow_hidden()
            .children(rows)
    }

    #[cfg(not(target_family = "wasm"))]
    fn mobile_recent_rows(
        &self,
        paths: &[std::path::PathBuf],
        foreground: Hsla,
        muted: Hsla,
        border: Hsla,
        hover: Hsla,
        cx: &mut Context<Self>,
    ) -> Div {
        self.check_recent_exists(cx);
        let rows = paths.iter().enumerate().map(|(index, path)| {
            let name: SharedString = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned())
                .into();
            let dir: SharedString = path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default().into();
            let exists = self.recent_exists.get(path).copied().unwrap_or(true);
            let ext: Option<SharedString> = path
                .extension()
                .and_then(|e| e.to_str())
                .filter(|e| !e.is_empty())
                .map(|e| e.to_ascii_uppercase().into());
            let path = path.clone();
            div()
                .id(("start-mobile-recent", index))
                .w_full()
                .min_h(px(MOBILE_TOUCH))
                .px_3()
                .py_2()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(border)
                .text_color(if exists { foreground } else { muted })
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_path_or_raise(path.clone(), window, cx);
                }))
                .when_some(ext, |row, ext| {
                    row.child(
                        div()
                            .min_w(px(42.))
                            .px_1p5()
                            .py_0p5()
                            .rounded(px(6.))
                            .bg(hover)
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted)
                            .text_center()
                            .child(ext),
                    )
                })
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).truncate().child(name))
                        .when(!dir.is_empty(), |d| d.child(div().text_xs().text_color(muted).truncate().child(dir))),
                )
        });
        v_flex()
            .w_full()
            .rounded(px(12.))
            .border_1()
            .border_color(border)
            .overflow_hidden()
            .children(rows)
    }

    #[cfg(target_family = "wasm")]
    fn mobile_recent_rows(
        &self,
        _paths: &[std::path::PathBuf],
        _foreground: Hsla,
        _muted: Hsla,
        _border: Hsla,
        _hover: Hsla,
        _cx: &mut Context<Self>,
    ) -> Div {
        div()
    }

    fn render_mobile_new_sheet(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (foreground, muted, _border, hover, popover) = (
            theme.foreground,
            theme.muted_foreground,
            theme.border,
            theme.muted,
            theme.popover,
        );
        let rows: [(&'static str, &'static str, &'static str, Option<&'static str>, fn(&mut Self, &mut Window, &mut Context<Self>)); 4] = [
            ("start-new-memo", "edit_note", "start.new-memo", Some("start.new-memo-sub"), |this, window, cx| {
                this.new_markdown_file(window, cx)
            }),
            ("start-new-word", "description", "start.new-word", None, |this, window, cx| {
                this.new_file(ibis_formats::Kind::Document, window, cx)
            }),
            ("start-new-excel", "table_chart", "start.new-excel", None, |this, window, cx| {
                this.new_file(ibis_formats::Kind::Workbook, window, cx)
            }),
            ("start-new-powerpoint", "slideshow", "start.new-powerpoint", None, |this, window, cx| {
                this.new_file(ibis_formats::Kind::Presentation, window, cx)
            }),
        ];
        let sheet_rows = rows.map(|(id, icon, title_key, sub_key, action)| {
            div()
                .id(id)
                .w_full()
                .min_h(px(MOBILE_TOUCH))
                .px_4()
                .flex()
                .items_center()
                .gap_3()
                .rounded(px(10.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(Icon::new(Material(icon)).size(px(24.)).text_color(rgb(LINK_COLOR)))
                .child(
                    div()
                        .flex_1()
                        .child(div().text_base().font_weight(FontWeight::MEDIUM).text_color(foreground).child(crate::i18n::text(title_key)))
                        .when_some(sub_key, |d, key| d.child(div().text_xs().text_color(muted).child(crate::i18n::text(key)))),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    set_mobile_new_sheet_open(false);
                    action(this, window, cx);
                }))
        });
        div()
            .id("start-new-sheet")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("start-new-sheet-dismiss")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .bg(hsla(0.0, 0.0, 0.0, 0.4))
                    .on_click(cx.listener(|_, _, _, cx| {
                        set_mobile_new_sheet_open(false);
                        cx.notify();
                    })),
            )
            .child(
                v_flex()
                    .w_full()
                    .bg(popover)
                    .rounded_tl(px(16.))
                    .rounded_tr(px(16.))
                    .px_4()
                    .pt_3()
                    .pb(px(12. + MOBILE_SAFE_BOTTOM))
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb_2()
                            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(foreground).child(crate::i18n::text("start.new-sheet")))
                            .child(
                                div()
                                    .id("start-new-sheet-close")
                                    .p_2()
                                    .cursor_pointer()
                                    .child(Icon::new(Material("close")).size(px(20.)).text_color(muted))
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        set_mobile_new_sheet_open(false);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .children(sheet_rows),
            )
    }

    /// 「ver.0.8.x」 directly under the logo lockup.
    fn version_line(muted: Hsla) -> impl IntoElement {
        div()
            .id("start-version")
            .w_full()
            .flex()
            .items_center()
            .justify_end()
            .text_xs()
            .text_color(muted)
            .mt(px(-10.))
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

    /// 新規作成 tile for a unified plain-text document (`text`: auto-detects Markdown / Marp / Sheet / Lawtext).
    fn new_markdown_tile(&self, cx: &mut Context<Self>) -> AnyElement {
        self.render_new_tile("text", 0x334155, |this, window, cx| this.new_markdown_file(window, cx), cx)
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
        let id = SharedString::from(format!("start-new-{label}"));
        let tile = div()
            .id(id.clone())
            .flex_none()
            .p(px(TILE_PADDING))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0xf0f4f9)))
            .child(div().relative().child(page).child(badge))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)));
        #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
        let tile = tile.relative().child(gpui_kit::canvas(
            |_, _, _| (),
            move |bounds, _, _, _| crate::scenario::button_probe::record_bounds(&id, bounds),
        ).absolute().inset_0());
        tile.into_any_element()
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
    /// When recents exist they appear above the sample / vault list, in the
    /// same slot the desktop build uses for recently opened files.
    #[cfg(target_family = "wasm")]
    fn recent_files_list(&self, column: Div, cx: &mut Context<Self>) -> Div {
        let Some(config) = start_view_config() else {
            return column;
        };
        if config.items.is_empty() && config.recents.is_empty() {
            return column;
        }
        let theme = cx.theme();
        let (foreground, muted, border, hover, background) =
            (theme.foreground, theme.muted_foreground, theme.border, theme.muted, theme.background);
        let recents = config.recents.clone();
        let column = if recents.is_empty() {
            column
        } else {
            column
                .child(
                    div()
                        .mt(px(BOTTOM_MARGIN - ICON_EDGE))
                        .mb(px(TITLE_GAP))
                        .text_sm()
                        .line_height(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(crate::i18n::text("start.recent")),
                )
                .child(self.web_recent_rows(&recents, foreground, muted, border, hover, false, cx))
                .child(self.web_recents_clear_link(muted, cx))
        };
        if config.items.is_empty() {
            return column;
        }
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
    fn classifies_sample_and_github_vault_urls() {
        assert!(is_sample_item_url("/samples/mermaid-showcase.md"));
        assert!(is_sample_item_url("./samples/a.docx"));
        assert!(is_github_vault_url("/api/github/file?path=notes.md"));
        assert!(is_github_vault_url("/api/github/tree"));
        assert!(!is_sample_item_url("/api/github/file?path=samples/x.md"));
        assert!(!is_github_vault_url("/samples/a.docx"));
        assert!(!is_sample_item_url("https://example.com/samples/a.docx"));
    }

    #[core::prelude::v1::test]
    fn mobile_samples_expand_only_on_empty_native_recents() {
        assert!(mobile_samples_expanded(true, true, false, None));
        assert!(!mobile_samples_expanded(false, true, false, None));
        assert!(!mobile_samples_expanded(true, false, false, None));
        assert!(!mobile_samples_expanded(true, true, true, None));
        assert!(mobile_samples_expanded(false, true, true, Some(true)));
        assert!(!mobile_samples_expanded(true, true, false, Some(false)));
    }

    #[core::prelude::v1::test]
    fn mobile_new_document_puts_memo_first() {
        assert_eq!(mobile_new_document_kinds(), ["memo", "word", "excel", "powerpoint"]);
    }

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

    #[core::prelude::v1::test]
    fn start_view_config_keeps_safe_recents_and_caps_them() {
        let json = r#"{
            "items": [],
            "recents": [
                { "id": "local:a.docx", "title": "a.docx", "subtitle": "1分前", "name": "a.docx", "badge": "DOCX", "action": "open-recent:local:a.docx" },
                { "id": "github:acme/docs@main:b.md", "title": "b.md", "subtitle": "acme/docs · 昨日", "name": "b.md", "action": "open-recent:github:acme/docs@main:b.md" },
                { "id": "bad", "title": "x", "name": "x", "action": "javascript:alert(1)" },
                { "id": "empty", "title": "", "name": "z.md", "action": "open-recent:empty" }
            ]
        }"#;
        set_start_view_config(json).expect("valid config");
        let cfg = start_view_config().expect("stored");
        assert_eq!(cfg.recents.len(), 2);
        assert_eq!(cfg.recents[0].id, "local:a.docx");
        assert!(is_open_recent_action(&cfg.recents[0].action));
        assert_eq!(cfg.recents[1].name, "b.md");
        assert!(!is_open_recent_action("javascript:alert(1)"));
        assert!(!is_open_recent_action("open-recent:"));

        let many = (0..12)
            .map(|i| format!(
                r#"{{ "id": "local:{i}.md", "title": "{i}.md", "name": "{i}.md", "action": "open-recent:local:{i}.md" }}"#
            ))
            .collect::<Vec<_>>()
            .join(",");
        set_start_view_config(&format!(r#"{{ "recents": [{many}] }}"#)).expect("cap");
        assert_eq!(start_view_config().expect("stored").recents.len(), 5);

        set_start_view_config("").expect("clear");
    }
}

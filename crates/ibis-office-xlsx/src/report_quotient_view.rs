// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use gpui_kit::*;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use serde_json::Value;

use super::{QueryReportPanelExtPrivate as _, QuotientFilterView, saved_check_text};
use crate::ext::*;
use crate::report_quotient::{ExclusionReason, RankingDirection, RankingPage};
use crate::shell::chrome::action_button;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::{format, text};

pub(super) fn signed_contribution(value: &Value) -> String {
    value.as_f64().map_or_else(|| "—".to_owned(), |number| {
        if number != 0.0 && !(0.000001..1e9).contains(&number.abs()) {
            format!("{number:+.6e}")
        } else {
            format!("{number:+.6}").trim_end_matches('0').trim_end_matches('.').to_owned()
        }
    })
}

pub(super) fn quotient_preview(
    comparison: &Value, filter: Option<&QuotientFilterView>, page: Option<&(RankingPage, Value)>, index: usize, selected: Option<usize>, generation: u64, cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    let rate = comparison["kind"] == "rate";
    let unit = if rate { text("query.report-quotient-pp").to_string() }
        else { comparison["unit"].as_str().unwrap_or_default().to_owned() };
    let heading = format("query.report-quotient-heading", &[
        ("table", &comparison["table"].as_str().unwrap_or_default()),
        ("metric", &comparison["metric"].as_str().unwrap_or_default()),
    ]);
    let mut section = v_flex().gap_1().child(div().text_sm().font_weight(FontWeight::BOLD).child(heading));
    let decomposition = &comparison["decomposition"];
    let ranking = &decomposition["ranking"];
    let Some(_) = ranking["groups"].as_array() else {
        return section.child(div().text_sm().child(text("query.report-quotient-unavailable")));
    };
    if decomposition["state"] == "totals_mismatch" {
        section = section.child(div().text_sm().child(text("query.report-quotient-mismatch")));
    }
    let delta = &comparison[if rate { "percentage_point_delta" } else { "delta" }];
    section = section.child(div().text_sm().child(saved_check_text(format("query.report-quotient-total", &[
        ("count", &ranking["group_count"]), ("total", &signed_contribution(delta)), ("unit", &unit),
    ]))));
    let mut controls = h_flex().gap_1();
    let direction = filter.map_or(RankingDirection::All, |filter| filter.criteria.direction);
    for (id, choice) in [("all", RankingDirection::All), ("increase", RankingDirection::Increase), ("decrease", RankingDirection::Decrease)] {
        let button = action_button(SharedString::from(format!("query-report-quotient-{id}-{index}")))
            .small().label(text(&format!("query.report-quotient-{id}")))
            .on_click(cx.listener(move |editor, _, _, cx| {
                editor.filter_quotient_ranking(index, generation, Some(choice), cx);
            }));
        controls = controls.child(if choice == direction { button.primary() } else { button });
    }
    section = section.child(controls);
    if let Some(input) = filter.and_then(|filter| filter.input.as_ref()) {
        section = section.child(h_flex().gap_1()
            .child(div().flex_1().min_w_0().child(Input::new(input).small()))
            .child(action_button(SharedString::from(format!("query-report-quotient-search-submit-{index}")))
                .small().label(text("query.report-quotient-search-submit"))
                .on_click(cx.listener(move |editor, _, _, cx| editor.filter_quotient_ranking(index, generation, None, cx)))));
    } else {
        section = section.child(action_button(SharedString::from(format!("query-report-quotient-search-{index}")))
            .small().label(text("query.report-quotient-search"))
            .on_click(cx.listener(move |editor, _, window, cx| editor.open_quotient_group_search(index, generation, window, cx))));
    }
    let excluded_page = page.and_then(|(kind, page)| matches!(kind, RankingPage::Excluded(..)).then_some(page));
    let page = page.and_then(|(kind, page)| matches!(kind, RankingPage::Remainder(_)).then_some(page));
    let ranking = if let Some(filtered) = filter.and_then(|filter| filter.ranking.as_ref()) {
        if filtered["state"] != "ready" && decomposition["state"] == "ready" {
            section = section.child(div().text_sm().child(text("query.report-quotient-mismatch")));
        }
        section = section.child(div().text_sm().child(saved_check_text(format("query.report-quotient-matched", &[
            ("count", &filtered["group_count"]), ("total", &signed_contribution(&filtered["total_contribution"])), ("unit", &unit),
        ]))));
        let excluded = &filtered["excluded"];
        let label = format("query.report-quotient-excluded", &[
            ("count", &excluded["count"]), ("total", &signed_contribution(&excluded["contribution"])), ("unit", &unit),
        ]);
        if excluded["count"].as_u64().unwrap_or_default() > 0 {
            let expanded = excluded_page.is_some_and(|page| page["reason"].is_null());
            let button = action_button(SharedString::from(format!("query-report-quotient-excluded-{index}")))
                .small().label(format!("{} {label}", if expanded { "▾" } else { "▸" }))
                .on_click(cx.listener(move |editor, _, _, cx| {
                    editor.set_quotient_page(index, generation, (!expanded).then_some(RankingPage::Excluded(0, None)), cx);
                }));
            #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
            let button = crate::shell::scenario::button_probe::observe_control(
                div().id(SharedString::from(format!("quotient-excluded-probe-{index}"))).child(button),
                SharedString::from(format!("query-report-check-text:{label}")).into(),
            );
            section = section.child(button);
        } else {
            section = section.child(div().text_sm().child(saved_check_text(label)));
        }
        for reason in ExclusionReason::ALL {
            let key = reason.key();
            let summary = &excluded["by_reason"][key];
            let label = format(&format!("query.report-quotient-subtotal-{key}"), &[
                ("count", &summary["count"]), ("total", &signed_contribution(&summary["contribution"])), ("unit", &unit),
            ]);
            if summary["count"].as_u64().unwrap_or_default() == 0 {
                section = section.child(div().pl_3().text_sm().child(saved_check_text(label)));
                continue;
            }
            let expanded = excluded_page.is_some_and(|page| page["reason"] == key);
            let button = action_button(SharedString::from(format!("query-report-quotient-excluded-{key}-{index}")))
                .small().label(format!("{} {label}", if expanded { "▾" } else { "▸" }))
                .on_click(cx.listener(move |editor, _, _, cx| {
                    editor.set_quotient_page(index, generation, (!expanded).then_some(RankingPage::Excluded(0, Some(reason))), cx);
                }));
            #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
            let button = crate::shell::scenario::button_probe::observe_control(
                div().id(SharedString::from(format!("quotient-excluded-probe-{key}-{index}"))).child(button),
                SharedString::from(format!("query-report-check-text:{label}")).into(),
            );
            section = section.child(div().pl_3().child(button));
        }
        if filtered["state"] == "ready" {
            section = section.child(div().text_sm().child(text("query.report-quotient-filter-reconciled")));
        }
        filtered
    } else {
        ranking
    };
    let groups = ranking["groups"].as_array().expect("quotient ranking groups");
    if groups.is_empty() && excluded_page.is_none() {
        return section.child(div().text_sm().child(text("query.report-quotient-no-matches")));
    }
    let remainder = &ranking["remainder"];
    let page_groups = page.and_then(|page| page["groups"].as_array()).into_iter().flatten();
    let remainder_count = remainder["count"].as_u64().unwrap_or_default();
    if remainder_count > 0 {
        section = section.child(div().text_sm().child(saved_check_text(format("query.report-quotient-top", &[
            ("count", &groups.len()), ("total", &signed_contribution(&ranking["top_contribution"])), ("unit", &unit),
        ]))));
    }
    let excluded_groups = excluded_page.and_then(|page| page["groups"].as_array()).map_or(&[][..], Vec::as_slice);
    let excluded_offset = excluded_page.and_then(|page| page["offset"].as_u64()).unwrap_or_default() as usize;
    let normal_count = ranking["group_count"].as_u64().unwrap_or_default() as usize;
    let page_offset = page.and_then(|page| page["offset"].as_u64()).unwrap_or_default() as usize;
    for (position, group) in excluded_groups.iter().chain(groups).chain(page_groups).enumerate() {
        if position == excluded_groups.len() && let Some(page) = excluded_page {
            section = section.child(quotient_page_navigation(page, true, index, generation, cx));
        }
        let is_excluded = position < excluded_groups.len();
        let position = if is_excluded { position } else { position - excluded_groups.len() };
        if !is_excluded && position == groups.len() {
            section = section.child(quotient_remainder_button(remainder, true, &unit, index, generation, cx));
        }
        let row = if is_excluded { normal_count + excluded_offset + position }
            else if position < groups.len() { position } else { position + page_offset };
        let key = group["key"].as_str().unwrap_or_default();
        let rank = if is_excluded { excluded_offset + position + 1 } else { row + 1 };
        let total = signed_contribution(&group["contribution"]);
        let label = format("query.report-quotient-row", &[("rank", &rank), ("key", &key), ("total", &total), ("unit", &unit)]);
        let id = SharedString::from(format!("query-report-quotient-{index}-{row}"));
        let button = action_button(id).small().label(label.clone()).on_click(cx.listener(move |editor, _, _, cx| {
            if let Some(review) = editor.query_reports_mut().filter(|panel| panel.generation == generation)
                .and_then(|panel| panel.review.as_mut())
            {
                if review.quotient_selection.get(&index) == Some(&row) {
                    review.quotient_selection.remove(&index);
                } else {
                    review.quotient_selection.insert(index, row);
                }
                cx.notify();
            }
        }));
        #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
        let button = crate::shell::scenario::button_probe::observe_control(div().id(SharedString::from(format!("quotient-row-probe-{index}-{row}"))).child(button), SharedString::from(format!("query-report-quotient-row:{label}")).into());
        section = section.child(button).child(div().text_sm().child(saved_check_text(format("query.report-quotient-parts", &[
            ("within", &signed_contribution(&group["within_group"])),
            ("composition", &signed_contribution(&group["composition"])),
        ]))));
        if is_excluded {
            let reason = match (group["excluded_by"]["direction"] == true, group["excluded_by"]["name"] == true) {
                (true, true) => "query.report-quotient-excluded-both",
                (true, false) => "query.report-quotient-excluded-direction",
                _ => "query.report-quotient-excluded-name",
            };
            section = section.child(div().text_sm().child(saved_check_text(text(reason).to_string())));
        }
        if selected == Some(row) {
            let fields = if rate { ["numerator", "denominator"] } else { ["sum", "count"] };
            for (side, side_label) in [("before", "query.report-quotient-before"), ("after", "query.report-quotient-after")] {
                let evidence = &group[side];
                section = section.child(div().text_sm().font_weight(FontWeight::BOLD).child(text(side_label)));
                let Some(sheet) = evidence["sheet"].as_str() else {
                    section = section.child(div().text_sm().child(text("query.report-quotient-absent")));
                    continue;
                };
                for field in fields {
                    let Some(cell) = evidence[field]["cell"].as_str() else { continue };
                    let name = format!("query.report-quotient-{field}");
                    let value = &evidence[field]["value"];
                    let value = if value.is_null() { "—".to_owned() } else { value.to_string() };
                    let line = format("query.report-quotient-evidence", &[
                        ("field", &text(&name)), ("sheet", &sheet), ("cell", &cell), ("value", &value),
                    ]);
                    if side == "before" {
                        let reference = format!("'{}'!{cell}", sheet.replace('\'', "''"));
                        let button = action_button(SharedString::from(format!("query-report-quotient-cell-{index}-{field}")))
                            .small().label(line.clone()).on_click(cx.listener(move |editor, _, window, cx| {
                                editor.go_to_reference(&reference, window, cx);
                            }));
                        #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
                        let button = crate::shell::scenario::button_probe::observe_control(div().id(SharedString::from(format!("quotient-evidence-probe-{index}-{field}"))).child(button), SharedString::from(format!("query-report-quotient-evidence:{line}")).into());
                        section = section.child(button);
                    } else {
                        section = section.child(div().text_sm().child(saved_check_text(line)));
                    }
                }
            }
        }
    }
    if groups.is_empty() && let Some(page) = excluded_page {
        section = section.child(quotient_page_navigation(page, true, index, generation, cx));
        section = section.child(div().text_sm().child(text("query.report-quotient-no-matches")));
    }
    if remainder_count > 0 && page.is_none() {
        section = section.child(quotient_remainder_button(remainder, false, &unit, index, generation, cx));
    }
    if let Some(page) = page {
        section = section.child(quotient_page_navigation(page, false, index, generation, cx));
    }
    section
}

fn quotient_page_navigation(
    page: &Value, excluded: bool, index: usize, generation: u64, cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    let page_offset = page["offset"].as_u64().unwrap_or_default() as usize;
    let size = page["page_size"].as_u64().unwrap_or_default() as usize;
    let next = page["next_offset"].as_u64().map(|offset| offset as usize);
    let kind = if excluded { "excluded-" } else { "" };
    let reason = page["reason"].as_str().and_then(|key| ExclusionReason::ALL.into_iter().find(|reason| reason.key() == key));
    let mut controls = h_flex().gap_1();
    for (action, offset) in [
        ("previous", (page_offset > 0).then(|| page_offset.saturating_sub(size))),
        ("next", next),
    ] {
        if let Some(offset) = offset {
            controls = controls.child(action_button(SharedString::from(format!("query-report-quotient-{kind}{action}-{index}")))
                .small().label(text(&format!("query.report-quotient-{action}")))
                .on_click(cx.listener(move |editor, _, _, cx| {
                    let page = if excluded { RankingPage::Excluded(offset, reason) } else { RankingPage::Remainder(offset) };
                    editor.set_quotient_page(index, generation, Some(page), cx);
                })));
        }
    }
    controls
}

fn quotient_remainder_button(
    remainder: &Value, expanded: bool, unit: &str, index: usize, generation: u64, cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    let label = format(if expanded { "query.report-quotient-remainder-expanded" } else { "query.report-quotient-remainder" }, &[
        ("count", &remainder["count"]), ("total", &signed_contribution(&remainder["contribution"])), ("unit", &unit),
    ]);
    let button = action_button(SharedString::from(format!("query-report-quotient-remainder-{index}")))
        .small().label(label.clone()).on_click(cx.listener(move |editor, _, _, cx| {
            editor.set_quotient_page(index, generation, (!expanded).then_some(RankingPage::Remainder(0)), cx);
        }));
    #[cfg(all(feature = "scenarios", any(target_os = "macos", target_os = "linux")))]
    let button = crate::shell::scenario::button_probe::observe_control(
        div().id(SharedString::from(format!("quotient-remainder-probe-{index}"))).child(button),
        SharedString::from(format!("query-report-quotient-remainder:{label}")).into(),
    );
    button
}

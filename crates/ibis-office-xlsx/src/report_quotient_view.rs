// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use gpui_kit::*;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};

use super::{QueryReportPanelExtPrivate as _, QuotientFilterView, QuotientPreview, saved_check_text};
use crate::ext::*;
use crate::report_quotient::{CheckState, ExclusionReason, Metric, RankPage, RankingDirection, RankingPage, Subtotal};
use crate::shell::chrome::action_button;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::{format, text};

pub(super) fn signed_contribution(value: Option<f64>) -> String {
    value.filter(|number| number.is_finite()).map_or_else(|| "—".to_owned(), |number| {
        if number != 0.0 && !(0.000001..1e9).contains(&number.abs()) {
            format!("{number:+.6e}")
        } else {
            format!("{number:+.6}").trim_end_matches('0').trim_end_matches('.').to_owned()
        }
    })
}

pub(super) fn quotient_preview(
    comparison: &QuotientPreview, filter: Option<&QuotientFilterView>, page: Option<&(RankingPage, RankPage)>, index: usize, selected: Option<usize>, generation: u64, cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    let heading = format("query.report-quotient-heading", &[
        ("table", &comparison.table), ("metric", &comparison.metric),
    ]);
    let mut section = v_flex().gap_1().child(div().text_sm().font_weight(FontWeight::BOLD).child(heading));
    let (Ok(analysis), Some(rankings)) = (&comparison.analysis, &comparison.summary) else {
        return section.child(div().text_sm().child(text("query.report-quotient-unavailable")));
    };
    if rankings.state == CheckState::NonFinite {
        return section.child(div().text_sm().child(text("query.report-quotient-unavailable")));
    }
    if rankings.state == CheckState::Mismatch {
        section = section.child(div().text_sm().child(text("query.report-quotient-mismatch")));
    }
    let unit = if matches!(analysis.kind, Metric::Rate) { text("query.report-quotient-pp").to_string() }
        else { analysis.unit.clone().unwrap_or_default() };
    let ranking = &rankings.ranking;
    section = section.child(div().text_sm().child(saved_check_text(format("query.report-quotient-total", &[
        ("count", &ranking.group_count), ("total", &signed_contribution(analysis.delta)), ("unit", &unit),
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
        if filtered.state != CheckState::Ready && rankings.state == CheckState::Ready {
            section = section.child(div().text_sm().child(text("query.report-quotient-mismatch")));
        }
        section = section.child(div().text_sm().child(saved_check_text(format("query.report-quotient-matched", &[
            ("count", &filtered.ranking.group_count), ("total", &signed_contribution(Some(filtered.total_contribution))), ("unit", &unit),
        ]))));
        let excluded = &filtered.excluded;
        let label = format("query.report-quotient-excluded", &[
            ("count", &excluded.count), ("total", &signed_contribution(Some(excluded.contribution))), ("unit", &unit),
        ]);
        if excluded.count > 0 {
            let expanded = excluded_page.is_some_and(|page| page.reason.is_none());
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
            let summary = &filtered.by_reason[reason as usize];
            let label = format(&format!("query.report-quotient-subtotal-{key}"), &[
                ("count", &summary.count), ("total", &signed_contribution(Some(summary.contribution))), ("unit", &unit),
            ]);
            if summary.count == 0 {
                section = section.child(div().pl_3().text_sm().child(saved_check_text(label)));
                continue;
            }
            let expanded = excluded_page.is_some_and(|page| page.reason == Some(reason));
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
        if filtered.state == CheckState::Ready {
            section = section.child(div().text_sm().child(text("query.report-quotient-filter-reconciled")));
        }
        &filtered.ranking
    } else {
        ranking
    };
    let groups = &ranking.groups;
    if groups.is_empty() && excluded_page.is_none() {
        return section.child(div().text_sm().child(text("query.report-quotient-no-matches")));
    }
    let remainder = &ranking.remainder;
    let page_groups = page.into_iter().flat_map(|page| &page.groups);
    let remainder_count = remainder.count;
    if remainder_count > 0 {
        section = section.child(div().text_sm().child(saved_check_text(format("query.report-quotient-top", &[
            ("count", &groups.len()), ("total", &signed_contribution(Some(ranking.top_contribution))), ("unit", &unit),
        ]))));
    }
    let excluded_groups = excluded_page.map_or(&[][..], |page| page.groups.as_slice());
    let excluded_offset = excluded_page.map_or(0, |page| page.offset);
    let normal_count = ranking.group_count;
    let page_offset = page.map_or(0, |page| page.offset);
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
        let key = &group.key;
        let rank = if is_excluded { excluded_offset + position + 1 } else { row + 1 };
        let total = signed_contribution(Some(group.contribution));
        let label = format("query.report-quotient-row", &[("rank", &rank), ("key", &key), ("total", &total), ("unit", &unit)]);
        let id = SharedString::from(format!("query-report-quotient-{index}-{row}"));
        let button = action_button(id).small().label(label.clone()).on_click(cx.listener(move |editor, _, _, cx| {
            if let Some(review) = editor.query_reports_mut().filter(|panel| panel.preview.generation == generation)
                .and_then(|panel| panel.preview.review_mut())
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
            ("within", &signed_contribution(Some(group.within_group))),
            ("composition", &signed_contribution(Some(group.composition))),
        ]))));
        if let Some(reason) = group.reason {
            let reason = match reason {
                ExclusionReason::Both => "query.report-quotient-excluded-both",
                ExclusionReason::Direction => "query.report-quotient-excluded-direction",
                ExclusionReason::Name => "query.report-quotient-excluded-name",
            };
            section = section.child(div().text_sm().child(saved_check_text(text(reason).to_string())));
        }
        if selected == Some(row) {
            let (_, _, left, right) = analysis.kind.fields();
            for (before, evidence, side_label) in [
                (true, &group.before, "query.report-quotient-before"), (false, &group.after, "query.report-quotient-after"),
            ] {
                section = section.child(div().text_sm().font_weight(FontWeight::BOLD).child(text(side_label)));
                let Some((sheet, evidence)) = evidence.as_ref().and_then(|evidence| Some((evidence.sheet.as_deref()?, evidence))) else {
                    section = section.child(div().text_sm().child(text("query.report-quotient-absent")));
                    continue;
                };
                for (field, input) in [left, right].into_iter().zip(&evidence.cells) {
                    let cell = &input.address;
                    let name = format!("query.report-quotient-{field}");
                    let value = input.value.map_or_else(|| "—".to_owned(), |value| format!("{value:?}"));
                    let line = format("query.report-quotient-evidence", &[
                        ("field", &text(&name)), ("sheet", &sheet), ("cell", &cell), ("value", &value),
                    ]);
                    if before {
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
    page: &RankPage, excluded: bool, index: usize, generation: u64, cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    let page_offset = page.offset;
    let size = page.page_size;
    let next = page.next_offset;
    let kind = if excluded { "excluded-" } else { "" };
    let reason = page.reason;
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
    remainder: &Subtotal, expanded: bool, unit: &str, index: usize, generation: u64, cx: &mut Context<SlideEditor>,
) -> impl IntoElement {
    let label = format(if expanded { "query.report-quotient-remainder-expanded" } else { "query.report-quotient-remainder" }, &[
        ("count", &remainder.count), ("total", &signed_contribution(Some(remainder.contribution))), ("unit", &unit),
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

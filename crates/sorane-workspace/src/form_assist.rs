// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Phase 1 Right-Dock Form Assist panel for `SoraneWorkspace`.
//! Connects Obsidian-style Vault profiles (`type: profile`), received VCs (`.sd-jwt` / `.mdoc`),
//! and form mappings (`type: form-mapping` / `DefinedName` / heuristic drafts) to active spreadsheets.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_formats::Kind;
use ibis_formats::form_assist::{
    FieldProvenance, FieldStatus, FormMapping, VaultCredentialSummary, VaultProfile,
    build_form_apply_command, current_date_ymd, evaluate_form_mapping,
    infer_draft_form_mapping_from_sheet, starter_profile_markdown,
    synthesize_form_mapping_from_sheet,
};
use ibis_office_shell::window_editors::WorkspaceEditorHost as _;

use crate::workspace::SoraneWorkspace;

#[derive(Clone, Debug, Default)]
pub struct FormAssistState {
    /// Selected profile path (`None` = use all Vault profiles).
    pub selected_profile_path: Option<String>,
    /// Selected form-mapping path (`None` = auto-detect best match).
    pub selected_mapping_path: Option<String>,
    /// Allow overwriting occupied cells when applying candidates.
    pub allow_overwrite: bool,
    /// Last action feedback message `(message, is_error)`.
    pub status_message: Option<(String, bool)>,
}

enum ResolvedSheetMapping {
    Vault(FormMapping),
    Synthesized(FormMapping),
    HeuristicDraft(FormMapping),
    None,
}

impl SoraneWorkspace {
    fn resolve_active_sheet_mapping(
        &self,
        presentation: &ibis_doc_model::Presentation,
        slide: ibis_doc_model::SlideId,
    ) -> ResolvedSheetMapping {
        if let Some(vault) = &self.vault {
            if let Some(selected_path) = &self.form_assist.selected_mapping_path
                && let Some(found) = vault
                    .vault_form_mappings()
                    .iter()
                    .find(|m| &m.path == selected_path)
            {
                return ResolvedSheetMapping::Vault(found.clone());
            }
            let matched = vault.matching_form_mappings_for_sheet(presentation, slide, None);
            if let Some(best) = matched.into_iter().next() {
                return ResolvedSheetMapping::Vault(best);
            }
        }
        if let Some(synth) = synthesize_form_mapping_from_sheet(presentation, slide) {
            return ResolvedSheetMapping::Synthesized(synth);
        }
        if let Some((draft, _)) = infer_draft_form_mapping_from_sheet(presentation, slide) {
            return ResolvedSheetMapping::HeuristicDraft(draft);
        }
        ResolvedSheetMapping::None
    }

    fn selected_profiles<'a>(&self, all: &'a [VaultProfile]) -> Vec<&'a VaultProfile> {
        if let Some(path) = &self.form_assist.selected_profile_path {
            let filtered: Vec<&VaultProfile> = all.iter().filter(|p| &p.path == path).collect();
            if !filtered.is_empty() {
                return filtered;
            }
        }
        all.iter().collect()
    }

    pub(crate) fn create_starter_profile(
        &mut self,
        scope: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(not(target_family = "wasm"))]
        {
            let Some(vault) = &self.vault else {
                self.form_assist.status_message = Some(("先に Vault フォルダを開いてください。".into(), true));
                cx.notify();
                return;
            };
            let (rel, content) = starter_profile_markdown(scope);
            let target = non_clobbering_path(vault.root(), rel);
            if let Some(parent) = target.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                self.form_assist.status_message =
                    Some((format!("フォルダ作成に失敗しました: {err}"), true));
                cx.notify();
                return;
            }
            if let Err(err) = std::fs::write(&target, content) {
                self.form_assist.status_message =
                    Some((format!("プロフィール作成に失敗しました: {err}"), true));
                cx.notify();
                return;
            }
            self.refresh_vault_folder(window, cx);
            if let Some(vault) = &self.vault
                && let Some(rel_created) = vault.relative_path_for(&target)
            {
                self.form_assist.selected_profile_path = Some(rel_created);
            }
            self.form_assist.status_message =
                Some((format!("サンプルプロフィールを作成しました: {}", target.file_name().unwrap_or_default().to_string_lossy()), false));
            self.open_path_in_split_pane(target, window, cx);
            cx.notify();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (scope, window);
            self.form_assist.status_message =
                Some(("プロフィール自動作成はデスクトップ版で利用できます。".into(), true));
            cx.notify();
        }
    }

    pub(crate) fn save_inferred_form_mapping(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(not(target_family = "wasm"))]
        {
            let Some(vault) = self.vault.clone() else {
                self.form_assist.status_message =
                    Some(("先に Vault フォルダを開いてください。".into(), true));
                cx.notify();
                return;
            };
            let Some(editor) = self.active_editor(cx) else {
                return;
            };
            let inferred = {
                let ed = editor.read(cx);
                infer_draft_form_mapping_from_sheet(ed.presentation(), ed.current_slide)
            };
            let Some((mapping, markdown)) = inferred else {
                self.form_assist.status_message = Some((
                    "シートから自動検出できる入力欄が見つかりませんでした。".into(),
                    true,
                ));
                cx.notify();
                return;
            };
            let target = non_clobbering_path(vault.root(), &mapping.path);
            if let Some(parent) = target.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                self.form_assist.status_message =
                    Some((format!("forms/ フォルダ作成に失敗しました: {err}"), true));
                cx.notify();
                return;
            }
            if let Err(err) = std::fs::write(&target, markdown) {
                self.form_assist.status_message =
                    Some((format!("フォーム対応表の保存に失敗しました: {err}"), true));
                cx.notify();
                return;
            }
            self.refresh_vault_folder(window, cx);
            if let Some(vault) = &self.vault
                && let Some(rel_created) = vault.relative_path_for(&target)
            {
                self.form_assist.selected_mapping_path = Some(rel_created);
            }
            self.form_assist.status_message = Some((
                format!(
                    "フォーム対応表を保存しました（{} 項目）",
                    mapping.fields.len()
                ),
                false,
            ));
            self.open_path_in_split_pane(target, window, cx);
            cx.notify();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = window;
            self.form_assist.status_message =
                Some(("フォーム対応表の保存はデスクトップ版で利用できます。".into(), true));
            cx.notify();
        }
    }

    pub(crate) fn apply_form_assist_to_active_sheet(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor(cx) else {
            return;
        };
        let Some(vault) = self.vault.clone() else {
            self.form_assist.status_message =
                Some(("Vault を開くと自動入力できます。".into(), true));
            cx.notify();
            return;
        };

        let profiles = self.selected_profiles(vault.vault_profiles());
        let credentials: Vec<&VaultCredentialSummary> = vault.vault_credentials().iter().collect();
        let today = current_date_ymd();
        let allow_overwrite = self.form_assist.allow_overwrite;

        let build_result = {
            let ed = editor.read(cx);
            if ed.is_read_only() || ed.is_protected() {
                Err("読み取り専用または保護されたシートには適用できません。".to_string())
            } else {
                let slide = ed.current_slide;
                let mapping = match self.resolve_active_sheet_mapping(ed.presentation(), slide) {
                    ResolvedSheetMapping::Vault(m)
                    | ResolvedSheetMapping::Synthesized(m)
                    | ResolvedSheetMapping::HeuristicDraft(m) => Some(m),
                    ResolvedSheetMapping::None => None,
                };
                if let Some(mapping) = mapping {
                    let eval = evaluate_form_mapping(
                        ed.presentation(),
                        slide,
                        &mapping,
                        &profiles,
                        &credentials,
                        today.as_deref(),
                    );
                    build_form_apply_command(
                        ed.presentation(),
                        slide,
                        &mapping,
                        &eval,
                        None,
                        allow_overwrite,
                    )
                } else {
                    Err("適用できるフォーム対応表がありません。".to_string())
                }
            }
        };

        match build_result {
            Ok((cmd, applied_count)) => {
                editor.update(cx, |ed, cx| {
                    ed.execute(vec![cmd], cx);
                });
                self.form_assist.status_message = Some((
                    format!(
                        "{applied_count} 項目をシートに適用しました（Ctrl/Cmd+Z で元に戻せます）"
                    ),
                    false,
                ));
            }
            Err(err) => {
                self.form_assist.status_message = Some((err, true));
            }
        }
        cx.notify();
    }

    pub(crate) fn render_form_assist_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let mut root = v_flex().gap_2().w_full();

        let Some(vault) = &self.vault else {
            return root
                .child(
                    div()
                        .p_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("Vault フォルダを開くと、profiles/*.md（自社・個人プロフィール）と credentials/*.sd-jwt / *.mdoc からスプレッドシートの申請書を 1 クリックで自動入力できます。"),
                )
                .child(
                    Button::new("ws-form-open-vault")
                        .primary()
                        .xsmall()
                        .label("Vault フォルダを開く…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.pick_vault_folder(window, cx);
                        })),
                )
                .into_any_element();
        };

        let all_profiles = vault.vault_profiles();
        let all_credentials = vault.vault_credentials();
        let all_mappings = vault.vault_form_mappings();

        // Asset summary strip
        root = root.child(
            h_flex()
                .gap(px(6.))
                .flex_wrap()
                .items_center()
                .px_1()
                .child(
                    div()
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(theme.muted)
                        .text_size(px(10.5))
                        .text_color(theme.foreground)
                        .child(format!("プロフィール {}", all_profiles.len())),
                )
                .child(
                    div()
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(theme.muted)
                        .text_size(px(10.5))
                        .text_color(theme.foreground)
                        .child(format!("保有 VC {}", all_credentials.len())),
                )
                .child(
                    div()
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(theme.muted)
                        .text_size(px(10.5))
                        .text_color(theme.foreground)
                        .child(format!("対応表 {}", all_mappings.len())),
                ),
        );

        // Starter profile creation buttons when no profile exists yet
        if all_profiles.is_empty() {
            root = root.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("まずはプロフィールを作成"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted_foreground)
                            .child("自社や個人の基本情報を Vault に置くと、あらゆる申請書・請求書へ 1 クリック入力できます。"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .pt_1()
                            .child(
                                Button::new("ws-form-create-company-profile")
                                    .primary()
                                    .xsmall()
                                    .label("+ 自社プロフィール")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.create_starter_profile("organization", window, cx);
                                    })),
                            )
                            .child(
                                Button::new("ws-form-create-person-profile")
                                    .xsmall()
                                    .label("+ 個人プロフィール")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.create_starter_profile("person", window, cx);
                                    })),
                            ),
                    ),
            );
        } else {
            // Profile selector pills
            let mut pill_row = h_flex().gap(px(4.)).flex_wrap().items_center();
            let all_selected = self.form_assist.selected_profile_path.is_none();
            pill_row = pill_row.child(
                div()
                    .id("ws-form-prof-all")
                    .cursor_pointer()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .text_size(px(10.5))
                    .when(all_selected, |d| {
                        d.bg(theme.accent)
                            .text_color(theme.accent_foreground)
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!all_selected, |d| {
                        d.bg(theme.muted)
                            .text_color(theme.muted_foreground)
                            .hover(|h| h.bg(theme.secondary))
                    })
                    .child("すべて")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.form_assist.selected_profile_path = None;
                        cx.notify();
                    })),
            );
            for (idx, prof) in all_profiles.iter().enumerate() {
                let is_sel = self
                    .form_assist
                    .selected_profile_path
                    .as_deref()
                    == Some(prof.path.as_str());
                let path_for_select = prof.path.clone();
                let full_for_edit = vault.root().join(&prof.path);
                pill_row = pill_row.child(
                    div()
                        .id(SharedString::from(format!("ws-form-prof:{idx}")))
                        .cursor_pointer()
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .text_size(px(10.5))
                        .when(is_sel, |d| {
                            d.bg(theme.accent)
                                .text_color(theme.accent_foreground)
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(!is_sel, |d| {
                            d.bg(theme.muted)
                                .text_color(theme.foreground)
                                .hover(|h| h.bg(theme.secondary))
                        })
                        .child(prof.title.clone())
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            if event.modifiers().platform || event.modifiers().control {
                                this.open_path_in_split_pane(full_for_edit.clone(), window, cx);
                            } else {
                                this.form_assist.selected_profile_path =
                                    Some(path_for_select.clone());
                                cx.notify();
                            }
                        })),
                );
            }
            root = root.child(pill_row);
        }

        // Check active tab kind
        let active_editor = self.active_editor(cx);
        let active_is_workbook = active_editor
            .as_ref()
            .is_some_and(|ed| Kind::of(ed.read(cx).presentation()) == Kind::Workbook);

        if active_is_workbook && let Some(editor) = active_editor {
            let selected_profiles = self.selected_profiles(all_profiles);
            let cred_refs: Vec<&VaultCredentialSummary> = all_credentials.iter().collect();
            let today = current_date_ymd();
            let allow_overwrite = self.form_assist.allow_overwrite;

            let (resolved, evaluation_opt) = {
                let ed = editor.read(cx);
                let slide = ed.current_slide;
                let resolved = self.resolve_active_sheet_mapping(ed.presentation(), slide);
                let eval = match &resolved {
                    ResolvedSheetMapping::Vault(m)
                    | ResolvedSheetMapping::Synthesized(m)
                    | ResolvedSheetMapping::HeuristicDraft(m) => Some(evaluate_form_mapping(
                        ed.presentation(),
                        slide,
                        m,
                        &selected_profiles,
                        &cred_refs,
                        today.as_deref(),
                    )),
                    ResolvedSheetMapping::None => None,
                };
                (resolved, eval)
            };

            match (&resolved, evaluation_opt) {
                (ResolvedSheetMapping::None, _) => {
                    root = root.child(
                        div()
                            .p_2()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.background)
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("このシートに一致するフォーム対応表・名前付きセル・標準見出しが見つかりませんでした。forms/*.form-map.md を作成するか、セルに名前（または見出し）を付けてください。"),
                    );
                }
                (mode, Some(eval)) => {
                    let (mapping_title, mode_badge, edit_vault_path, is_draft) = match mode {
                        ResolvedSheetMapping::Vault(m) => (
                            m.title.clone(),
                            "Vault 対応表",
                            Some(vault.root().join(&m.path)),
                            false,
                        ),
                        ResolvedSheetMapping::Synthesized(m) => {
                            (m.title.clone(), "名前付きセル", None, false)
                        }
                        ResolvedSheetMapping::HeuristicDraft(m) => {
                            (m.title.clone(), "見出し自動検出", None, true)
                        }
                        ResolvedSheetMapping::None => unreachable!(),
                    };

                    // Mapping header card
                    let mut header_card = v_flex()
                        .gap_1()
                        .p_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .truncate()
                                        .child(mapping_title),
                                )
                                .child(
                                    div()
                                        .px(px(5.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .bg(theme.muted)
                                        .text_size(px(10.))
                                        .text_color(theme.muted_foreground)
                                        .child(mode_badge),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(format!(
                                    "入力候補 {} · 入力済み {} · VC由来 {} 件",
                                    eval.ready_count, eval.occupied_count, eval.vc_backed_count
                                )),
                        );

                    if eval.missing_required_count > 0 || eval.overflow_count > 0 {
                        header_card = header_card.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(rgb(0xC62828))
                                .child(format!(
                                    "要確認: 必須未入力 {} · 字数超過 {}",
                                    eval.missing_required_count, eval.overflow_count
                                )),
                        );
                    }

                    let applicable = eval.ready_count
                        + if allow_overwrite {
                            eval.occupied_count
                        } else {
                            0
                        };
                    let mut action_row = h_flex().gap_1().flex_wrap().pt_1().child(
                        Button::new("ws-form-apply-all")
                            .primary()
                            .xsmall()
                            .label(format!("一括適用 ({applicable} 件)"))
                            .disabled(applicable == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.apply_form_assist_to_active_sheet(cx);
                            })),
                    );
                    if eval.occupied_count > 0 {
                        action_row = action_row.child(
                            Button::new("ws-form-toggle-overwrite")
                                .xsmall()
                                .label(if allow_overwrite {
                                    "上書き: ON"
                                } else {
                                    "上書き: OFF"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.form_assist.allow_overwrite =
                                        !this.form_assist.allow_overwrite;
                                    cx.notify();
                                })),
                        );
                    }
                    if is_draft {
                        action_row = action_row.child(
                            Button::new("ws-form-save-draft-map")
                                .xsmall()
                                .label("対応表を Vault に保存")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_inferred_form_mapping(window, cx);
                                })),
                        );
                    } else if let Some(map_path) = edit_vault_path {
                        action_row = action_row.child(
                            Button::new("ws-form-edit-map")
                                .xsmall()
                                .label("対応表を編集")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_path_in_split_pane(map_path.clone(), window, cx);
                                })),
                        );
                    }
                    header_card = header_card.child(action_row);
                    root = root.child(header_card);

                    // Field plan cards
                    for (idx, field) in eval.fields.iter().enumerate() {
                        let status_label = match field.status {
                            FieldStatus::Ready => "入力候補",
                            FieldStatus::Occupied => "入力済み",
                            FieldStatus::MissingSource => "手入力 / 未取得",
                            FieldStatus::BlockedTarget => "保護・不可",
                            FieldStatus::UnsupportedValue => "形式不一致",
                        };
                        let prov_badge = match &field.provenance {
                            Some(FieldProvenance::Credential {
                                format,
                                verified_digests,
                                ..
                            }) => {
                                let mark = if *verified_digests { "🛡" } else { "⚠" };
                                Some((format!("VC:{format} {mark}"), rgb(0x1B5E20), rgb(0xE8F5E9)))
                            }
                            Some(FieldProvenance::Profile { title, .. }) => {
                                Some((format!("Vault:{title}"), rgb(0x0D47A1), rgb(0xE3F2FD)))
                            }
                            Some(FieldProvenance::DefaultValue) => {
                                Some(("既定値".to_string(), rgb(0x4E342E), rgb(0xEFEBE9)))
                            }
                            None => None,
                        };

                        let mut card = v_flex()
                            .id(SharedString::from(format!("ws-form-field:{idx}")))
                            .p(px(6.))
                            .gap(px(3.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(if field.exceeds_max_chars {
                                Hsla::from(rgb(0xD32F2F))
                            } else {
                                theme.border
                            })
                            .bg(theme.background)
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .truncate()
                                            .child(format!(
                                                "{}{} ({})",
                                                field.label,
                                                if field.required { " *" } else { "" },
                                                field.target_summary
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(theme.muted_foreground)
                                            .child(status_label),
                                    ),
                            );

                        if let Some((badge_text, fg, bg)) = prov_badge {
                            card = card.child(
                                div()
                                    .px(px(5.))
                                    .py(px(1.))
                                    .rounded(px(3.))
                                    .bg(bg)
                                    .text_size(px(10.))
                                    .text_color(fg)
                                    .child(badge_text),
                            );
                        }

                        if !field.current_display.is_empty() {
                            card = card.child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(format!("現在: {}", field.current_display)),
                            );
                        }
                        if let Some(proposed) = &field.proposed_display {
                            card = card.child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.foreground)
                                    .truncate()
                                    .child(format!("候補: {proposed}")),
                            );
                        }
                        if let Some(limit) = field.max_chars {
                            card = card.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(if field.exceeds_max_chars {
                                        Hsla::from(rgb(0xC62828))
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .child(format!("文字数: {} / {limit}", field.char_count)),
                            );
                        }
                        root = root.child(card);
                    }
                }
                _ => {}
            }
        } else {
            // Non-workbook active tab: show Vault Form Assets overview
            root = root.child(
                div()
                    .p_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Excel (.xlsx) 等のスプレッドシートを開くと、Vault のプロフィールと保有 VC から自動で入力候補が表示されます。"),
            );

            if !all_profiles.is_empty() {
                root = root.child(
                    div()
                        .px_1()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child("プロフィール（クリックで開く）"),
                );
                for (idx, prof) in all_profiles.iter().enumerate() {
                    let full = vault.root().join(&prof.path);
                    root = root.child(
                        div()
                            .id(SharedString::from(format!("ws-form-asset-prof:{idx}")))
                            .cursor_pointer()
                            .px(px(6.))
                            .py(px(4.))
                            .rounded(px(4.))
                            .hover(|h| h.bg(theme.secondary))
                            .text_xs()
                            .child(format!("{}（{} 項目）", prof.title, prof.fields.len()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_path_in_active_pane(full.clone(), window, cx);
                            })),
                    );
                }
            }

            if !all_credentials.is_empty() {
                root = root.child(
                    div()
                        .px_1()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child("保有 VC（.sd-jwt / .mdoc）"),
                );
                for (idx, cred) in all_credentials.iter().enumerate() {
                    let full = vault.root().join(&cred.path);
                    root = root.child(
                        div()
                            .id(SharedString::from(format!("ws-form-asset-cred:{idx}")))
                            .cursor_pointer()
                            .px(px(6.))
                            .py(px(4.))
                            .rounded(px(4.))
                            .hover(|h| h.bg(theme.secondary))
                            .text_xs()
                            .child(format!(
                                "🛡 [{}] {}（{} クレーム）",
                                cred.format,
                                cred.path,
                                cred.claims.len()
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_path_in_active_pane(full.clone(), window, cx);
                            })),
                    );
                }
            }

            if !all_mappings.is_empty() {
                root = root.child(
                    div()
                        .px_1()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child("フォーム対応表（forms/）"),
                );
                for (idx, map) in all_mappings.iter().enumerate() {
                    let full = vault.root().join(&map.path);
                    root = root.child(
                        div()
                            .id(SharedString::from(format!("ws-form-asset-map:{idx}")))
                            .cursor_pointer()
                            .px(px(6.))
                            .py(px(4.))
                            .rounded(px(4.))
                            .hover(|h| h.bg(theme.secondary))
                            .text_xs()
                            .child(format!("{}（{} 項目）", map.title, map.fields.len()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_path_in_active_pane(full.clone(), window, cx);
                            })),
                    );
                }
            }
        }

        if let Some((msg, is_err)) = &self.form_assist.status_message {
            root = root.child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(if *is_err {
                        rgb(0xC62828)
                    } else {
                        rgb(0x2E7D32)
                    })
                    .child(msg.clone()),
            );
        }

        root.into_any_element()
    }
}

#[cfg(not(target_family = "wasm"))]
fn non_clobbering_path(root: &std::path::Path, rel: &str) -> PathBuf {
    let base = root.join(rel);
    if !base.exists() {
        return base;
    }
    let parent = base.parent().unwrap_or(root);
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("form");
    let ext = base.extension().and_then(|e| e.to_str()).unwrap_or("md");
    for n in 2..=99 {
        let cand = parent.join(format!("{stem}-{n}.{ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    base
}

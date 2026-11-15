// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! 開発 / 書式 > SD-JWT の生成とプレビュー・OID4VP 提示パネル.
//! 初心者向けのシンプルな3ステップ（1. ひな形を作る → 2. 見せる項目を選ぶ → 3. 出力する）を
//! 基本画面とし、発行者設定・任意範囲バインド・OID4VP 提示は折りたたみ可能な詳細セクションにまとめる。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use ibis_doc_model::{
    BoundClaim, CellRange, Es256KeyPair, KeyBindingJwtOutput, Oid4vpRequest,
    ParsedCredentialFormat, SdJwtOptions, SdJwtOutput, SdJwtSchema, SdJwtTemplateKind,
    SheetClaimBindings, bind_range_as_sd_claim_commands, build_sd_jwt_template_commands,
    extract_sheet_claims, issue_sd_jwt, parse_mdoc_text, parse_sd_jwt,
};

use crate::ext::*;
use crate::shell::chrome::SidebarPanel;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::{format as tr_format, text};
use crate::shell::palette::rgb;

pub(crate) struct SdJwtPanel {
    pub(crate) iss: Entity<InputState>,
    pub(crate) vct: Entity<InputState>,
    pub(crate) issuer_jwk: Entity<InputState>,
    pub(crate) bind_path: Entity<InputState>,
    pub(crate) schema_json: Entity<InputState>,
    pub(crate) import_token: Entity<InputState>,
    pub(crate) oid4vp_request: Entity<InputState>,
    pub(crate) include_cnf: bool,
    pub(crate) show_import: bool,
    pub(crate) show_advanced: bool,
    pub(crate) bindings: SheetClaimBindings,
    pub(crate) output: Option<SdJwtOutput>,
    pub(crate) kb_output: Option<KeyBindingJwtOutput>,
    pub(crate) message: String,
    pub(crate) is_error: bool,
    pub(crate) ui: Rc<RefCell<BTreeMap<String, Bounds<Pixels>>>>,
}

fn format_bound_claim_line(c: &BoundClaim) -> String {
    let mode = if c.selective {
        if c.disclose {
            text("xlsx.sd-jwt.tag-disclose")
        } else {
            text("xlsx.sd-jwt.tag-hidden")
        }
    } else {
        text("xlsx.sd-jwt.tag-fixed")
    };
    format!("{mode} {} = {}", c.path_str, c.value.to_json())
}

#[ibis_office_macros::editor_ext(SdJwtPanelExt)]
impl SlideEditor {
    pub fn open_sd_jwt_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_cell_edit(cx);
        let iss = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value("https://issuer.example.com", window, cx);
            s
        });
        let vct = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(
                "https://credentials.example.com/identity_credential",
                window,
                cx,
            );
            s
        });
        let issuer_jwk = cx.new(|cx| InputState::new(window, cx));
        let bind_path = cx.new(|cx| InputState::new(window, cx));
        let schema_json = cx.new(|cx| InputState::new(window, cx));
        let import_token = cx.new(|cx| InputState::new(window, cx));
        let oid4vp_request = cx.new(|cx| InputState::new(window, cx));
        let slide = self.current_slide;
        let bindings = extract_sheet_claims(self.presentation(), slide).unwrap_or(
            SheetClaimBindings {
                slide,
                claims: Vec::new(),
                disclosures: BTreeMap::new(),
            },
        );
        let holder_key = Es256KeyPair::demo_holder_key();
        let mut options = SdJwtOptions::default();
        options.holder_public_jwk = Some(holder_key.to_public_jwk());
        let sd_out = issue_sd_jwt(&bindings, &options, &Es256KeyPair::demo_issuer_key());
        let kb_out = sd_out.attach_key_binding(
            "https://verifier.example.com",
            "n-0S6_WzA2Mj",
            1_700_000_000,
            &holder_key,
        );

        let panel = SdJwtPanel {
            iss,
            vct,
            issuer_jwk,
            bind_path,
            schema_json,
            import_token,
            oid4vp_request,
            include_cnf: true,
            show_import: false,
            show_advanced: false,
            bindings,
            output: Some(sd_out),
            kb_output: Some(kb_out),
            message: String::new(),
            is_error: false,
            ui: Default::default(),
        };
        let state = self.wb_mut();
        state.close_settings();
        state.panels.sd_jwt = Some(panel);
        self.sidebar_open = true;
        cx.notify();
    }

    pub(crate) fn regenerate_sd_jwt(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.sheet_state().and_then(|s| s.panels.sd_jwt.as_ref()) else {
            return;
        };
        let iss_val = p.iss.read(cx).value().trim().to_string();
        let vct_val = p.vct.read(cx).value().trim().to_string();
        let jwk_val = p.issuer_jwk.read(cx).value().trim().to_string();
        let oid4vp_val = p.oid4vp_request.read(cx).value().trim().to_string();
        let include_cnf = p.include_cnf;

        let key = if jwk_val.is_empty() {
            Es256KeyPair::demo_issuer_key()
        } else {
            match Es256KeyPair::from_jwk_json(&jwk_val) {
                Some(k) => k,
                None => {
                    if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
                        panel.message = text("xlsx.sd-jwt.invalid-jwk").to_string();
                        panel.is_error = true;
                    }
                    cx.notify();
                    return;
                }
            }
        };

        let parsed_req = Oid4vpRequest::parse(&oid4vp_val);
        let (aud, nonce) = match &parsed_req {
            Some(r) => (r.client_id.clone(), r.nonce.clone()),
            None => (
                "https://verifier.example.com".to_string(),
                "n-0S6_WzA2Mj".to_string(),
            ),
        };

        let slide = self.current_slide;
        let bindings = extract_sheet_claims(self.presentation(), slide).unwrap_or(
            SheetClaimBindings {
                slide,
                claims: Vec::new(),
                disclosures: BTreeMap::new(),
            },
        );
        let holder_key = Es256KeyPair::demo_holder_key();
        let mut options = SdJwtOptions::default();
        options.default_iss = if iss_val.is_empty() {
            None
        } else {
            Some(iss_val)
        };
        options.default_vct = if vct_val.is_empty() {
            None
        } else {
            Some(vct_val)
        };
        options.holder_public_jwk = if include_cnf {
            Some(holder_key.to_public_jwk())
        } else {
            None
        };
        let output = issue_sd_jwt(&bindings, &options, &key);
        let kb_output = output.attach_key_binding(&aud, &nonce, 1_700_000_000, &holder_key);
        if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
            panel.bindings = bindings;
            panel.output = Some(output);
            panel.kb_output = Some(kb_output);
            panel.message.clear();
            panel.is_error = false;
        }
        cx.notify();
    }

    pub(crate) fn apply_sd_jwt_template(
        &mut self,
        kind: SdJwtTemplateKind,
        cx: &mut Context<Self>,
    ) {
        self.commit_cell_edit(cx);
        let slide = self.current_slide;
        let cmds = build_sd_jwt_template_commands(self.presentation(), slide, kind);
        if !cmds.is_empty() {
            self.run_sheet(cmds, cx);
        }
        self.regenerate_sd_jwt(cx);
        if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
            panel.message = text("xlsx.sd-jwt.template-applied").to_string();
            panel.is_error = false;
        }
        cx.notify();
    }

    pub(crate) fn infer_sd_jwt_schema(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_cell_edit(cx);
        let slide = self.current_slide;
        let Some(schema) = SdJwtSchema::infer_from_sheet(self.presentation(), slide, None) else {
            if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
                panel.message = text("xlsx.sd-jwt.no-claims").to_string();
                panel.is_error = true;
            }
            cx.notify();
            return;
        };
        let json = schema.to_json_pretty();
        let schema_entity = self
            .sheet_state()
            .and_then(|s| s.panels.sd_jwt.as_ref())
            .map(|p| p.schema_json.clone());
        if let Some(entity) = schema_entity {
            entity.update(cx, |s, cx| s.set_value(json, window, cx));
        }
        if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
            panel.message = text("xlsx.sd-jwt.schema-inferred").to_string();
            panel.is_error = false;
        }
        cx.notify();
    }

    pub(crate) fn apply_sd_jwt_schema(&mut self, cx: &mut Context<Self>) {
        self.commit_cell_edit(cx);
        let Some(p) = self.sheet_state().and_then(|s| s.panels.sd_jwt.as_ref()) else {
            return;
        };
        let raw_json = p.schema_json.read(cx).value().to_string();
        let schema = match SdJwtSchema::from_json(&raw_json) {
            Ok(s) => s,
            Err(err) => {
                if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
                    panel.message = err;
                    panel.is_error = true;
                }
                cx.notify();
                return;
            }
        };
        let slide = self.current_slide;
        let cmds = schema.build_sheet_commands(self.presentation(), slide);
        if !cmds.is_empty() {
            self.run_sheet(cmds, cx);
        }
        self.regenerate_sd_jwt(cx);
        if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
            panel.message = text("xlsx.sd-jwt.schema-applied").to_string();
            panel.is_error = false;
        }
        cx.notify();
    }

    pub(crate) fn import_sd_jwt_or_mdoc(&mut self, cx: &mut Context<Self>) {
        self.commit_cell_edit(cx);
        let Some(p) = self.sheet_state().and_then(|s| s.panels.sd_jwt.as_ref()) else {
            return;
        };
        let raw = p.import_token.read(cx).value().trim().to_string();
        let jwk_raw = p.issuer_jwk.read(cx).value().trim().to_string();
        let jwk_opt = (!jwk_raw.is_empty()).then_some(jwk_raw.as_str());

        let parsed_res = if raw.contains('~') || raw.contains('.') {
            parse_sd_jwt(&raw, jwk_opt)
        } else {
            parse_mdoc_text(&raw, jwk_opt)
        };

        let parsed = match parsed_res {
            Ok(p) => p,
            Err(err) => {
                if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
                    panel.message = err;
                    panel.is_error = true;
                }
                cx.notify();
                return;
            }
        };

        let slide = self.current_slide;
        let cmds = parsed.build_sheet_commands(self.presentation(), slide);
        if !cmds.is_empty() {
            self.run_sheet(cmds, cx);
        }
        self.regenerate_sd_jwt(cx);

        let fmt_str = match parsed.format {
            ParsedCredentialFormat::SdJwt => "SD-JWT",
            ParsedCredentialFormat::Mdoc => "mDOC",
        };
        let verified_str = parsed.digests_verified.to_string();
        let total_str = parsed.digests_total.to_string();
        let sig_str = match parsed.signature_valid {
            Some(true) => "OK",
            Some(false) => "INVALID",
            None => "N/A",
        };
        let msg = tr_format(
            "xlsx.sd-jwt.import-ok",
            &[
                ("format", &fmt_str),
                ("verified", &verified_str),
                ("total", &total_str),
                ("sig", &sig_str),
            ],
        );
        if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
            panel.message = msg;
            panel.is_error = parsed.signature_valid == Some(false);
        }
        cx.notify();
    }

    pub(crate) fn bind_selection_to_sd_jwt(&mut self, selective: bool, cx: &mut Context<Self>) {
        self.commit_cell_edit(cx);
        let Some(p) = self.sheet_state().and_then(|s| s.panels.sd_jwt.as_ref()) else {
            return;
        };
        let mut raw_path = p.bind_path.read(cx).value().trim().to_string();
        let range = self.selected_range().or_else(|| {
            let (col, row) = self.current_cell()?;
            Some(CellRange {
                first_row: row,
                first_column: col,
                last_row: row,
                last_column: col,
            })
        });
        let Some(range) = range else {
            if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
                panel.message = text("xlsx.sd-jwt.bind-error").to_string();
                panel.is_error = true;
            }
            cx.notify();
            return;
        };

        // If `bind_path` is left empty and there is a label in the cell to the left, infer from it.
        if raw_path.is_empty() && range.first_column > 0 {
            if let Some((_, sheet)) = self.current_sheet()
                && let Some(left_cell) = sheet.cell(range.first_row, range.first_column - 1)
            {
                let t = left_cell.text.trim();
                if let (Some(open), Some(close)) = (t.rfind('('), t.rfind(')'))
                    && open + 1 < close
                {
                    raw_path = t[open + 1..close].trim().to_string();
                } else if !t.is_empty() {
                    raw_path = t.to_string();
                }
            }
        }

        let slide = self.current_slide;
        let Some((full_name, cmds)) =
            bind_range_as_sd_claim_commands(self.presentation(), slide, range, &raw_path, selective)
        else {
            if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
                panel.message = text("xlsx.sd-jwt.bind-error").to_string();
                panel.is_error = true;
            }
            cx.notify();
            return;
        };

        self.run_sheet(cmds, cx);
        self.regenerate_sd_jwt(cx);
        let msg = tr_format("xlsx.sd-jwt.bound-status", &[("name", &full_name)]);
        if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
            panel.message = msg;
            panel.is_error = false;
        }
        cx.notify();
    }

    pub(crate) fn toggle_sd_jwt_claim_disclose(&mut self, path_str: &str, cx: &mut Context<Self>) {
        let slide = self.current_slide;
        if let Some(bindings) = extract_sheet_claims(self.presentation(), slide) {
            let cmds = bindings.toggle_claim_disclose_commands(self.presentation(), path_str);
            if !cmds.is_empty() {
                self.run_sheet(cmds, cx);
            }
        }
        self.regenerate_sd_jwt(cx);
    }

    pub(crate) fn apply_oid4vp_request(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.sheet_state().and_then(|s| s.panels.sd_jwt.as_ref()) else {
            return;
        };
        let raw_req = p.oid4vp_request.read(cx).value().trim().to_string();
        let Some(req) = Oid4vpRequest::parse(&raw_req) else {
            if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
                panel.message = text("xlsx.sd-jwt.invalid-oid4vp").to_string();
                panel.is_error = true;
            }
            cx.notify();
            return;
        };

        let slide = self.current_slide;
        if let Some(bindings) = extract_sheet_claims(self.presentation(), slide)
            && !req.requested_claims.is_empty()
        {
            let req_refs: Vec<&str> = req.requested_claims.iter().map(String::as_str).collect();
            let cmds = bindings.set_disclose_commands(self.presentation(), &req_refs, true);
            if !cmds.is_empty() {
                self.run_sheet(cmds, cx);
            }
        }

        self.regenerate_sd_jwt(cx);
        let count_str = req.requested_claims.len().to_string();
        let applied_msg = tr_format(
            "xlsx.sd-jwt.oid4vp-applied",
            &[("aud", &req.client_id), ("count", &count_str)],
        );
        if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
            panel.message = applied_msg;
            panel.is_error = false;
        }
        cx.notify();
    }

    pub fn sd_jwt_panel(&mut self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let p = self.sheet_state()?.panels.sd_jwt.as_ref()?;
        let include_cnf = p.include_cnf;
        let show_import = p.show_import;
        let show_advanced = p.show_advanced;
        let total_claims = p.bindings.claims.len();
        let disclosed_count = p
            .bindings
            .claims
            .iter()
            .filter(|c| !c.selective || c.disclose)
            .count();
        let claims: Vec<(String, bool, bool, String)> = p
            .bindings
            .claims
            .iter()
            .map(|c| {
                (
                    c.path_str.clone(),
                    c.selective,
                    c.disclose,
                    format_bound_claim_line(c),
                )
            })
            .collect();
        let presentation_token = p
            .output
            .as_ref()
            .map(|o| o.sd_jwt_presentation.clone())
            .unwrap_or_default();
        let full_token = p
            .output
            .as_ref()
            .map(|o| o.sd_jwt_all.clone())
            .unwrap_or_default();
        let payload_json = p
            .output
            .as_ref()
            .map(|o| o.payload_json.clone())
            .unwrap_or_default();
        let disclosed_json_pretty = p
            .output
            .as_ref()
            .map(|o| o.disclosed_claims.to_json_pretty())
            .unwrap_or_default();
        let vp_token = p
            .kb_output
            .as_ref()
            .map(|k| k.vp_token.clone())
            .unwrap_or_default();
        let message = p.message.clone();
        let is_error = p.is_error;

        let mark = |id: &str, child: AnyElement| {
            let ui = p.ui.clone();
            let id = id.to_string();
            div()
                .relative()
                .child(child)
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, _, _| {
                            ui.borrow_mut().insert(id.clone(), bounds);
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .into_any_element()
        };

        let mut body = v_flex()
            .gap_2()
            .px(px(16.))
            .py(px(8.))
            .p_0()
            // Step 1: Template scaffolding
            .child(div().text_sm().child(text("xlsx.sd-jwt.step1")))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        crate::shell::chrome::action_button("sd-jwt-template-pid")
                            .small()
                            .label(text("xlsx.sd-jwt.template-pid"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.apply_sd_jwt_template(SdJwtTemplateKind::Pid, cx)
                            })),
                    )
                    .child(
                        crate::shell::chrome::action_button("sd-jwt-template-mdl")
                            .small()
                            .label(text("xlsx.sd-jwt.template-mdl"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.apply_sd_jwt_template(SdJwtTemplateKind::Iso18013Mdl, cx)
                            })),
                    )
                    .child(
                        crate::shell::chrome::action_button("sd-jwt-toggle-import")
                            .small()
                            .label(text(if show_import {
                                "xlsx.sd-jwt.import-hide"
                            } else {
                                "xlsx.sd-jwt.import-toggle"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                                    panel.show_import = !panel.show_import;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        crate::shell::chrome::action_button("sd-jwt-open-assistant")
                            .small()
                            .label("Assistant で相談...")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.wb_mut().panels.sd_jwt = None;
                                this.open_ai_assistant_panel(window, cx);
                            })),
                    ),
            );

        if show_import {
            body = body
                .child(div().text_xs().child(text("xlsx.sd-jwt.import-input")))
                .child(mark(
                    "import_token",
                    crate::shell::chrome::text_field(&p.import_token)
                        .full_width()
                        .into_any_element(),
                ))
                .child(
                    crate::shell::chrome::action_button("sd-jwt-import-apply")
                        .small()
                        .label(text("xlsx.sd-jwt.import-apply"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.import_sd_jwt_or_mdoc(cx)
                        })),
                );
        }

        // Step 2: Interactive disclosure list
        body = body.child(div().text_sm().child(text("xlsx.sd-jwt.step2")));

        if claims.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(rgb(0x666666))
                    .child(text("xlsx.sd-jwt.empty-guide")),
            );
        } else {
            let disclosed_str = disclosed_count.to_string();
            let total_str = total_claims.to_string();
            let summary_line = tr_format(
                "xlsx.sd-jwt.disclosed-count",
                &[("disclosed", &disclosed_str), ("total", &total_str)],
            );
            body = body.child(
                div()
                    .text_xs()
                    .text_color(rgb(0x2E7D32))
                    .child(SharedString::from(summary_line)),
            );

            let mut list = v_flex().gap_1();
            for (idx, (path_str, selective, disclose, line)) in claims.into_iter().enumerate() {
                if selective {
                    let btn_id = SharedString::from(format!("sd-jwt-claim-toggle-{idx}"));
                    let btn = crate::shell::chrome::action_button(btn_id)
                        .small()
                        .label(SharedString::from(line))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_sd_jwt_claim_disclose(&path_str, cx);
                        }));
                    list = list.child(if disclose { btn.primary() } else { btn });
                } else {
                    list = list.child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x555555))
                            .child(SharedString::from(line)),
                    );
                }
            }
            body = body.child(list);

            // Step 3: Export & human-readable JSON preview
            body = body
                .child(div().text_sm().child(text("xlsx.sd-jwt.step3")))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x555555))
                        .child(text("xlsx.sd-jwt.disclosed-preview")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x333333))
                        .child(SharedString::from(disclosed_json_pretty)),
                );

            let pres_copy = presentation_token.clone();
            let vp_step3_copy = vp_token.clone();
            body = body.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        crate::shell::chrome::action_button("sd-jwt-copy-presentation")
                            .primary()
                            .label(text("xlsx.sd-jwt.copy-sd-jwt"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.regenerate_sd_jwt(cx);
                                let token = this
                                    .sheet_state()
                                    .and_then(|s| s.panels.sd_jwt.as_ref())
                                    .and_then(|p| p.output.as_ref())
                                    .map(|o| o.sd_jwt_presentation.clone())
                                    .unwrap_or_else(|| pres_copy.clone());
                                cx.write_to_clipboard(ClipboardItem::new_string(token));
                                if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                                    panel.message = text("xlsx.sd-jwt.copied").to_string();
                                    panel.is_error = false;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        crate::shell::chrome::action_button("sd-jwt-copy-vp-step3")
                            .small()
                            .label(text("xlsx.sd-jwt.copy-vp-short"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.regenerate_sd_jwt(cx);
                                let token = this
                                    .sheet_state()
                                    .and_then(|s| s.panels.sd_jwt.as_ref())
                                    .and_then(|p| p.kb_output.as_ref())
                                    .map(|k| k.vp_token.clone())
                                    .unwrap_or_else(|| vp_step3_copy.clone());
                                cx.write_to_clipboard(ClipboardItem::new_string(token));
                                if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                                    panel.message = text("xlsx.sd-jwt.copied").to_string();
                                    panel.is_error = false;
                                }
                                cx.notify();
                            })),
                    ),
            );
        }

        // Collapsible Advanced & OID4VP section
        body = body.child(
            crate::shell::chrome::action_button("sd-jwt-toggle-advanced")
                .small()
                .label(text(if show_advanced {
                    "xlsx.sd-jwt.advanced-hide"
                } else {
                    "xlsx.sd-jwt.advanced-show"
                }))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                        panel.show_advanced = !panel.show_advanced;
                    }
                    cx.notify();
                })),
        );

        if show_advanced {
            let full_copy = full_token.clone();
            let payload_copy = payload_json.clone();
            let vp_copy = vp_token.clone();

            body = body
                .child(div().text_sm().child(text("xlsx.sd-jwt.bind-selection")))
                .child(mark(
                    "bind_path",
                    crate::shell::chrome::text_field(&p.bind_path)
                        .full_width()
                        .into_any_element(),
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            crate::shell::chrome::action_button("sd-jwt-bind-sd")
                                .small()
                                .label(text("xlsx.sd-jwt.bind-sd"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.bind_selection_to_sd_jwt(true, cx)
                                })),
                        )
                        .child(
                            crate::shell::chrome::action_button("sd-jwt-bind-claim")
                                .small()
                                .label(text("xlsx.sd-jwt.bind-claim"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.bind_selection_to_sd_jwt(false, cx)
                                })),
                        ),
                )
                .child(div().text_sm().child(text("xlsx.sd-jwt.iss")))
                .child(mark(
                    "iss",
                    crate::shell::chrome::text_field(&p.iss)
                        .full_width()
                        .into_any_element(),
                ))
                .child(div().text_sm().child(text("xlsx.sd-jwt.vct")))
                .child(mark(
                    "vct",
                    crate::shell::chrome::text_field(&p.vct)
                        .full_width()
                        .into_any_element(),
                ))
                .child(div().text_sm().child(text("xlsx.sd-jwt.issuer-jwk")))
                .child(mark(
                    "issuer_jwk",
                    crate::shell::chrome::text_field(&p.issuer_jwk)
                        .full_width()
                        .into_any_element(),
                ))
                .child(mark(
                    "include_cnf",
                    {
                        let btn = crate::shell::chrome::action_button("sd-jwt-include-cnf")
                            .small()
                            .label(text("xlsx.sd-jwt.include-cnf"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                                    panel.include_cnf = !panel.include_cnf;
                                }
                                this.regenerate_sd_jwt(cx);
                            }));
                        if include_cnf { btn.primary() } else { btn }.into_any_element()
                    },
                ))
                .child(div().text_sm().child(text("xlsx.sd-jwt.oid4vp-request")))
                .child(mark(
                    "oid4vp_request",
                    crate::shell::chrome::text_field(&p.oid4vp_request)
                        .full_width()
                        .into_any_element(),
                ))
                .child(
                    crate::shell::chrome::action_button("sd-jwt-apply-oid4vp")
                        .small()
                        .label(text("xlsx.sd-jwt.apply-oid4vp"))
                        .on_click(cx.listener(|this, _, _, cx| this.apply_oid4vp_request(cx))),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            crate::shell::chrome::action_button("sd-jwt-copy-vp-token")
                                .small()
                                .label(text("xlsx.sd-jwt.copy-vp-token"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.regenerate_sd_jwt(cx);
                                    let token = this
                                        .sheet_state()
                                        .and_then(|s| s.panels.sd_jwt.as_ref())
                                        .and_then(|p| p.kb_output.as_ref())
                                        .map(|k| k.vp_token.clone())
                                        .unwrap_or_else(|| vp_copy.clone());
                                    cx.write_to_clipboard(ClipboardItem::new_string(token));
                                    if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                                        panel.message = text("xlsx.sd-jwt.copied").to_string();
                                        panel.is_error = false;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            crate::shell::chrome::action_button("sd-jwt-copy-all")
                                .small()
                                .label(text("xlsx.sd-jwt.copy-all"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.regenerate_sd_jwt(cx);
                                    let token = this
                                        .sheet_state()
                                        .and_then(|s| s.panels.sd_jwt.as_ref())
                                        .and_then(|p| p.output.as_ref())
                                        .map(|o| o.sd_jwt_all.clone())
                                        .unwrap_or_else(|| full_copy.clone());
                                    cx.write_to_clipboard(ClipboardItem::new_string(token));
                                    if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                                        panel.message = text("xlsx.sd-jwt.copied").to_string();
                                        panel.is_error = false;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            crate::shell::chrome::action_button("sd-jwt-copy-payload")
                                .small()
                                .label(text("xlsx.sd-jwt.copy-payload"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.regenerate_sd_jwt(cx);
                                    let payload = this
                                        .sheet_state()
                                        .and_then(|s| s.panels.sd_jwt.as_ref())
                                        .and_then(|p| p.output.as_ref())
                                        .map(|o| o.payload_json.clone())
                                        .unwrap_or_else(|| payload_copy.clone());
                                    cx.write_to_clipboard(ClipboardItem::new_string(payload));
                                    if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                                        panel.message = text("xlsx.sd-jwt.copied").to_string();
                                        panel.is_error = false;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().text_sm().child(text("xlsx.sd-jwt.schema-section")))
                .child(mark(
                    "schema_json",
                    crate::shell::chrome::text_field(&p.schema_json)
                        .full_width()
                        .into_any_element(),
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            crate::shell::chrome::action_button("sd-jwt-infer-schema")
                                .small()
                                .label(text("xlsx.sd-jwt.infer-schema"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.infer_sd_jwt_schema(window, cx)
                                })),
                        )
                        .child(
                            crate::shell::chrome::action_button("sd-jwt-apply-schema")
                                .small()
                                .label(text("xlsx.sd-jwt.apply-schema"))
                                .on_click(cx.listener(|this, _, _, cx| this.apply_sd_jwt_schema(cx))),
                        )
                        .child(
                            crate::shell::chrome::action_button("sd-jwt-copy-schema")
                                .small()
                                .label(text("xlsx.sd-jwt.copy-schema"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let raw = this
                                        .sheet_state()
                                        .and_then(|s| s.panels.sd_jwt.as_ref())
                                        .map(|p| p.schema_json.read(cx).value().to_string())
                                        .unwrap_or_default();
                                    cx.write_to_clipboard(ClipboardItem::new_string(raw));
                                    if let Some(panel) = this.wb_mut().panels.sd_jwt.as_mut() {
                                        panel.message = text("xlsx.sd-jwt.copied").to_string();
                                        panel.is_error = false;
                                    }
                                    cx.notify();
                                })),
                        ),
                );
        }

        if !message.is_empty() {
            let color = if is_error {
                rgb(0xD9534F)
            } else {
                rgb(0x2E7D32)
            };
            body = body.child(
                div()
                    .text_sm()
                    .text_color(color)
                    .child(SharedString::from(message)),
            );
        }

        let actions = crate::shell::chrome::action_bar()
            .action(
                crate::shell::chrome::ActionDefinition::new(
                    "sd-jwt-generate",
                    "",
                    crate::shell::chrome::ActionButtonKind::Primary,
                )
                .label(text("xlsx.sd-jwt.generate"))
                .on_click(cx.listener(|this, _, _, cx| this.regenerate_sd_jwt(cx))),
            )
            .action(crate::shell::chrome::dialog_cancel_definition(cx));

        Some(SidebarPanel::with_footer(
            text("xlsx.sd-jwt.title"),
            body.into_any_element(),
            actions,
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_caption_is_in_both_languages() {
        for table in [
            include_str!("../../ibis-office-base/i18n/ja/xlsx.tsv"),
            include_str!("../../ibis-office-base/i18n/en/xlsx.tsv"),
        ] {
            for id in [
                "xlsx.sd-jwt",
                "xlsx.sd-jwt.title",
                "xlsx.sd-jwt.iss",
                "xlsx.sd-jwt.vct",
                "xlsx.sd-jwt.include-cnf",
                "xlsx.sd-jwt.issuer-jwk",
                "xlsx.sd-jwt.claims-summary",
                "xlsx.sd-jwt.no-claims",
                "xlsx.sd-jwt.generate",
                "xlsx.sd-jwt.copy-presentation",
                "xlsx.sd-jwt.copy-all",
                "xlsx.sd-jwt.copy-payload",
                "xlsx.sd-jwt.copied",
                "xlsx.sd-jwt.invalid-jwk",
                "xlsx.sd-jwt.oid4vp-request",
                "xlsx.sd-jwt.apply-oid4vp",
                "xlsx.sd-jwt.copy-vp-token",
                "xlsx.sd-jwt.oid4vp-applied",
                "xlsx.sd-jwt.invalid-oid4vp",
                "xlsx.sd-jwt.templates",
                "xlsx.sd-jwt.template-pid",
                "xlsx.sd-jwt.template-mdl",
                "xlsx.sd-jwt.template-applied",
                "xlsx.sd-jwt.bind-selection",
                "xlsx.sd-jwt.bind-sd",
                "xlsx.sd-jwt.bind-claim",
                "xlsx.sd-jwt.bound-status",
                "xlsx.sd-jwt.bind-error",
                "xlsx.sd-jwt.step1",
                "xlsx.sd-jwt.step2",
                "xlsx.sd-jwt.step3",
                "xlsx.sd-jwt.empty-guide",
                "xlsx.sd-jwt.disclosed-count",
                "xlsx.sd-jwt.disclosed-preview",
                "xlsx.sd-jwt.copy-sd-jwt",
                "xlsx.sd-jwt.advanced-show",
                "xlsx.sd-jwt.advanced-hide",
                "xlsx.sd-jwt.tag-disclose",
                "xlsx.sd-jwt.tag-hidden",
                "xlsx.sd-jwt.tag-fixed",
                "xlsx.sd-jwt.schema-section",
                "xlsx.sd-jwt.infer-schema",
                "xlsx.sd-jwt.apply-schema",
                "xlsx.sd-jwt.copy-schema",
                "xlsx.sd-jwt.schema-inferred",
                "xlsx.sd-jwt.schema-applied",
                "xlsx.sd-jwt.import-toggle",
                "xlsx.sd-jwt.import-hide",
                "xlsx.sd-jwt.import-input",
                "xlsx.sd-jwt.import-apply",
                "xlsx.sd-jwt.import-ok",
                "xlsx.sd-jwt.copy-vp-short",
            ] {
                assert!(
                    table.lines().any(|l| l.split('\t').next() == Some(id)),
                    "{id} is missing"
                );
            }
        }
    }
}

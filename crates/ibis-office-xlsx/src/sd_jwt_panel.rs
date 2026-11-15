// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! 開発 / データ > SD-JWT の生成とプレビュー・OID4VP 提示パネル.
//! シート上のフォーム コントロールおよび定義された名前 (`claim.*`, `sd.*`, `disclose.*`) から
//! RFC 9901 SD-JWT および OID4VP `vp_token` (Key Binding JWT `kb+jwt` 付) を生成し、
//! テンプレートからのフォーム自動生成、選択範囲のワンクリック `sd.*` バインド、
//! OID4VP Authorization Request によるシート上 `disclose.*` チェックボックスの自動同期を行う。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use ibis_doc_model::{
    BoundClaim, CellRange, Es256KeyPair, KeyBindingJwtOutput, Oid4vpRequest, SdJwtOptions,
    SdJwtOutput, SdJwtTemplateKind, SheetClaimBindings, bind_range_as_sd_claim_commands,
    build_sd_jwt_template_commands, extract_sheet_claims, issue_sd_jwt,
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
    pub(crate) oid4vp_request: Entity<InputState>,
    pub(crate) include_cnf: bool,
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
            "[SD: ✓]"
        } else {
            "[SD: -]"
        }
    } else {
        "[Plain]"
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
            oid4vp_request,
            include_cnf: true,
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

        // If `bind_path` is left empty and there is a label in the cell to the left, infer from it!
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
        let claims: Vec<(String, bool, String)> = p
            .bindings
            .claims
            .iter()
            .map(|c| (c.path_str.clone(), c.selective, format_bound_claim_line(c)))
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
            .child(div().text_sm().child(text("xlsx.sd-jwt.templates")))
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
                    ),
            )
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
            .child(div().text_sm().child(text("xlsx.sd-jwt.claims-summary")));

        if claims.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(rgb(0x666666))
                    .child(text("xlsx.sd-jwt.no-claims")),
            );
        } else {
            let mut list = v_flex().gap_1();
            for (idx, (path_str, selective, line)) in claims.into_iter().enumerate() {
                if selective {
                    let btn_id = SharedString::from(format!("sd-jwt-claim-toggle-{idx}"));
                    list = list.child(
                        crate::shell::chrome::action_button(btn_id)
                            .small()
                            .label(SharedString::from(line))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_sd_jwt_claim_disclose(&path_str, cx);
                            })),
                    );
                } else {
                    list = list.child(div().text_xs().child(SharedString::from(line)));
                }
            }
            body = body.child(list);
        }

        if !payload_json.is_empty() {
            let preview = if presentation_token.len() > 180 {
                format!("{}…", &presentation_token[..180])
            } else {
                presentation_token.clone()
            };
            body = body.child(
                div()
                    .text_xs()
                    .text_color(rgb(0x333333))
                    .child(SharedString::from(preview)),
            );
        }

        let pres_copy = presentation_token.clone();
        let full_copy = full_token.clone();
        let payload_copy = payload_json.clone();
        let vp_copy = vp_token.clone();
        body = body.child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    crate::shell::chrome::action_button("sd-jwt-copy-presentation")
                        .small()
                        .label(text("xlsx.sd-jwt.copy-presentation"))
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
        );

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
            ] {
                assert!(
                    table.lines().any(|l| l.split('\t').next() == Some(id)),
                    "{id} is missing"
                );
            }
        }
    }
}

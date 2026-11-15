// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! 開発 / データ > SD-JWT の生成とプレビュー パネル.
//! シート上のフォーム コントロールおよび定義された名前 (`claim.*`, `sd.*`, `disclose.*`) から
//! RFC 9901 SD-JWT を生成し、選択的開示状態の確認とクリップボードへのコピーを行う。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use ibis_doc_model::{
    BoundClaim, Es256KeyPair, SdJwtOptions, SdJwtOutput, SheetClaimBindings, extract_sheet_claims,
    issue_sd_jwt,
};

use crate::ext::*;
use crate::shell::chrome::SidebarPanel;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::text;
use crate::shell::palette::rgb;

pub(crate) struct SdJwtPanel {
    pub(crate) iss: Entity<InputState>,
    pub(crate) vct: Entity<InputState>,
    pub(crate) issuer_jwk: Entity<InputState>,
    pub(crate) include_cnf: bool,
    pub(crate) bindings: SheetClaimBindings,
    pub(crate) output: Option<SdJwtOutput>,
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
        let slide = self.current_slide;
        let bindings = extract_sheet_claims(self.presentation(), slide).unwrap_or(
            SheetClaimBindings {
                slide,
                claims: Vec::new(),
                disclosures: BTreeMap::new(),
            },
        );
        let mut options = SdJwtOptions::default();
        options.holder_public_jwk = Some(Es256KeyPair::demo_holder_key().to_public_jwk());
        let output = Some(issue_sd_jwt(
            &bindings,
            &options,
            &Es256KeyPair::demo_issuer_key(),
        ));

        let panel = SdJwtPanel {
            iss,
            vct,
            issuer_jwk,
            include_cnf: true,
            bindings,
            output,
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

        let slide = self.current_slide;
        let bindings = extract_sheet_claims(self.presentation(), slide).unwrap_or(
            SheetClaimBindings {
                slide,
                claims: Vec::new(),
                disclosures: BTreeMap::new(),
            },
        );
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
            Some(Es256KeyPair::demo_holder_key().to_public_jwk())
        } else {
            None
        };
        let output = issue_sd_jwt(&bindings, &options, &key);
        if let Some(panel) = self.wb_mut().panels.sd_jwt.as_mut() {
            panel.bindings = bindings;
            panel.output = Some(output);
            panel.message.clear();
            panel.is_error = false;
        }
        cx.notify();
    }

    pub fn sd_jwt_panel(&mut self, cx: &mut Context<Self>) -> Option<SidebarPanel> {
        let p = self.sheet_state()?.panels.sd_jwt.as_ref()?;
        let include_cnf = p.include_cnf;
        let claims: Vec<String> = p
            .bindings
            .claims
            .iter()
            .map(format_bound_claim_line)
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
            for line in claims {
                list = list.child(div().text_xs().child(SharedString::from(line)));
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
            ] {
                assert!(
                    table.lines().any(|l| l.split('\t').next() == Some(id)),
                    "{id} is missing"
                );
            }
        }
    }
}

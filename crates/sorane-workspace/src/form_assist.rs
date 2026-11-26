// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Phase 1–3 Right-Dock Form Assist, VC Submission, and Verifier Batch BI panel for `SoraneWorkspace`.
//!
//! - **Fill mode (`入力`)**: connects Obsidian-style Vault profiles (`type: profile`) and received VCs
//!   (`.sd-jwt` / `.mdoc`) to **Spreadsheets (`.xlsx` / `.ods` / `.sheet.md`)**, **Documents
//!   (`.docx` content controls / legacy fields & `.md` `{{form:...}}`)**, and **PDF AcroForms (`.pdf`)**
//!   through a single unified IR and UI.
//! - **Structured Preview Reverse Links**: when viewing a `.sd-jwt` / `.mdoc` credential tab, offers
//!   1-click actions to jump to an open form for filling or present it via OID4VP.
//! - **Submit mode (`提出`)**: manages persistent desktop ES256 keys (outside Vault/Git), loads Vault
//!   `requests/*.req.md` or pasted `openid4vp://` requests, selectively re-presents Vault `.sd-jwt`
//!   credentials or active-sheet claims via `direct_post`, and records OKF `type: submission-receipt`
//!   notes with `[[Wiki links]]` under `submissions/`.
//! - **Verify mode (`検証`)**: batch-verifies all `.sd-jwt` / `.mdoc` credentials in the Vault against
//!   optional `requests/*.req.md` policies & Issuer public JWKs, and exports live formula-driven
//!   `.sheet.md` BI workbooks (`SORANE.JSONTABLE`, `COUNTIFS`, `SORANE.FIELDMATCH`), CSV, and JSON.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ibis_doc_model::{
    Oid4vpRequest, SdJwtOptions, extract_sheet_claims, issue_sd_jwt,
    list_sd_jwt_disclosure_paths, present_existing_sd_jwt,
};
use ibis_formats::Kind;
use ibis_formats::form_assist::{
    FieldProvenance, FieldStatus, FormEvaluation, FormMapping, PdfFormFieldTarget,
    PresentationRequestNote, SubmissionReceipt, VaultCredentialSummary, VaultProfile,
    build_flow_form_apply_commands, build_form_apply_command, current_date_ymd,
    current_utc_timestamp_iso, evaluate_flow_form, evaluate_form_mapping, evaluate_pdf_form,
    infer_draft_form_mapping_from_sheet, render_submission_receipt_markdown, sha256_hex,
    starter_presentation_request_markdown, starter_profile_markdown,
    submission_receipt_relative_path, synthesize_form_mapping_from_sheet,
};
#[cfg(not(target_family = "wasm"))]
use ibis_formats::form_assist::{fill_pdf_form_values, inspect_pdf_form_fields};
use ibis_formats::verifier_batch::{
    BatchVerifyPolicy, BatchVerifyReport, VerifyVerdict, render_batch_verify_csv,
    render_batch_verify_json, render_batch_verify_sheet_md, verify_credential_batch,
};
use ibis_office_shell::vc_transport::{
    KeyRole, LoadedKey, generate_and_save_key, generate_random_nonce, load_issuer_key,
    load_or_create_holder_key, send_oid4vp_direct_post, validate_direct_post_uri,
};
use ibis_office_shell::window_editors::WorkspaceEditorHost as _;

use crate::model::TabState;
use crate::workspace::SoraneWorkspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FormAssistMode {
    #[default]
    Fill,
    Submit,
    Verify,
}

type PdfInspectCache = Option<(PathBuf, Result<(bool, Vec<PdfFormFieldTarget>), String>)>;

#[derive(Clone, Debug)]
pub struct FormAssistState {
    pub mode: FormAssistMode,
    /// Selected profile path (`None` = use all Vault profiles).
    pub selected_profile_path: Option<String>,
    /// Selected form-mapping path (`None` = auto-detect best match).
    pub selected_mapping_path: Option<String>,
    /// Allow overwriting occupied cells / controls when applying candidates.
    pub allow_overwrite: bool,
    /// Selected Vault `.sd-jwt` path for Submit mode (`None` = active spreadsheet bindings).
    pub selected_credential_path: Option<String>,
    /// Disclosure toggle overrides for `selected_credential_path` (`path_str -> disclose`).
    pub credential_disclosures: BTreeMap<String, bool>,
    /// Cached disclosable `(path_str, display_val)` list for `selected_credential_path`.
    pub credential_disclosure_items: Vec<(String, String)>,
    /// Selected Vault `requests/*.req.md` path (`None` = use pasted `oid4vp_input`).
    pub selected_request_path: Option<String>,
    /// Editable OID4VP URI / JSON authorization request input.
    pub oid4vp_input: Entity<InputState>,
    /// Cached key display summaries (refreshed on open and key actions to avoid disk I/O per frame).
    pub holder_key_label: String,
    pub holder_key_is_sandbox: bool,
    pub issuer_key_label: String,
    pub issuer_key_is_sandbox: bool,
    /// True while an async `direct_post` HTTP request is in flight.
    pub submit_busy: bool,
    /// Most recently written `submissions/*.md` receipt path (absolute) for quick jump.
    pub last_receipt_path: Option<PathBuf>,
    /// Phase 3: cached AcroForm inspection result for the active PDF tab (avoids re-reading on repaint).
    pub cached_pdf_fields: RefCell<PdfInspectCache>,
    /// Phase 3: selected Vault `requests/*.req.md` policy for Verifier batch checks (`None` = all).
    pub verifier_request_path: Option<String>,
    /// Phase 3: optional trusted Issuer Public P-256 JWK JSON for Verifier batch checks.
    pub verifier_jwk_input: Entity<InputState>,
    /// Phase 3: most recent batch verification report.
    pub last_verify_report: Option<BatchVerifyReport>,
    /// Last action feedback message `(message, is_error)`.
    pub status_message: Option<(String, bool)>,
}

impl FormAssistState {
    pub fn new(window: &mut Window, cx: &mut Context<SoraneWorkspace>) -> Self {
        let oid4vp_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("openid4vp://?... または JSON 要求を貼り付け")
        });
        let verifier_jwk_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("任意: 信頼する Issuer 公開鍵 JWK JSON")
        });
        let mut state = Self {
            mode: FormAssistMode::Fill,
            selected_profile_path: None,
            selected_mapping_path: None,
            allow_overwrite: false,
            selected_credential_path: None,
            credential_disclosures: BTreeMap::new(),
            credential_disclosure_items: Vec::new(),
            selected_request_path: None,
            oid4vp_input,
            holder_key_label: String::new(),
            holder_key_is_sandbox: true,
            issuer_key_label: String::new(),
            issuer_key_is_sandbox: true,
            submit_busy: false,
            last_receipt_path: None,
            cached_pdf_fields: RefCell::new(None),
            verifier_request_path: None,
            verifier_jwk_input,
            last_verify_report: None,
            status_message: None,
        };
        state.refresh_key_badges();
        state
    }

    pub fn refresh_key_badges(&mut self) {
        let holder = load_or_create_holder_key();
        self.holder_key_label = format!("{} [{}]", holder.backend.label_ja(), holder.thumbprint);
        self.holder_key_is_sandbox = holder.backend.is_sandbox();

        let issuer = load_issuer_key().unwrap_or_else(LoadedKey::sandbox_issuer);
        self.issuer_key_label = format!("{} [{}]", issuer.backend.label_ja(), issuer.thumbprint);
        self.issuer_key_is_sandbox = issuer.backend.is_sandbox();
    }
}

enum ResolvedSheetMapping {
    Vault(FormMapping),
    Synthesized(FormMapping),
    HeuristicDraft(FormMapping),
    None,
}

struct PreparedSubmission {
    vp_token: String,
    disclosed_fields: Vec<String>,
    source_paths: Vec<String>,
    key_storage: String,
    holder_thumbprint: String,
    cnf_mismatch_warning: bool,
}

fn current_unix_timestamp() -> i64 {
    #[cfg(not(target_family = "wasm"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(1_700_000_000)
    }
    #[cfg(target_family = "wasm")]
    {
        1_700_000_000
    }
}

fn is_markdown_path(path: Option<&std::path::Path>) -> bool {
    path.and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
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

    fn active_tab_path(&self) -> Option<PathBuf> {
        self.panes
            .iter()
            .find(|p| p.id == self.active_pane)
            .and_then(|p| p.tabs.get(p.active_tab))
            .and_then(|t| t.path.clone())
    }

    fn active_tab_vault_credential<'a>(
        &self,
        all_credentials: &'a [VaultCredentialSummary],
    ) -> Option<&'a VaultCredentialSummary> {
        let vault = self.vault.as_ref()?;
        let active_path = self.active_tab_path()?;
        let rel = vault.relative_path_for(&active_path)?;
        all_credentials.iter().find(|c| c.path == rel)
    }

    fn has_open_fillable_tab(&self) -> bool {
        self.panes.iter().any(|p| {
            p.tabs
                .iter()
                .any(|t| matches!(t.kind, Kind::Workbook | Kind::Document | Kind::Pdf))
        })
    }

    fn focus_first_open_fillable_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Prefer an open Workbook first, then a DOCX/Markdown Document, then a PDF.
        let active_path = self.active_tab_path();
        let find_kind = |wanted: Kind| {
            self.panes.iter().find_map(|p| {
                p.tabs
                    .iter()
                    .enumerate()
                    .find(|(_, t)| t.kind == wanted && t.path != active_path)
                    .map(|(idx, _)| (p.id, idx))
            })
        };
        let target = find_kind(Kind::Workbook)
            .or_else(|| find_kind(Kind::Document))
            .or_else(|| find_kind(Kind::Pdf));
        let Some((pane_id, tab_idx)) = target else {
            return false;
        };
        self.active_pane = pane_id;
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) {
            pane.active_tab = tab_idx;
            if let Some(TabState::Resident(ed)) = pane.tabs.get(tab_idx).map(|t| &t.state) {
                let focus = ed.read(cx).focus_handle.clone();
                focus.focus(window, cx);
            }
        }
        cx.notify();
        true
    }

    #[cfg(not(target_family = "wasm"))]
    fn inspect_active_pdf_cached(&self, pdf_path: &std::path::Path) -> Result<(bool, Vec<PdfFormFieldTarget>), String> {
        let mut cache = self.form_assist.cached_pdf_fields.borrow_mut();
        if let Some((cached_path, res)) = cache.as_ref()
            && cached_path == pdf_path
        {
            return res.clone();
        }
        let res = std::fs::read(pdf_path)
            .map_err(|e| format!("PDF の読み込みに失敗しました: {e}"))
            .and_then(inspect_pdf_form_fields);
        *cache = Some((pdf_path.to_path_buf(), res.clone()));
        res
    }

    fn select_vault_credential_for_submit(&mut self, rel_path: Option<String>, cx: &mut Context<Self>) {
        self.form_assist.selected_credential_path = rel_path.clone();
        self.form_assist.credential_disclosures.clear();
        self.form_assist.credential_disclosure_items.clear();

        #[cfg(not(target_family = "wasm"))]
        if let (Some(vault), Some(rel)) = (&self.vault, &rel_path) {
            let full = vault.root().join(rel);
            if let Ok(raw) = std::fs::read_to_string(&full)
                && let Ok(paths) = list_sd_jwt_disclosure_paths(&raw)
            {
                for (path_str, val) in paths {
                    self.form_assist
                        .credential_disclosures
                        .insert(path_str.clone(), true);
                    self.form_assist
                        .credential_disclosure_items
                        .push((path_str, val.to_json()));
                }
            }
        }
        cx.notify();
    }

    fn active_oid4vp_request(&self, cx: &App) -> (Option<Oid4vpRequest>, Option<PresentationRequestNote>) {
        let pasted = self.form_assist.oid4vp_input.read(cx).value().trim().to_string();
        if !pasted.is_empty()
            && let Some(req) = Oid4vpRequest::parse(&pasted)
        {
            return (Some(req), None);
        }
        if let Some(vault) = &self.vault
            && let Some(req_path) = &self.form_assist.selected_request_path
            && let Some(note) = vault
                .vault_presentation_requests()
                .iter()
                .find(|r| &r.path == req_path)
        {
            let fallback_nonce = generate_random_nonce();
            return (Some(note.to_oid4vp_request(&fallback_nonce)), Some(note.clone()));
        }
        (None, None)
    }

    fn match_disclosures_to_request(&mut self, cx: &mut Context<Self>) {
        let (req_opt, _) = self.active_oid4vp_request(cx);
        let Some(req) = req_opt else {
            self.form_assist.status_message = Some((
                "先に提出先リクエストを選択するか OID4VP 要求を貼り付けてください。".into(),
                true,
            ));
            cx.notify();
            return;
        };
        if req.requested_claims.is_empty() {
            self.form_assist.status_message =
                Some(("要求に開示対象クレームが含まれていません。".into(), true));
            cx.notify();
            return;
        }

        if self.form_assist.selected_credential_path.is_some() {
            let mut matched = 0usize;
            for (path_str, _) in &self.form_assist.credential_disclosure_items {
                let want = req.requested_claims.iter().any(|r| r == path_str);
                if want {
                    matched += 1;
                }
                self.form_assist
                    .credential_disclosures
                    .insert(path_str.clone(), want);
            }
            self.form_assist.status_message = Some((
                format!(
                    "要求項目に合わせて開示設定を更新しました（{} / {} 件一致）",
                    matched,
                    req.requested_claims.len()
                ),
                false,
            ));
            cx.notify();
            return;
        }

        if let Some(editor) = self.active_editor(cx) {
            let cmds = {
                let ed = editor.read(cx);
                extract_sheet_claims(ed.presentation(), ed.current_slide).map(|bindings| {
                    let refs: Vec<&str> =
                        req.requested_claims.iter().map(String::as_str).collect();
                    bindings.set_disclose_commands(ed.presentation(), &refs, true)
                })
            };
            if let Some(cmds) = cmds
                && !cmds.is_empty()
            {
                editor.update(cx, |ed, cx| {
                    ed.execute(cmds, cx);
                });
                self.form_assist.status_message = Some((
                    format!(
                        "シートの開示チェックボックスを要求に同期しました（{} 項目）",
                        req.requested_claims.len()
                    ),
                    false,
                ));
            } else {
                self.form_assist.status_message = Some((
                    "シートに一致する sd.* / disclose.* バインディングが見つかりませんでした。"
                        .into(),
                    true,
                ));
            }
            cx.notify();
        }
    }

    fn prepare_submission_payload(
        &self,
        req: &Oid4vpRequest,
        cx: &App,
    ) -> Result<PreparedSubmission, String> {
        let holder = load_or_create_holder_key();
        let now = current_unix_timestamp();

        if let Some(cred_rel) = &self.form_assist.selected_credential_path {
            #[cfg(not(target_family = "wasm"))]
            {
                let vault = self
                    .vault
                    .as_ref()
                    .ok_or_else(|| "Vault が開かれていません。".to_string())?;
                let raw = std::fs::read_to_string(vault.root().join(cred_rel))
                    .map_err(|e| format!("VC ファイルの読み込みに失敗しました: {e}"))?;
                let selected: Vec<String> = self
                    .form_assist
                    .credential_disclosures
                    .iter()
                    .filter_map(|(k, &v)| v.then(|| k.clone()))
                    .collect();
                let sel_opt = if self.form_assist.credential_disclosure_items.is_empty() {
                    None
                } else {
                    Some(selected.as_slice())
                };
                let presented = present_existing_sd_jwt(
                    &raw,
                    sel_opt,
                    &req.client_id,
                    &req.nonce,
                    now,
                    &holder.key,
                )?;
                return Ok(PreparedSubmission {
                    vp_token: presented.kb.vp_token,
                    disclosed_fields: presented.disclosed_paths,
                    source_paths: vec![cred_rel.clone()],
                    key_storage: holder.backend.code().to_string(),
                    holder_thumbprint: holder.thumbprint,
                    cnf_mismatch_warning: presented.holder_cnf_matches == Some(false),
                });
            }
            #[cfg(target_family = "wasm")]
            {
                let _ = cred_rel;
                return Err("Vault VC の再提示はデスクトップ版で利用できます。".to_string());
            }
        }

        let editor = self
            .active_editor(cx)
            .ok_or_else(|| "アクティブなスプレッドシートがありません。".to_string())?;
        let ed = editor.read(cx);
        if Kind::of(ed.presentation()) != Kind::Workbook {
            return Err(
                "スプレッドシートを開くか、Vault 内の .sd-jwt 証明書を選択してください。"
                    .to_string(),
            );
        }
        let bindings = extract_sheet_claims(ed.presentation(), ed.current_slide).ok_or_else(|| {
            "このシートに claim.* / sd.* バインディングがありません。データ > 提出・証明 (SD-JWT) で設定するか、Vault の .sd-jwt を選択してください。"
                .to_string()
        })?;
        let issuer = load_issuer_key().unwrap_or_else(LoadedKey::sandbox_issuer);
        let mut opts = SdJwtOptions::default();
        opts.default_iat = Some(now);
        opts.holder_public_jwk = Some(holder.key.to_public_jwk());
        let sd_out = issue_sd_jwt(&bindings, &opts, &issuer.key);
        let kb = sd_out.attach_key_binding(&req.client_id, &req.nonce, now, &holder.key);

        let disclosed_fields: Vec<String> = bindings
            .claims
            .iter()
            .filter(|c| !c.selective || c.disclose)
            .map(|c| c.path_str.clone())
            .collect();
        let mut source_paths = Vec::new();
        if let (Some(vault), Some(sheet_path)) = (&self.vault, ed.file_path())
            && let Some(rel) = vault.relative_path_for(&sheet_path)
        {
            source_paths.push(rel);
        }
        if let Some(prof) = &self.form_assist.selected_profile_path {
            source_paths.push(prof.clone());
        }

        let key_storage = if issuer.backend.is_sandbox() {
            format!("holder:{}+issuer:sandbox_demo", holder.backend.code())
        } else {
            format!("holder:{}+issuer:{}", holder.backend.code(), issuer.backend.code())
        };

        Ok(PreparedSubmission {
            vp_token: kb.vp_token,
            disclosed_fields,
            source_paths,
            key_storage,
            holder_thumbprint: holder.thumbprint,
            cnf_mismatch_warning: false,
        })
    }

    #[cfg(not(target_family = "wasm"))]
    fn write_submission_receipt_to_vault(
        &mut self,
        receipt: &SubmissionReceipt,
        slug_hint: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PathBuf, String> {
        let vault = self
            .vault
            .as_ref()
            .ok_or_else(|| "Vault が開かれていません。".to_string())?;
        let date = current_date_ymd().unwrap_or_else(|| "undated".to_string());
        let rel = submission_receipt_relative_path(&date, slug_hint);
        let target = non_clobbering_path(vault.root(), &rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("submissions/ フォルダ作成に失敗しました: {e}"))?;
        }
        let md = render_submission_receipt_markdown(receipt);
        std::fs::write(&target, md)
            .map_err(|e| format!("提出記録の保存に失敗しました: {e}"))?;
        self.form_assist.last_receipt_path = Some(target.clone());
        self.refresh_vault_folder(window, cx);
        Ok(target)
    }

    pub(crate) fn submit_oid4vp_direct_post(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.form_assist.submit_busy {
            return;
        }
        let (req_opt, req_note) = self.active_oid4vp_request(cx);
        let Some(req) = req_opt else {
            self.form_assist.status_message = Some((
                "提出先リクエストを選択するか OID4VP URI / JSON を貼り付けてください。".into(),
                true,
            ));
            cx.notify();
            return;
        };
        let Some(response_uri) = req.response_uri.clone() else {
            self.form_assist.status_message =
                Some(("OID4VP 要求に response_uri が含まれていません。".into(), true));
            cx.notify();
            return;
        };
        if let Err(err) = validate_direct_post_uri(&response_uri) {
            self.form_assist.status_message = Some((err, true));
            cx.notify();
            return;
        }
        let prepared = match self.prepare_submission_payload(&req, cx) {
            Ok(p) => p,
            Err(err) => {
                self.form_assist.status_message = Some((err, true));
                cx.notify();
                return;
            }
        };

        let form_body = req.build_direct_post_body(&prepared.vp_token);
        let payload_sha256 = sha256_hex(form_body.as_bytes());
        let recipient = req.client_id.clone();
        let request_path = req_note.as_ref().map(|n| n.path.clone());
        let form_id = req_note.as_ref().map(|n| n.request_id.clone());
        let form_mapping_path = self.form_assist.selected_mapping_path.clone();
        let slug_hint = form_id.clone().unwrap_or_else(|| "oid4vp".to_string());
        let response_uri_for_send = response_uri.clone();

        self.form_assist.submit_busy = true;
        self.form_assist.status_message = Some((format!("送信中: {response_uri} …"), false));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { send_oid4vp_direct_post(&response_uri_for_send, &form_body) })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| {
                ws.form_assist.submit_busy = false;
                match res {
                    Ok(outcome) => {
                        let ok_status = (200..300).contains(&outcome.status);
                        let mut notes_parts = Vec::new();
                        if prepared.cnf_mismatch_warning {
                            notes_parts.push(
                                "注意: VC の cnf.jwk が現在の Holder 鍵と一致していません。"
                                    .to_string(),
                            );
                        }
                        if let Some(redir) = &outcome.redirect_uri {
                            notes_parts.push(format!("redirect_uri: {redir}"));
                        }
                        if !ok_status && !outcome.body.trim().is_empty() {
                            notes_parts.push(format!("response: {}", outcome.body.trim()));
                        }

                        #[cfg(not(target_family = "wasm"))]
                        let receipt_note = if ok_status && ws.vault.is_some() {
                            let receipt = SubmissionReceipt {
                                submitted_at: current_utc_timestamp_iso(),
                                recipient: recipient.clone(),
                                response_uri: Some(outcome.response_uri.clone()),
                                mode: "oid4vp_direct_post".to_string(),
                                http_status: Some(outcome.status),
                                form_id: form_id.clone(),
                                form_mapping_path: form_mapping_path.clone(),
                                request_path: request_path.clone(),
                                source_paths: prepared.source_paths.clone(),
                                artifact_path: None,
                                disclosed_fields: prepared.disclosed_fields.clone(),
                                payload_sha256: payload_sha256.clone(),
                                key_storage: prepared.key_storage.clone(),
                                holder_key_thumbprint: Some(prepared.holder_thumbprint.clone()),
                                notes: (!notes_parts.is_empty()).then(|| notes_parts.join("\n")),
                            };
                            match ws.write_submission_receipt_to_vault(
                                &receipt,
                                &slug_hint,
                                window,
                                cx,
                            ) {
                                Ok(path) => format!(
                                    " · 記録保存: {}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ),
                                Err(err) => format!(" · 記録保存失敗: {err}"),
                            }
                        } else {
                            String::new()
                        };
                        #[cfg(target_family = "wasm")]
                        let receipt_note = String::new();

                        if ok_status {
                            let warn = if prepared.cnf_mismatch_warning {
                                "（⚠ Holder 鍵不一致）"
                            } else {
                                ""
                            };
                            ws.form_assist.status_message = Some((
                                format!(
                                    "OID4VP 送信完了 (HTTP {}){warn}{receipt_note}",
                                    outcome.status
                                ),
                                false,
                            ));
                        } else {
                            ws.form_assist.status_message = Some((
                                format!(
                                    "提出先エラー (HTTP {}): {}",
                                    outcome.status,
                                    outcome.body.trim()
                                ),
                                true,
                            ));
                        }
                    }
                    Err(err) => {
                        ws.form_assist.status_message = Some((err, true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn copy_vp_token_and_record(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (req_opt, req_note) = self.active_oid4vp_request(cx);
        let req = req_opt.unwrap_or_else(|| Oid4vpRequest {
            client_id: "https://verifier.example.com".to_string(),
            nonce: generate_random_nonce(),
            response_uri: None,
            response_mode: Some("direct_post".to_string()),
            state: None,
            dcql_credential_id: None,
            requested_claims: Vec::new(),
        });
        let prepared = match self.prepare_submission_payload(&req, cx) {
            Ok(p) => p,
            Err(err) => {
                self.form_assist.status_message = Some((err, true));
                cx.notify();
                return;
            }
        };
        let payload_sha256 = sha256_hex(prepared.vp_token.as_bytes());
        cx.write_to_clipboard(ClipboardItem::new_string(prepared.vp_token));

        #[cfg(not(target_family = "wasm"))]
        let receipt_note = if self.vault.is_some() {
            let receipt = SubmissionReceipt {
                submitted_at: current_utc_timestamp_iso(),
                recipient: req.client_id.clone(),
                response_uri: req.response_uri.clone(),
                mode: "vp_token_copy".to_string(),
                http_status: None,
                form_id: req_note.as_ref().map(|n| n.request_id.clone()),
                form_mapping_path: self.form_assist.selected_mapping_path.clone(),
                request_path: req_note.as_ref().map(|n| n.path.clone()),
                source_paths: prepared.source_paths,
                artifact_path: None,
                disclosed_fields: prepared.disclosed_fields,
                payload_sha256,
                key_storage: prepared.key_storage,
                holder_key_thumbprint: Some(prepared.holder_thumbprint),
                notes: prepared
                    .cnf_mismatch_warning
                    .then(|| "注意: VC の cnf.jwk が現在の Holder 鍵と一致していません。".into()),
            };
            let slug = req_note
                .as_ref()
                .map(|n| n.request_id.as_str())
                .unwrap_or("vp-copy");
            match self.write_submission_receipt_to_vault(&receipt, slug, window, cx) {
                Ok(p) => format!(
                    "（提出記録: {}）",
                    p.file_name().unwrap_or_default().to_string_lossy()
                ),
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };
        #[cfg(target_family = "wasm")]
        let receipt_note = {
            let _ = (window, req_note, payload_sha256);
            String::new()
        };

        self.form_assist.status_message = Some((
            format!("vp_token をクリップボードにコピーしました{receipt_note}"),
            false,
        ));
        cx.notify();
    }

    pub(crate) fn save_sheet_vc_to_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let (bindings, sheet_rel) = {
                let ed = editor.read(cx);
                let b = extract_sheet_claims(ed.presentation(), ed.current_slide);
                let rel = ed.file_path().and_then(|p| vault.relative_path_for(&p));
                (b, rel)
            };
            let Some(bindings) = bindings else {
                self.form_assist.status_message = Some((
                    "シートに claim.* / sd.* バインディングがありません。".into(),
                    true,
                ));
                cx.notify();
                return;
            };
            let holder = load_or_create_holder_key();
            let issuer = load_issuer_key().unwrap_or_else(LoadedKey::sandbox_issuer);
            let mut opts = SdJwtOptions::default();
            opts.default_iat = Some(current_unix_timestamp());
            opts.holder_public_jwk = Some(holder.key.to_public_jwk());
            let sd_out = issue_sd_jwt(&bindings, &opts, &issuer.key);

            let date = current_date_ymd().unwrap_or_else(|| "undated".to_string());
            let rel_cred = format!("credentials/{date}-sheet.sd-jwt");
            let target = non_clobbering_path(vault.root(), &rel_cred);
            if let Some(parent) = target.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                self.form_assist.status_message =
                    Some((format!("credentials/ 作成失敗: {err}"), true));
                cx.notify();
                return;
            }
            if let Err(err) = std::fs::write(&target, &sd_out.sd_jwt_all) {
                self.form_assist.status_message = Some((format!("VC 保存失敗: {err}"), true));
                cx.notify();
                return;
            }
            let artifact_rel = vault.relative_path_for(&target);
            let disclosed_fields: Vec<String> =
                bindings.claims.iter().map(|c| c.path_str.clone()).collect();
            let receipt = SubmissionReceipt {
                submitted_at: current_utc_timestamp_iso(),
                recipient: "local-vault".to_string(),
                response_uri: None,
                mode: "vc_issue_sd_jwt".to_string(),
                http_status: None,
                form_id: None,
                form_mapping_path: self.form_assist.selected_mapping_path.clone(),
                request_path: None,
                source_paths: sheet_rel.into_iter().collect(),
                artifact_path: artifact_rel.clone(),
                disclosed_fields,
                payload_sha256: sha256_hex(sd_out.sd_jwt_all.as_bytes()),
                key_storage: format!(
                    "holder:{}+issuer:{}",
                    holder.backend.code(),
                    issuer.backend.code()
                ),
                holder_key_thumbprint: Some(holder.thumbprint),
                notes: None,
            };
            let _ = self.write_submission_receipt_to_vault(&receipt, "vc-issue", window, cx);
            if let Some(rel) = artifact_rel {
                self.select_vault_credential_for_submit(Some(rel), cx);
            }
            self.form_assist.status_message = Some((
                format!(
                    "SD-JWT を Vault に保存しました: {}",
                    target.file_name().unwrap_or_default().to_string_lossy()
                ),
                false,
            ));
            cx.notify();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = window;
            self.form_assist.status_message =
                Some(("Vault への VC 保存はデスクトップ版で利用できます。".into(), true));
            cx.notify();
        }
    }

    pub(crate) fn create_starter_presentation_request(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(not(target_family = "wasm"))]
        {
            let Some(vault) = &self.vault else {
                self.form_assist.status_message =
                    Some(("先に Vault フォルダを開いてください。".into(), true));
                cx.notify();
                return;
            };
            let (rel, content) = starter_presentation_request_markdown();
            let target = non_clobbering_path(vault.root(), rel);
            if let Some(parent) = target.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                self.form_assist.status_message =
                    Some((format!("requests/ フォルダ作成に失敗しました: {err}"), true));
                cx.notify();
                return;
            }
            if let Err(err) = std::fs::write(&target, content) {
                self.form_assist.status_message =
                    Some((format!("提出要求テンプレート作成に失敗しました: {err}"), true));
                cx.notify();
                return;
            }
            self.refresh_vault_folder(window, cx);
            if let Some(vault) = &self.vault
                && let Some(rel_created) = vault.relative_path_for(&target)
            {
                self.form_assist.selected_request_path = Some(rel_created);
            }
            self.form_assist.status_message = Some((
                format!(
                    "提出要求テンプレートを作成しました: {}",
                    target.file_name().unwrap_or_default().to_string_lossy()
                ),
                false,
            ));
            self.open_path_in_split_pane(target, window, cx);
            cx.notify();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = window;
            self.form_assist.status_message = Some((
                "提出要求テンプレートの作成はデスクトップ版で利用できます。".into(),
                true,
            ));
            cx.notify();
        }
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
                self.form_assist.status_message =
                    Some(("先に Vault フォルダを開いてください。".into(), true));
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
            self.form_assist.status_message = Some((
                format!(
                    "サンプルプロフィールを作成しました: {}",
                    target.file_name().unwrap_or_default().to_string_lossy()
                ),
                false,
            ));
            self.open_path_in_split_pane(target, window, cx);
            cx.notify();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (scope, window);
            self.form_assist.status_message = Some((
                "プロフィール自動作成はデスクトップ版で利用できます。".into(),
                true,
            ));
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
                format!("フォーム対応表を保存しました（{} 項目）", mapping.fields.len()),
                false,
            ));
            self.open_path_in_split_pane(target, window, cx);
            cx.notify();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = window;
            self.form_assist.status_message = Some((
                "フォーム対応表の保存はデスクトップ版で利用できます。".into(),
                true,
            ));
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

    pub(crate) fn apply_form_assist_to_active_document(&mut self, cx: &mut Context<Self>) {
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
                Err("読み取り専用または保護された文書には適用できません。".to_string())
            } else if let Some(flow) = ed.presentation().flow.as_ref() {
                let flatten_md = is_markdown_path(ed.file_path().as_deref());
                let (targets, eval) =
                    evaluate_flow_form(flow, &profiles, &credentials, today.as_deref());
                build_flow_form_apply_commands(flow, &targets, &eval, allow_overwrite, flatten_md)
            } else {
                Err("開いている文書にフォーム欄が見つかりません。".to_string())
            }
        };

        match build_result {
            Ok((cmds, applied_count)) => {
                editor.update(cx, |ed, cx| {
                    ed.execute(cmds, cx);
                });
                self.form_assist.status_message = Some((
                    format!(
                        "{applied_count} 項目を文書に適用しました（Ctrl/Cmd+Z で元に戻せます）"
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

    pub(crate) fn apply_form_assist_to_active_pdf(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(not(target_family = "wasm"))]
        {
            let Some(vault) = self.vault.clone() else {
                self.form_assist.status_message =
                    Some(("Vault を開くと PDF フォームを自動入力できます。".into(), true));
                cx.notify();
                return;
            };
            let Some(pdf_path) = self.active_tab_path() else {
                return;
            };
            let Ok(pdf_bytes) = std::fs::read(&pdf_path) else {
                self.form_assist.status_message =
                    Some(("PDF ファイルの読み込みに失敗しました。".into(), true));
                cx.notify();
                return;
            };
            let Ok((_, targets)) = inspect_pdf_form_fields(pdf_bytes.clone()) else {
                self.form_assist.status_message =
                    Some(("この PDF に入力可能な AcroForm テキスト欄が見つかりませんでした。".into(), true));
                cx.notify();
                return;
            };

            let profiles = self.selected_profiles(vault.vault_profiles());
            let credentials: Vec<&VaultCredentialSummary> =
                vault.vault_credentials().iter().collect();
            let today = current_date_ymd();
            let allow_overwrite = self.form_assist.allow_overwrite;
            let eval = evaluate_pdf_form(&targets, &profiles, &credentials, today.as_deref());

            let mut values = BTreeMap::new();
            for field in &eval.fields {
                let allowed = field.status == FieldStatus::Ready
                    || (allow_overwrite && field.status == FieldStatus::Occupied);
                if allowed && let Some(ref disp) = field.proposed_display {
                    values.insert(field.id.clone(), disp.clone());
                }
            }
            if values.is_empty() {
                self.form_assist.status_message =
                    Some(("PDF に適用できる入力候補がありません。".into(), true));
                cx.notify();
                return;
            }

            match fill_pdf_form_values(pdf_bytes, &values) {
                Ok((filled, applied_count)) => {
                    let stem = pdf_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("form");
                    let rel_dir = pdf_path
                        .parent()
                        .and_then(|p| p.strip_prefix(vault.root()).ok())
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    let rel_out = if rel_dir.is_empty() {
                        format!("{stem}-filled.pdf")
                    } else {
                        format!("{rel_dir}/{stem}-filled.pdf")
                    };
                    let target = non_clobbering_path(vault.root(), &rel_out);
                    if let Err(err) = std::fs::write(&target, filled) {
                        self.form_assist.status_message =
                            Some((format!("記入済み PDF の保存に失敗しました: {err}"), true));
                    } else {
                        self.refresh_vault_folder(window, cx);
                        self.form_assist.status_message = Some((
                            format!(
                                "{applied_count} 項目を記入した PDF を保存しました: {}",
                                target.file_name().unwrap_or_default().to_string_lossy()
                            ),
                            false,
                        ));
                        self.open_path_in_split_pane(target, window, cx);
                    }
                }
                Err(err) => {
                    self.form_assist.status_message = Some((err, true));
                }
            }
            cx.notify();
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = window;
            self.form_assist.status_message =
                Some(("PDF AcroForm 自動記入はデスクトップ版で利用できます。".into(), true));
            cx.notify();
        }
    }

    pub(crate) fn run_vault_batch_verify(&mut self, cx: &mut Context<Self>) {
        #[cfg(not(target_family = "wasm"))]
        {
            let Some(vault) = self.vault.clone() else {
                self.form_assist.status_message =
                    Some(("先に Vault フォルダを開いてください。".into(), true));
                cx.notify();
                return;
            };
            let jwk_raw = self
                .form_assist
                .verifier_jwk_input
                .read(cx)
                .value()
                .trim()
                .to_string();
            let issuer_jwk_json = (!jwk_raw.is_empty()).then_some(jwk_raw);
            let now = Some(current_unix_timestamp());

            let policy = if let Some(req_path) = &self.form_assist.verifier_request_path
                && let Some(note) = vault
                    .vault_presentation_requests()
                    .iter()
                    .find(|r| &r.path == req_path)
            {
                BatchVerifyPolicy::from_request_note(note, issuer_jwk_json, now)
            } else {
                BatchVerifyPolicy {
                    label: "Vault 全件検証".to_string(),
                    expected_paths: Vec::new(),
                    expected_vct: None,
                    issuer_jwk_json,
                    expected_audience: None,
                    expected_nonce: None,
                    now_unix: now,
                }
            };

            let mut loaded: Vec<(String, String, Vec<u8>)> = Vec::new();
            for cred in vault.vault_credentials() {
                let full = vault.root().join(&cred.path);
                let ext = full
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(&cred.format)
                    .to_string();
                if let Ok(bytes) = std::fs::read(&full) {
                    loaded.push((cred.path.clone(), ext, bytes));
                }
            }
            let refs: Vec<(&str, &str, &[u8])> = loaded
                .iter()
                .map(|(p, ext, b)| (p.as_str(), ext.as_str(), b.as_slice()))
                .collect();
            let report = verify_credential_batch(&refs, &policy);
            self.form_assist.status_message = Some((
                format!(
                    "{} 件の VC を検証しました（検証済 {} · ダイジェストOK {} · 要確認 {} · 無効 {}）",
                    report.total,
                    report.verified_count,
                    report.unchecked_ok_count,
                    report.needs_review_count,
                    report.invalid_count
                ),
                report.invalid_count > 0,
            ));
            self.form_assist.last_verify_report = Some(report);
            cx.notify();
        }
        #[cfg(target_family = "wasm")]
        {
            self.form_assist.status_message =
                Some(("一括 VC 検証はデスクトップ版で利用できます。".into(), true));
            cx.notify();
        }
    }

    pub(crate) fn export_batch_verify_sheet_md(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(not(target_family = "wasm"))]
        {
            let Some(vault) = self.vault.clone() else {
                return;
            };
            let Some(report) = self.form_assist.last_verify_report.as_ref() else {
                return;
            };
            let date = current_date_ymd().unwrap_or_else(|| "undated".to_string());
            let rel = format!("verifications/{date}-vc-verify.sheet.md");
            let target = non_clobbering_path(vault.root(), &rel);
            if let Some(parent) = target.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                self.form_assist.status_message =
                    Some((format!("verifications/ 作成失敗: {err}"), true));
                cx.notify();
                return;
            }
            let md = render_batch_verify_sheet_md(report);
            if let Err(err) = std::fs::write(&target, md) {
                self.form_assist.status_message =
                    Some((format!("BI シート保存失敗: {err}"), true));
                cx.notify();
                return;
            }
            self.refresh_vault_folder(window, cx);
            self.form_assist.status_message = Some((
                format!(
                    "BI 集計シートを保存して開きました: {}",
                    target.file_name().unwrap_or_default().to_string_lossy()
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
                Some(("BI シート出力はデスクトップ版で利用できます。".into(), true));
            cx.notify();
        }
    }

    pub(crate) fn export_batch_verify_csv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(not(target_family = "wasm"))]
        {
            let Some(vault) = self.vault.clone() else {
                return;
            };
            let Some(report) = self.form_assist.last_verify_report.as_ref() else {
                return;
            };
            let date = current_date_ymd().unwrap_or_else(|| "undated".to_string());
            let rel = format!("verifications/{date}-vc-verify.csv");
            let target = non_clobbering_path(vault.root(), &rel);
            if let Some(parent) = target.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                self.form_assist.status_message =
                    Some((format!("verifications/ 作成失敗: {err}"), true));
                cx.notify();
                return;
            }
            let csv = render_batch_verify_csv(report);
            if let Err(err) = std::fs::write(&target, csv) {
                self.form_assist.status_message = Some((format!("CSV 保存失敗: {err}"), true));
                cx.notify();
                return;
            }
            self.refresh_vault_folder(window, cx);
            self.form_assist.status_message = Some((
                format!(
                    "検証結果 CSV を保存しました: {}",
                    target.file_name().unwrap_or_default().to_string_lossy()
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
                Some(("CSV 出力はデスクトップ版で利用できます。".into(), true));
            cx.notify();
        }
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
                        .child("Vault フォルダを開くと、Excel / Word / Markdown / PDF のフォーム自動入力、OID4VP 提出、および保有 VC の一括検証・BI 集計ができます。"),
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
        let all_requests = vault.vault_presentation_requests();

        // 3-Mode switcher: 入力 (Fill) | 提出 (Submit) | 検証 (Verify)
        let mode = self.form_assist.mode;
        root = root.child(
            h_flex()
                .gap(px(4.))
                .w_full()
                .child(
                    div()
                        .id("ws-form-mode-fill")
                        .flex_1()
                        .cursor_pointer()
                        .px(px(6.))
                        .py(px(4.))
                        .rounded(px(5.))
                        .text_size(px(11.))
                        .text_center()
                        .when(mode == FormAssistMode::Fill, |d| {
                            d.bg(theme.accent)
                                .text_color(theme.accent_foreground)
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(mode != FormAssistMode::Fill, |d| {
                            d.bg(theme.muted)
                                .text_color(theme.muted_foreground)
                                .hover(|h| h.bg(theme.secondary))
                        })
                        .child("📝 入力")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.form_assist.mode = FormAssistMode::Fill;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("ws-form-mode-submit")
                        .flex_1()
                        .cursor_pointer()
                        .px(px(6.))
                        .py(px(4.))
                        .rounded(px(5.))
                        .text_size(px(11.))
                        .text_center()
                        .when(mode == FormAssistMode::Submit, |d| {
                            d.bg(theme.accent)
                                .text_color(theme.accent_foreground)
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(mode != FormAssistMode::Submit, |d| {
                            d.bg(theme.muted)
                                .text_color(theme.muted_foreground)
                                .hover(|h| h.bg(theme.secondary))
                        })
                        .child("🛡 提出")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.form_assist.mode = FormAssistMode::Submit;
                            this.form_assist.refresh_key_badges();
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("ws-form-mode-verify")
                        .flex_1()
                        .cursor_pointer()
                        .px(px(6.))
                        .py(px(4.))
                        .rounded(px(5.))
                        .text_size(px(11.))
                        .text_center()
                        .when(mode == FormAssistMode::Verify, |d| {
                            d.bg(theme.accent)
                                .text_color(theme.accent_foreground)
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(mode != FormAssistMode::Verify, |d| {
                            d.bg(theme.muted)
                                .text_color(theme.muted_foreground)
                                .hover(|h| h.bg(theme.secondary))
                        })
                        .child("✅ 検証")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.form_assist.mode = FormAssistMode::Verify;
                            cx.notify();
                        })),
                ),
        );

        // Structured-preview reverse link card whenever a Vault `.sd-jwt` / `.mdoc` tab is active
        if let Some(active_cred) = self.active_tab_vault_credential(all_credentials) {
            let cred_path = active_cred.path.clone();
            let is_sd_jwt = active_cred.format == "sd-jwt";
            let has_fillable = self.has_open_fillable_tab();
            let mut rev_actions = h_flex().gap_1().flex_wrap().pt_1();
            if has_fillable {
                rev_actions = rev_actions.child(
                    Button::new("ws-cred-use-for-sheet")
                        .primary()
                        .xsmall()
                        .label("開いているフォームへ入力に使う")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.form_assist.mode = FormAssistMode::Fill;
                            if this.focus_first_open_fillable_tab(window, cx) {
                                this.form_assist.status_message = Some((
                                    "開いているフォームに切り替えました。保有 VC の値が優先候補として表示されます。"
                                        .into(),
                                    false,
                                ));
                            }
                            cx.notify();
                        })),
                );
            }
            if is_sd_jwt {
                let cred_for_submit = cred_path.clone();
                rev_actions = rev_actions.child(
                    Button::new("ws-cred-present-oid4vp")
                        .xsmall()
                        .label("OID4VP で提示する")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.form_assist.mode = FormAssistMode::Submit;
                            this.form_assist.refresh_key_badges();
                            this.select_vault_credential_for_submit(
                                Some(cred_for_submit.clone()),
                                cx,
                            );
                        })),
                );
            }
            root = root.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(Hsla::from(rgb(0x2E7D32)))
                    .bg(Hsla::from(rgb(0xF1F8E9)))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0x1B5E20))
                            .child(format!("🛡 表示中の VC [{}]", active_cred.format)),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgb(0x2E7D32))
                            .truncate()
                            .child(format!(
                                "{} · ダイジェスト {}/{} · {}",
                                active_cred.vct_or_doctype,
                                active_cred.digests_verified,
                                active_cred.digests_total,
                                active_cred.sig_status
                            )),
                    )
                    .child(rev_actions),
            );
        }

        match mode {
            FormAssistMode::Fill => {
                root = self.render_fill_mode(
                    root,
                    vault.root(),
                    all_profiles,
                    all_credentials,
                    all_mappings,
                    cx,
                );
            }
            FormAssistMode::Submit => {
                root = self.render_submit_mode(root, vault.root(), all_credentials, all_requests, cx);
            }
            FormAssistMode::Verify => {
                root = self.render_verify_mode(root, vault.root(), all_credentials, all_requests, cx);
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

        if let Some(receipt_path) = &self.form_assist.last_receipt_path {
            let open_path = receipt_path.clone();
            let name = receipt_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            root = root.child(
                Button::new("ws-form-open-last-receipt")
                    .xsmall()
                    .label(format!("提出記録を開く: {name}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_path_in_split_pane(open_path.clone(), window, cx);
                    })),
            );
        }

        root.into_any_element()
    }

    fn append_field_plan_cards(&self, mut root: Div, eval: &FormEvaluation, cx: &App) -> Div {
        let theme = cx.theme().clone();
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
        root
    }

    fn append_vault_assets_overview(
        &self,
        mut root: Div,
        vault_root: &std::path::Path,
        all_profiles: &[VaultProfile],
        all_credentials: &[VaultCredentialSummary],
        all_mappings: &[FormMapping],
        hint: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        root = root.child(
            div()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(hint.to_string()),
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
                let full = vault_root.join(&prof.path);
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
                let full = vault_root.join(&cred.path);
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
                let full = vault_root.join(&map.path);
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
        root
    }

    fn render_fill_mode(
        &self,
        mut root: Div,
        vault_root: &std::path::Path,
        all_profiles: &[VaultProfile],
        all_credentials: &[VaultCredentialSummary],
        all_mappings: &[FormMapping],
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();

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
                            .child("自社や個人の基本情報を Vault に置くと、Excel / Word / Markdown / PDF フォームへ 1 クリック入力できます。"),
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
                let is_sel = self.form_assist.selected_profile_path.as_deref()
                    == Some(prof.path.as_str());
                let path_for_select = prof.path.clone();
                let full_for_edit = vault_root.join(&prof.path);
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

        let active_editor = self.active_editor(cx);
        let active_kind = active_editor
            .as_ref()
            .map(|ed| ed.read(cx).format.kind());
        let selected_profiles = self.selected_profiles(all_profiles);
        let cred_refs: Vec<&VaultCredentialSummary> = all_credentials.iter().collect();
        let today = current_date_ymd();
        let allow_overwrite = self.form_assist.allow_overwrite;

        match (active_kind, active_editor) {
            (Some(Kind::Workbook), Some(editor)) => {
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
                                Some(vault_root.join(&m.path)),
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
                        action_row = action_row.child(
                            Button::new("ws-form-goto-submit")
                                .xsmall()
                                .label("提出へ進む →")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.form_assist.mode = FormAssistMode::Submit;
                                    this.form_assist.refresh_key_badges();
                                    cx.notify();
                                })),
                        );
                        header_card = header_card.child(action_row);
                        root = root.child(header_card);
                        root = self.append_field_plan_cards(root, &eval, cx);
                    }
                    _ => {}
                }
            }
            (Some(Kind::Document), Some(editor)) => {
                let (is_md, doc_title, eval_opt) = {
                    let ed = editor.read(cx);
                    let is_md = is_markdown_path(ed.file_path().as_deref());
                    let title = ed
                        .file_name
                        .clone()
                        .unwrap_or_else(|| SharedString::from("文書フォーム"));
                    let eval = ed.presentation().flow.as_ref().and_then(|flow| {
                        let (targets, eval) = evaluate_flow_form(
                            flow,
                            &selected_profiles,
                            &cred_refs,
                            today.as_deref(),
                        );
                        (!targets.is_empty()).then_some(eval)
                    });
                    (is_md, title, eval)
                };

                if let Some(eval) = eval_opt {
                    let badge = if is_md {
                        "Markdown {{form:*}}"
                    } else {
                        "DOCX フォーム欄"
                    };
                    let applicable = eval.ready_count
                        + if allow_overwrite {
                            eval.occupied_count
                        } else {
                            0
                        };
                    let mut action_row = h_flex().gap_1().flex_wrap().pt_1().child(
                        Button::new("ws-form-apply-doc")
                            .primary()
                            .xsmall()
                            .label(format!("文書へ一括適用 ({applicable} 件)"))
                            .disabled(applicable == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.apply_form_assist_to_active_document(cx);
                            })),
                    );
                    if eval.occupied_count > 0 {
                        action_row = action_row.child(
                            Button::new("ws-form-doc-toggle-overwrite")
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
                                        .child(doc_title),
                                )
                                .child(
                                    div()
                                        .px(px(5.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .bg(theme.muted)
                                        .text_size(px(10.))
                                        .text_color(theme.muted_foreground)
                                        .child(badge),
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
                        )
                        .child(action_row);
                    if eval.overflow_count > 0 {
                        header_card = header_card.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(rgb(0xC62828))
                                .child(format!("要確認: 字数超過 {} 件", eval.overflow_count)),
                        );
                    }
                    root = root.child(header_card);
                    root = self.append_field_plan_cards(root, &eval, cx);
                } else {
                    root = self.append_vault_assets_overview(
                        root,
                        vault_root,
                        all_profiles,
                        all_credentials,
                        all_mappings,
                        "Excel (.xlsx) / Word (.docx) フォーム / Markdown ({{form:キー名}}) / 入力可能 PDF を開くと、プロフィールと保有 VC から自動で入力候補が表示されます。",
                        cx,
                    );
                }
            }
            (Some(Kind::Pdf), _) => {
                #[cfg(not(target_family = "wasm"))]
                if let Some(pdf_path) = self.active_tab_path() {
                    match self.inspect_active_pdf_cached(&pdf_path) {
                        Ok((is_pdfa2b, targets)) if !targets.is_empty() => {
                            let eval = evaluate_pdf_form(
                                &targets,
                                &selected_profiles,
                                &cred_refs,
                                today.as_deref(),
                            );
                            let badge = if is_pdfa2b {
                                "PDF/A-2b AcroForm"
                            } else {
                                "PDF AcroForm"
                            };
                            let applicable = eval.ready_count
                                + if allow_overwrite {
                                    eval.occupied_count
                                } else {
                                    0
                                };
                            let title = pdf_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            let mut action_row = h_flex().gap_1().flex_wrap().pt_1().child(
                                Button::new("ws-form-apply-pdf")
                                    .primary()
                                    .xsmall()
                                    .label(format!("記入済み PDF を保存 ({applicable} 件)"))
                                    .disabled(applicable == 0)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.apply_form_assist_to_active_pdf(window, cx);
                                    })),
                            );
                            if eval.occupied_count > 0 {
                                action_row = action_row.child(
                                    Button::new("ws-form-pdf-toggle-overwrite")
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
                                                .child(title),
                                        )
                                        .child(
                                            div()
                                                .px(px(5.))
                                                .py(px(1.))
                                                .rounded(px(4.))
                                                .bg(theme.muted)
                                                .text_size(px(10.))
                                                .text_color(theme.muted_foreground)
                                                .child(badge),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(theme.muted_foreground)
                                        .child(format!(
                                            "入力候補 {} · 入力済み {} · VC由来 {} 件",
                                            eval.ready_count,
                                            eval.occupied_count,
                                            eval.vc_backed_count
                                        )),
                                )
                                .child(action_row);
                            if eval.overflow_count > 0 {
                                header_card = header_card.child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(rgb(0xC62828))
                                        .child(format!(
                                            "要確認: /MaxLen 字数超過 {} 件",
                                            eval.overflow_count
                                        )),
                                );
                            }
                            root = root.child(header_card);
                            root = self.append_field_plan_cards(root, &eval, cx);
                        }
                        _ => {
                            root = self.append_vault_assets_overview(
                                root,
                                vault_root,
                                all_profiles,
                                all_credentials,
                                all_mappings,
                                "この PDF には編集可能な AcroForm テキスト欄が見つかりませんでした。フォーム付き PDF / Excel / Word / Markdown を開くと自動入力できます。",
                                cx,
                            );
                        }
                    }
                }
                #[cfg(target_family = "wasm")]
                {
                    root = self.append_vault_assets_overview(
                        root,
                        vault_root,
                        all_profiles,
                        all_credentials,
                        all_mappings,
                        "PDF AcroForm 自動入力はデスクトップ版で利用できます。",
                        cx,
                    );
                }
            }
            _ => {
                root = self.append_vault_assets_overview(
                    root,
                    vault_root,
                    all_profiles,
                    all_credentials,
                    all_mappings,
                    "Excel (.xlsx) / Word (.docx) フォーム / Markdown ({{form:キー名}}) / 入力可能 PDF を開くと、プロフィールと保有 VC から自動で入力候補が表示されます。",
                    cx,
                );
            }
        }

        root
    }

    fn render_submit_mode(
        &self,
        mut root: Div,
        vault_root: &std::path::Path,
        all_credentials: &[VaultCredentialSummary],
        all_requests: &[PresentationRequestNote],
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        let using_vault_cred = self.form_assist.selected_credential_path.is_some();
        let effective_sandbox = self.form_assist.holder_key_is_sandbox
            || (!using_vault_cred && self.form_assist.issuer_key_is_sandbox);

        // 1. Key & Trust Card
        let mut key_card = v_flex()
            .gap(px(4.))
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(if effective_sandbox {
                Hsla::from(rgb(0xF0C36D))
            } else {
                theme.border
            })
            .bg(if effective_sandbox {
                Hsla::from(rgb(0xFFF8E1))
            } else {
                theme.background
            })
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if effective_sandbox {
                        "⚠ 鍵・トラスト状態（一部 Sandbox 鍵）"
                    } else {
                        "🔐 鍵・トラスト状態（永続鍵）"
                    }),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(theme.muted_foreground)
                    .child(format!("Holder 鍵: {}", self.form_assist.holder_key_label)),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(theme.muted_foreground)
                    .child(format!("Issuer 鍵: {}", self.form_assist.issuer_key_label)),
            );
        if self.form_assist.issuer_key_is_sandbox {
            key_card = key_card.child(
                Button::new("ws-vc-gen-issuer-key")
                    .xsmall()
                    .label("永続発行鍵を生成（Vault 外に 0600 保存）")
                    .on_click(cx.listener(|this, _, _, cx| {
                        match generate_and_save_key(KeyRole::Issuer) {
                            Ok(loaded) => {
                                this.form_assist.refresh_key_badges();
                                this.form_assist.status_message = Some((
                                    format!(
                                        "永続発行鍵を作成しました ({} [{}])",
                                        loaded.backend.label_ja(),
                                        loaded.thumbprint
                                    ),
                                    false,
                                ));
                            }
                            Err(err) => {
                                this.form_assist.status_message = Some((err, true));
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        root = root.child(key_card);

        // 2. Submission Source Selector: Active Sheet vs Vault `.sd-jwt`
        let sd_jwt_creds: Vec<&VaultCredentialSummary> = all_credentials
            .iter()
            .filter(|c| c.format == "sd-jwt")
            .collect();
        let active_sheet_claims_count = self
            .active_editor(cx)
            .and_then(|ed| {
                let e = ed.read(cx);
                (Kind::of(e.presentation()) == Kind::Workbook)
                    .then(|| extract_sheet_claims(e.presentation(), e.current_slide))
                    .flatten()
            })
            .map(|b| b.claims.len())
            .unwrap_or(0);

        let mut source_pills = h_flex().gap(px(4.)).flex_wrap();
        source_pills = source_pills.child(
            div()
                .id("ws-submit-src-sheet")
                .cursor_pointer()
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(10.5))
                .when(!using_vault_cred, |d| {
                    d.bg(theme.accent)
                        .text_color(theme.accent_foreground)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(using_vault_cred, |d| {
                    d.bg(theme.muted)
                        .text_color(theme.foreground)
                        .hover(|h| h.bg(theme.secondary))
                })
                .child(format!("開いているシート ({active_sheet_claims_count})"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.select_vault_credential_for_submit(None, cx);
                })),
        );
        for (idx, cred) in sd_jwt_creds.iter().enumerate() {
            let is_sel = self.form_assist.selected_credential_path.as_deref()
                == Some(cred.path.as_str());
            let rel = cred.path.clone();
            let short_name = cred.path.rsplit('/').next().unwrap_or(&cred.path).to_string();
            source_pills = source_pills.child(
                div()
                    .id(SharedString::from(format!("ws-submit-src-cred:{idx}")))
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
                    .child(format!("🛡 {short_name}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_vault_credential_for_submit(Some(rel.clone()), cx);
                    })),
            );
        }

        let mut source_card = v_flex()
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
                    .child("1. 提出する証明書 / シート"),
            )
            .child(source_pills);

        if using_vault_cred {
            if self.form_assist.credential_disclosure_items.is_empty() {
                source_card = source_card.child(
                    div()
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .child("選択的開示クレームはありません（プレーンクレームのみ提示されます）。"),
                );
            } else {
                let mut disc_list = v_flex().gap(px(3.)).pt_1();
                for (idx, (path_str, val_json)) in
                    self.form_assist.credential_disclosure_items.iter().enumerate()
                {
                    let on = self
                        .form_assist
                        .credential_disclosures
                        .get(path_str)
                        .copied()
                        .unwrap_or(true);
                    let toggle_path = path_str.clone();
                    disc_list = disc_list.child(
                        div()
                            .id(SharedString::from(format!("ws-submit-disc:{idx}")))
                            .cursor_pointer()
                            .px(px(6.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .text_size(px(10.5))
                            .truncate()
                            .when(on, |d| {
                                d.bg(Hsla::from(rgb(0xE8F5E9)))
                                    .text_color(rgb(0x1B5E20))
                            })
                            .when(!on, |d| {
                                d.bg(theme.muted).text_color(theme.muted_foreground)
                            })
                            .child(format!(
                                "{} {} = {}",
                                if on { "☑" } else { "☐" },
                                path_str,
                                val_json
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let cur = this
                                    .form_assist
                                    .credential_disclosures
                                    .get(&toggle_path)
                                    .copied()
                                    .unwrap_or(true);
                                this.form_assist
                                    .credential_disclosures
                                    .insert(toggle_path.clone(), !cur);
                                cx.notify();
                            })),
                    );
                }
                source_card = source_card.child(disc_list);
            }
        } else if active_sheet_claims_count == 0 {
            source_card = source_card.child(
                div()
                    .text_size(px(10.5))
                    .text_color(theme.muted_foreground)
                    .child("開いているシートに claim.*/sd.* がありません。Vault の .sd-jwt を選ぶか、シートのデータ > 提出・証明でクレームを設定してください。"),
            );
        }
        root = root.child(source_card);

        // 3. Verifier / OID4VP Request Card
        let mut req_card = v_flex()
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
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("2. 提出先リクエスト (OID4VP)"),
                    )
                    .child(
                        Button::new("ws-submit-new-req")
                            .xsmall()
                            .label("+ 要求作成")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_starter_presentation_request(window, cx);
                            })),
                    ),
            );

        if !all_requests.is_empty() {
            let mut req_pills = h_flex().gap(px(4.)).flex_wrap();
            for (idx, req_note) in all_requests.iter().enumerate() {
                let is_sel = self.form_assist.selected_request_path.as_deref()
                    == Some(req_note.path.as_str());
                let rel = req_note.path.clone();
                let full_edit = vault_root.join(&req_note.path);
                req_pills = req_pills.child(
                    div()
                        .id(SharedString::from(format!("ws-submit-req:{idx}")))
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
                        .child(req_note.title.clone())
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            if event.modifiers().platform || event.modifiers().control {
                                this.open_path_in_split_pane(full_edit.clone(), window, cx);
                            } else {
                                this.form_assist.selected_request_path = Some(rel.clone());
                                cx.notify();
                            }
                        })),
                );
            }
            req_card = req_card.child(req_pills);
        }

        req_card = req_card.child(Input::new(&self.form_assist.oid4vp_input).xsmall());

        let (active_req, _) = self.active_oid4vp_request(cx);
        let mut can_direct_post = false;
        if let Some(req) = &active_req {
            let uri_badge = match req.response_uri.as_deref() {
                Some(uri) => match validate_direct_post_uri(uri) {
                    Ok(v) if v.is_loopback => {
                        can_direct_post = true;
                        format!("ローカルテスト先: {uri}")
                    }
                    Ok(_) => {
                        can_direct_post = true;
                        format!("HTTPS 送信先: {uri}")
                    }
                    Err(e) => format!("⚠ 送信不可: {e}"),
                },
                None => "response_uri 未指定（コピー提出のみ）".to_string(),
            };
            req_card = req_card
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(format!("aud: {}", req.client_id)),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(if can_direct_post {
                            Hsla::from(rgb(0x1B5E20))
                        } else {
                            theme.muted_foreground
                        })
                        .truncate()
                        .child(uri_badge),
                );
            if !req.requested_claims.is_empty() {
                req_card = req_card
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(theme.foreground)
                            .child(format!("要求項目: {}", req.requested_claims.join(", "))),
                    )
                    .child(
                        Button::new("ws-submit-match-claims")
                            .xsmall()
                            .label("要求項目に開示設定を自動マッチ")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.match_disclosures_to_request(cx);
                            })),
                    );
            }
        }
        root = root.child(req_card);

        // 4. Submit Actions
        let has_source = using_vault_cred || active_sheet_claims_count > 0;
        let busy = self.form_assist.submit_busy;
        let mut submit_actions = h_flex()
            .gap_1()
            .flex_wrap()
            .child(
                Button::new("ws-submit-direct-post")
                    .primary()
                    .xsmall()
                    .label(if busy {
                        "送信中…"
                    } else {
                        "OID4VP 送信 (direct_post)"
                    })
                    .disabled(busy || !can_direct_post || !has_source)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.submit_oid4vp_direct_post(window, cx);
                    })),
            )
            .child(
                Button::new("ws-submit-copy-vp")
                    .xsmall()
                    .label("vp_token をコピー")
                    .disabled(!has_source)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.copy_vp_token_and_record(window, cx);
                    })),
            );
        if !using_vault_cred && active_sheet_claims_count > 0 {
            submit_actions = submit_actions.child(
                Button::new("ws-submit-save-sdjwt")
                    .xsmall()
                    .label("VC (.sd-jwt) を Vault に保存")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save_sheet_vc_to_vault(window, cx);
                    })),
            );
        }
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
                        .child("3. 提出・記録 (submissions/)"),
                )
                .child(submit_actions),
        );

        root
    }

    fn render_verify_mode(
        &self,
        mut root: Div,
        vault_root: &std::path::Path,
        all_credentials: &[VaultCredentialSummary],
        all_requests: &[PresentationRequestNote],
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();

        // 1. Batch Verification Policy Card
        let mut policy_pills = h_flex().gap(px(4.)).flex_wrap();
        let all_policy_sel = self.form_assist.verifier_request_path.is_none();
        policy_pills = policy_pills.child(
            div()
                .id("ws-verify-pol-all")
                .cursor_pointer()
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(10.5))
                .when(all_policy_sel, |d| {
                    d.bg(theme.accent)
                        .text_color(theme.accent_foreground)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!all_policy_sel, |d| {
                    d.bg(theme.muted)
                        .text_color(theme.foreground)
                        .hover(|h| h.bg(theme.secondary))
                })
                .child("全件チェック")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.form_assist.verifier_request_path = None;
                    cx.notify();
                })),
        );
        for (idx, req_note) in all_requests.iter().enumerate() {
            let is_sel = self.form_assist.verifier_request_path.as_deref()
                == Some(req_note.path.as_str());
            let rel = req_note.path.clone();
            policy_pills = policy_pills.child(
                div()
                    .id(SharedString::from(format!("ws-verify-pol:{idx}")))
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
                    .child(req_note.title.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.form_assist.verifier_request_path = Some(rel.clone());
                        cx.notify();
                    })),
            );
        }

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
                        .child(format!(
                            "受領 VC 一括検証（対象 {} 件）",
                            all_credentials.len()
                        )),
                )
                .child(policy_pills)
                .child(Input::new(&self.form_assist.verifier_jwk_input).xsmall())
                .child(
                    Button::new("ws-verify-run-batch")
                        .primary()
                        .xsmall()
                        .label("Vault 内 VC を一括検証")
                        .disabled(all_credentials.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run_vault_batch_verify(cx);
                        })),
                ),
        );

        // 2. Results KPI + BI Export Card
        if let Some(report) = &self.form_assist.last_verify_report {
            let json_for_copy = render_batch_verify_json(report);
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
                            .child(format!("検証サマリー（{}）", report.policy_label)),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .flex_wrap()
                            .child(
                                div()
                                    .px(px(5.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(rgb(0xE8F5E9))
                                    .text_size(px(10.))
                                    .text_color(rgb(0x1B5E20))
                                    .child(format!("検証済 {}", report.verified_count)),
                            )
                            .child(
                                div()
                                    .px(px(5.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(rgb(0xE3F2FD))
                                    .text_size(px(10.))
                                    .text_color(rgb(0x0D47A1))
                                    .child(format!("ダイジェストOK {}", report.unchecked_ok_count)),
                            )
                            .child(
                                div()
                                    .px(px(5.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(rgb(0xFFF8E1))
                                    .text_size(px(10.))
                                    .text_color(rgb(0x8A5A00))
                                    .child(format!("要確認 {}", report.needs_review_count)),
                            )
                            .child(
                                div()
                                    .px(px(5.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(rgb(0xFFEBEE))
                                    .text_size(px(10.))
                                    .text_color(rgb(0xC62828))
                                    .child(format!("無効 {}", report.invalid_count)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .pt_1()
                            .child(
                                Button::new("ws-verify-export-sheet")
                                    .primary()
                                    .xsmall()
                                    .label("BI シート (.sheet.md) 展開")
                                    .disabled(report.items.is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.export_batch_verify_sheet_md(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("ws-verify-export-csv")
                                    .xsmall()
                                    .label("CSV 保存")
                                    .disabled(report.items.is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.export_batch_verify_csv(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("ws-verify-copy-json")
                                    .xsmall()
                                    .label("JSON コピー")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            json_for_copy.clone(),
                                        ));
                                        this.form_assist.status_message = Some((
                                            "検証結果 JSON をクリップボードにコピーしました。"
                                                .into(),
                                            false,
                                        ));
                                        cx.notify();
                                    })),
                            ),
                    ),
            );

            // 3. Per-credential verdict cards
            for (idx, item) in report.items.iter().enumerate() {
                let (fg, bg) = match item.verdict {
                    VerifyVerdict::Verified => (rgb(0x1B5E20), rgb(0xE8F5E9)),
                    VerifyVerdict::DigestsOkUncheckedSig => (rgb(0x0D47A1), rgb(0xE3F2FD)),
                    VerifyVerdict::NeedsReview => (rgb(0x8A5A00), rgb(0xFFF8E1)),
                    VerifyVerdict::Invalid => (rgb(0xC62828), rgb(0xFFEBEE)),
                };
                let full_path = vault_root.join(&item.path);
                let mut card = v_flex()
                    .id(SharedString::from(format!("ws-verify-item:{idx}")))
                    .cursor_pointer()
                    .p(px(6.))
                    .gap(px(2.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .hover(|h| h.bg(theme.secondary))
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
                                    .child(item.path.clone()),
                            )
                            .child(
                                div()
                                    .px(px(5.))
                                    .py(px(1.))
                                    .rounded(px(3.))
                                    .bg(bg)
                                    .text_size(px(9.5))
                                    .text_color(fg)
                                    .child(item.verdict.label_ja()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(format!(
                                "[{}] {} · ダイジェスト {}/{} · クレーム {}",
                                item.format,
                                item.issuer.as_deref().unwrap_or("(発行者なし)"),
                                item.digests_verified,
                                item.digests_total,
                                item.claims.len()
                            )),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_path_in_split_pane(full_path.clone(), window, cx);
                    }));

                if !item.issues.is_empty() {
                    card = card.child(
                        div()
                            .text_size(px(10.))
                            .text_color(fg)
                            .child(item.issues.join(" / ")),
                    );
                }
                root = root.child(card);
            }
        } else if all_credentials.is_empty() {
            root = root.child(
                div()
                    .p_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Vault 内に .sd-jwt または .mdoc ファイルがありません。受領した証明書を credentials/ 等に配置すると一括検証・BI シート展開できます。"),
            );
        }

        root
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

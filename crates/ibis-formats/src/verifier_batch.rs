// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Phase 3 Verifier Batch Verification Engine & BI Export.
//!
//! Verifies collections of `.sd-jwt` and `.mdoc` credentials against an optional policy
//! (e.g. a Vault `requests/*.req.md` note and optional trusted Issuer public JWK), producing
//! structured diagnostics plus ready-to-open soraneDocs `.sheet.md` BI workbooks
//! (`SORANE.JSONTABLE`, `UNIQUE`, `COUNTIF`, `COUNTIFS`, `SORANE.FIELDMATCH`), CSV, and JSON.

use std::collections::{BTreeMap, BTreeSet};

use ibis_doc_model::sd_jwt::{Es256KeyPair, base64url_decode, verify_es256};
use ibis_doc_model::sd_jwt_binding::ClaimValue;
use ibis_doc_model::sd_jwt_import::{
    ParsedCredentialFormat, parse_sd_jwt, parse_single_mdoc_bytes, parse_single_mdoc_text,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::form_assist::{
    PresentationRequestNote, current_utc_timestamp_iso, value_as_display_string,
};

/// Verification policy applied across a batch of received credentials.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchVerifyPolicy {
    pub label: String,
    pub expected_paths: Vec<String>,
    pub expected_vct: Option<String>,
    /// Optional trusted Issuer public P-256 JWK JSON for strict ES256 signature verification.
    pub issuer_jwk_json: Option<String>,
    pub expected_audience: Option<String>,
    pub expected_nonce: Option<String>,
    /// Current Unix timestamp for `exp` / `nbf` validation (`None` skips time check).
    pub now_unix: Option<i64>,
}

impl BatchVerifyPolicy {
    /// Builds a batch verification policy from an OKF `type: presentation-request` Vault note.
    pub fn from_request_note(
        note: &PresentationRequestNote,
        issuer_jwk_json: Option<String>,
        now_unix: Option<i64>,
    ) -> Self {
        Self {
            label: note.title.clone(),
            expected_paths: note.requested_claims.clone(),
            expected_vct: note.vct.clone(),
            issuer_jwk_json,
            expected_audience: Some(note.client_id.clone()),
            expected_nonce: note.nonce.clone(),
            now_unix,
        }
    }
}

/// High-level verification verdict for a single credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyVerdict {
    /// Issuer signature explicitly verified, all disclosure digests match, policy & expiry satisfied.
    Verified,
    /// Disclosure digests & policy satisfied, but no trusted Issuer JWK was configured (or demo key used).
    DigestsOkUncheckedSig,
    /// Cryptographically parseable, but missing required claims, VCT mismatch, expired, or KB mismatch.
    NeedsReview,
    /// Parse error, invalid signature, or disclosure digest mismatch.
    Invalid,
}

impl VerifyVerdict {
    pub fn code(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::DigestsOkUncheckedSig => "unchecked_ok",
            Self::NeedsReview => "needs_review",
            Self::Invalid => "invalid",
        }
    }

    pub fn label_ja(self) -> &'static str {
        match self {
            Self::Verified => "検証済",
            Self::DigestsOkUncheckedSig => "ダイジェストOK（署名未確認）",
            Self::NeedsReview => "要確認",
            Self::Invalid => "無効",
        }
    }
}

/// Detailed verification diagnostics and extracted claims for one credential file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchVerifyItem {
    pub path: String,
    pub format: String,
    pub vct_or_doctype: String,
    pub issuer: Option<String>,
    pub verdict: VerifyVerdict,
    pub issuer_sig_status: String,
    pub digests_verified: usize,
    pub digests_total: usize,
    pub kb_present: bool,
    pub kb_valid: Option<bool>,
    pub kb_sd_hash_valid: Option<bool>,
    pub nonce_matches: Option<bool>,
    pub audience_matches: Option<bool>,
    pub expired: Option<bool>,
    pub vct_matches: Option<bool>,
    pub disclosed_paths: Vec<String>,
    pub missing_paths: Vec<String>,
    pub extra_paths: Vec<String>,
    pub claims: BTreeMap<String, Value>,
    pub issues: Vec<String>,
}

/// Summary report across all verified credentials in a batch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchVerifyReport {
    pub verified_at: String,
    pub policy_label: String,
    pub total: usize,
    pub verified_count: usize,
    pub unchecked_ok_count: usize,
    pub needs_review_count: usize,
    pub invalid_count: usize,
    pub all_claim_columns: Vec<String>,
    pub items: Vec<BatchVerifyItem>,
}

fn claim_value_to_json(v: &ClaimValue) -> Value {
    serde_json::from_str(&v.to_json()).unwrap_or(Value::Null)
}

fn decode_jwt_json(seg: &str) -> Option<Value> {
    serde_json::from_slice(&base64url_decode(seg)?).ok()
}

fn public_coords_from_value(jwk: &Value) -> Option<([u8; 32], [u8; 32])> {
    if jwk["kty"] != "EC" || jwk["crv"] != "P-256" {
        return None;
    }
    let x = base64url_decode(jwk["x"].as_str()?)?.try_into().ok()?;
    let y = base64url_decode(jwk["y"].as_str()?)?.try_into().ok()?;
    Some((x, y))
}

fn inspect_sd_jwt_kb_and_times(
    token: &str,
) -> (Option<i64>, Option<i64>, bool, Option<(Value, bool)>) {
    let trimmed = token.trim();
    let Some((issuer_jwt, rest)) = trimmed.split_once('~') else {
        return (None, None, false, None);
    };
    let issuer_payload = issuer_jwt.split('.').nth(1).and_then(decode_jwt_json);
    let exp = issuer_payload
        .as_ref()
        .and_then(|p| p.get("exp"))
        .and_then(Value::as_i64);
    let nbf = issuer_payload
        .as_ref()
        .and_then(|p| p.get("nbf"))
        .and_then(Value::as_i64);

    let last = rest.rsplit('~').next().unwrap_or("").trim();
    if last.is_empty() || !last.contains('.') {
        return (exp, nbf, false, None);
    }

    let kb = (|| {
        let (input, sig_b64) = last.rsplit_once('.')?;
        let (hdr_b64, pay_b64) = input.split_once('.')?;
        let header = decode_jwt_json(hdr_b64)?;
        let payload = decode_jwt_json(pay_b64)?;
        let valid = (|| {
            let jwk = &issuer_payload.as_ref()?["cnf"]["jwk"];
            let (x, y) = public_coords_from_value(jwk)?;
            let sig: [u8; 64] = base64url_decode(sig_b64)?.try_into().ok()?;
            Some(
                header["alg"] == "ES256"
                    && header["typ"] == "kb+jwt"
                    && verify_es256(&x, &y, input.as_bytes(), &sig),
            )
        })()
        .unwrap_or(false);
        Some((payload, valid))
    })();

    (exp, nbf, true, kb)
}

fn is_signed_by_demo_issuer_sd_jwt(token: &str) -> bool {
    let Some(issuer_jwt) = token.trim().split('~').next() else {
        return false;
    };
    let Some((input, sig_b64)) = issuer_jwt.rsplit_once('.') else {
        return false;
    };
    let Some(sig_vec) = base64url_decode(sig_b64) else {
        return false;
    };
    let Ok(sig): Result<[u8; 64], _> = sig_vec.try_into() else {
        return false;
    };
    let demo = Es256KeyPair::demo_issuer_key();
    verify_es256(&demo.x, &demo.y, input.as_bytes(), &sig)
}

/// Verifies a single `.sd-jwt` or `.mdoc` credential against `policy`.
pub fn verify_credential_bytes(
    path: &str,
    ext: &str,
    bytes: &[u8],
    policy: &BatchVerifyPolicy,
) -> BatchVerifyItem {
    let ext_lower = ext.to_ascii_lowercase();
    let trusted_jwk = policy
        .issuer_jwk_json
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let parsed_res = if ext_lower == "sd-jwt" {
        std::str::from_utf8(bytes)
            .map_err(|_| "UTF-8 として読み込めませんでした".to_string())
            .and_then(|s| parse_sd_jwt(s, trusted_jwk))
    } else if ext_lower == "mdoc" {
        parse_single_mdoc_bytes(bytes, trusted_jwk).or_else(|_| {
            std::str::from_utf8(bytes)
                .map_err(|_| "mDOC バイナリ/テキストの解析に失敗しました".to_string())
                .and_then(|s| parse_single_mdoc_text(s, trusted_jwk))
        })
    } else {
        Err(format!("未対応の拡張子です: {ext}"))
    };

    let parsed = match parsed_res {
        Ok(p) => p,
        Err(err) => {
            return BatchVerifyItem {
                path: path.to_string(),
                format: ext_lower,
                vct_or_doctype: String::new(),
                issuer: None,
                verdict: VerifyVerdict::Invalid,
                issuer_sig_status: "invalid".to_string(),
                digests_verified: 0,
                digests_total: 0,
                kb_present: false,
                kb_valid: None,
                kb_sd_hash_valid: None,
                nonce_matches: None,
                audience_matches: None,
                expired: None,
                vct_matches: None,
                disclosed_paths: Vec::new(),
                missing_paths: policy.expected_paths.clone(),
                extra_paths: Vec::new(),
                claims: BTreeMap::new(),
                issues: vec![format!("解析エラー: {err}")],
            };
        }
    };

    let format = match parsed.format {
        ParsedCredentialFormat::SdJwt => "sd-jwt".to_string(),
        ParsedCredentialFormat::Mdoc => "mdoc".to_string(),
    };

    let mut issues = Vec::new();
    let mut hard_invalid = false;
    let mut needs_review = false;

    // 1. Issuer signature assessment (do NOT treat built-in demo key as trusted when no explicit JWK is provided)
    let raw_text = std::str::from_utf8(bytes).ok();
    let demo_signed = format == "sd-jwt"
        && trusted_jwk.is_none()
        && raw_text.is_some_and(is_signed_by_demo_issuer_sd_jwt);
    let (issuer_sig_status, sig_trusted) = if trusted_jwk.is_some() {
        match parsed.signature_valid {
            Some(true) => ("verified".to_string(), true),
            Some(false) => {
                hard_invalid = true;
                issues.push("指定された Issuer 公開鍵で署名が一致しませんでした。".to_string());
                ("invalid".to_string(), false)
            }
            None => {
                needs_review = true;
                issues.push("Issuer 署名を検証できませんでした。".to_string());
                ("unchecked".to_string(), false)
            }
        }
    } else if demo_signed {
        issues.push("Sandbox デモ鍵で署名されています（本番信頼なし）。".to_string());
        ("sandbox_demo".to_string(), false)
    } else {
        match parsed.signature_valid {
            Some(true) => ("embedded_jwk_valid".to_string(), false),
            Some(false) => {
                hard_invalid = true;
                issues.push("埋め込み鍵での署名検証に失敗しました。".to_string());
                ("invalid".to_string(), false)
            }
            None => ("unchecked".to_string(), false),
        }
    };

    // 2. Disclosure digest verification
    if parsed.digests_verified < parsed.digests_total {
        hard_invalid = true;
        issues.push(format!(
            "ダイジェスト不一致 ({}/{})",
            parsed.digests_verified, parsed.digests_total
        ));
    }
    if parsed.claims.iter().any(|c| c.digest_verified == Some(false)) {
        hard_invalid = true;
        issues.push("改ざんされたクレームダイジェストを検出しました。".to_string());
    }

    // 3. SD-JWT time validity & KB-JWT checks
    let (expired, kb_present, kb_valid, nonce_matches, audience_matches) = if format == "sd-jwt"
        && let Some(txt) = raw_text
    {
        let (exp, nbf, kb_present, kb) = inspect_sd_jwt_kb_and_times(txt);
        let expired = match (exp, policy.now_unix) {
            (Some(e), Some(now)) => {
                let is_exp = e < now;
                if is_exp {
                    needs_review = true;
                    issues.push(format!("有効期限切れ (exp={e})"));
                }
                Some(is_exp)
            }
            _ => None,
        };
        if let (Some(n), Some(now)) = (nbf, policy.now_unix)
            && n > now + 60
        {
            needs_review = true;
            issues.push(format!("有効開始前 (nbf={n})"));
        }
        let kb_valid = kb.as_ref().map(|(_, v)| *v);
        if kb_valid == Some(false) {
            hard_invalid = true;
            issues.push("Holder Key Binding (kb+jwt) の署名が無効です。".to_string());
        }
        if parsed.kb_sd_hash_valid == Some(false) {
            hard_invalid = true;
            issues.push("kb+jwt の sd_hash が一致しません。".to_string());
        }
        let nonce_matches = policy.expected_nonce.as_deref().map(|expected| {
            let ok = kb
                .as_ref()
                .and_then(|(p, _)| p.get("nonce").and_then(Value::as_str))
                == Some(expected);
            if !ok {
                needs_review = true;
                issues.push("kb+jwt の nonce が要求値と一致しません。".to_string());
            }
            ok
        });
        let audience_matches = policy.expected_audience.as_deref().map(|expected| {
            if !kb_present {
                return false;
            }
            let ok = kb
                .as_ref()
                .and_then(|(p, _)| p.get("aud").and_then(Value::as_str))
                == Some(expected);
            if !ok {
                needs_review = true;
                issues.push("kb+jwt の aud が提出先 client_id と一致しません。".to_string());
            }
            ok
        });
        (expired, kb_present, kb_valid, nonce_matches, audience_matches)
    } else {
        (None, false, None, None, None)
    };

    // 4. VCT / docType check
    let vct_matches = policy.expected_vct.as_deref().map(|expected| {
        let ok = parsed.vct_or_doctype == expected;
        if !ok {
            needs_review = true;
            issues.push(format!(
                "VCT/docType 不一致（期待: {expected}, 実際: {}）",
                parsed.vct_or_doctype
            ));
        }
        ok
    });

    // 5. Claims extraction & expected_paths diff
    let mut claims = BTreeMap::new();
    let mut actual_set = BTreeSet::new();
    for c in &parsed.claims {
        if matches!(c.path.as_str(), "iss" | "vct") {
            continue;
        }
        actual_set.insert(c.path.clone());
        claims.insert(c.path.clone(), claim_value_to_json(&c.value));
    }
    let disclosed_paths: Vec<String> = actual_set.iter().cloned().collect();

    let mut missing_paths = Vec::new();
    let mut extra_paths = Vec::new();
    if !policy.expected_paths.is_empty() {
        let expected_set: BTreeSet<String> = policy.expected_paths.iter().cloned().collect();
        missing_paths = expected_set.difference(&actual_set).cloned().collect();
        extra_paths = actual_set.difference(&expected_set).cloned().collect();
        if !missing_paths.is_empty() {
            needs_review = true;
            issues.push(format!("必須クレーム不足: {}", missing_paths.join(", ")));
        }
    }

    let verdict = if hard_invalid {
        VerifyVerdict::Invalid
    } else if needs_review {
        VerifyVerdict::NeedsReview
    } else if sig_trusted {
        VerifyVerdict::Verified
    } else {
        VerifyVerdict::DigestsOkUncheckedSig
    };

    BatchVerifyItem {
        path: path.to_string(),
        format,
        vct_or_doctype: parsed.vct_or_doctype,
        issuer: parsed.issuer,
        verdict,
        issuer_sig_status,
        digests_verified: parsed.digests_verified,
        digests_total: parsed.digests_total,
        kb_present,
        kb_valid,
        kb_sd_hash_valid: parsed.kb_sd_hash_valid,
        nonce_matches,
        audience_matches,
        expired,
        vct_matches,
        disclosed_paths,
        missing_paths,
        extra_paths,
        claims,
        issues,
    }
}

/// Runs batch verification across a slice of `(relative_path, ext, raw_bytes)` credential entries.
pub fn verify_credential_batch(
    entries: &[(&str, &str, &[u8])],
    policy: &BatchVerifyPolicy,
) -> BatchVerifyReport {
    let mut items = Vec::with_capacity(entries.len());
    let mut verified_count = 0usize;
    let mut unchecked_ok_count = 0usize;
    let mut needs_review_count = 0usize;
    let mut invalid_count = 0usize;
    let mut all_cols = BTreeSet::new();

    for &(path, ext, bytes) in entries {
        let item = verify_credential_bytes(path, ext, bytes, policy);
        match item.verdict {
            VerifyVerdict::Verified => verified_count += 1,
            VerifyVerdict::DigestsOkUncheckedSig => unchecked_ok_count += 1,
            VerifyVerdict::NeedsReview => needs_review_count += 1,
            VerifyVerdict::Invalid => invalid_count += 1,
        }
        for k in item.claims.keys() {
            all_cols.insert(k.clone());
        }
        items.push(item);
    }

    BatchVerifyReport {
        verified_at: current_utc_timestamp_iso(),
        policy_label: if policy.label.trim().is_empty() {
            "Vault 全件検証".to_string()
        } else {
            policy.label.clone()
        },
        total: items.len(),
        verified_count,
        unchecked_ok_count,
        needs_review_count,
        invalid_count,
        all_claim_columns: all_cols.into_iter().collect(),
        items,
    }
}

fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn col_letters(mut index_zero: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push((b'A' + (index_zero % 26) as u8) as char);
        if index_zero < 26 {
            break;
        }
        index_zero = index_zero / 26 - 1;
    }
    out.into_iter().rev().collect()
}

/// Renders a [`BatchVerifyReport`] as a live soraneDocs Markdown Workbook (`.sheet.md`) with:
/// - `Raw`: raw records + structured `payload_json`
/// - `一覧`: dynamic `=SORANE.JSONTABLE(...)` projection of verification fields & disclosed claims
/// - `集計`: `=UNIQUE(...)`, `=COUNTIF(...)`, `=COUNTIFS(...)` verdict & issuer rollup
/// - `突合`: ready-to-use `=SORANE.FIELDMATCH(...)` matching table
pub fn render_batch_verify_sheet_md(report: &BatchVerifyReport) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("ibis-workbook: true\n");
    out.push_str("type: verification-report\n");
    out.push_str(&format!("verified_at: \"{}\"\n", report.verified_at));
    out.push_str(&format!(
        "policy: \"{}\"\n",
        report.policy_label.replace('"', "\\\"")
    ));
    out.push_str("---\n\n");

    // Sheet 1: Raw
    out.push_str("<!-- xlsx-sheet \"Raw\" -->\n\n");
    out.push_str("| record_id | file_path | format | issuer | verdict | payload_json |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for (idx, item) in report.items.iter().enumerate() {
        let rec_id = format!("vc-{:03}", idx + 1);
        let issuer = item.issuer.as_deref().unwrap_or("(未指定)");
        let payload = json!({
            "file_path": item.path,
            "format": item.format,
            "vct": item.vct_or_doctype,
            "issuer": issuer,
            "verification": {
                "verdict": item.verdict.code(),
                "verdict_ja": item.verdict.label_ja(),
                "issuer_sig": item.issuer_sig_status,
                "digests": format!("{}/{}", item.digests_verified, item.digests_total),
                "missing_count": item.missing_paths.len(),
                "issues": item.issues.join("; "),
            },
            "claims": item.claims,
        });
        let payload_str = escape_md_cell(&payload.to_string());
        out.push_str(&format!(
            "| {rec_id} | {} | {} | {} | {} | {payload_str} |\n",
            escape_md_cell(&item.path),
            escape_md_cell(&item.format),
            escape_md_cell(issuer),
            item.verdict.code(),
        ));
    }
    out.push('\n');

    // Sheet 2: 一覧 (driven by SORANE.JSONTABLE)
    let claim_cols: Vec<String> = report.all_claim_columns.iter().take(12).cloned().collect();
    let mut headers = vec![
        "記録ID".to_string(),
        "ファイル".to_string(),
        "判定".to_string(),
        "署名状態".to_string(),
        "ダイジェスト".to_string(),
        "発行者".to_string(),
        "種別(VCT)".to_string(),
        "不足項目数".to_string(),
    ];
    let mut json_paths = vec![
        "record_id".to_string(),
        "file_path".to_string(),
        "verification.verdict_ja".to_string(),
        "verification.issuer_sig".to_string(),
        "verification.digests".to_string(),
        "issuer".to_string(),
        "vct".to_string(),
        "verification.missing_count".to_string(),
    ];
    for c in &claim_cols {
        headers.push(c.clone());
        json_paths.push(format!("claims.{c}"));
    }

    let last_col = col_letters(headers.len().saturating_sub(1));
    out.push_str("<!-- xlsx-sheet \"一覧\" -->\n\n");
    out.push_str(&format!("| {} |\n", headers.iter().map(|h| escape_md_cell(h)).collect::<Vec<_>>().join(" | ")));
    out.push_str(&format!(
        "| {} |\n",
        vec!["---"; headers.len()].join(" | ")
    ));
    out.push_str(&format!(
        "| {} |\n",
        json_paths
            .iter()
            .map(|p| escape_md_cell(p))
            .collect::<Vec<_>>()
            .join(" | ")
    ));
    let mut formula_row = vec![String::new(); headers.len()];
    formula_row[0] = "=FILTER(Raw!A2:A1000,Raw!A2:A1000<>\"\")".to_string();
    formula_row[1] = format!("=SORANE.JSONTABLE(FILTER(Raw!F2:F1000,Raw!F2:F1000<>\"\"),B2:{last_col}2)");
    out.push_str(&format!("| {} |\n\n", formula_row.join(" | ")));

    // Sheet 3: 集計 (UNIQUE + COUNTIF + COUNTIFS)
    out.push_str("<!-- xlsx-sheet \"集計\" -->\n\n");
    out.push_str("| 発行者 | 総件数 | 検証済 | ダイジェストOK | 要確認 | 無効 |\n");
    out.push_str("| --- | ---: | ---: | ---: | ---: | ---: |\n");
    out.push_str("| =UNIQUE(FILTER(Raw!D2:D1000,Raw!D2:D1000<>\"\")) | =COUNTIF(Raw!D2:D1000,A2#) | =COUNTIFS(Raw!D2:D1000,A2#,Raw!E2:E1000,\"verified\") | =COUNTIFS(Raw!D2:D1000,A2#,Raw!E2:E1000,\"unchecked_ok\") | =COUNTIFS(Raw!D2:D1000,A2#,Raw!E2:E1000,\"needs_review\") | =COUNTIFS(Raw!D2:D1000,A2#,Raw!E2:E1000,\"invalid\") |\n\n");

    // Sheet 4: 突合 (SORANE.FIELDMATCH)
    let match_col_idx = claim_cols
        .iter()
        .position(|c| {
            let l = c.to_ascii_lowercase();
            l.contains("name") || c.contains("氏名") || c.contains("会社")
        })
        .unwrap_or(0);
    let target_col_letter = if !claim_cols.is_empty() {
        col_letters(8 + match_col_idx)
    } else {
        "B".to_string()
    };
    let sample_ref = report
        .items
        .first()
        .and_then(|item| {
            claim_cols
                .get(match_col_idx)
                .and_then(|k| item.claims.get(k))
                .and_then(value_as_display_string)
        })
        .unwrap_or_else(|| "照合基準を入力".to_string());
    let last_row = (report.items.len() + 2).max(3);
    out.push_str("<!-- xlsx-sheet \"突合\" -->\n\n");
    out.push_str("| 対象値 | 照合基準 | 判定 (FIELDMATCH) |\n");
    out.push_str("| --- | --- | ---: |\n");
    out.push_str(&format!(
        "| =FILTER(一覧!{target_col_letter}3:{target_col_letter}1000,一覧!{target_col_letter}3:{target_col_letter}1000<>\"\") | {} | =SORANE.FIELDMATCH(A2:A{last_row},B2,\"name\") |\n",
        escape_md_cell(&sample_ref)
    ));

    out
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Renders a [`BatchVerifyReport`] as an RFC 4180 UTF-8 CSV table for SQLite / external BI ingestion.
pub fn render_batch_verify_csv(report: &BatchVerifyReport) -> String {
    let mut out = String::new();
    let mut header = vec![
        "file_path",
        "format",
        "vct_or_doctype",
        "issuer",
        "verdict",
        "issuer_sig_status",
        "digests_verified",
        "digests_total",
        "missing_paths",
        "issues",
    ];
    let claim_refs: Vec<&str> = report.all_claim_columns.iter().map(String::as_str).collect();
    header.extend(claim_refs.iter().copied());
    out.push_str(
        &header
            .into_iter()
            .map(csv_escape)
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');

    for item in &report.items {
        let mut row: Vec<String> = vec![
            item.path.clone(),
            item.format.clone(),
            item.vct_or_doctype.clone(),
            item.issuer.clone().unwrap_or_default(),
            item.verdict.code().to_string(),
            item.issuer_sig_status.clone(),
            item.digests_verified.to_string(),
            item.digests_total.to_string(),
            item.missing_paths.join("; "),
            item.issues.join("; "),
        ];
        for col in &report.all_claim_columns {
            let val = item
                .claims
                .get(col)
                .and_then(value_as_display_string)
                .unwrap_or_default();
            row.push(val);
        }
        out.push_str(&row.iter().map(|s| csv_escape(s)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

/// Renders a [`BatchVerifyReport`] as pretty-printed JSON.
pub fn render_batch_verify_json(report: &BatchVerifyReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use ibis_doc_model::sd_jwt::{Es256KeyPair, SdJwtOptions, issue_sd_jwt};
    use ibis_doc_model::sd_jwt_binding::{
        BindingSource, BoundClaim, ClaimValue, SheetClaimBindings,
    };
    use ibis_doc_model::SlideId;

    use super::*;

    fn make_sd_jwt(
        issuer_key: &Es256KeyPair,
        holder_key: &Es256KeyPair,
        given_name: &str,
        exp: Option<i64>,
        include_region: bool,
    ) -> String {
        let mut claims = vec![BoundClaim {
            path: vec!["given_name".into()],
            path_str: "given_name".into(),
            selective: true,
            disclose: true,
            value: ClaimValue::String(given_name.into()),
            source: BindingSource::Cell { row: 1, column: 1 },
        }];
        if include_region {
            claims.push(BoundClaim {
                path: vec!["address".into(), "region".into()],
                path_str: "address.region".into(),
                selective: true,
                disclose: true,
                value: ClaimValue::String("東京都".into()),
                source: BindingSource::Cell { row: 2, column: 1 },
            });
        }
        let bindings = SheetClaimBindings {
            slide: SlideId(1),
            claims,
            disclosures: Default::default(),
        };
        let mut opts = SdJwtOptions::default();
        opts.default_iss = Some("https://issuer.example.jp".into());
        opts.default_vct = Some("urn:jp:test:pid:1".into());
        opts.default_exp = exp;
        opts.holder_public_jwk = Some(holder_key.to_public_jwk());
        let issued = issue_sd_jwt(&bindings, &opts, issuer_key);
        issued
            .attach_key_binding("https://verifier.example.jp", "n-batch-1", 1_760_000_000, holder_key)
            .vp_token
    }

    #[test]
    fn batch_verifier_classifies_verdicts_and_exports_sheet_md_csv_json() {
        let issuer_key = Es256KeyPair::from_secret_bytes([7u8; 32]).unwrap();
        let holder_key = Es256KeyPair::from_secret_bytes([9u8; 32]).unwrap();
        let wrong_issuer = Es256KeyPair::from_secret_bytes([11u8; 32]).unwrap();

        let valid_tok = make_sd_jwt(&issuer_key, &holder_key, "花子", Some(1_900_000_000), true);
        let expired_missing_tok =
            make_sd_jwt(&issuer_key, &holder_key, "太郎", Some(1_600_000_000), false);
        let bad_sig_tok = make_sd_jwt(&wrong_issuer, &holder_key, "不正", Some(1_900_000_000), true);

        let policy = BatchVerifyPolicy {
            label: "テスト一括検証".into(),
            expected_paths: vec!["given_name".into(), "address.region".into()],
            expected_vct: Some("urn:jp:test:pid:1".into()),
            issuer_jwk_json: Some(issuer_key.to_public_jwk_json()),
            expected_audience: Some("https://verifier.example.jp".into()),
            expected_nonce: Some("n-batch-1".into()),
            now_unix: Some(1_760_000_100),
        };

        let entries = [
            ("credentials/hanako.sd-jwt", "sd-jwt", valid_tok.as_bytes()),
            (
                "credentials/taro-expired.sd-jwt",
                "sd-jwt",
                expired_missing_tok.as_bytes(),
            ),
            (
                "credentials/bad-sig.sd-jwt",
                "sd-jwt",
                bad_sig_tok.as_bytes(),
            ),
        ];

        let report = verify_credential_batch(&entries, &policy);
        assert_eq!(report.total, 3);
        assert_eq!(report.verified_count, 1);
        assert_eq!(report.needs_review_count, 1);
        assert_eq!(report.invalid_count, 1);

        assert_eq!(report.items[0].verdict, VerifyVerdict::Verified);
        assert_eq!(report.items[0].kb_valid, Some(true));
        assert_eq!(report.items[0].audience_matches, Some(true));
        assert_eq!(report.items[0].nonce_matches, Some(true));

        assert_eq!(report.items[1].verdict, VerifyVerdict::NeedsReview);
        assert_eq!(report.items[1].expired, Some(true));
        assert_eq!(report.items[1].missing_paths, vec!["address.region".to_string()]);

        assert_eq!(report.items[2].verdict, VerifyVerdict::Invalid);
        assert_eq!(report.items[2].issuer_sig_status, "invalid");

        // Verify BI exports: `.sheet.md` (with JSONTABLE / COUNTIFS / FIELDMATCH), CSV, JSON
        let sheet_md = render_batch_verify_sheet_md(&report);
        assert!(sheet_md.contains("ibis-workbook: true"));
        assert!(sheet_md.contains("<!-- xlsx-sheet \"Raw\" -->"));
        assert!(sheet_md.contains("<!-- xlsx-sheet \"一覧\" -->"));
        assert!(sheet_md.contains("=SORANE.JSONTABLE("));
        assert!(sheet_md.contains("=COUNTIFS("));
        assert!(sheet_md.contains("=SORANE.FIELDMATCH("));
        // Confirm soraneDocs's own Markdown workbook importer parses it cleanly into a 4-sheet Workbook!
        let opened = crate::markdown::open_markdown_workbook(sheet_md.as_bytes().to_vec())
            .expect("generated BI .sheet.md imports cleanly as a workbook");
        assert_eq!(crate::Kind::of(&opened.presentation), crate::Kind::Workbook);
        assert_eq!(opened.presentation.slides.len(), 4);
        let sheet_names: Vec<&str> = opened
            .presentation
            .workbook
            .as_ref()
            .unwrap()
            .sheets
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(sheet_names, vec!["Raw", "一覧", "集計", "突合"]);

        let csv = render_batch_verify_csv(&report);
        assert!(csv.contains("file_path,format,vct_or_doctype"));
        assert!(csv.contains("credentials/hanako.sd-jwt"));
        assert!(csv.contains("花子"));

        let json_str = render_batch_verify_json(&report);
        let reparsed: BatchVerifyReport = serde_json::from_str(&json_str).unwrap();
        assert_eq!(reparsed.total, 3);
    }
}

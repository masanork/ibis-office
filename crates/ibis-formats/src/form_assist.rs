// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Phase 1 Form Assist core: OKF `type: profile` & `type: form-mapping` parsers,
//! Vault credential claim indexing, Japanese Era / split-cell / checkbox-mark transforms,
//! heuristic draft mapping inference from unlabeled worksheets, and 1-Undo `SetCells` command building.

use std::collections::{BTreeMap, BTreeSet};

use ibis_doc_model::sd_jwt_binding::ClaimValue;
use ibis_doc_model::sd_jwt_import::{
    ParsedCredentialFormat, parse_sd_jwt, parse_single_mdoc_bytes, parse_single_mdoc_text,
};
use ibis_doc_model::sheet::{CellValue, Sheet, cell_name};
use ibis_doc_model::sheet_calc::sheet_of_slide;
use ibis_doc_model::sheet_command::{CellEdit, CellInput, SheetCommand};
use ibis_doc_model::{Command, Presentation, SlideId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// 1. Vault Profile (`type: profile`)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultProfile {
    pub path: String,
    pub profile_id: String,
    pub title: String,
    pub scope: String,
    pub fields: BTreeMap<String, Value>,
}

fn flatten_yaml_fields(prefix: &str, value: &Value, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_yaml_fields(&key, v, out);
            }
        }
        Value::Null => {}
        other => {
            if !prefix.is_empty() {
                out.insert(prefix.to_string(), other.clone());
            }
        }
    }
}

/// Parses an OKF `type: profile` Markdown note into a [`VaultProfile`].
pub fn parse_profile_markdown(path: &str, text: &str) -> Option<VaultProfile> {
    let fm = crate::okf::frontmatter_map(text).ok()??;
    let doc_type = fm.get("type")?.as_str()?.trim();
    if !doc_type.eq_ignore_ascii_case("profile") {
        return None;
    }
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .trim_end_matches(".markdown")
        .to_string();
    let title = fm
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&stem)
        .to_string();

    let ext = fm.get("x-sorane-profile").and_then(Value::as_object);
    let profile_id = ext
        .and_then(|o| o.get("profile_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&stem)
        .to_string();
    let scope = ext
        .and_then(|o| o.get("scope"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("custom")
        .to_string();

    let mut fields = BTreeMap::new();
    if let Some(ext_fields) = ext.and_then(|o| o.get("fields")) {
        flatten_yaml_fields("", ext_fields, &mut fields);
    } else if let Some(top_fields) = fm.get("fields") {
        flatten_yaml_fields("", top_fields, &mut fields);
    }
    // Also support flat custom keys in `x-sorane-profile` outside reserved meta keys.
    if fields.is_empty()
        && let Some(ext_obj) = ext
    {
        for (k, v) in ext_obj {
            if matches!(k.as_str(), "version" | "profile_id" | "scope" | "fields") {
                continue;
            }
            flatten_yaml_fields(k, v, &mut fields);
        }
    }

    Some(VaultProfile {
        path: path.to_string(),
        profile_id,
        title,
        scope,
        fields,
    })
}

/// Canonical alias groups shared across Vault profiles and SD-JWT PID / mDL claims.
const FIELD_ALIAS_GROUPS: &[&[&str]] = &[
    &["organization.name", "company.name", "company_name", "employer.name"],
    &["organization.name_kana", "company.name_kana", "company_name_kana"],
    &["organization.corporate_number", "corporate_number"],
    &["organization.invoice_number", "invoice_number", "t_number"],
    &["organization.representative", "representative", "ceo"],
    &["person.family_name", "family_name", "last_name"],
    &["person.given_name", "given_name", "first_name"],
    &["person.full_name", "full_name", "name"],
    &["person.name_kana", "name_kana", "furigana"],
    &["person.birthdate", "birthdate", "birth_date"],
    &["address.postal_code", "postal_code", "zip_code"],
    &["address.prefecture", "address.region", "prefecture", "region"],
    &["address.locality", "locality", "city"],
    &["address.street", "address.street_address", "street_address"],
    &["address.full", "address.formatted", "address"],
    &["contact.phone", "phone", "phone_number", "tel"],
    &["contact.email", "email", "mail"],
    &["bank.name", "bank_name"],
    &["bank.branch", "bank_branch"],
    &["bank.account_type", "account_type"],
    &["bank.account_number", "account_number"],
];

pub fn alias_candidates(key: &str) -> Vec<String> {
    let norm = key.trim().to_ascii_lowercase();
    let mut out = vec![key.trim().to_string()];
    if norm != out[0] {
        out.push(norm.clone());
    }
    for group in FIELD_ALIAS_GROUPS {
        if group.iter().any(|candidate| *candidate == norm) {
            for candidate in *group {
                if !out.iter().any(|existing| existing == candidate) {
                    out.push((*candidate).to_string());
                }
            }
        }
    }
    out
}

impl VaultProfile {
    /// Looks up a field value by exact key, canonical alias, or synthesized composite
    /// (`person.full_name` from `family_name` + `given_name`, `address.full` from parts).
    pub fn resolve_value(&self, key: &str) -> Option<(String, Value)> {
        let candidates = alias_candidates(key);
        for cand in &candidates {
            if let Some(val) = self.fields.get(cand) {
                return Some((cand.clone(), val.clone()));
            }
        }
        // Case-insensitive fallback
        for cand in &candidates {
            let lower = cand.to_ascii_lowercase();
            if let Some((k, val)) = self
                .fields
                .iter()
                .find(|(k, _)| k.to_ascii_lowercase() == lower)
            {
                return Some((k.clone(), val.clone()));
            }
        }
        // Synthesize `person.full_name` from family_name + given_name
        if candidates.iter().any(|c| c == "person.full_name") {
            let family = self
                .resolve_direct_str(&["person.family_name", "family_name"])
                .unwrap_or_default();
            let given = self
                .resolve_direct_str(&["person.given_name", "given_name"])
                .unwrap_or_default();
            if !family.is_empty() && !given.is_empty() {
                return Some((
                    "person.family_name+given_name".to_string(),
                    Value::String(format!("{family} {given}")),
                ));
            }
        }
        // Synthesize `address.full` from prefecture + locality + street
        if candidates.iter().any(|c| c == "address.full") {
            let pref = self
                .resolve_direct_str(&["address.prefecture", "address.region", "prefecture"])
                .unwrap_or_default();
            let loc = self
                .resolve_direct_str(&["address.locality", "locality", "city"])
                .unwrap_or_default();
            let street = self
                .resolve_direct_str(&["address.street", "address.street_address", "street_address"])
                .unwrap_or_default();
            let combined = format!("{pref}{loc}{street}");
            if !combined.trim().is_empty() {
                return Some(("address.*".to_string(), Value::String(combined)));
            }
        }
        None
    }

    fn resolve_direct_str(&self, keys: &[&str]) -> Option<String> {
        for key in keys {
            if let Some(val) = self.fields.get(*key)
                && let Some(s) = value_as_display_string(val)
                && !s.is_empty()
            {
                return Some(s);
            }
        }
        None
    }
}

/// Starter OKF `type: profile` Markdown templates (`"organization"` or `"person"`).
pub fn starter_profile_markdown(scope: &str) -> (&'static str, &'static str) {
    if scope.eq_ignore_ascii_case("person") {
        (
            "profiles/me.md",
            r#"---
type: profile
title: 本人プロフィール（個人）
tags: [profile, person]
x-sorane-profile:
  version: 1
  profile_id: person-default
  scope: person
  fields:
    person.family_name: 山田
    person.given_name: 花子
    person.full_name: 山田 花子
    person.name_kana: ヤマダ ハナコ
    person.birthdate: "1990-05-15"
    address.postal_code: "100-0001"
    address.prefecture: 東京都
    address.locality: 千代田区千代田
    address.street: "1-1-1"
    contact.phone: "090-1234-5678"
    contact.email: hanako@example.com
---

# 本人プロフィール

各種申請書・届出書のフォーム自動入力に使う個人プロフィールです。内容を書き換えて保存すると、スプレッドシートの Form Assist から 1 クリックで入力できます。
"#,
        )
    } else {
        (
            "profiles/company.md",
            r#"---
type: profile
title: 自社プロフィール（法人）
tags: [profile, company]
x-sorane-profile:
  version: 1
  profile_id: company-default
  scope: organization
  fields:
    organization.name: 株式会社サンプル
    organization.name_kana: カブシキガイシャサンプル
    organization.corporate_number: "1234567890123"
    organization.invoice_number: T1234567890123
    organization.representative: 山田 太郎
    address.postal_code: "100-0001"
    address.prefecture: 東京都
    address.locality: 千代田区千代田
    address.street: "1-1-1"
    contact.phone: "03-1234-5678"
    contact.email: billing@example.co.jp
    bank.name: サンプル銀行
    bank.branch: 本店営業部
    bank.account_type: 普通
    bank.account_number: "0123456"
---

# 自社プロフィール

請求書・見積書・各種申請書の自動入力に使う法人マスタです。内容を書き換えて保存すると、Form Assist から 1 クリックで帳票へ反映できます。
"#,
        )
    }
}

// ---------------------------------------------------------------------------
// 2. Vault Credential Summary (`.sd-jwt` / `.mdoc`)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCredentialSummary {
    pub path: String,
    pub format: String,
    pub vct_or_doctype: String,
    pub issuer: Option<String>,
    pub digests_verified: usize,
    pub digests_total: usize,
    pub sig_status: String,
    pub claims: BTreeMap<String, Value>,
}

fn claim_value_to_json(v: &ClaimValue) -> Value {
    serde_json::from_str(&v.to_json()).unwrap_or(Value::Null)
}

/// Parses a Vault `.sd-jwt` or `.mdoc` file into a cached [`VaultCredentialSummary`].
pub fn parse_credential_bytes(
    path: &str,
    ext: &str,
    bytes: &[u8],
) -> Option<VaultCredentialSummary> {
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let ext_lower = ext.to_ascii_lowercase();
    let parsed = match ext_lower.as_str() {
        "sd-jwt" => {
            let text = std::str::from_utf8(bytes).ok()?;
            parse_sd_jwt(text.trim(), None).ok()?
        }
        "mdoc" => match std::str::from_utf8(bytes) {
            Ok(text)
                if text
                    .trim()
                    .bytes()
                    .all(|b| b.is_ascii_graphic() || b.is_ascii_whitespace()) =>
            {
                parse_single_mdoc_text(text.trim(), None).ok()?
            }
            _ => parse_single_mdoc_bytes(bytes, None).ok()?,
        },
        _ => return None,
    };

    let format = match parsed.format {
        ParsedCredentialFormat::SdJwt => "sd-jwt".to_string(),
        ParsedCredentialFormat::Mdoc => "mdoc".to_string(),
    };
    let sig_status = match parsed.signature_valid {
        Some(true) => "valid".to_string(),
        Some(false) => "invalid".to_string(),
        None => "unchecked".to_string(),
    };
    let mut claims = BTreeMap::new();
    for claim in &parsed.claims {
        let json_val = claim_value_to_json(&claim.value);
        if json_val.is_null() {
            continue;
        }
        if let Some((_, element)) = &claim.mdoc_element {
            claims.entry(element.clone()).or_insert_with(|| json_val.clone());
        }
        claims.insert(claim.path.clone(), json_val);
    }

    Some(VaultCredentialSummary {
        path: path.to_string(),
        format,
        vct_or_doctype: parsed.vct_or_doctype,
        issuer: parsed.issuer,
        digests_verified: parsed.digests_verified,
        digests_total: parsed.digests_total,
        sig_status,
        claims,
    })
}

impl VaultCredentialSummary {
    pub fn resolve_value(&self, key: &str) -> Option<(String, Value)> {
        let candidates = alias_candidates(key);
        for cand in &candidates {
            if let Some(val) = self.claims.get(cand) {
                return Some((cand.clone(), val.clone()));
            }
        }
        for cand in &candidates {
            let lower = cand.to_ascii_lowercase();
            if let Some((k, val)) = self
                .claims
                .iter()
                .find(|(k, _)| k.to_ascii_lowercase() == lower)
            {
                return Some((k.clone(), val.clone()));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// 3. External Form Mapping (`type: form-mapping`) & Target Transforms
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormAnchor {
    pub cell: String,
    pub contains: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormMatch {
    pub template_sha256: Option<String>,
    pub sheet: Option<String>,
    pub anchors: Vec<FormAnchor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormTarget {
    Cell(String),
    DefinedName(String),
    SplitDateYmd {
        year: String,
        month: String,
        day: String,
    },
    SplitDateEra {
        era: Option<String>,
        year: String,
        month: String,
        day: String,
    },
    RadioMark {
        checked: String,
        unchecked: String,
        options: BTreeMap<String, String>,
    },
    SplitDigits {
        cells: Vec<String>,
        lengths: Vec<usize>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormSourceRef {
    Profile(String),
    Credential(String),
    Any(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormValueType {
    #[default]
    Text,
    Date,
    DateJa,
    Integer,
    Bool,
    Enum,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    pub id: String,
    pub label: String,
    pub target: FormTarget,
    pub sources: Vec<FormSourceRef>,
    pub value_type: FormValueType,
    pub required: bool,
    pub max_chars: Option<usize>,
    pub default: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormMapping {
    pub path: String,
    pub form_id: String,
    pub form_version: Option<String>,
    pub title: String,
    pub target_format: String,
    pub matcher: FormMatch,
    pub fields: Vec<FormField>,
    /// True if auto-synthesized from worksheet DefinedNames rather than a Vault `.form-map.md`.
    pub synthesized: bool,
}

fn parse_target_value(val: &Value) -> Option<FormTarget> {
    if let Some(cell) = val.as_str() {
        let trimmed = cell.trim();
        if !trimmed.is_empty() {
            return Some(FormTarget::Cell(trimmed.to_string()));
        }
    }
    let obj = val.as_object()?;
    if let Some(cell) = obj.get("cell").and_then(Value::as_str) {
        return Some(FormTarget::Cell(cell.trim().to_string()));
    }
    if let Some(name) = obj.get("name").and_then(Value::as_str) {
        return Some(FormTarget::DefinedName(name.trim().to_string()));
    }
    if let Some(ymd) = obj.get("split_date_ymd").and_then(Value::as_object) {
        return Some(FormTarget::SplitDateYmd {
            year: ymd.get("year")?.as_str()?.trim().to_string(),
            month: ymd.get("month")?.as_str()?.trim().to_string(),
            day: ymd.get("day")?.as_str()?.trim().to_string(),
        });
    }
    if let Some(era_obj) = obj.get("split_date_era").and_then(Value::as_object) {
        return Some(FormTarget::SplitDateEra {
            era: era_obj
                .get("era")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string()),
            year: era_obj.get("year")?.as_str()?.trim().to_string(),
            month: era_obj.get("month")?.as_str()?.trim().to_string(),
            day: era_obj.get("day")?.as_str()?.trim().to_string(),
        });
    }
    if let Some(radio) = obj.get("radio_mark").and_then(Value::as_object) {
        let checked = radio
            .get("checked")
            .and_then(Value::as_str)
            .unwrap_or("☑")
            .to_string();
        let unchecked = radio
            .get("unchecked")
            .and_then(Value::as_str)
            .unwrap_or("□")
            .to_string();
        let opts_obj = radio.get("options")?.as_object()?;
        let mut options = BTreeMap::new();
        for (k, v) in opts_obj {
            if let Some(cell) = v.as_str() {
                options.insert(k.clone(), cell.trim().to_string());
            }
        }
        if !options.is_empty() {
            return Some(FormTarget::RadioMark {
                checked,
                unchecked,
                options,
            });
        }
    }
    if let Some(digits) = obj.get("split_digits").and_then(Value::as_object) {
        let cells: Vec<String> = digits
            .get("cells")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
            .collect();
        let lengths: Vec<usize> = digits
            .get("lengths")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as usize))
            .collect();
        if !cells.is_empty() && cells.len() == lengths.len() {
            return Some(FormTarget::SplitDigits { cells, lengths });
        }
    }
    None
}

fn parse_sources_value(val: Option<&Value>, fallback_id: &str) -> Vec<FormSourceRef> {
    let mut out = Vec::new();
    if let Some(arr) = val.and_then(Value::as_array) {
        for item in arr {
            if let Some(s) = item.as_str() {
                out.push(FormSourceRef::Any(s.trim().to_string()));
            } else if let Some(obj) = item.as_object() {
                if let Some(p) = obj.get("profile").and_then(Value::as_str) {
                    out.push(FormSourceRef::Profile(p.trim().to_string()));
                }
                if let Some(c) = obj.get("credential").and_then(Value::as_str) {
                    out.push(FormSourceRef::Credential(c.trim().to_string()));
                }
                if let Some(a) = obj.get("any").and_then(Value::as_str) {
                    out.push(FormSourceRef::Any(a.trim().to_string()));
                }
            }
        }
    }
    if out.is_empty() {
        out.push(FormSourceRef::Any(fallback_id.to_string()));
    }
    out
}

fn parse_value_type(raw: Option<&str>) -> FormValueType {
    match raw.map(str::trim).unwrap_or("text").to_ascii_lowercase().as_str() {
        "date" => FormValueType::Date,
        "date-ja" | "date_ja" => FormValueType::DateJa,
        "integer" | "int" | "number" => FormValueType::Integer,
        "bool" | "boolean" => FormValueType::Bool,
        "enum" => FormValueType::Enum,
        _ => FormValueType::Text,
    }
}

/// Parses an OKF `type: form-mapping` Markdown note into a [`FormMapping`].
pub fn parse_form_mapping_markdown(path: &str, text: &str) -> Option<FormMapping> {
    let fm = crate::okf::frontmatter_map(text).ok()??;
    let doc_type = fm.get("type")?.as_str()?.trim();
    if !doc_type.eq_ignore_ascii_case("form-mapping") {
        return None;
    }
    let ext = fm.get("x-sorane-form-mapping")?.as_object()?;
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .trim_end_matches(".form-map")
        .to_string();
    let title = fm
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&stem)
        .to_string();
    let form_id = ext
        .get("form_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&stem)
        .to_string();
    let form_version = ext
        .get("form_version")
        .and_then(|v| match v {
            Value::String(s) => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .filter(|s| !s.is_empty());
    let target_format = ext
        .get("format")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "xlsx".to_string());

    let mut matcher = FormMatch::default();
    if let Some(m) = ext.get("match").and_then(Value::as_object) {
        matcher.template_sha256 = m
            .get("template_sha256")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('<'))
            .map(str::to_ascii_lowercase);
        matcher.sheet = m
            .get("sheet")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(anchors) = m.get("anchors").and_then(Value::as_array) {
            for a in anchors {
                if let Some(obj) = a.as_object()
                    && let (Some(cell), Some(contains)) = (
                        obj.get("cell").and_then(Value::as_str),
                        obj.get("contains").and_then(Value::as_str),
                    )
                {
                    matcher.anchors.push(FormAnchor {
                        cell: cell.trim().to_string(),
                        contains: contains.to_string(),
                    });
                }
            }
        }
    }

    let mut fields = Vec::new();
    if let Some(arr) = ext.get("fields").and_then(Value::as_array) {
        for item in arr {
            let Some(obj) = item.as_object() else {
                continue;
            };
            let Some(id) = obj.get("id").and_then(Value::as_str).map(str::trim) else {
                continue;
            };
            if id.is_empty() {
                continue;
            }
            let Some(target) = obj.get("target").and_then(parse_target_value) else {
                continue;
            };
            let label = obj
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id)
                .to_string();
            let sources = parse_sources_value(obj.get("sources"), id);
            let value_type = parse_value_type(obj.get("value_type").and_then(Value::as_str));
            let required = obj.get("required").and_then(Value::as_bool).unwrap_or(false);
            let max_chars = obj
                .get("max_chars")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let default = obj
                .get("default")
                .and_then(value_as_display_string)
                .filter(|s| !s.is_empty());
            fields.push(FormField {
                id: id.to_string(),
                label,
                target,
                sources,
                value_type,
                required,
                max_chars,
                default,
            });
        }
    }

    if fields.is_empty() {
        return None;
    }

    Some(FormMapping {
        path: path.to_string(),
        form_id,
        form_version,
        title,
        target_format,
        matcher,
        fields,
        synthesized: false,
    })
}

// ---------------------------------------------------------------------------
// 4. A1 Cell Address & DefinedName Resolution Helpers
// ---------------------------------------------------------------------------

/// Parses a single A1 reference like `"B4"`, `"$AA$3"`, or `"Sheet1!$C$10"` -> `(0-based row, 0-based col)`.
pub fn parse_a1_cell(raw: &str) -> Option<(u32, u32)> {
    let part = raw.trim().trim_start_matches('=');
    let cell_part = match part.rsplit_once('!') {
        Some((_, right)) => right,
        None => part,
    };
    if cell_part.contains(':') {
        return None;
    }
    let clean: String = cell_part.chars().filter(|&c| c != '$').collect();
    let letters_len = clean.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    if letters_len == 0 || letters_len > 3 || letters_len == clean.len() {
        return None;
    }
    let (col_str, row_str) = clean.split_at(letters_len);
    if !row_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let row_1based: u32 = row_str.parse().ok()?;
    if row_1based == 0 || row_1based > 1_048_576 {
        return None;
    }
    let mut col_1based: u32 = 0;
    for ch in col_str.bytes() {
        let v = (ch.to_ascii_uppercase() - b'A') as u32 + 1;
        col_1based = col_1based.checked_mul(26)?.checked_add(v)?;
    }
    if col_1based == 0 || col_1based > 16_384 {
        return None;
    }
    Some((row_1based - 1, col_1based - 1))
}

fn defined_name_formula_sheet(formula: &str) -> Option<String> {
    let trimmed = formula.trim().trim_start_matches('=');
    let (sheet_part, _) = trimmed.rsplit_once('!')?;
    let unquoted = sheet_part
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .map(|s| s.replace("''", "'"))
        .unwrap_or_else(|| sheet_part.to_string());
    Some(unquoted)
}

fn resolve_defined_name_cell(
    presentation: &Presentation,
    slide: SlideId,
    name: &str,
) -> Option<(u32, u32)> {
    let workbook = presentation.workbook.as_deref()?;
    let local_idx = workbook.sheets.iter().position(|s| s.slide == Some(slide));
    let mut matching = workbook
        .names
        .iter()
        .filter(|dn| !dn.hidden && dn.name.eq_ignore_ascii_case(name));
    let chosen = matching
        .clone()
        .find(|dn| dn.sheet.is_some() && dn.sheet == local_idx)
        .or_else(|| matching.find(|dn| dn.sheet.is_none()))?;
    if let Some(target_sheet) = defined_name_formula_sheet(&chosen.formula)
        && workbook.sheet_index(&target_sheet) != local_idx
    {
        return None;
    }
    parse_a1_cell(&chosen.formula)
}

fn sheet_name_for_slide(presentation: &Presentation, slide: SlideId) -> String {
    if let Some(wb) = presentation.workbook.as_deref()
        && let Some(entry) = wb.sheets.iter().find(|s| s.slide == Some(slide))
    {
        return entry.name.clone();
    }
    presentation
        .slide(slide)
        .and_then(|s| s.name.clone())
        .unwrap_or_else(|| "Sheet1".to_string())
}

// ---------------------------------------------------------------------------
// 5. Japanese Era & Date Parsing / Formatting
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimpleDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

fn era_start_year(era: &str) -> Option<i32> {
    match era {
        "令和" | "R" | "r" => Some(2018),
        "平成" | "H" | "h" => Some(1988),
        "昭和" | "S" | "s" => Some(1925),
        "大正" | "T" | "t" => Some(1911),
        "明治" | "M" | "m" => Some(1867),
        _ => None,
    }
}

fn valid_ymd(year: i32, month: u32, day: u32) -> bool {
    if !(1868..=2999).contains(&year) || !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if leap { 29 } else { 28 },
        _ => 0,
    };
    day <= max_day
}

/// Parses ISO (`YYYY-MM-DD`, `YYYY/MM/DD`) and Japanese Era (`令和8年11月25日`, `令和元年5月1日`, `R8.11.25`) dates.
pub fn parse_date_ymd(raw: &str) -> Option<SimpleDate> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // Check Japanese Era prefix (kanji or single-letter R/H/S/T/M)
    for prefix in ["令和", "平成", "昭和", "大正", "明治", "R", "r", "H", "h", "S", "s", "T", "t", "M", "m"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let base = era_start_year(prefix)?;
            let normalized = rest.replace('元', "1");
            let nums: Vec<u32> = normalized
                .split(|c: char| !c.is_ascii_digit())
                .filter(|seg| !seg.is_empty())
                .filter_map(|seg| seg.parse::<u32>().ok())
                .collect();
            if nums.len() == 3 && nums[0] >= 1 {
                let year = base + nums[0] as i32;
                if valid_ymd(year, nums[1], nums[2]) {
                    return Some(SimpleDate {
                        year,
                        month: nums[1],
                        day: nums[2],
                    });
                }
            }
            return None;
        }
    }
    // Standard YYYY-MM-DD / YYYY/MM/DD / YYYY年M月D日
    let nums: Vec<u32> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|seg| !seg.is_empty())
        .filter_map(|seg| seg.parse::<u32>().ok())
        .collect();
    if nums.len() >= 3 {
        let year = nums[0] as i32;
        if valid_ymd(year, nums[1], nums[2]) {
            return Some(SimpleDate {
                year,
                month: nums[1],
                day: nums[2],
            });
        }
    }
    None
}

/// Converts a Gregorian date into `(era_name, era_year, month, day)`.
pub fn to_japanese_era(date: SimpleDate) -> Option<(&'static str, u32, u32, u32)> {
    let key = (date.year, date.month, date.day);
    if key >= (2019, 5, 1) {
        Some(("令和", (date.year - 2018) as u32, date.month, date.day))
    } else if key >= (1989, 1, 8) {
        Some(("平成", (date.year - 1988) as u32, date.month, date.day))
    } else if key >= (1926, 12, 25) {
        Some(("昭和", (date.year - 1925) as u32, date.month, date.day))
    } else if key >= (1912, 7, 30) {
        Some(("大正", (date.year - 1911) as u32, date.month, date.day))
    } else if key >= (1868, 1, 25) {
        Some(("明治", (date.year - 1867) as u32, date.month, date.day))
    } else {
        None
    }
}

/// Formats a date in Japanese Era notation (e.g. `"令和8年11月25日"`, `"令和元年5月1日"`).
pub fn format_date_ja(date: SimpleDate) -> String {
    match to_japanese_era(date) {
        Some((era, 1, m, d)) => format!("{era}元年{m}月{d}日"),
        Some((era, y, m, d)) => format!("{era}{y}年{m}月{d}日"),
        None => format!("{:04}-{:02}-{:02}", date.year, date.month, date.day),
    }
}

// ---------------------------------------------------------------------------
// 6. Auto-Synthesize & Draft Heuristic Inference from Active Worksheet
// ---------------------------------------------------------------------------

/// Synthesizes a [`FormMapping`] directly from visible single-cell `DefinedName`s on `slide`.
pub fn synthesize_form_mapping_from_sheet(
    presentation: &Presentation,
    slide: SlideId,
) -> Option<FormMapping> {
    let wb = presentation.workbook.as_deref()?;
    let local_idx = wb.sheets.iter().position(|s| s.slide == Some(slide));
    let sheet_name = sheet_name_for_slide(presentation, slide);
    let mut fields = Vec::new();
    let mut seen_names = BTreeSet::new();

    for dn in &wb.names {
        if dn.hidden || dn.name.starts_with("_xlnm") {
            continue;
        }
        if dn.name.to_ascii_lowercase().starts_with("disclose.") {
            continue;
        }
        if dn.sheet.is_some() && dn.sheet != local_idx {
            continue;
        }
        if resolve_defined_name_cell(presentation, slide, &dn.name).is_none() {
            continue;
        }
        if !seen_names.insert(dn.name.to_ascii_lowercase()) {
            continue;
        }
        let logical = dn
            .name
            .strip_prefix("sd.")
            .or_else(|| dn.name.strip_prefix("claim."))
            .unwrap_or(&dn.name);
        let value_type = if logical.contains("date") {
            FormValueType::Date
        } else {
            FormValueType::Text
        };
        fields.push(FormField {
            id: dn.name.clone(),
            label: logical.to_string(),
            target: FormTarget::DefinedName(dn.name.clone()),
            sources: vec![FormSourceRef::Any(logical.to_string())],
            value_type,
            required: false,
            max_chars: None,
            default: None,
        });
    }

    if fields.is_empty() {
        return None;
    }

    Some(FormMapping {
        path: format!("(sheet:{sheet_name})"),
        form_id: slugify_id(&sheet_name),
        form_version: None,
        title: format!("{sheet_name}（名前付きセル自動検出）"),
        target_format: "xlsx".to_string(),
        matcher: FormMatch {
            template_sha256: None,
            sheet: Some(sheet_name),
            anchors: Vec::new(),
        },
        fields,
        synthesized: true,
    })
}

const HEURISTIC_LABEL_RULES: &[(&[&str], &str, &str, FormValueType)] = &[
    (&["適格請求書", "登録番号", "インボイス"], "organization.invoice_number", "登録番号", FormValueType::Text),
    (&["法人番号"], "organization.corporate_number", "法人番号", FormValueType::Text),
    (&["会社名", "法人名", "事業所名", "商号", "団体名", "company"], "organization.name", "会社名", FormValueType::Text),
    (&["代表者", "代表取締役"], "organization.representative", "代表者", FormValueType::Text),
    (&["フリガナ", "ふりがな", "カナ"], "person.name_kana", "ふりがな", FormValueType::Text),
    (&["氏名", "お名前", "申請者名", "担当者", "full name"], "person.full_name", "氏名", FormValueType::Text),
    (&["生年月日", "birthdate", "birth date"], "person.birthdate", "生年月日", FormValueType::Date),
    (&["郵便番号", "〒", "postal"], "address.postal_code", "郵便番号", FormValueType::Text),
    (&["住所", "所在地", "address"], "address.full", "住所", FormValueType::Text),
    (&["電話番号", "連絡先", "tel", "phone"], "contact.phone", "電話番号", FormValueType::Text),
    (&["メール", "email", "e-mail", "mail"], "contact.email", "メールアドレス", FormValueType::Text),
    (&["銀行名", "金融機関", "振込先銀行"], "bank.name", "銀行名", FormValueType::Text),
    (&["支店名", "支店"], "bank.branch", "支店名", FormValueType::Text),
    (&["口座番号"], "bank.account_number", "口座番号", FormValueType::Text),
    (&["申請日", "請求日", "作成日", "提出日", "発行日"], "document.date", "日付", FormValueType::Date),
];

fn slugify_id(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if (ch == '-' || ch == '_' || ch == ' ') && !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    if out.is_empty() {
        "sheet-form".to_string()
    } else {
        out.trim_end_matches('-').to_string()
    }
}

/// Scans an unlabeled worksheet for common form labels and adjacent input cells (including merged anchors),
/// returning a live draft [`FormMapping`] plus ready-to-save OKF `type: form-mapping` Markdown text.
pub fn infer_draft_form_mapping_from_sheet(
    presentation: &Presentation,
    slide: SlideId,
) -> Option<(FormMapping, String)> {
    let selected = presentation.slide(slide)?;
    let sheet = sheet_of_slide(selected)?;
    let sheet_name = sheet_name_for_slide(presentation, slide);

    let mut fields = Vec::new();
    let mut anchors = Vec::new();
    let mut used_cells = BTreeSet::new();
    let mut used_ids = BTreeSet::new();

    // Sort cells top-to-bottom, left-to-right for deterministic mapping generation.
    let mut cells: Vec<(u32, u32, &str)> = sheet
        .cells
        .iter()
        .filter(|c| !c.text.trim().is_empty() && c.formula.is_none())
        .map(|c| (c.row, c.column, c.text.trim()))
        .collect();
    cells.sort_by_key(|&(r, c, _)| (r, c));

    for (row, col, label_text) in cells {
        if label_text.chars().count() > 28 {
            continue;
        }
        let lower = label_text.to_ascii_lowercase();
        let matched = HEURISTIC_LABEL_RULES.iter().find(|(patterns, _, _, _)| {
            patterns
                .iter()
                .any(|pat| label_text.contains(pat) || lower.contains(pat))
        });
        let Some((_, field_id, _, value_type)) = matched else {
            continue;
        };
        if used_ids.contains(*field_id) {
            continue;
        }

        // Skip past the label cell's own merged span to find the immediate right-hand input cell.
        let label_end_col = sheet
            .merge_at(row, col)
            .map_or(col, |range| range.last_column);
        let cand_col = label_end_col + 1;
        if cand_col >= 64 {
            continue;
        }
        let (target_row, target_col) = match sheet.merge_at(row, cand_col) {
            Some(range) => (range.first_row, range.first_column),
            None => (row, cand_col),
        };
        if !used_cells.insert((target_row, target_col)) {
            continue;
        }
        // Skip if target cell has a formula or looks like another header label
        if let Some(existing) = sheet.cell(target_row, target_col)
            && existing.formula.is_some()
        {
            continue;
        }

        used_ids.insert((*field_id).to_string());
        let addr = cell_name(target_col, target_row);
        if anchors.len() < 3 {
            anchors.push(FormAnchor {
                cell: cell_name(col, row),
                contains: label_text.to_string(),
            });
        }
        let default = if *field_id == "document.date" {
            Some("today".to_string())
        } else {
            None
        };
        let sources = if *field_id == "document.date" {
            Vec::new()
        } else {
            vec![FormSourceRef::Any((*field_id).to_string())]
        };
        fields.push(FormField {
            id: (*field_id).to_string(),
            label: label_text.to_string(),
            target: FormTarget::Cell(addr),
            sources,
            value_type: *value_type,
            required: false,
            max_chars: None,
            default,
        });
    }

    if fields.is_empty() {
        return None;
    }

    let form_id = slugify_id(&sheet_name);
    let path = format!("forms/{form_id}.form-map.md");
    let mapping = FormMapping {
        path: path.clone(),
        form_id: form_id.clone(),
        form_version: Some("1".to_string()),
        title: format!("{sheet_name} フォーム対応表"),
        target_format: "xlsx".to_string(),
        matcher: FormMatch {
            template_sha256: None,
            sheet: Some(sheet_name.clone()),
            anchors: anchors.clone(),
        },
        fields: fields.clone(),
        synthesized: false,
    };

    let markdown = render_form_mapping_markdown(&mapping);
    Some((mapping, markdown))
}

/// Serializes a [`FormMapping`] as an editable OKF `type: form-mapping` Markdown document.
pub fn render_form_mapping_markdown(mapping: &FormMapping) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("type: form-mapping\n");
    out.push_str(&format!("title: {}\n", yaml_quote(&mapping.title)));
    out.push_str("tags: [form-mapping]\n");
    out.push_str("x-sorane-form-mapping:\n");
    out.push_str("  version: 1\n");
    out.push_str(&format!("  form_id: {}\n", yaml_quote(&mapping.form_id)));
    if let Some(ver) = &mapping.form_version {
        out.push_str(&format!("  form_version: {}\n", yaml_quote(ver)));
    }
    out.push_str(&format!("  format: {}\n", mapping.target_format));
    out.push_str("  match:\n");
    if let Some(sha) = &mapping.matcher.template_sha256 {
        out.push_str(&format!("    template_sha256: {}\n", yaml_quote(sha)));
    }
    if let Some(sheet) = &mapping.matcher.sheet {
        out.push_str(&format!("    sheet: {}\n", yaml_quote(sheet)));
    }
    if !mapping.matcher.anchors.is_empty() {
        out.push_str("    anchors:\n");
        for a in &mapping.matcher.anchors {
            out.push_str(&format!(
                "      - cell: {}\n        contains: {}\n",
                a.cell,
                yaml_quote(&a.contains)
            ));
        }
    }
    out.push_str("  fields:\n");
    for f in &mapping.fields {
        out.push_str(&format!("    - id: {}\n", f.id));
        out.push_str(&format!("      label: {}\n", yaml_quote(&f.label)));
        match &f.target {
            FormTarget::Cell(c) => out.push_str(&format!("      target: {{ cell: {c} }}\n")),
            FormTarget::DefinedName(n) => {
                out.push_str(&format!("      target: {{ name: {} }}\n", yaml_quote(n)));
            }
            FormTarget::SplitDateYmd { year, month, day } => {
                out.push_str(&format!(
                    "      target:\n        split_date_ymd: {{ year: {year}, month: {month}, day: {day} }}\n"
                ));
            }
            FormTarget::SplitDateEra {
                era,
                year,
                month,
                day,
            } => {
                let era_kv = era
                    .as_ref()
                    .map(|e| format!("era: {e}, "))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "      target:\n        split_date_era: {{ {era_kv}year: {year}, month: {month}, day: {day} }}\n"
                ));
            }
            FormTarget::RadioMark {
                checked,
                unchecked,
                options,
            } => {
                out.push_str("      target:\n        radio_mark:\n");
                out.push_str(&format!("          checked: {}\n", yaml_quote(checked)));
                out.push_str(&format!("          unchecked: {}\n", yaml_quote(unchecked)));
                out.push_str("          options:\n");
                for (k, cell) in options {
                    out.push_str(&format!("            {k}: {cell}\n"));
                }
            }
            FormTarget::SplitDigits { cells, lengths } => {
                let cells_str = cells.join(", ");
                let lens_str: Vec<String> = lengths.iter().map(|n| n.to_string()).collect();
                out.push_str(&format!(
                    "      target:\n        split_digits: {{ cells: [{cells_str}], lengths: [{}] }}\n",
                    lens_str.join(", ")
                ));
            }
        }
        if !f.sources.is_empty() {
            out.push_str("      sources:\n");
            for s in &f.sources {
                match s {
                    FormSourceRef::Profile(k) => out.push_str(&format!("        - profile: {k}\n")),
                    FormSourceRef::Credential(k) => {
                        out.push_str(&format!("        - credential: {k}\n"))
                    }
                    FormSourceRef::Any(k) => out.push_str(&format!("        - any: {k}\n")),
                }
            }
        }
        let vt = match f.value_type {
            FormValueType::Text => "text",
            FormValueType::Date => "date",
            FormValueType::DateJa => "date_ja",
            FormValueType::Integer => "integer",
            FormValueType::Bool => "bool",
            FormValueType::Enum => "enum",
        };
        out.push_str(&format!("      value_type: {vt}\n"));
        if let Some(def) = &f.default {
            out.push_str(&format!("      default: {}\n", yaml_quote(def)));
        }
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", mapping.title));
    out.push_str("シートの見出しから自動生成されたフォーム対応表ドラフトです。必要に応じてセル番地や `sources` を調整してください。\n");
    out
}

fn yaml_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

// ---------------------------------------------------------------------------
// 7. Sheet Matching, Evaluation & 1-Undo Command Generation
// ---------------------------------------------------------------------------

/// Scores how well a [`FormMapping`] matches `(presentation, slide, file_sha256)`.
/// Returns `None` if it does not match.
pub fn score_form_mapping_match(
    mapping: &FormMapping,
    presentation: &Presentation,
    slide: SlideId,
    file_sha256: Option<&str>,
) -> Option<u32> {
    if mapping.target_format != "xlsx" {
        return None;
    }
    let selected = presentation.slide(slide)?;
    let sheet = sheet_of_slide(selected)?;
    let active_sheet_name = sheet_name_for_slide(presentation, slide);

    let mut score = 0u32;
    if let Some(expected_sha) = &mapping.matcher.template_sha256 {
        let actual = file_sha256?;
        if !actual.eq_ignore_ascii_case(expected_sha) {
            return None;
        }
        score += 100;
    }
    if let Some(expected_sheet) = &mapping.matcher.sheet {
        if expected_sheet != &active_sheet_name {
            return None;
        }
        score += 20;
    }
    if !mapping.matcher.anchors.is_empty() {
        for anchor in &mapping.matcher.anchors {
            let (r, c) = parse_a1_cell(&anchor.cell)?;
            let text = sheet.cell(r, c).map_or("", |cell| cell.text.as_str());
            if !text.contains(&anchor.contains) {
                return None;
            }
            score += 10;
        }
    }
    if score == 0 {
        return None;
    }
    Some(score)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldProvenance {
    Credential {
        path: String,
        format: String,
        issuer: Option<String>,
        claim_key: String,
        verified_digests: bool,
    },
    Profile {
        path: String,
        title: String,
        field_key: String,
    },
    DefaultValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldStatus {
    Ready,
    Occupied,
    MissingSource,
    BlockedTarget,
    UnsupportedValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormFieldPlan {
    pub id: String,
    pub label: String,
    pub target_summary: String,
    pub current_display: String,
    pub proposed_display: Option<String>,
    pub proposed_raw: Option<Value>,
    pub provenance: Option<FieldProvenance>,
    pub status: FieldStatus,
    pub required: bool,
    pub max_chars: Option<usize>,
    pub char_count: usize,
    pub exceeds_max_chars: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormEvaluation {
    pub fields: Vec<FormFieldPlan>,
    pub ready_count: usize,
    pub occupied_count: usize,
    pub vc_backed_count: usize,
    pub missing_required_count: usize,
    pub overflow_count: usize,
}

pub fn value_as_display_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
        _ => None,
    }
}

fn target_cells_for_field(
    presentation: &Presentation,
    slide: SlideId,
    target: &FormTarget,
) -> Result<(Vec<(u32, u32)>, String), String> {
    match target {
        FormTarget::Cell(addr) => {
            let (r, c) = parse_a1_cell(addr).ok_or_else(|| format!("invalid cell {addr}"))?;
            Ok((vec![(r, c)], cell_name(c, r)))
        }
        FormTarget::DefinedName(name) => {
            let (r, c) = resolve_defined_name_cell(presentation, slide, name)
                .ok_or_else(|| format!("missing defined name {name}"))?;
            Ok((vec![(r, c)], format!("{} ({})", cell_name(c, r), name)))
        }
        FormTarget::SplitDateYmd { year, month, day } => {
            let y = parse_a1_cell(year).ok_or("invalid year cell")?;
            let m = parse_a1_cell(month).ok_or("invalid month cell")?;
            let d = parse_a1_cell(day).ok_or("invalid day cell")?;
            Ok((vec![y, m, d], format!("{year}/{month}/{day}")))
        }
        FormTarget::SplitDateEra {
            era,
            year,
            month,
            day,
        } => {
            let mut cells = Vec::new();
            let mut summary = Vec::new();
            if let Some(e) = era {
                cells.push(parse_a1_cell(e).ok_or("invalid era cell")?);
                summary.push(e.clone());
            }
            cells.push(parse_a1_cell(year).ok_or("invalid era year cell")?);
            cells.push(parse_a1_cell(month).ok_or("invalid month cell")?);
            cells.push(parse_a1_cell(day).ok_or("invalid day cell")?);
            summary.extend([year.clone(), month.clone(), day.clone()]);
            Ok((cells, summary.join("/")))
        }
        FormTarget::RadioMark { options, .. } => {
            let mut cells = Vec::new();
            let mut names = Vec::new();
            for addr in options.values() {
                cells.push(parse_a1_cell(addr).ok_or("invalid radio option cell")?);
                names.push(addr.clone());
            }
            Ok((cells, names.join(", ")))
        }
        FormTarget::SplitDigits { cells: addrs, .. } => {
            let mut cells = Vec::new();
            for addr in addrs {
                cells.push(parse_a1_cell(addr).ok_or("invalid split digit cell")?);
            }
            Ok((cells, addrs.join("-")))
        }
    }
}

fn is_cell_blocked(sheet: &Sheet, row: u32, col: u32) -> bool {
    sheet.protection.is_some()
        || sheet.spills.values().any(|range| {
            (range.first_row..=range.last_row).contains(&row)
                && (range.first_column..=range.last_column).contains(&col)
        })
        || sheet.hidden_rows.contains_key(&row)
        || sheet.hidden_columns.contains_key(&col)
        || sheet
            .merge_at(row, col)
            .is_some_and(|range| range.first_row != row || range.first_column != col)
        || sheet.cell(row, col).is_some_and(|cell| cell.formula.is_some())
}

fn cell_is_nonempty(sheet: &Sheet, row: u32, col: u32) -> bool {
    sheet
        .cell(row, col)
        .is_some_and(|c| !matches!(c.value, CellValue::Empty))
}

fn read_current_display(
    sheet: &Sheet,
    target: &FormTarget,
    coords: &[(u32, u32)],
) -> String {
    match target {
        FormTarget::RadioMark {
            checked, options, ..
        } => {
            for (opt_key, addr) in options {
                if let Some((r, c)) = parse_a1_cell(addr)
                    && sheet.cell(r, c).is_some_and(|cell| cell.text.trim() == checked)
                {
                    return opt_key.clone();
                }
            }
            String::new()
        }
        _ => {
            let parts: Vec<String> = coords
                .iter()
                .map(|&(r, c)| {
                    sheet
                        .cell(r, c)
                        .map(|cell| cell.text.trim().to_string())
                        .unwrap_or_default()
                })
                .filter(|s| !s.is_empty())
                .collect();
            parts.join(" / ")
        }
    }
}

fn resolve_field_source(
    field: &FormField,
    profiles: &[&VaultProfile],
    credentials: &[&VaultCredentialSummary],
    today_ymd: Option<&str>,
) -> Option<(Value, FieldProvenance)> {
    for src in &field.sources {
        match src {
            FormSourceRef::Credential(key) => {
                for cred in credentials {
                    if let Some((matched_key, val)) = cred.resolve_value(key) {
                        return Some((
                            val,
                            FieldProvenance::Credential {
                                path: cred.path.clone(),
                                format: cred.format.clone(),
                                issuer: cred.issuer.clone(),
                                claim_key: matched_key,
                                verified_digests: cred.digests_verified == cred.digests_total
                                    && cred.sig_status != "invalid",
                            },
                        ));
                    }
                }
            }
            FormSourceRef::Profile(key) => {
                for prof in profiles {
                    if let Some((matched_key, val)) = prof.resolve_value(key) {
                        return Some((
                            val,
                            FieldProvenance::Profile {
                                path: prof.path.clone(),
                                title: prof.title.clone(),
                                field_key: matched_key,
                            },
                        ));
                    }
                }
            }
            FormSourceRef::Any(key) => {
                // Prioritize VC credential over profile when both exist!
                for cred in credentials {
                    if let Some((matched_key, val)) = cred.resolve_value(key) {
                        return Some((
                            val,
                            FieldProvenance::Credential {
                                path: cred.path.clone(),
                                format: cred.format.clone(),
                                issuer: cred.issuer.clone(),
                                claim_key: matched_key,
                                verified_digests: cred.digests_verified == cred.digests_total
                                    && cred.sig_status != "invalid",
                            },
                        ));
                    }
                }
                for prof in profiles {
                    if let Some((matched_key, val)) = prof.resolve_value(key) {
                        return Some((
                            val,
                            FieldProvenance::Profile {
                                path: prof.path.clone(),
                                title: prof.title.clone(),
                                field_key: matched_key,
                            },
                        ));
                    }
                }
            }
        }
    }

    if let Some(def) = &field.default {
        if def.eq_ignore_ascii_case("today") {
            if let Some(today) = today_ymd {
                return Some((Value::String(today.to_string()), FieldProvenance::DefaultValue));
            }
        } else {
            return Some((Value::String(def.clone()), FieldProvenance::DefaultValue));
        }
    }
    None
}

fn build_edits_for_field(
    presentation: &Presentation,
    slide: SlideId,
    field: &FormField,
    proposed: &Value,
) -> Result<(Vec<CellEdit>, String), String> {
    let raw_str = value_as_display_string(proposed).ok_or("unsupported value type")?;
    match &field.target {
        FormTarget::Cell(addr) => {
            let (r, c) = parse_a1_cell(addr).ok_or("invalid cell")?;
            let (cv, disp) = convert_scalar(field.value_type, proposed, &raw_str)?;
            Ok((
                vec![CellEdit {
                    row: r,
                    column: c,
                    input: CellInput::Value(cv),
                }],
                disp,
            ))
        }
        FormTarget::DefinedName(name) => {
            let (r, c) =
                resolve_defined_name_cell(presentation, slide, name).ok_or("missing name")?;
            let (cv, disp) = convert_scalar(field.value_type, proposed, &raw_str)?;
            Ok((
                vec![CellEdit {
                    row: r,
                    column: c,
                    input: CellInput::Value(cv),
                }],
                disp,
            ))
        }
        FormTarget::SplitDateYmd { year, month, day } => {
            let d = parse_date_ymd(&raw_str).ok_or("invalid date")?;
            let (yr, yc) = parse_a1_cell(year).ok_or("invalid year cell")?;
            let (mr, mc) = parse_a1_cell(month).ok_or("invalid month cell")?;
            let (dr, dc) = parse_a1_cell(day).ok_or("invalid day cell")?;
            Ok((
                vec![
                    CellEdit {
                        row: yr,
                        column: yc,
                        input: CellInput::Value(CellValue::Number(d.year as f64)),
                    },
                    CellEdit {
                        row: mr,
                        column: mc,
                        input: CellInput::Value(CellValue::Number(d.month as f64)),
                    },
                    CellEdit {
                        row: dr,
                        column: dc,
                        input: CellInput::Value(CellValue::Number(d.day as f64)),
                    },
                ],
                format!("{:04}-{:02}-{:02}", d.year, d.month, d.day),
            ))
        }
        FormTarget::SplitDateEra {
            era,
            year,
            month,
            day,
        } => {
            let d = parse_date_ymd(&raw_str).ok_or("invalid date")?;
            let (era_name, era_year, m, day_num) =
                to_japanese_era(d).ok_or("date out of Japanese era range")?;
            let mut edits = Vec::new();
            if let Some(era_addr) = era {
                let (er, ec) = parse_a1_cell(era_addr).ok_or("invalid era cell")?;
                edits.push(CellEdit {
                    row: er,
                    column: ec,
                    input: CellInput::Value(CellValue::Text(era_name.to_string())),
                });
            }
            let (yr, yc) = parse_a1_cell(year).ok_or("invalid era year cell")?;
            let (mr, mc) = parse_a1_cell(month).ok_or("invalid month cell")?;
            let (dr, dc) = parse_a1_cell(day).ok_or("invalid day cell")?;
            edits.push(CellEdit {
                row: yr,
                column: yc,
                input: CellInput::Value(CellValue::Number(era_year as f64)),
            });
            edits.push(CellEdit {
                row: mr,
                column: mc,
                input: CellInput::Value(CellValue::Number(m as f64)),
            });
            edits.push(CellEdit {
                row: dr,
                column: dc,
                input: CellInput::Value(CellValue::Number(day_num as f64)),
            });
            Ok((edits, format_date_ja(d)))
        }
        FormTarget::RadioMark {
            checked,
            unchecked,
            options,
        } => {
            let chosen = raw_str.trim();
            if !options.contains_key(chosen) {
                return Err(format!("unknown enum option {chosen}"));
            }
            let mut edits = Vec::with_capacity(options.len());
            for (opt_key, addr) in options {
                let (r, c) = parse_a1_cell(addr).ok_or("invalid radio option cell")?;
                let mark = if opt_key == chosen {
                    checked.clone()
                } else {
                    unchecked.clone()
                };
                edits.push(CellEdit {
                    row: r,
                    column: c,
                    input: CellInput::Value(CellValue::Text(mark)),
                });
            }
            Ok((edits, format!("{checked} {chosen}")))
        }
        FormTarget::SplitDigits { cells, lengths } => {
            let digits: String = raw_str.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            let total_len: usize = lengths.iter().sum();
            if digits.len() < total_len {
                return Err("not enough characters for split_digits".into());
            }
            let mut edits = Vec::with_capacity(cells.len());
            let mut pos = 0usize;
            let mut parts = Vec::with_capacity(cells.len());
            for (addr, &len) in cells.iter().zip(lengths.iter()) {
                let (r, c) = parse_a1_cell(addr).ok_or("invalid split_digits cell")?;
                let seg = digits[pos..pos + len].to_string();
                pos += len;
                parts.push(seg.clone());
                edits.push(CellEdit {
                    row: r,
                    column: c,
                    input: CellInput::Value(CellValue::Text(seg)),
                });
            }
            Ok((edits, parts.join("-")))
        }
    }
}

fn convert_scalar(
    value_type: FormValueType,
    raw_val: &Value,
    raw_str: &str,
) -> Result<(CellValue, String), String> {
    match value_type {
        FormValueType::DateJa => {
            let d = parse_date_ymd(raw_str).ok_or("invalid date")?;
            let formatted = format_date_ja(d);
            Ok((CellValue::Text(formatted.clone()), formatted))
        }
        FormValueType::Date => {
            if let Some(d) = parse_date_ymd(raw_str) {
                let iso = format!("{:04}-{:02}-{:02}", d.year, d.month, d.day);
                Ok((CellValue::Text(iso.clone()), iso))
            } else {
                Err("invalid date".into())
            }
        }
        FormValueType::Integer => {
            let n = raw_val
                .as_i64()
                .or_else(|| raw_str.trim().parse::<i64>().ok())
                .ok_or("invalid integer")?;
            if n.unsigned_abs() > 999_999_999_999_999 {
                return Err("integer out of Excel precision range".into());
            }
            Ok((CellValue::Number(n as f64), n.to_string()))
        }
        FormValueType::Bool => {
            let b = raw_val
                .as_bool()
                .or_else(|| match raw_str.trim().to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => Some(true),
                    "false" | "0" | "no" => Some(false),
                    _ => None,
                })
                .ok_or("invalid bool")?;
            Ok((CellValue::Bool(b), b.to_string()))
        }
        FormValueType::Text | FormValueType::Enum => {
            Ok((CellValue::Text(raw_str.to_string()), raw_str.to_string()))
        }
    }
}

/// Evaluates a [`FormMapping`] against the active worksheet and Vault profiles/credentials.
pub fn evaluate_form_mapping(
    presentation: &Presentation,
    slide: SlideId,
    mapping: &FormMapping,
    profiles: &[&VaultProfile],
    credentials: &[&VaultCredentialSummary],
    today_ymd: Option<&str>,
) -> FormEvaluation {
    let sheet = presentation.slide(slide).and_then(sheet_of_slide);
    let mut plans = Vec::with_capacity(mapping.fields.len());
    let mut ready_count = 0usize;
    let mut occupied_count = 0usize;
    let mut vc_backed_count = 0usize;
    let mut missing_required_count = 0usize;
    let mut overflow_count = 0usize;

    for field in &mapping.fields {
        let resolved_coords = target_cells_for_field(presentation, slide, &field.target);
        let (coords, target_summary) = match resolved_coords {
            Ok(pair) => pair,
            Err(err) => {
                plans.push(FormFieldPlan {
                    id: field.id.clone(),
                    label: field.label.clone(),
                    target_summary: err,
                    current_display: String::new(),
                    proposed_display: None,
                    proposed_raw: None,
                    provenance: None,
                    status: FieldStatus::BlockedTarget,
                    required: field.required,
                    max_chars: field.max_chars,
                    char_count: 0,
                    exceeds_max_chars: false,
                });
                continue;
            }
        };

        let current_display = sheet
            .map(|s| read_current_display(s, &field.target, &coords))
            .unwrap_or_default();
        let blocked = sheet.is_none_or(|s| coords.iter().any(|&(r, c)| is_cell_blocked(s, r, c)));
        let occupied = !blocked
            && sheet.is_some_and(|s| coords.iter().any(|&(r, c)| cell_is_nonempty(s, r, c)));

        let source_hit = resolve_field_source(field, profiles, credentials, today_ymd);
        let (proposed_raw, provenance) = match source_hit {
            Some((v, prov)) => (Some(v), Some(prov)),
            None => (None, None),
        };

        let (status, proposed_display) = if blocked {
            (FieldStatus::BlockedTarget, None)
        } else if let Some(ref val) = proposed_raw {
            match build_edits_for_field(presentation, slide, field, val) {
                Ok((_, disp)) => {
                    if occupied {
                        (FieldStatus::Occupied, Some(disp))
                    } else {
                        (FieldStatus::Ready, Some(disp))
                    }
                }
                Err(_) => (FieldStatus::UnsupportedValue, value_as_display_string(val)),
            }
        } else {
            (FieldStatus::MissingSource, None)
        };

        let effective_text = proposed_display
            .as_deref()
            .filter(|_| matches!(status, FieldStatus::Ready | FieldStatus::Occupied))
            .unwrap_or(&current_display);
        let char_count = effective_text.chars().count();
        let exceeds_max_chars = field.max_chars.is_some_and(|limit| char_count > limit);

        if status == FieldStatus::Ready {
            ready_count += 1;
        }
        if status == FieldStatus::Occupied {
            occupied_count += 1;
        }
        if matches!(provenance, Some(FieldProvenance::Credential { .. }))
            && matches!(status, FieldStatus::Ready | FieldStatus::Occupied)
        {
            vc_backed_count += 1;
        }
        if field.required && matches!(status, FieldStatus::MissingSource) && current_display.is_empty()
        {
            missing_required_count += 1;
        }
        if exceeds_max_chars {
            overflow_count += 1;
        }

        plans.push(FormFieldPlan {
            id: field.id.clone(),
            label: field.label.clone(),
            target_summary,
            current_display,
            proposed_display,
            proposed_raw,
            provenance,
            status,
            required: field.required,
            max_chars: field.max_chars,
            char_count,
            exceeds_max_chars,
        });
    }

    FormEvaluation {
        fields: plans,
        ready_count,
        occupied_count,
        vc_backed_count,
        missing_required_count,
        overflow_count,
    }
}

/// Builds a single undoable `Command::Sheet(SheetCommand::SetCells { .. })` for all selected
/// (or all eligible when `selected_ids` is `None`) fields in `evaluation`.
pub fn build_form_apply_command(
    presentation: &Presentation,
    slide: SlideId,
    mapping: &FormMapping,
    evaluation: &FormEvaluation,
    selected_ids: Option<&BTreeSet<String>>,
    allow_overwrite: bool,
) -> Result<(Command, usize), String> {
    let mut edits = Vec::new();
    let mut applied_fields = 0usize;
    let mut seen_cells = BTreeSet::new();

    for (field, plan) in mapping.fields.iter().zip(evaluation.fields.iter()) {
        if let Some(sel) = selected_ids
            && !sel.contains(&field.id)
        {
            continue;
        }
        let allowed = plan.status == FieldStatus::Ready
            || (allow_overwrite && plan.status == FieldStatus::Occupied);
        if !allowed {
            continue;
        }
        let Some(ref raw) = plan.proposed_raw else {
            continue;
        };
        let (field_edits, _) = build_edits_for_field(presentation, slide, field, raw)?;
        for edit in field_edits {
            if !seen_cells.insert((edit.row, edit.column)) {
                return Err(format!(
                    "duplicate target cell {}",
                    cell_name(edit.column, edit.row)
                ));
            }
            edits.push(edit);
        }
        applied_fields += 1;
    }

    if edits.is_empty() {
        return Err("適用できる入力候補がありません".to_string());
    }

    Ok((
        Command::Sheet(SheetCommand::SetCells {
            sheet: slide,
            edits,
        }),
        applied_fields,
    ))
}

/// Helper to return current local/UTC date as `"YYYY-MM-DD"` on native platforms.
pub fn current_date_ymd() -> Option<String> {
    #[cfg(not(target_family = "wasm"))]
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;
        let (year, m, d, _, _, _) = civil_datetime_from_unix(secs);
        Some(format!("{year:04}-{m:02}-{d:02}"))
    }
    #[cfg(target_family = "wasm")]
    {
        None
    }
}

/// Helper to return current UTC timestamp as ISO-8601 `"YYYY-MM-DDTHH:MM:SSZ"`.
pub fn current_utc_timestamp_iso() -> String {
    #[cfg(not(target_family = "wasm"))]
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (year, m, d, hh, mm, ss) = civil_datetime_from_unix(secs);
        format!("{year:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
    }
    #[cfg(target_family = "wasm")]
    {
        "1970-01-01T00:00:00Z".to_string()
    }
}

#[cfg(not(target_family = "wasm"))]
fn civil_datetime_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d, hh, mm, ss)
}

/// Computes lowercase hex SHA-256 digest of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = ibis_doc_model::sd_jwt::sha256(data);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

// ---------------------------------------------------------------------------
// 6. OKF `type: presentation-request` (`requests/*.req.md`)
// ---------------------------------------------------------------------------

/// Parsed OKF `type: presentation-request` Vault note describing a Verifier / OID4VP submission target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationRequestNote {
    pub path: String,
    pub request_id: String,
    pub title: String,
    pub purpose: Option<String>,
    pub client_id: String,
    pub response_uri: Option<String>,
    pub response_mode: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub dcql_credential_id: Option<String>,
    pub vct: Option<String>,
    pub requested_claims: Vec<String>,
}

impl PresentationRequestNote {
    /// Converts this Vault request note into an [`ibis_doc_model::Oid4vpRequest`].
    pub fn to_oid4vp_request(&self, fallback_nonce: &str) -> ibis_doc_model::Oid4vpRequest {
        ibis_doc_model::Oid4vpRequest {
            client_id: self.client_id.clone(),
            nonce: self
                .nonce
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| fallback_nonce.to_string()),
            response_uri: self.response_uri.clone(),
            response_mode: self
                .response_mode
                .clone()
                .or_else(|| Some("direct_post".to_string())),
            state: self.state.clone(),
            dcql_credential_id: self.dcql_credential_id.clone(),
            requested_claims: self.requested_claims.clone(),
        }
    }
}

/// Parses an OKF `type: presentation-request` Markdown note into a [`PresentationRequestNote`].
pub fn parse_presentation_request_markdown(
    path: &str,
    text: &str,
) -> Option<PresentationRequestNote> {
    let fm = crate::okf::frontmatter_map(text).ok()??;
    let doc_type = fm.get("type")?.as_str()?.trim();
    if !doc_type.eq_ignore_ascii_case("presentation-request") {
        return None;
    }

    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .trim_end_matches(".markdown")
        .trim_end_matches(".req")
        .to_string();
    let title = fm
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&stem)
        .to_string();

    let ext = fm
        .get("x-sorane-presentation-request")
        .or_else(|| fm.get("x-sorane-request"))
        .and_then(Value::as_object);

    let get_str = |key: &str| -> Option<String> {
        ext.and_then(|o| o.get(key))
            .or_else(|| fm.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    // If a raw `openid4vp_uri` is embedded, parse it as a base and overlay explicit YAML fields.
    let parsed_uri = get_str("openid4vp_uri")
        .or_else(|| get_str("request_uri"))
        .and_then(|u| ibis_doc_model::Oid4vpRequest::parse(&u));

    let request_id = get_str("request_id")
        .or_else(|| get_str("id"))
        .unwrap_or_else(|| stem.clone());
    let purpose = get_str("purpose");
    let client_id = get_str("client_id")
        .or_else(|| parsed_uri.as_ref().map(|r| r.client_id.clone()))
        .unwrap_or_else(|| "https://verifier.example.com".to_string());
    let response_uri = get_str("response_uri")
        .or_else(|| get_str("redirect_uri"))
        .or_else(|| parsed_uri.as_ref().and_then(|r| r.response_uri.clone()));
    let response_mode = get_str("response_mode")
        .or_else(|| parsed_uri.as_ref().and_then(|r| r.response_mode.clone()));
    let state = get_str("state").or_else(|| parsed_uri.as_ref().and_then(|r| r.state.clone()));
    let nonce = get_str("nonce").or_else(|| parsed_uri.as_ref().map(|r| r.nonce.clone()));
    let vct = get_str("vct");
    let mut dcql_credential_id = get_str("dcql_credential_id")
        .or_else(|| get_str("credential_id"))
        .or_else(|| {
            parsed_uri
                .as_ref()
                .and_then(|r| r.dcql_credential_id.clone())
        });

    let mut requested_claims = Vec::new();
    if let Some(uri_req) = &parsed_uri {
        for c in &uri_req.requested_claims {
            if !requested_claims.contains(c) {
                requested_claims.push(c.clone());
            }
        }
    }

    // Parse `claims` array (`["given_name", "address.region"]`) or `dcql_query` object
    let claims_val = ext.and_then(|o| o.get("claims")).or_else(|| fm.get("claims"));
    if let Some(Value::Array(arr)) = claims_val {
        for item in arr {
            if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                if !requested_claims.iter().any(|existing| existing == s) {
                    requested_claims.push(s.to_string());
                }
            } else if let Some(path_arr) = item.get("path").and_then(Value::as_array) {
                let joined: Vec<&str> = path_arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                if !joined.is_empty() {
                    let p = joined.join(".");
                    if !requested_claims.contains(&p) {
                        requested_claims.push(p);
                    }
                }
            }
        }
    }
    if let Some(dcql_val) = ext
        .and_then(|o| o.get("dcql_query"))
        .or_else(|| fm.get("dcql_query"))
    {
        if dcql_credential_id.is_none() {
            dcql_credential_id = dcql_val
                .get("credentials")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|c| c.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if let Ok(dcql_json) = serde_json::to_string(dcql_val)
            && let Some(parsed) = ibis_doc_model::Oid4vpRequest::parse(&dcql_json)
        {
            for c in parsed.requested_claims {
                if !requested_claims.contains(&c) {
                    requested_claims.push(c);
                }
            }
        }
    }

    Some(PresentationRequestNote {
        path: path.to_string(),
        request_id,
        title,
        purpose,
        client_id,
        response_uri,
        response_mode,
        state,
        nonce,
        dcql_credential_id,
        vct,
        requested_claims,
    })
}

/// Starter OKF `type: presentation-request` template for quick testing in a Vault.
pub fn starter_presentation_request_markdown() -> (&'static str, &'static str) {
    (
        "requests/sample-verifier.req.md",
        r#"---
type: presentation-request
title: サンプル提出先（OID4VP direct_post）
tags: [request, oid4vp]
x-sorane-presentation-request:
  version: 1
  request_id: sample-verifier
  purpose: 提出テスト（氏名・所属・住所の一部のみ開示）
  client_id: https://verifier.example.org
  response_uri: https://verifier.example.org/oid4vp/direct_post
  response_mode: direct_post
  dcql_credential_id: credential_1
  claims:
    - given_name
    - family_name
    - address.region
---

# サンプル提出先リクエスト

提出先の `client_id` / `response_uri` / `claims` を書き換えて利用してください。
ローカル検証用のモック Verifier (`http://127.0.0.1:...`) も指定できます。
"#,
    )
}

// ---------------------------------------------------------------------------
// 7. OKF `type: submission-receipt` (`submissions/YYYY-MM-DD-*.md`)
// ---------------------------------------------------------------------------

/// Immutable audit/receipt metadata recorded in the Vault after a VC/OID4VP submission or issuance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionReceipt {
    pub submitted_at: String,
    pub recipient: String,
    pub response_uri: Option<String>,
    /// `"oid4vp_direct_post"` | `"vc_issue_sd_jwt"` | `"vc_issue_mdoc"` | `"vp_token_copy"`
    pub mode: String,
    pub http_status: Option<u16>,
    pub form_id: Option<String>,
    pub form_mapping_path: Option<String>,
    pub request_path: Option<String>,
    pub source_paths: Vec<String>,
    pub artifact_path: Option<String>,
    pub disclosed_fields: Vec<String>,
    pub payload_sha256: String,
    pub key_storage: String,
    pub holder_key_thumbprint: Option<String>,
    pub notes: Option<String>,
}

fn wiki_link_for_vault_path(rel_path: &str) -> String {
    let trimmed = rel_path.trim().trim_start_matches('/');
    let without_md = trimmed
        .strip_suffix(".md")
        .or_else(|| trimmed.strip_suffix(".markdown"))
        .unwrap_or(trimmed);
    format!("[[{without_md}]]")
}

/// Computes a safe relative Vault path for a submission receipt (`submissions/YYYY-MM-DD-<slug>.md`).
pub fn submission_receipt_relative_path(date_ymd: &str, slug_hint: &str) -> String {
    let slug: String = slug_hint
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let slug = if slug.is_empty() {
        "submission".to_string()
    } else {
        slug.chars().take(40).collect()
    };
    let date_prefix = if date_ymd.len() >= 10 {
        &date_ymd[..10]
    } else {
        "undated"
    };
    format!("submissions/{date_prefix}-{slug}.md")
}

/// Renders a [`SubmissionReceipt`] as an OKF `type: submission-receipt` Markdown note with
/// `[[Wiki links]]` connecting the submission to its source profiles, credentials, mappings, and requests.
pub fn render_submission_receipt_markdown(receipt: &SubmissionReceipt) -> String {
    let mut out = String::new();
    let title = format!(
        "提出記録: {} ({})",
        receipt.recipient,
        receipt.submitted_at.get(..10).unwrap_or(&receipt.submitted_at)
    );
    out.push_str("---\n");
    out.push_str("type: submission-receipt\n");
    out.push_str(&format!("title: {}\n", yaml_quote(&title)));
    out.push_str("tags: [submission, vc]\n");
    out.push_str(&format!("submitted_at: {}\n", yaml_quote(&receipt.submitted_at)));
    out.push_str(&format!("recipient: {}\n", yaml_quote(&receipt.recipient)));
    if let Some(uri) = &receipt.response_uri {
        out.push_str(&format!("response_uri: {}\n", yaml_quote(uri)));
    }
    out.push_str(&format!("mode: {}\n", yaml_quote(&receipt.mode)));
    if let Some(status) = receipt.http_status {
        out.push_str(&format!("http_status: {status}\n"));
    }
    if let Some(form_id) = &receipt.form_id {
        out.push_str(&format!("form_id: {}\n", yaml_quote(form_id)));
    }
    if let Some(map_path) = &receipt.form_mapping_path {
        out.push_str(&format!("form_mapping: {}\n", yaml_quote(map_path)));
    }
    if let Some(req_path) = &receipt.request_path {
        out.push_str(&format!("request: {}\n", yaml_quote(req_path)));
    }
    if let Some(art_path) = &receipt.artifact_path {
        out.push_str(&format!("artifact: {}\n", yaml_quote(art_path)));
    }
    out.push_str(&format!(
        "payload_sha256: {}\n",
        yaml_quote(&receipt.payload_sha256)
    ));
    out.push_str(&format!(
        "key_storage: {}\n",
        yaml_quote(&receipt.key_storage)
    ));
    if let Some(thumb) = &receipt.holder_key_thumbprint {
        out.push_str(&format!("holder_key_thumbprint: {}\n", yaml_quote(thumb)));
    }
    if !receipt.disclosed_fields.is_empty() {
        out.push_str("disclosed_fields:\n");
        for f in &receipt.disclosed_fields {
            out.push_str(&format!("  - {}\n", yaml_quote(f)));
        }
    }
    out.push_str("---\n\n");

    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!("- **提出日時 (UTC)**: `{}`\n", receipt.submitted_at));
    out.push_str(&format!("- **提出先**: `{}`\n", receipt.recipient));
    if let Some(uri) = &receipt.response_uri {
        out.push_str(&format!("- **送信先 URI**: `{uri}`\n"));
    }
    out.push_str(&format!("- **提出方式**: `{}`\n", receipt.mode));
    if let Some(status) = receipt.http_status {
        out.push_str(&format!("- **HTTP ステータス**: `{status}`\n"));
    }
    out.push_str(&format!(
        "- **鍵保管**: `{}`{}\n",
        receipt.key_storage,
        receipt
            .holder_key_thumbprint
            .as_deref()
            .map(|t| format!(" (`{t}`)"))
            .unwrap_or_default()
    ));
    out.push_str(&format!(
        "- **ペイロード SHA-256**: `{}`\n",
        receipt.payload_sha256
    ));

    if !receipt.disclosed_fields.is_empty() {
        out.push_str("\n## 開示した項目\n\n");
        for f in &receipt.disclosed_fields {
            out.push_str(&format!("- `{f}`\n"));
        }
    }

    let has_links = receipt.form_mapping_path.is_some()
        || receipt.request_path.is_some()
        || receipt.artifact_path.is_some()
        || !receipt.source_paths.is_empty();
    if has_links {
        out.push_str("\n## 関連ファイル\n\n");
        if let Some(req) = &receipt.request_path {
            out.push_str(&format!("- 提出要求: {}\n", wiki_link_for_vault_path(req)));
        }
        if let Some(map) = &receipt.form_mapping_path {
            out.push_str(&format!("- フォーム対応表: {}\n", wiki_link_for_vault_path(map)));
        }
        if let Some(art) = &receipt.artifact_path {
            out.push_str(&format!("- 出力ファイル: {}\n", wiki_link_for_vault_path(art)));
        }
        for src in &receipt.source_paths {
            out.push_str(&format!("- 参照元: {}\n", wiki_link_for_vault_path(src)));
        }
    }

    if let Some(notes) = receipt.notes.as_deref().filter(|s| !s.trim().is_empty()) {
        out.push_str("\n## メモ\n\n");
        out.push_str(notes.trim());
        out.push('\n');
    }

    out
}

// ---------------------------------------------------------------------------
// 8. Unified DOCX & Markdown (`FlowDocument`) Form Assist (Phase 3)
// ---------------------------------------------------------------------------

/// Kind of form target discovered inside an open DOCX or Markdown [`ibis_doc_model::FlowDocument`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowFormFieldKind {
    /// Word `w:sdt` plain-text or rich-text content control (including Markdown `{{form:<id>}}`).
    ContentControlText {
        /// True when the control tag starts with `ibis.form.` (e.g. imported from Markdown `{{form:<id>}}`).
        is_ibis_form_tag: bool,
    },
    /// Word `w:sdt` date picker content control.
    DatePicker { date_format: String },
    /// Word `w:sdt` checkbox content control.
    CheckBox { checked: bool },
    /// Word `w:sdt` dropdown list / combo box content control (`(display, value)` items).
    DropDownList {
        items: Vec<(String, String)>,
        selected: usize,
    },
    /// Legacy Word `FORMTEXT` field (`w:ffData`).
    LegacyTextInput,
    /// Literal `{{form:<id>}}` placeholder inside a plain text run (if not yet converted to a control).
    TextPlaceholder {
        /// End character offset within the paragraph (exclusive).
        end_offset: usize,
    },
}

/// A single form field target discovered in a [`ibis_doc_model::FlowDocument`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowFormTarget {
    pub id: String,
    pub label: String,
    pub pos: ibis_doc_model::FlowPos,
    pub kind: FlowFormFieldKind,
    pub current_display: String,
    pub is_empty_or_placeholder: bool,
    pub locked: bool,
    pub max_chars: Option<usize>,
}

fn normalize_form_field_key(raw: &str) -> String {
    let trimmed = raw.trim();
    for prefix in ["ibis.form.", "claim.", "sd.", "form."] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    // Strip XFA / AcroForm hierarchical index wrappers like `form1[0].Page1[0].company_name[0]`
    if let Some(last) = trimmed.rsplit('.').next()
        && trimmed.contains('[')
    {
        let stripped = last.split('[').next().unwrap_or(last).trim();
        if !stripped.is_empty() {
            return stripped.to_string();
        }
    }
    trimmed.to_string()
}

fn heuristic_key_for_label_or_id(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    HEURISTIC_LABEL_RULES
        .iter()
        .find(|(patterns, _, _, _)| {
            patterns
                .iter()
                .any(|pat| trimmed.contains(pat) || lower.contains(pat))
        })
        .map(|(_, field_id, _, _)| *field_id)
}

fn resolve_multi_format_value(
    field_id: &str,
    label: &str,
    profiles: &[&VaultProfile],
    credentials: &[&VaultCredentialSummary],
    today_ymd: Option<&str>,
) -> Option<(Value, FieldProvenance)> {
    let mut keys = Vec::new();
    let norm = normalize_form_field_key(field_id);
    if !norm.is_empty() {
        keys.push(norm.clone());
    }
    if let Some(h) = heuristic_key_for_label_or_id(&norm)
        && !keys.iter().any(|k| k == h)
    {
        keys.push(h.to_string());
    }
    if let Some(h) = heuristic_key_for_label_or_id(label)
        && !keys.iter().any(|k| k == h)
    {
        keys.push(h.to_string());
    }

    let sources: Vec<FormSourceRef> = keys.into_iter().map(FormSourceRef::Any).collect();
    let default = if sources.iter().any(|s| match s {
        FormSourceRef::Any(k) => k == "document.date" || k == "date" || k == "issue_date",
        _ => false,
    }) {
        Some("today".to_string())
    } else {
        None
    };

    let synthetic = FormField {
        id: field_id.to_string(),
        label: label.to_string(),
        target: FormTarget::Cell("A1".to_string()),
        sources,
        value_type: FormValueType::Text,
        required: false,
        max_chars: None,
        default,
    };
    resolve_field_source(&synthetic, profiles, credentials, today_ymd)
}

fn format_flow_value_for_target(target: &FlowFormTarget, raw: &Value) -> Result<String, String> {
    let s = value_as_display_string(raw).ok_or_else(|| "unsupported value".to_string())?;
    match &target.kind {
        FlowFormFieldKind::DatePicker { date_format } => {
            if let Some(d) = parse_date_ymd(&s) {
                let want_ja = date_format.contains('g')
                    || date_format.contains('e')
                    || date_format.contains('年')
                    || target.label.contains("和暦");
                if want_ja {
                    Ok(format_date_ja(d))
                } else {
                    Ok(format!("{:04}-{:02}-{:02}", d.year, d.month, d.day))
                }
            } else {
                Ok(s)
            }
        }
        FlowFormFieldKind::CheckBox { .. } => {
            let b = raw
                .as_bool()
                .or_else(|| match s.trim().to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" | "はい" | "有" | "☑" => Some(true),
                    "false" | "0" | "no" | "off" | "いいえ" | "無" | "□" => Some(false),
                    _ => None,
                })
                .ok_or_else(|| "invalid boolean".to_string())?;
            Ok(if b { "☑ (true)" } else { "☐ (false)" }.to_string())
        }
        FlowFormFieldKind::DropDownList { items, .. } => {
            let needle = s.trim().to_ascii_lowercase();
            items
                .iter()
                .find(|(disp, val)| {
                    disp.trim().to_ascii_lowercase() == needle
                        || val.trim().to_ascii_lowercase() == needle
                })
                .map(|(disp, _)| disp.clone())
                .ok_or_else(|| format!("dropdown option not found: {s}"))
        }
        _ => Ok(s),
    }
}

/// Discovers all DOCX content controls, legacy Word form fields, and `{{form:<id>}}` placeholders
/// across body paragraphs and tables in a [`ibis_doc_model::FlowDocument`].
pub fn discover_flow_form_targets(doc: &ibis_doc_model::FlowDocument) -> Vec<FlowFormTarget> {
    use ibis_doc_model::flow::{FlowRunKind, FormFieldKind};
    use ibis_doc_model::flow_command::{FlowPos, all_paragraphs};

    let mut targets = Vec::new();
    for para in all_paragraphs(doc) {
        let mut offset = 0usize;
        for run in &para.runs {
            let pos = FlowPos::new(para.id, offset);
            let ctrl = run.control.as_deref();
            let locked = ctrl.is_some_and(|c| c.lock.keeps_content());
            let ctrl_tag = ctrl.map(|c| c.tag.trim()).unwrap_or("");
            let ctrl_title = ctrl.map(|c| c.title.trim()).unwrap_or("");

            match &run.kind {
                FlowRunKind::PlainText { text, placeholder }
                | FlowRunKind::RichText { text, placeholder } => {
                    let is_ibis_form_tag = ctrl_tag.starts_with(crate::form_template::PREFIX);
                    let raw_id = if !ctrl_tag.is_empty() {
                        ctrl_tag
                    } else if !ctrl_title.is_empty() {
                        ctrl_title
                    } else {
                        ""
                    };
                    if !raw_id.is_empty() {
                        let id = normalize_form_field_key(raw_id);
                        let label = if !ctrl_title.is_empty() {
                            ctrl_title.to_string()
                        } else {
                            id.clone()
                        };
                        let trimmed = text.trim();
                        let is_empty = *placeholder
                            || trimmed.is_empty()
                            || trimmed == "…"
                            || trimmed == "...";
                        targets.push(FlowFormTarget {
                            id,
                            label,
                            pos,
                            kind: FlowFormFieldKind::ContentControlText { is_ibis_form_tag },
                            current_display: if is_empty {
                                String::new()
                            } else {
                                text.clone()
                            },
                            is_empty_or_placeholder: is_empty,
                            locked,
                            max_chars: None,
                        });
                    }
                }
                FlowRunKind::DatePicker {
                    full_date,
                    date_format,
                    text,
                } => {
                    let raw_id = if !ctrl_tag.is_empty() {
                        ctrl_tag
                    } else if !ctrl_title.is_empty() {
                        ctrl_title
                    } else {
                        "document.date"
                    };
                    let id = normalize_form_field_key(raw_id);
                    let label = if !ctrl_title.is_empty() {
                        ctrl_title.to_string()
                    } else {
                        id.clone()
                    };
                    let is_empty = full_date.trim().is_empty() && text.trim().is_empty();
                    targets.push(FlowFormTarget {
                        id,
                        label,
                        pos,
                        kind: FlowFormFieldKind::DatePicker {
                            date_format: date_format.clone(),
                        },
                        current_display: if text.trim().is_empty() {
                            full_date.clone()
                        } else {
                            text.clone()
                        },
                        is_empty_or_placeholder: is_empty,
                        locked,
                        max_chars: None,
                    });
                }
                FlowRunKind::CheckBox { checked, .. } => {
                    let raw_id = if !ctrl_tag.is_empty() {
                        ctrl_tag
                    } else {
                        ctrl_title
                    };
                    if !raw_id.is_empty() {
                        let id = normalize_form_field_key(raw_id);
                        let label = if !ctrl_title.is_empty() {
                            ctrl_title.to_string()
                        } else {
                            id.clone()
                        };
                        targets.push(FlowFormTarget {
                            id,
                            label,
                            pos,
                            kind: FlowFormFieldKind::CheckBox { checked: *checked },
                            current_display: if *checked { "☑" } else { "☐" }.to_string(),
                            is_empty_or_placeholder: !*checked,
                            locked,
                            max_chars: None,
                        });
                    }
                }
                FlowRunKind::DropDownList {
                    items, selected, ..
                } => {
                    let raw_id = if !ctrl_tag.is_empty() {
                        ctrl_tag
                    } else {
                        ctrl_title
                    };
                    if !raw_id.is_empty() {
                        let id = normalize_form_field_key(raw_id);
                        let label = if !ctrl_title.is_empty() {
                            ctrl_title.to_string()
                        } else {
                            id.clone()
                        };
                        let current = items
                            .get(*selected)
                            .map(|(d, _)| d.clone())
                            .unwrap_or_default();
                        targets.push(FlowFormTarget {
                            id,
                            label,
                            pos,
                            kind: FlowFormFieldKind::DropDownList {
                                items: items.clone(),
                                selected: *selected,
                            },
                            current_display: current.clone(),
                            is_empty_or_placeholder: current.trim().is_empty() || *selected == 0,
                            locked,
                            max_chars: None,
                        });
                    }
                }
                FlowRunKind::FormField(ff) => {
                    if let FormFieldKind::TextInput {
                        value, max_length, ..
                    } = &ff.kind
                    {
                        let raw_id = ff.name.trim();
                        if !raw_id.is_empty() {
                            let id = normalize_form_field_key(raw_id);
                            let is_empty = value.trim().is_empty();
                            targets.push(FlowFormTarget {
                                id: id.clone(),
                                label: raw_id.to_string(),
                                pos,
                                kind: FlowFormFieldKind::LegacyTextInput,
                                current_display: value.clone(),
                                is_empty_or_placeholder: is_empty,
                                locked: !ff.enabled,
                                max_chars: (*max_length > 0).then_some(*max_length),
                            });
                        }
                    }
                }
                FlowRunKind::Text(text) if text.contains("{{form:") => {
                    let mut search_byte = 0usize;
                    while let Some(rel) = text[search_byte..].find("{{form:") {
                        let start_byte = search_byte + rel;
                        let after = &text[start_byte + "{{form:".len()..];
                        let Some(close_rel) = after.find("}}") else {
                            break;
                        };
                        let raw_id = after[..close_rel].trim();
                        let end_byte = start_byte + "{{form:".len() + close_rel + "}}".len();
                        if crate::form_template::valid_field_id(raw_id) {
                            let start_chars = text[..start_byte].chars().count();
                            let end_chars = text[..end_byte].chars().count();
                            let id = normalize_form_field_key(raw_id);
                            targets.push(FlowFormTarget {
                                id: id.clone(),
                                label: id,
                                pos: FlowPos::new(para.id, offset + start_chars),
                                kind: FlowFormFieldKind::TextPlaceholder {
                                    end_offset: offset + end_chars,
                                },
                                current_display: String::new(),
                                is_empty_or_placeholder: true,
                                locked: false,
                                max_chars: None,
                            });
                        }
                        search_byte = end_byte;
                    }
                }
                _ => {}
            }

            offset += run.len();
        }
    }
    targets
}

/// Evaluates all discovered DOCX / Markdown form targets against Vault profiles and VCs.
pub fn evaluate_flow_form(
    doc: &ibis_doc_model::FlowDocument,
    profiles: &[&VaultProfile],
    credentials: &[&VaultCredentialSummary],
    today_ymd: Option<&str>,
) -> (Vec<FlowFormTarget>, FormEvaluation) {
    let targets = discover_flow_form_targets(doc);
    let mut plans = Vec::with_capacity(targets.len());
    let mut ready_count = 0usize;
    let mut occupied_count = 0usize;
    let mut vc_backed_count = 0usize;
    let mut overflow_count = 0usize;

    for target in &targets {
        let target_summary = match &target.kind {
            FlowFormFieldKind::ContentControlText {
                is_ibis_form_tag: true,
            } => format!("form:{}", target.id),
            FlowFormFieldKind::ContentControlText { .. } => "テキスト欄".to_string(),
            FlowFormFieldKind::DatePicker { .. } => "日付選択".to_string(),
            FlowFormFieldKind::CheckBox { .. } => "チェックボックス".to_string(),
            FlowFormFieldKind::DropDownList { .. } => "ドロップダウン".to_string(),
            FlowFormFieldKind::LegacyTextInput => "フォーム入力".to_string(),
            FlowFormFieldKind::TextPlaceholder { .. } => format!("{{{{form:{}}}}}", target.id),
        };

        let source_hit = resolve_multi_format_value(
            &target.id,
            &target.label,
            profiles,
            credentials,
            today_ymd,
        );
        let (proposed_raw, provenance) = match source_hit {
            Some((v, prov)) => (Some(v), Some(prov)),
            None => (None, None),
        };

        let (status, proposed_display) = if target.locked {
            (FieldStatus::BlockedTarget, None)
        } else if let Some(ref raw) = proposed_raw {
            match format_flow_value_for_target(target, raw) {
                Ok(disp) => {
                    if target.is_empty_or_placeholder {
                        (FieldStatus::Ready, Some(disp))
                    } else {
                        (FieldStatus::Occupied, Some(disp))
                    }
                }
                Err(_) => (FieldStatus::UnsupportedValue, value_as_display_string(raw)),
            }
        } else {
            (FieldStatus::MissingSource, None)
        };

        let effective = proposed_display
            .as_deref()
            .filter(|_| matches!(status, FieldStatus::Ready | FieldStatus::Occupied))
            .unwrap_or(&target.current_display);
        let char_count = effective.chars().count();
        let exceeds_max_chars = target.max_chars.is_some_and(|limit| char_count > limit);

        if status == FieldStatus::Ready {
            ready_count += 1;
        }
        if status == FieldStatus::Occupied {
            occupied_count += 1;
        }
        if matches!(provenance, Some(FieldProvenance::Credential { .. }))
            && matches!(status, FieldStatus::Ready | FieldStatus::Occupied)
        {
            vc_backed_count += 1;
        }
        if exceeds_max_chars {
            overflow_count += 1;
        }

        plans.push(FormFieldPlan {
            id: target.id.clone(),
            label: target.label.clone(),
            target_summary,
            current_display: target.current_display.clone(),
            proposed_display,
            proposed_raw,
            provenance,
            status,
            required: false,
            max_chars: target.max_chars,
            char_count,
            exceeds_max_chars,
        });
    }

    (
        targets,
        FormEvaluation {
            fields: plans,
            ready_count,
            occupied_count,
            vc_backed_count,
            missing_required_count: 0,
            overflow_count,
        },
    )
}

fn flatten_block_form_controls(
    blocks: &mut [ibis_doc_model::flow::Block],
    replacements: &BTreeMap<(u32, usize), String>,
    placeholder_replacements: &BTreeMap<(u32, usize, usize), String>,
    applied: &mut usize,
) {
    use ibis_doc_model::flow::{Block, FlowRunKind};
    for block in blocks {
        match block {
            Block::Paragraph(para) => {
                let mut offset = 0usize;
                for run in &mut para.runs {
                    let run_len = run.len();
                    if let Some(new_text) = replacements.get(&(para.id, offset)) {
                        run.kind = FlowRunKind::Text(new_text.clone());
                        run.control = None;
                        *applied += 1;
                    } else if let FlowRunKind::Text(text) = &mut run.kind {
                        let matching: Vec<(usize, usize, String)> = placeholder_replacements
                            .iter()
                            .filter(|&(&(pid, start, end), _)| {
                                pid == para.id && start >= offset && end <= offset + run_len
                            })
                            .map(|(&(_, start, end), val)| (start - offset, end - offset, val.clone()))
                            .collect();
                        if !matching.is_empty() {
                            let mut chars: Vec<char> = text.chars().collect();
                            for (start_c, end_c, val) in matching.into_iter().rev() {
                                if end_c <= chars.len() && start_c <= end_c {
                                    chars.splice(start_c..end_c, val.chars());
                                    *applied += 1;
                                }
                            }
                            *text = chars.into_iter().collect();
                        }
                    }
                    offset += run_len;
                }
            }
            Block::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        flatten_block_form_controls(
                            &mut cell.blocks,
                            replacements,
                            placeholder_replacements,
                            applied,
                        );
                    }
                }
            }
        }
    }
}

/// Builds undoable `Command::Flow(...)` edits for all eligible fields in a DOCX or Markdown document.
///
/// When `flatten_markdown_controls` is `true` (e.g. an open `.md` file containing `{{form:<id>}}`
/// controls), replaces each `ibis.form.*` control run with a plain `FlowRunKind::Text` run via an
/// atomic `FlowCommand::ReplaceDocument` so saving the Markdown file persists the filled text.
pub fn build_flow_form_apply_commands(
    doc: &ibis_doc_model::FlowDocument,
    targets: &[FlowFormTarget],
    evaluation: &FormEvaluation,
    allow_overwrite: bool,
    flatten_markdown_controls: bool,
) -> Result<(Vec<Command>, usize), String> {
    use ibis_doc_model::flow_command::{FlowCommand, FlowPos};

    if flatten_markdown_controls {
        let mut control_replacements = BTreeMap::new();
        let mut placeholder_replacements = BTreeMap::new();
        for (target, plan) in targets.iter().zip(evaluation.fields.iter()) {
            let allowed = plan.status == FieldStatus::Ready
                || (allow_overwrite && plan.status == FieldStatus::Occupied);
            if !allowed {
                continue;
            }
            let Some(ref raw) = plan.proposed_raw else {
                continue;
            };
            let display_text = format_flow_value_for_target(target, raw)?;
            match &target.kind {
                FlowFormFieldKind::ContentControlText { .. } => {
                    control_replacements.insert((target.pos.para, target.pos.offset), display_text);
                }
                FlowFormFieldKind::TextPlaceholder { end_offset } => {
                    placeholder_replacements.insert(
                        (target.pos.para, target.pos.offset, *end_offset),
                        display_text,
                    );
                }
                _ => {}
            }
        }
        if !control_replacements.is_empty() || !placeholder_replacements.is_empty() {
            let mut updated = doc.clone();
            let mut applied = 0usize;
            flatten_block_form_controls(
                &mut updated.body,
                &control_replacements,
                &placeholder_replacements,
                &mut applied,
            );
            flatten_block_form_controls(
                &mut updated.header,
                &control_replacements,
                &placeholder_replacements,
                &mut applied,
            );
            flatten_block_form_controls(
                &mut updated.footer,
                &control_replacements,
                &placeholder_replacements,
                &mut applied,
            );
            if applied > 0 {
                return Ok((
                    vec![Command::Flow(FlowCommand::ReplaceDocument(Box::new(updated)))],
                    applied,
                ));
            }
        }
    }

    // Process targets in reverse paragraph/offset/run order so earlier edits (including expanding
    // a zero-length empty DatePicker or PlainText) never shift subsequent target offsets.
    let mut indices: Vec<usize> = (0..targets.len()).collect();
    indices.sort_by(|&a, &b| {
        targets[b]
            .pos
            .para
            .cmp(&targets[a].pos.para)
            .then_with(|| targets[b].pos.offset.cmp(&targets[a].pos.offset))
            .then_with(|| b.cmp(&a))
    });

    let mut cmds = Vec::new();
    let mut applied = 0usize;

    for idx in indices {
        let target = &targets[idx];
        let plan = &evaluation.fields[idx];
        let allowed = plan.status == FieldStatus::Ready
            || (allow_overwrite && plan.status == FieldStatus::Occupied);
        if !allowed {
            continue;
        }
        let Some(ref raw) = plan.proposed_raw else {
            continue;
        };
        let display_text = format_flow_value_for_target(target, raw)?;

        match &target.kind {
            FlowFormFieldKind::ContentControlText { .. } => {
                cmds.push(Command::Flow(FlowCommand::SetContentControlText {
                    at: target.pos,
                    text: display_text,
                    placeholder: false,
                }));
                applied += 1;
            }
            FlowFormFieldKind::DatePicker { .. } => {
                let s = value_as_display_string(raw).unwrap_or_else(|| display_text.clone());
                let full_date = parse_date_ymd(&s)
                    .map(|d| format!("{:04}-{:02}-{:02}T00:00:00Z", d.year, d.month, d.day))
                    .unwrap_or(s);
                cmds.push(Command::Flow(FlowCommand::SetDatePickerDate {
                    at: target.pos,
                    full_date,
                    text: display_text,
                }));
                applied += 1;
            }
            FlowFormFieldKind::CheckBox { checked } => {
                let desired = display_text.starts_with('☑');
                if desired != *checked {
                    cmds.push(Command::Flow(FlowCommand::ToggleCheckBox {
                        at: target.pos,
                    }));
                }
                applied += 1;
            }
            FlowFormFieldKind::DropDownList { items, .. } => {
                let needle = value_as_display_string(raw)
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                if let Some(index) = items.iter().position(|(disp, val)| {
                    disp.trim().to_ascii_lowercase() == needle
                        || val.trim().to_ascii_lowercase() == needle
                }) {
                    cmds.push(Command::Flow(FlowCommand::SelectDropDownItem {
                        at: target.pos,
                        index,
                    }));
                    applied += 1;
                }
            }
            FlowFormFieldKind::LegacyTextInput => {
                cmds.push(Command::Flow(FlowCommand::SetFormFieldText {
                    at: target.pos,
                    text: display_text,
                }));
                applied += 1;
            }
            FlowFormFieldKind::TextPlaceholder { end_offset } => {
                let to = FlowPos::new(target.pos.para, *end_offset);
                cmds.push(Command::Flow(FlowCommand::Delete {
                    from: target.pos,
                    to,
                }));
                cmds.push(Command::Flow(FlowCommand::InsertText {
                    at: target.pos,
                    text: display_text,
                    format: None,
                }));
                applied += 1;
            }
        }
    }

    if cmds.is_empty() {
        return Err("適用できるフォーム入力候補がありません。".to_string());
    }
    Ok((cmds, applied))
}

// ---------------------------------------------------------------------------
// 9. Unified PDF AcroForm Form Assist (Phase 3, native)
// ---------------------------------------------------------------------------

/// Editable text field discovered in a PDF AcroForm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfFormFieldTarget {
    pub name: String,
    pub current_value: String,
    pub max_len: Option<usize>,
    pub multiline: bool,
    pub widget_count: usize,
}

/// Inspects editable AcroForm text fields in any PDF file (including PDF/A-2b and standard forms).
#[cfg(not(target_family = "wasm"))]
pub fn inspect_pdf_form_fields(bytes: Vec<u8>) -> Result<(bool, Vec<PdfFormFieldTarget>), String> {
    let mut pdf =
        ibis_pdf::PdfContainer::from_bytes(bytes).map_err(|e| format!("PDF 読込エラー: {e}"))?;
    let is_pdfa2b = ibis_pdf::pdfa::inspect(&mut pdf)
        .ok()
        .and_then(|info| info.declared_profile().map(|p| p == "PDF/A-2b"))
        .unwrap_or(false);
    let fields =
        ibis_pdf::acroform::text_fields_any(&mut pdf).map_err(|e| format!("AcroForm 解析エラー: {e}"))?;
    let targets = fields
        .into_iter()
        .map(|(name, field)| PdfFormFieldTarget {
            name,
            current_value: field.current_value(),
            max_len: field.max_len,
            multiline: field.multiline,
            widget_count: field.widget_count(),
        })
        .collect();
    Ok((is_pdfa2b, targets))
}

/// Evaluates discovered PDF AcroForm fields against Vault profiles and VCs using the unified IR.
pub fn evaluate_pdf_form(
    targets: &[PdfFormFieldTarget],
    profiles: &[&VaultProfile],
    credentials: &[&VaultCredentialSummary],
    today_ymd: Option<&str>,
) -> FormEvaluation {
    let mut plans = Vec::with_capacity(targets.len());
    let mut ready_count = 0usize;
    let mut occupied_count = 0usize;
    let mut vc_backed_count = 0usize;
    let mut overflow_count = 0usize;

    for target in targets {
        let norm_id = normalize_form_field_key(&target.name);
        let source_hit =
            resolve_multi_format_value(&norm_id, &target.name, profiles, credentials, today_ymd);
        let (proposed_raw, provenance) = match source_hit {
            Some((v, prov)) => (Some(v), Some(prov)),
            None => (None, None),
        };
        let proposed_display = proposed_raw.as_ref().and_then(value_as_display_string);
        let is_empty = target.current_value.trim().is_empty();
        let status = match &proposed_display {
            Some(_) if is_empty => FieldStatus::Ready,
            Some(_) => FieldStatus::Occupied,
            None => FieldStatus::MissingSource,
        };
        let effective = proposed_display
            .as_deref()
            .filter(|_| matches!(status, FieldStatus::Ready | FieldStatus::Occupied))
            .unwrap_or(&target.current_value);
        let char_count = effective.chars().count();
        let exceeds_max_chars = target.max_len.is_some_and(|limit| char_count > limit);

        if status == FieldStatus::Ready {
            ready_count += 1;
        }
        if status == FieldStatus::Occupied {
            occupied_count += 1;
        }
        if matches!(provenance, Some(FieldProvenance::Credential { .. }))
            && matches!(status, FieldStatus::Ready | FieldStatus::Occupied)
        {
            vc_backed_count += 1;
        }
        if exceeds_max_chars {
            overflow_count += 1;
        }

        plans.push(FormFieldPlan {
            id: target.name.clone(),
            label: norm_id,
            target_summary: format!("PDF:{}", target.name),
            current_display: target.current_value.clone(),
            proposed_display,
            proposed_raw,
            provenance,
            status,
            required: false,
            max_chars: target.max_len,
            char_count,
            exceeds_max_chars,
        });
    }

    FormEvaluation {
        fields: plans,
        ready_count,
        occupied_count,
        vc_backed_count,
        missing_required_count: 0,
        overflow_count,
    }
}

/// Fills matching PDF AcroForm text fields and returns an incrementally saved PDF copy.
#[cfg(not(target_family = "wasm"))]
pub fn fill_pdf_form_values(
    bytes: Vec<u8>,
    values: &BTreeMap<String, String>,
) -> Result<(Vec<u8>, usize), String> {
    let mut pdf =
        ibis_pdf::PdfContainer::from_bytes(bytes).map_err(|e| format!("PDF 読込エラー: {e}"))?;
    let fields =
        ibis_pdf::acroform::text_fields_any(&mut pdf).map_err(|e| format!("AcroForm 解析エラー: {e}"))?;
    let mut applied = 0usize;
    for (name, field) in fields {
        if let Some(val) = values.get(&name) {
            if let Some(limit) = field.max_len
                && val.encode_utf16().count() > limit
            {
                return Err(format!("PDF 欄 `{name}` の文字数上限 ({limit}) を超過しています。"));
            }
            let sanitized = if field.multiline {
                val.replace('\t', " ")
            } else {
                val.replace(['\r', '\n', '\t'], " ")
            };
            field
                .set_text_value(&mut pdf, &sanitized)
                .map_err(|e| format!("PDF 欄 `{name}` の書き込みに失敗しました: {e}"))?;
            applied += 1;
        }
    }
    if applied == 0 {
        return Err("PDF に書き込む入力候補がありません。".to_string());
    }
    ibis_pdf::acroform::set_need_appearances(&mut pdf)
        .map_err(|e| format!("NeedAppearances 設定に失敗しました: {e}"))?;
    let out = pdf
        .save(ibis_pdf::SaveMode::Incremental)
        .map_err(|e| format!("PDF 保存に失敗しました: {e}"))?;
    Ok((out, applied))
}

#[allow(dead_code)]
fn _ensure_map_import(_: &Map<String, Value>) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use ibis_doc_model::sheet::{CellKind, CellRange, SheetCell, SheetState, WorkbookSheet};
    use ibis_doc_model::{Document, EmuRect, Shape, ShapeId, ShapeKind, Slide, Workbook};

    fn make_cell(row: u32, col: u32, text: &str) -> SheetCell {
        SheetCell {
            row,
            column: col,
            value: CellValue::Text(text.to_string()),
            text: text.to_string(),
            kind: CellKind::Text,
            formula: None,
            style: 0,
            phonetic: None,
            runs: None,
        }
    }

    fn sample_invoice_presentation() -> (Presentation, SlideId) {
        let slide_id = SlideId(1);
        let mut sheet = Sheet::default();
        // Labels
        sheet.cells.push(make_cell(0, 0, "請求書"));
        sheet.cells.push(make_cell(2, 0, "会社名"));
        sheet.cells.push(make_cell(3, 0, "適格請求書登録番号"));
        sheet.cells.push(make_cell(4, 0, "郵便番号"));
        sheet.cells.push(make_cell(5, 0, "請求日"));
        sheet.cells.push(make_cell(6, 0, "課税区分"));
        // Merged input cell for company name: B3:D3 (0-based row 2, col 1..=3)
        sheet.merges.push(CellRange {
            first_row: 2,
            first_column: 1,
            last_row: 2,
            last_column: 3,
        });

        let mut slide = Slide::new(slide_id);
        slide.name = Some("請求書".into());
        slide.shapes.push(Shape::new(
            ShapeId(1),
            EmuRect::default(),
            ShapeKind::Sheet(Arc::new(sheet)),
        ));
        let workbook = Workbook {
            sheets: vec![WorkbookSheet {
                name: "請求書".into(),
                state: SheetState::Visible,
                slide: Some(slide_id),
                hidden: None,
                other: false,
            }],
            ..Workbook::default()
        };
        let mut pres = Presentation::default();
        pres.slides = vec![slide];
        pres.workbook = Some(Arc::new(workbook));
        (pres, slide_id)
    }

    #[test]
    fn parses_profile_markdown_and_resolves_aliases_and_composites() {
        let (path, md) = starter_profile_markdown("organization");
        let prof = parse_profile_markdown(path, md).expect("organization profile parses");
        assert_eq!(prof.scope, "organization");
        assert_eq!(
            prof.resolve_value("company_name").unwrap().1,
            Value::String("株式会社サンプル".into())
        );
        assert_eq!(
            prof.resolve_value("address.full").unwrap().1,
            Value::String("東京都千代田区千代田1-1-1".into())
        );

        let (me_path, me_md) = starter_profile_markdown("person");
        let me = parse_profile_markdown(me_path, me_md).expect("person profile parses");
        assert_eq!(
            me.resolve_value("birthdate").unwrap().1,
            Value::String("1990-05-15".into())
        );
    }

    #[test]
    fn japanese_era_and_iso_date_roundtrip() {
        let d = parse_date_ymd("2026-11-25").unwrap();
        assert_eq!(format_date_ja(d), "令和8年11月25日");
        assert_eq!(parse_date_ymd("令和8年11月25日"), Some(d));
        assert_eq!(parse_date_ymd("R8.11.25"), Some(d));
        let d1 = parse_date_ymd("2019-05-01").unwrap();
        assert_eq!(format_date_ja(d1), "令和元年5月1日");
        assert_eq!(parse_date_ymd("令和元年5月1日"), Some(d1));
    }

    #[test]
    fn form_mapping_evaluates_merged_split_date_radio_and_vc_priority() {
        let (pres, slide_id) = sample_invoice_presentation();
        let (_, prof_md) = starter_profile_markdown("organization");
        let profile = parse_profile_markdown("profiles/company.md", prof_md).unwrap();

        let mapping_md = r#"---
type: form-mapping
title: テスト請求書マッピング
x-sorane-form-mapping:
  version: 1
  form_id: test-invoice
  format: xlsx
  match:
    sheet: 請求書
    anchors:
      - cell: A1
        contains: 請求書
  fields:
    - id: company
      label: 会社名
      target: { cell: B3 }
      sources:
        - any: organization.name
      required: true
    - id: invoice_no
      label: 登録番号
      target: { cell: B4 }
      sources:
        - any: organization.invoice_number
    - id: postal
      label: 郵便番号
      target:
        split_digits: { cells: [B5, D5], lengths: [3, 4] }
      sources:
        - profile: address.postal_code
    - id: issue_date
      label: 請求日
      target:
        split_date_era: { era: B6, year: C6, month: D6, day: E6 }
      value_type: date
      default: today
    - id: tax_type
      label: 課税区分
      target:
        radio_mark:
          checked: "☑"
          unchecked: "□"
          options:
            taxable: B7
            exempt: D7
      value_type: enum
      default: taxable
---
"#;
        let mapping = parse_form_mapping_markdown("forms/test-invoice.form-map.md", mapping_md)
            .expect("form mapping parses");
        assert!(score_form_mapping_match(&mapping, &pres, slide_id, None).is_some());

        // Add a VC credential overriding organization.name to test VC priority & badge!
        let mut vc_claims = BTreeMap::new();
        vc_claims.insert(
            "organization.name".to_string(),
            Value::String("株式会社VC検証済み".into()),
        );
        let cred = VaultCredentialSummary {
            path: "credentials/org.sd-jwt".into(),
            format: "sd-jwt".into(),
            vct_or_doctype: "https://example.org/org".into(),
            issuer: Some("https://gov.example.jp".into()),
            digests_verified: 1,
            digests_total: 1,
            sig_status: "valid".into(),
            claims: vc_claims,
        };

        let eval = evaluate_form_mapping(
            &pres,
            slide_id,
            &mapping,
            &[&profile],
            &[&cred],
            Some("2026-11-25"),
        );
        assert_eq!(eval.ready_count, 5);
        assert_eq!(eval.vc_backed_count, 1);
        assert!(matches!(
            eval.fields[0].provenance,
            Some(FieldProvenance::Credential { .. })
        ));
        assert_eq!(
            eval.fields[0].proposed_display.as_deref(),
            Some("株式会社VC検証済み")
        );

        let (cmd, count) =
            build_form_apply_command(&pres, slide_id, &mapping, &eval, None, false).unwrap();
        assert_eq!(count, 5);

        let mut doc = Document::new(pres);
        doc.execute(vec![cmd]).expect("1-undo apply succeeds");
        let sheet = sheet_of_slide(&doc.presentation().slides[0]).unwrap();
        // Merged anchor B3
        assert_eq!(
            sheet.cell(2, 1).unwrap().value,
            CellValue::Text("株式会社VC検証済み".into())
        );
        // Leading-zero split postal code B5="100", D5="0001"
        assert_eq!(sheet.cell(4, 1).unwrap().value, CellValue::Text("100".into()));
        assert_eq!(sheet.cell(4, 3).unwrap().value, CellValue::Text("0001".into()));
        // Split Japanese era date B6="令和", C6=8, D6=11, E6=25
        assert_eq!(sheet.cell(5, 1).unwrap().value, CellValue::Text("令和".into()));
        assert_eq!(sheet.cell(5, 2).unwrap().value, CellValue::Number(8.0));
        assert_eq!(sheet.cell(5, 3).unwrap().value, CellValue::Number(11.0));
        assert_eq!(sheet.cell(5, 4).unwrap().value, CellValue::Number(25.0));
        // Radio mark B7="☑", D7="□"
        assert_eq!(sheet.cell(6, 1).unwrap().value, CellValue::Text("☑".into()));
        assert_eq!(sheet.cell(6, 3).unwrap().value, CellValue::Text("□".into()));
    }

    #[test]
    fn heuristic_inference_generates_parseable_okf_form_mapping() {
        let (pres, slide_id) = sample_invoice_presentation();
        let (draft, md) = infer_draft_form_mapping_from_sheet(&pres, slide_id)
            .expect("draft mapping inferred from unlabeled sheet");
        assert!(draft.fields.iter().any(|f| f.id == "organization.name"));
        assert!(draft.fields.iter().any(|f| f.id == "organization.invoice_number"));
        let reparsed =
            parse_form_mapping_markdown(&draft.path, &md).expect("rendered OKF markdown reparses");
        assert_eq!(reparsed.fields.len(), draft.fields.len());
    }

    #[test]
    fn presentation_request_and_submission_receipt_roundtrip() {
        let (path, md) = starter_presentation_request_markdown();
        let req_note = parse_presentation_request_markdown(path, md)
            .expect("starter presentation-request parses");
        assert_eq!(req_note.request_id, "sample-verifier");
        assert_eq!(req_note.client_id, "https://verifier.example.org");
        assert_eq!(
            req_note.response_uri.as_deref(),
            Some("https://verifier.example.org/oid4vp/direct_post")
        );
        assert_eq!(req_note.dcql_credential_id.as_deref(), Some("credential_1"));
        assert_eq!(
            req_note.requested_claims,
            vec!["given_name", "family_name", "address.region"]
        );

        let oid = req_note.to_oid4vp_request("nonce-fresh-123");
        assert_eq!(oid.nonce, "nonce-fresh-123");
        assert_eq!(oid.dcql_credential_id.as_deref(), Some("credential_1"));

        let receipt = SubmissionReceipt {
            submitted_at: "2026-11-26T01:23:45Z".into(),
            recipient: req_note.client_id.clone(),
            response_uri: req_note.response_uri.clone(),
            mode: "oid4vp_direct_post".into(),
            http_status: Some(200),
            form_id: Some("sample-verifier".into()),
            form_mapping_path: Some("forms/invoice.form-map.md".into()),
            request_path: Some(req_note.path.clone()),
            source_paths: vec![
                "profiles/me.md".into(),
                "credentials/pid.sd-jwt".into(),
            ],
            artifact_path: None,
            disclosed_fields: vec!["given_name".into(), "address.region".into()],
            payload_sha256: sha256_hex(b"vp_token=demo"),
            key_storage: "local_app_data".into(),
            holder_key_thumbprint: Some("abc123def456".into()),
            notes: Some("ローカル検証完了".into()),
        };
        let rel = submission_receipt_relative_path("2026-11-26", "Sample Verifier!");
        assert_eq!(rel, "submissions/2026-11-26-sample-verifier.md");
        let rendered = render_submission_receipt_markdown(&receipt);
        let fm = crate::okf::frontmatter_map(&rendered).unwrap().unwrap();
        assert_eq!(fm["type"], Value::String("submission-receipt".into()));
        assert_eq!(fm["mode"], Value::String("oid4vp_direct_post".into()));
        assert!(rendered.contains("[[profiles/me]]"));
        assert!(rendered.contains("[[credentials/pid.sd-jwt]]"));
        assert!(rendered.contains("[[forms/invoice.form-map]]"));
        assert!(rendered.contains("[[requests/sample-verifier.req]]"));
    }

    #[test]
    fn flow_document_docx_and_markdown_form_assist_roundtrip() {
        use ibis_doc_model::flow::{
            Block, ControlInfo, FlowDocument, FlowParagraph, FlowRun, FlowRunKind, FlowRunStyle,
            FormFieldData, FormFieldKind,
        };

        let (_, org_md) = starter_profile_markdown("organization");
        let profile = parse_profile_markdown("profiles/company.md", org_md).unwrap();

        // 1. Markdown `{{form:...}}` imported via `markdown::read_markdown`
        let md_src = "# 申請書\n\n会社名: {{form:organization.name}}\n\n登録番号: {{form:organization.invoice_number}}\n";
        let md_pres = crate::markdown::read_markdown(md_src.as_bytes()).expect("read markdown");
        let md_flow = md_pres.flow.as_ref().expect("markdown has FlowDocument");
        let (md_targets, md_eval) =
            evaluate_flow_form(md_flow, &[&profile], &[], Some("2026-11-26"));
        assert_eq!(md_targets.len(), 2);
        assert_eq!(md_eval.ready_count, 2);
        let (md_cmds, md_count) =
            build_flow_form_apply_commands(md_flow, &md_targets, &md_eval, false, true).unwrap();
        assert_eq!(md_count, 2);
        let mut md_doc = Document::new(md_pres);
        md_doc.execute(md_cmds).expect("apply markdown form");
        let exported_md =
            crate::markdown::export_markdown(md_doc.presentation().flow.as_ref().unwrap(), &[]);
        assert!(exported_md.contains("株式会社サンプル"));
        assert!(exported_md.contains("T1234567890123"));
        assert!(!exported_md.contains("{{form:"));

        // 2. DOCX content controls (PlainText + DatePicker + Legacy FORMTEXT) with 1-Undo apply
        let mut p1 = FlowParagraph {
            id: 1,
            ..Default::default()
        };
        let mut r_company = FlowRun::new(
            FlowRunKind::PlainText {
                text: "クリックして入力".into(),
                placeholder: true,
            },
            FlowRunStyle::default(),
        );
        r_company.control = Some(Box::new(ControlInfo {
            title: "会社名".into(),
            tag: "ibis.form.organization.name".into(),
            ..Default::default()
        }));
        p1.runs.push(r_company);

        let mut r_date = FlowRun::new(
            FlowRunKind::DatePicker {
                full_date: String::new(),
                date_format: "ggge年M月d日".into(),
                text: String::new(),
            },
            FlowRunStyle::default(),
        );
        r_date.control = Some(Box::new(ControlInfo {
            title: "作成日".into(),
            tag: "document.date".into(),
            ..Default::default()
        }));
        p1.runs.push(r_date);

        let r_legacy = FlowRun::new(
            FlowRunKind::FormField(FormFieldData {
                name: "postal_code".into(),
                enabled: true,
                calc_on_exit: false,
                kind: FormFieldKind::TextInput {
                    value: String::new(),
                    default: String::new(),
                    max_length: 10,
                },
            }),
            FlowRunStyle::default(),
        );
        p1.runs.push(r_legacy);

        let mut flow = FlowDocument::default();
        flow.body.push(Block::Paragraph(p1));
        let mut docx_pres = Presentation::default();
        docx_pres.flow = Some(Arc::new(flow));

        let docx_flow = docx_pres.flow.as_ref().unwrap();
        let (targets, eval) =
            evaluate_flow_form(docx_flow, &[&profile], &[], Some("2026-11-26"));
        assert_eq!(targets.len(), 3);
        assert_eq!(eval.ready_count, 3);
        let (cmds, applied) =
            build_flow_form_apply_commands(docx_flow, &targets, &eval, false, false).unwrap();
        assert_eq!(applied, 3);

        let mut docx_doc = Document::new(docx_pres);
        docx_doc.execute(cmds).expect("apply docx controls");
        let after = docx_doc.presentation().flow.as_ref().unwrap();
        let Block::Paragraph(para) = &after.body[0] else {
            panic!("expected paragraph");
        };
        assert!(matches!(
            &para.runs[0].kind,
            FlowRunKind::PlainText { text, placeholder: false } if text == "株式会社サンプル"
        ));
        assert!(matches!(
            &para.runs[1].kind,
            FlowRunKind::DatePicker { text, .. } if text == "令和8年11月26日"
        ));
        assert!(matches!(
            &para.runs[2].kind,
            FlowRunKind::FormField(FormFieldData {
                kind: FormFieldKind::TextInput { value, .. },
                ..
            }) if value == "100-0001"
        ));
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn pdf_acroform_form_assist_roundtrip() {
        fn build_minimal_acroform_pdf() -> Vec<u8> {
            let objects: Vec<&[u8]> = vec![
                b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R] >> >>\nendobj\n",
                b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
                b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R 5 0 R] >>\nendobj\n",
                b"4 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Tx /T (company_name) /Rect [50 700 300 720] /P 3 0 R /MaxLen 40 >>\nendobj\n",
                b"5 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Tx /T (postal_code) /Rect [50 660 200 680] /P 3 0 R /MaxLen 10 >>\nendobj\n",
            ];
            let mut out = Vec::from(&b"%PDF-1.7\n"[..]);
            let mut offsets = Vec::new();
            for obj in &objects {
                offsets.push(out.len());
                out.extend_from_slice(obj);
            }
            let xref_pos = out.len();
            out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
            for off in offsets {
                out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
            }
            out.extend_from_slice(
                format!(
                    "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
                    objects.len() + 1
                )
                .as_bytes(),
            );
            out
        }

        let pdf_bytes = build_minimal_acroform_pdf();
        let (_, org_md) = starter_profile_markdown("organization");
        let profile = parse_profile_markdown("profiles/company.md", org_md).unwrap();

        let (is_pdfa2b, targets) = inspect_pdf_form_fields(pdf_bytes.clone()).expect("inspect pdf");
        assert!(!is_pdfa2b);
        assert_eq!(targets.len(), 2);

        let eval = evaluate_pdf_form(&targets, &[&profile], &[], Some("2026-11-26"));
        assert_eq!(eval.ready_count, 2);

        let mut fill_map = BTreeMap::new();
        for field in &eval.fields {
            if let Some(val) = &field.proposed_display {
                fill_map.insert(field.id.clone(), val.clone());
            }
        }
        let (filled_bytes, applied) = fill_pdf_form_values(pdf_bytes, &fill_map).expect("fill pdf");
        assert_eq!(applied, 2);

        // Re-inspect filled PDF and confirm both UTF-16BE Japanese & ASCII values persisted!
        let (_, after_targets) = inspect_pdf_form_fields(filled_bytes).expect("re-inspect pdf");
        let after_map: BTreeMap<String, String> = after_targets
            .into_iter()
            .map(|t| (t.name, t.current_value))
            .collect();
        assert_eq!(
            after_map.get("company_name").map(String::as_str),
            Some("株式会社サンプル")
        );
        assert_eq!(
            after_map.get("postal_code").map(String::as_str),
            Some("100-0001")
        );
    }
}



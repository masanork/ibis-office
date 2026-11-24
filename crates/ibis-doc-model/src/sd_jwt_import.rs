//! Parsers, verifiers, and worksheet importers for existing `.sd-jwt` (RFC 9901)
//! and ISO/IEC 18013-5 `.mdoc` (CBOR / COSE_Sign1) credentials.

use std::collections::{BTreeMap, BTreeSet};

use crate::command::Command;
use crate::model::{Presentation, SlideId};
use crate::sd_jwt::{Es256KeyPair, base64url_decode, base64url_encode, sha256, verify_es256};
use crate::sd_jwt_binding::{
    ClaimValue, abs_cell_ref, clear_sd_jwt_bindings_commands, slide_sheet_formula_prefix,
};
use crate::sheet::{CellStyle, CellValue};
use crate::sheet_command::{CellInput, PutCell, SheetCommand};

/// Source credential format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParsedCredentialFormat {
    SdJwt,
    Mdoc,
}

/// A single claim extracted from an `.sd-jwt` or `.mdoc` credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedClaim {
    pub mdoc_element: Option<(String, String)>,
    /// SD-JWT object member names, before joining them for worksheet display.
    pub sd_jwt_path: Option<Vec<String>>,
    pub path: String,
    pub value: ClaimValue,
    pub selective: bool,
    pub disclose: bool,
    pub digest_verified: Option<bool>,
}

/// Unified parsed credential ready for verification inspection and worksheet import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCredential {
    pub format: ParsedCredentialFormat,
    pub vct_or_doctype: String,
    pub issuer: Option<String>,
    pub signature_valid: Option<bool>,
    pub kb_sd_hash_valid: Option<bool>,
    pub digests_verified: usize,
    pub digests_total: usize,
    pub claims: Vec<ParsedClaim>,
}

impl ParsedCredential {
    /// Generates undoable worksheet commands to display and edit this credential on `slide_id`.
    /// Scaffolds `claim.vct`, `claim.iss` (when present), `sd.*` / `claim.*`, and `disclose.*` checkboxes.
    pub fn build_sheet_commands(
        &self,
        presentation: &Presentation,
        slide_id: SlideId,
    ) -> Vec<Command> {
        let date1904 = presentation.workbook.as_ref().is_some_and(|w| w.date1904);
        let sheet_prefix = slide_sheet_formula_prefix(presentation, slide_id);

        let mut header_style = CellStyle::default();
        header_style.bold = true;

        let mut date_style = CellStyle::default();
        date_style.number_format = "yyyy-mm-dd".to_string();

        let mut text_style = CellStyle::default();
        text_style.number_format = "@".to_string();

        let mut checkbox_style = CellStyle::default();
        checkbox_style.checkbox = true;

        let mut cells: Vec<PutCell> = Vec::new();
        let mut name_cmds = clear_sd_jwt_bindings_commands(presentation, slide_id);

        let put_text = |cells: &mut Vec<PutCell>, r: u32, c: u32, s: &str, style: Option<CellStyle>| {
            cells.push(PutCell {
                row: r,
                column: c,
                input: CellInput::Value(CellValue::Text(s.to_string())),
                style,
            });
        };

        let format_tag = match self.format {
            ParsedCredentialFormat::SdJwt => "SD-JWT",
            ParsedCredentialFormat::Mdoc => "mDOC (ISO 18013-5)",
        };
        put_text(
            &mut cells,
            0,
            0,
            &format!("Claim ({format_tag})"),
            Some(header_style.clone()),
        );
        put_text(&mut cells, 0, 1, "Value", Some(header_style.clone()));
        put_text(&mut cells, 0, 2, "Disclose", Some(header_style.clone()));
        put_text(&mut cells, 0, 3, "Verification", Some(header_style.clone()));

        let mut row: u32 = 1;

        // vct / docType
        put_text(&mut cells, row, 0, "vct", None);
        put_text(
            &mut cells,
            row,
            1,
            &self.vct_or_doctype,
            Some(text_style.clone()),
        );
        put_text(&mut cells, row, 2, "Always (claim.vct)", None);
        let sig_status = match self.signature_valid {
            Some(true) => "Signature OK",
            Some(false) => "Signature INVALID",
            None => "Signature unchecked",
        };
        put_text(&mut cells, row, 3, sig_status, None);
        name_cmds.push(Command::Sheet(SheetCommand::SetName {
            name: "claim.vct".into(),
            sheet: None,
            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, row))),
        }));
        row += 1;

        if let Some(iss) = &self.issuer {
            put_text(&mut cells, row, 0, "iss", None);
            put_text(&mut cells, row, 1, iss, Some(text_style.clone()));
            put_text(&mut cells, row, 2, "Always (claim.iss)", None);
            put_text(&mut cells, row, 3, "Issuer", None);
            name_cmds.push(Command::Sheet(SheetCommand::SetName {
                name: "claim.iss".into(),
                sheet: None,
                formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, row))),
            }));
            row += 1;
        }

        for claim in &self.claims {
            if claim.path == "vct" || claim.path == "iss" {
                continue;
            }
            let verify_text = match claim.digest_verified {
                Some(true) => "Digest OK",
                Some(false) => "Digest MISMATCH",
                None => "Plain claim",
            };
            let prefix = if claim.selective { "sd" } else { "claim" };

            match &claim.value {
                ClaimValue::Array(items) if items.iter().all(|v| v.as_object().is_some()) && !items.is_empty() => {
                    // 2D object table
                    let mut col_keys: Vec<String> = Vec::new();
                    for item in items {
                        if let Some(obj) = item.as_object() {
                            for k in obj.keys() {
                                if !col_keys.contains(k) {
                                    col_keys.push(k.clone());
                                }
                            }
                        }
                    }
                    put_text(&mut cells, row, 0, &claim.path, Some(header_style.clone()));
                    if claim.selective {
                        cells.push(PutCell {
                            row,
                            column: 1,
                            input: CellInput::Value(CellValue::Bool(claim.disclose)),
                            style: Some(checkbox_style.clone()),
                        });
                        name_cmds.push(Command::Sheet(SheetCommand::SetName {
                            name: format!("disclose.{}", claim.path),
                            sheet: None,
                            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, row))),
                        }));
                    } else {
                        put_text(&mut cells, row, 1, "Always", None);
                    }
                    put_text(&mut cells, row, 2, verify_text, None);
                    let header_row = row + 1;
                    for (ci, key) in col_keys.iter().enumerate() {
                        put_text(
                            &mut cells,
                            header_row,
                            ci as u32,
                            key,
                            Some(header_style.clone()),
                        );
                    }
                    let mut data_row = header_row + 1;
                    for item in items {
                        if let Some(obj) = item.as_object() {
                            for (ci, key) in col_keys.iter().enumerate() {
                                if let Some(val) = obj.get(key) {
                                    let (ci_val, st) =
                                        claim_to_cell_input(val, date1904, &date_style, &text_style);
                                    cells.push(PutCell {
                                        row: data_row,
                                        column: ci as u32,
                                        input: ci_val,
                                        style: st,
                                    });
                                }
                            }
                            data_row += 1;
                        }
                    }
                    let last_col = (col_keys.len().saturating_sub(1)) as u32;
                    let last_row = data_row.saturating_sub(1);
                    name_cmds.push(Command::Sheet(SheetCommand::SetName {
                        name: format!("{prefix}.{}", claim.path),
                        sheet: None,
                        formula: Some(format!(
                            "{sheet_prefix}{}:{}",
                            abs_cell_ref(0, header_row),
                            abs_cell_ref(last_col, last_row)
                        )),
                    }));
                    row = data_row;
                }
                ClaimValue::Array(items) if !items.is_empty() => {
                    // 1D primitive array
                    put_text(&mut cells, row, 0, &claim.path, None);
                    for (i, item) in items.iter().enumerate() {
                        let (ci_val, st) =
                            claim_to_cell_input(item, date1904, &date_style, &text_style);
                        cells.push(PutCell {
                            row,
                            column: 1 + i as u32,
                            input: ci_val,
                            style: st,
                        });
                    }
                    let last_col = 1 + (items.len().saturating_sub(1)) as u32;
                    name_cmds.push(Command::Sheet(SheetCommand::SetName {
                        name: format!("{prefix}.{}", claim.path),
                        sheet: None,
                        formula: Some(format!(
                            "{sheet_prefix}{}:{}",
                            abs_cell_ref(1, row),
                            abs_cell_ref(last_col, row)
                        )),
                    }));
                    if claim.selective {
                        let chk_col = last_col + 1;
                        cells.push(PutCell {
                            row,
                            column: chk_col,
                            input: CellInput::Value(CellValue::Bool(claim.disclose)),
                            style: Some(checkbox_style.clone()),
                        });
                        name_cmds.push(Command::Sheet(SheetCommand::SetName {
                            name: format!("disclose.{}", claim.path),
                            sheet: None,
                            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(chk_col, row))),
                        }));
                        put_text(&mut cells, row, chk_col + 1, verify_text, None);
                    } else {
                        put_text(&mut cells, row, last_col + 1, verify_text, None);
                    }
                    row += 1;
                }
                _ => {
                    put_text(&mut cells, row, 0, &claim.path, None);
                    let (ci_val, st) =
                        claim_to_cell_input(&claim.value, date1904, &date_style, &text_style);
                    cells.push(PutCell {
                        row,
                        column: 1,
                        input: ci_val,
                        style: st,
                    });
                    name_cmds.push(Command::Sheet(SheetCommand::SetName {
                        name: format!("{prefix}.{}", claim.path),
                        sheet: None,
                        formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, row))),
                    }));
                    if claim.selective {
                        cells.push(PutCell {
                            row,
                            column: 2,
                            input: CellInput::Value(CellValue::Bool(claim.disclose)),
                            style: Some(checkbox_style.clone()),
                        });
                        name_cmds.push(Command::Sheet(SheetCommand::SetName {
                            name: format!("disclose.{}", claim.path),
                            sheet: None,
                            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(2, row))),
                        }));
                    } else {
                        put_text(&mut cells, row, 2, &format!("Always (claim.{})", claim.path), None);
                    }
                    put_text(&mut cells, row, 3, verify_text, None);
                    row += 1;
                }
            }
        }

        let mut cmds = vec![Command::Sheet(SheetCommand::PasteCells {
            sheet: slide_id,
            cells,
        })];
        cmds.extend(name_cmds);
        cmds
    }
}

fn claim_to_cell_input(
    value: &ClaimValue,
    date1904: bool,
    date_style: &CellStyle,
    text_style: &CellStyle,
) -> (CellInput, Option<CellStyle>) {
    match value {
        ClaimValue::Bool(b) => (CellInput::Value(CellValue::Bool(*b)), None),
        ClaimValue::Int(n) => (CellInput::Value(CellValue::Number(*n as f64)), None),
        ClaimValue::Float(f) => (CellInput::Value(CellValue::Number(*f)), None),
        ClaimValue::String(s) => {
            if s.len() == 10
                && let Some((y, m, d)) = parse_ymd(s)
            {
                let serial = ibis_calc::eval::convert::serial_from_date(y, m, d, date1904) as f64;
                return (
                    CellInput::Value(CellValue::Number(serial)),
                    Some(date_style.clone()),
                );
            }
            (
                CellInput::Value(CellValue::Text(s.clone())),
                Some(text_style.clone()),
            )
        }
        ClaimValue::Null => (CellInput::Value(CellValue::Empty), None),
        other => (
            CellInput::Value(CellValue::Text(other.to_json())),
            Some(text_style.clone()),
        ),
    }
}

fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let mut parts = s.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

fn parse_p256_public_coords_from_jwk(jwk_json: &str) -> Option<([u8; 32], [u8; 32])> {
    let val = ClaimValue::parse_json(jwk_json).ok()?;
    let obj = val.as_object()?;
    let x_b64 = obj.get("x").and_then(ClaimValue::as_str)?;
    let y_b64 = obj.get("y").and_then(ClaimValue::as_str)?;
    let x_vec = base64url_decode(x_b64)?;
    let y_vec = base64url_decode(y_b64)?;
    if x_vec.len() != 32 || y_vec.len() != 32 {
        return None;
    }
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    x.copy_from_slice(&x_vec);
    y.copy_from_slice(&y_vec);
    Some((x, y))
}

// ============================================================================
// RFC 9901 SD-JWT Parser & Verifier
// ============================================================================

/// Parses and verifies an RFC 9901 SD-JWT compact serialization (`<jwt>~<disc1>~...~[kb-jwt]`).
pub fn parse_sd_jwt(
    compact: &str,
    issuer_jwk_json: Option<&str>,
) -> Result<ParsedCredential, String> {
    let trimmed = compact.trim();
    if trimmed.is_empty() {
        return Err("empty SD-JWT input".to_string());
    }
    let parts: Vec<&str> = trimmed.split('~').collect();
    if parts.is_empty() || parts[0].is_empty() {
        return Err("missing issuer-signed JWT".to_string());
    }
    let issuer_jwt = parts[0];
    let jwt_segs: Vec<&str> = issuer_jwt.split('.').collect();
    if jwt_segs.len() != 3 {
        return Err("issuer JWT must have 3 dot-separated segments".to_string());
    }

    let header_bytes = base64url_decode(jwt_segs[0])
        .ok_or_else(|| "invalid base64url in JWT header".to_string())?;
    let payload_bytes = base64url_decode(jwt_segs[1])
        .ok_or_else(|| "invalid base64url in JWT payload".to_string())?;
    let sig_bytes = base64url_decode(jwt_segs[2])
        .ok_or_else(|| "invalid base64url in JWT signature".to_string())?;

    let header_str =
        std::str::from_utf8(&header_bytes).map_err(|_| "JWT header is not UTF-8".to_string())?;
    let payload_str =
        std::str::from_utf8(&payload_bytes).map_err(|_| "JWT payload is not UTF-8".to_string())?;

    let header_val = ClaimValue::parse_json(header_str)?;
    let payload_val = ClaimValue::parse_json(payload_str)?;
    let payload_obj = payload_val
        .as_object()
        .ok_or_else(|| "JWT payload must be a JSON object".to_string())?;

    // Optional ES256 signature verification
    let signing_input = format!("{}.{}", jwt_segs[0], jwt_segs[1]);
    let pub_coords = issuer_jwk_json
        .and_then(parse_p256_public_coords_from_jwk)
        .or_else(|| {
            header_val
                .as_object()
                .and_then(|h| h.get("jwk"))
                .map(ClaimValue::to_json)
                .and_then(|j| parse_p256_public_coords_from_jwk(&j))
        });

    let signature_valid = if let Some((px, py)) = pub_coords {
        if sig_bytes.len() == 64 {
            let mut sig = [0u8; 64];
            sig.copy_from_slice(&sig_bytes);
            Some(verify_es256(&px, &py, signing_input.as_bytes(), &sig))
        } else {
            Some(false)
        }
    } else if sig_bytes.len() == 64 {
        // Check against built-in demo issuer key if no explicit JWK was provided
        let demo = Es256KeyPair::demo_issuer_key();
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&sig_bytes);
        if verify_es256(&demo.x, &demo.y, signing_input.as_bytes(), &sig) {
            Some(true)
        } else {
            None
        }
    } else {
        None
    };

    // Parse disclosures and optional trailing KB-JWT
    let mut disclosures_by_digest: BTreeMap<String, (Option<String>, ClaimValue)> = BTreeMap::new();
    let mut kb_sd_hash_valid: Option<bool> = None;

    let trailing = &parts[1..];
    for (idx, part) in trailing.iter().enumerate() {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        // Last non-empty part containing '.' is a Key Binding JWT
        if idx == trailing.len() - 1 && p.contains('.') {
            let kb_segs: Vec<&str> = p.split('.').collect();
            if kb_segs.len() == 3
                && let Some(kb_payload_bytes) = base64url_decode(kb_segs[1])
                && let Ok(kb_payload_str) = std::str::from_utf8(&kb_payload_bytes)
                && let Ok(kb_val) = ClaimValue::parse_json(kb_payload_str)
                && let Some(sd_hash) = kb_val
                    .as_object()
                    .and_then(|m| m.get("sd_hash"))
                    .and_then(ClaimValue::as_str)
            {
                let without_kb_end = trimmed.rfind('~').unwrap_or(trimmed.len());
                let sd_jwt_with_tilde = &trimmed[..=without_kb_end];
                let expected_hash = base64url_encode(&sha256(sd_jwt_with_tilde.as_bytes()));
                kb_sd_hash_valid = Some(sd_hash == expected_hash);
            }
            continue;
        }

        let disc_bytes = base64url_decode(p)
            .ok_or_else(|| format!("invalid base64url in disclosure #{idx}"))?;
        let disc_str = std::str::from_utf8(&disc_bytes)
            .map_err(|_| format!("disclosure #{idx} is not UTF-8"))?;
        let disc_val = ClaimValue::parse_json(disc_str)?;
        let arr = disc_val
            .as_array()
            .ok_or_else(|| format!("disclosure #{idx} must be a JSON array"))?;
        let digest = base64url_encode(&sha256(p.as_bytes()));
        match arr.len() {
            3 => {
                let name = arr[1]
                    .as_str()
                    .ok_or_else(|| format!("disclosure #{idx} claim name must be a string"))?
                    .to_string();
                disclosures_by_digest.insert(digest, (Some(name), arr[2].clone()));
            }
            2 => {
                disclosures_by_digest.insert(digest, (None, arr[1].clone()));
            }
            _ => {
                return Err(format!(
                    "disclosure #{idx} must have 2 or 3 elements, got {}",
                    arr.len()
                ));
            }
        }
    }

    let vct_or_doctype = payload_obj
        .get("vct")
        .and_then(ClaimValue::as_str)
        .unwrap_or("urn:example:sd-jwt:1")
        .to_string();
    let issuer = payload_obj
        .get("iss")
        .and_then(ClaimValue::as_str)
        .map(str::to_string);

    let mut claims = Vec::new();
    let mut digests_verified = 0usize;
    let mut digests_total = 0usize;

    collect_sd_jwt_object(
        payload_obj,
        &[],
        &mut disclosures_by_digest,
        &mut claims,
        &mut digests_verified,
        &mut digests_total,
    )?;

    Ok(ParsedCredential {
        format: ParsedCredentialFormat::SdJwt,
        vct_or_doctype,
        issuer,
        signature_valid,
        kb_sd_hash_valid,
        digests_verified,
        digests_total,
        claims,
    })
}

fn collect_sd_jwt_object(
    obj: &BTreeMap<String, ClaimValue>,
    prefix: &[String],
    disclosures: &mut BTreeMap<String, (Option<String>, ClaimValue)>,
    out: &mut Vec<ParsedClaim>,
    digests_verified: &mut usize,
    digests_total: &mut usize,
) -> Result<(), String> {
    if prefix.len() >= 64 {
        return Err("SD-JWT claim nesting limit exceeded".into());
    }
    // 1. Plain claims (skip metadata fields at root)
    for (k, v) in obj {
        if k == "_sd" || k == "_sd_alg" {
            continue;
        }
        if prefix.is_empty() && matches!(k.as_str(), "cnf" | "iat" | "exp" | "nbf" | "jti") {
            continue;
        }
        let mut path = prefix.to_vec();
        path.push(k.clone());
        match v {
            ClaimValue::Object(child) if child.contains_key("_sd") || !child.is_empty() => {
                collect_sd_jwt_object(child, &path, disclosures, out, digests_verified, digests_total)?;
            }
            ClaimValue::Array(items) => {
                let resolved = resolve_sd_jwt_array(items, disclosures, digests_verified, digests_total);
                out.push(ParsedClaim {
                    mdoc_element: None,
                    path: path.join("."),
                    sd_jwt_path: Some(path),
                    value: ClaimValue::Array(resolved),
                    selective: false,
                    disclose: true,
                    digest_verified: None,
                });
            }
            other => {
                out.push(ParsedClaim {
                    mdoc_element: None,
                    path: path.join("."),
                    sd_jwt_path: Some(path),
                    value: other.clone(),
                    selective: false,
                    disclose: true,
                    digest_verified: None,
                });
            }
        }
    }

    // 2. Selectively disclosable claims referenced by `_sd`
    if let Some(ClaimValue::Array(sd_digests)) = obj.get("_sd") {
        for d_val in sd_digests {
            let Some(digest) = d_val.as_str() else {
                continue;
            };
            *digests_total += 1;
            if let Some((Some(claim_name), claim_val)) = disclosures.remove(digest) {
                *digests_verified += 1;
                let mut path = prefix.to_vec();
                path.push(claim_name.clone());
                match claim_val {
                    ClaimValue::Object(child) if child.contains_key("_sd") => {
                        collect_sd_jwt_object(
                            &child,
                            &path,
                            disclosures,
                            out,
                            digests_verified,
                            digests_total,
                        )?;
                    }
                    ClaimValue::Array(items) => {
                        let resolved = resolve_sd_jwt_array(
                            &items,
                            disclosures,
                            digests_verified,
                            digests_total,
                        );
                        out.push(ParsedClaim {
                            mdoc_element: None,
                            path: path.join("."),
                            sd_jwt_path: Some(path),
                            value: ClaimValue::Array(resolved),
                            selective: true,
                            disclose: true,
                            digest_verified: Some(true),
                        });
                    }
                    other => {
                        out.push(ParsedClaim {
                            mdoc_element: None,
                            path: path.join("."),
                            sd_jwt_path: Some(path),
                            value: other.clone(),
                            selective: true,
                            disclose: true,
                            digest_verified: Some(true),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

fn resolve_sd_jwt_array(
    items: &[ClaimValue],
    disclosures: &mut BTreeMap<String, (Option<String>, ClaimValue)>,
    digests_verified: &mut usize,
    digests_total: &mut usize,
) -> Vec<ClaimValue> {
    let mut resolved = Vec::new();
    for item in items {
        if let Some(obj) = item.as_object()
            && obj.len() == 1
            && let Some(digest) = obj.get("...").and_then(ClaimValue::as_str)
        {
            *digests_total += 1;
            if let Some((None, elem_val)) = disclosures.remove(digest) {
                *digests_verified += 1;
                resolved.push(elem_val);
            }
        } else {
            resolved.push(item.clone());
        }
    }
    resolved
}

// ============================================================================
// RFC 8949 CBOR Codec + ISO/IEC 18013-5 mDOC Parser & Verifier
// ============================================================================

/// Minimal RFC 8949 CBOR value representation.
#[derive(Clone, Debug, PartialEq)]
pub enum CborValue {
    Unsigned(u64),
    Negative(i128),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<CborValue>),
    Map(Vec<(CborValue, CborValue)>),
    Tag(u64, Box<CborValue>),
    Bool(bool),
    Null,
    Float(f64),
}

impl CborValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s.as_str()),
            Self::Tag(_, inner) => inner.as_text(),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(b) => Some(b.as_slice()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[CborValue]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn map_get(&self, key: &str) -> Option<&CborValue> {
        let Self::Map(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|(k, _)| k.as_text() == Some(key))
            .map(|(_, v)| v)
    }

    pub fn map_get_int(&self, key: i128) -> Option<&CborValue> {
        let Self::Map(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|(k, _)| match k {
                Self::Unsigned(u) => *u as i128 == key,
                Self::Negative(n) => *n == key,
                _ => false,
            })
            .map(|(_, v)| v)
    }

    pub fn to_claim_value(&self) -> ClaimValue {
        match self {
            Self::Unsigned(u) => {
                if let Ok(i) = i64::try_from(*u) {
                    ClaimValue::Int(i)
                } else {
                    ClaimValue::Float(*u as f64)
                }
            }
            Self::Negative(n) => {
                if let Ok(i) = i64::try_from(*n) {
                    ClaimValue::Int(i)
                } else {
                    ClaimValue::Float(*n as f64)
                }
            }
            Self::Bytes(b) => ClaimValue::String(base64url_encode(b)),
            Self::Text(s) => ClaimValue::String(s.clone()),
            Self::Array(items) => {
                ClaimValue::Array(items.iter().map(CborValue::to_claim_value).collect())
            }
            Self::Map(entries) => {
                let mut map = BTreeMap::new();
                for (k, v) in entries {
                    let key_str = match k {
                        CborValue::Text(s) => s.clone(),
                        CborValue::Unsigned(u) => u.to_string(),
                        CborValue::Negative(n) => n.to_string(),
                        other => format!("{other:?}"),
                    };
                    map.insert(key_str, v.to_claim_value());
                }
                ClaimValue::Object(map)
            }
            Self::Tag(_, inner) => inner.to_claim_value(),
            Self::Bool(b) => ClaimValue::Bool(*b),
            Self::Null => ClaimValue::Null,
            Self::Float(f) => ClaimValue::Float(*f),
        }
    }
}

/// Decodes a single RFC 8949 CBOR item from `bytes`.
pub fn decode_cbor(bytes: &[u8]) -> Result<CborValue, String> {
    let mut pos = 0usize;
    decode_cbor_at(bytes, &mut pos, 0)
}

fn read_cbor_arg(bytes: &[u8], pos: &mut usize, ai: u8) -> Result<u64, String> {
    match ai {
        0..=23 => Ok(ai as u64),
        24 => {
            let b = *bytes
                .get(*pos)
                .ok_or_else(|| "unexpected EOF reading 1-byte CBOR arg".to_string())?;
            *pos += 1;
            Ok(b as u64)
        }
        25 => {
            let slice = bytes
                .get(*pos..*pos + 2)
                .ok_or_else(|| "unexpected EOF reading 2-byte CBOR arg".to_string())?;
            *pos += 2;
            Ok(u16::from_be_bytes(slice.try_into().unwrap()) as u64)
        }
        26 => {
            let slice = bytes
                .get(*pos..*pos + 4)
                .ok_or_else(|| "unexpected EOF reading 4-byte CBOR arg".to_string())?;
            *pos += 4;
            Ok(u32::from_be_bytes(slice.try_into().unwrap()) as u64)
        }
        27 => {
            let slice = bytes
                .get(*pos..*pos + 8)
                .ok_or_else(|| "unexpected EOF reading 8-byte CBOR arg".to_string())?;
            *pos += 8;
            Ok(u64::from_be_bytes(slice.try_into().unwrap()))
        }
        _ => Err(format!("unsupported CBOR additional info {ai}")),
    }
}

fn decode_cbor_at(bytes: &[u8], pos: &mut usize, depth: usize) -> Result<CborValue, String> {
    if depth >= 64 {
        return Err("CBOR nesting limit exceeded".into());
    }
    let initial = *bytes
        .get(*pos)
        .ok_or_else(|| "unexpected end of CBOR input".to_string())?;
    *pos += 1;
    let major = initial >> 5;
    let ai = initial & 0x1F;

    match major {
        0 => Ok(CborValue::Unsigned(read_cbor_arg(bytes, pos, ai)?)),
        1 => {
            let arg = read_cbor_arg(bytes, pos, ai)?;
            Ok(CborValue::Negative(-1 - (arg as i128)))
        }
        2 => {
            let len = usize::try_from(read_cbor_arg(bytes, pos, ai)?)
                .map_err(|_| "CBOR length exceeds address space".to_string())?;
            let slice = bytes
                .get(*pos..).and_then(|tail| tail.get(..len))
                .ok_or_else(|| "CBOR byte string exceeds input".to_string())?;
            *pos += len;
            Ok(CborValue::Bytes(slice.to_vec()))
        }
        3 => {
            let len = usize::try_from(read_cbor_arg(bytes, pos, ai)?)
                .map_err(|_| "CBOR length exceeds address space".to_string())?;
            let slice = bytes
                .get(*pos..).and_then(|tail| tail.get(..len))
                .ok_or_else(|| "CBOR text string exceeds input".to_string())?;
            *pos += len;
            let s = std::str::from_utf8(slice)
                .map_err(|_| "invalid UTF-8 in CBOR text string".to_string())?;
            Ok(CborValue::Text(s.to_string()))
        }
        4 => {
            let len = usize::try_from(read_cbor_arg(bytes, pos, ai)?)
                .map_err(|_| "CBOR length exceeds address space".to_string())?;
            let mut items = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                items.push(decode_cbor_at(bytes, pos, depth + 1)?);
            }
            Ok(CborValue::Array(items))
        }
        5 => {
            let len = usize::try_from(read_cbor_arg(bytes, pos, ai)?)
                .map_err(|_| "CBOR length exceeds address space".to_string())?;
            let mut entries = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                let k = decode_cbor_at(bytes, pos, depth + 1)?;
                let v = decode_cbor_at(bytes, pos, depth + 1)?;
                entries.push((k, v));
            }
            Ok(CborValue::Map(entries))
        }
        6 => {
            let tag = read_cbor_arg(bytes, pos, ai)?;
            let inner = decode_cbor_at(bytes, pos, depth + 1)?;
            Ok(CborValue::Tag(tag, Box::new(inner)))
        }
        7 => match ai {
            20 => Ok(CborValue::Bool(false)),
            21 => Ok(CborValue::Bool(true)),
            22 | 23 => Ok(CborValue::Null),
            27 => {
                let slice = bytes
                    .get(*pos..*pos + 8)
                    .ok_or_else(|| "unexpected EOF reading CBOR f64".to_string())?;
                *pos += 8;
                let f = f64::from_be_bytes(slice.try_into().unwrap());
                Ok(CborValue::Float(f))
            }
            _ => Err(format!("unsupported CBOR simple/float value ai={ai}")),
        },
        _ => unreachable!(),
    }
}

/// Encodes a `CborValue` into canonical RFC 8949 CBOR bytes.
pub fn encode_cbor(val: &CborValue) -> Vec<u8> {
    let mut out = Vec::new();
    encode_cbor_into(val, &mut out);
    out
}

fn write_cbor_head(out: &mut Vec<u8>, major: u8, arg: u64) {
    let m = major << 5;
    if arg <= 23 {
        out.push(m | (arg as u8));
    } else if arg <= u8::MAX as u64 {
        out.push(m | 24);
        out.push(arg as u8);
    } else if arg <= u16::MAX as u64 {
        out.push(m | 25);
        out.extend_from_slice(&(arg as u16).to_be_bytes());
    } else if arg <= u32::MAX as u64 {
        out.push(m | 26);
        out.extend_from_slice(&(arg as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&arg.to_be_bytes());
    }
}

fn encode_cbor_into(val: &CborValue, out: &mut Vec<u8>) {
    match val {
        CborValue::Unsigned(u) => write_cbor_head(out, 0, *u),
        CborValue::Negative(n) => {
            let arg = (-1 - *n) as u64;
            write_cbor_head(out, 1, arg);
        }
        CborValue::Bytes(b) => {
            write_cbor_head(out, 2, b.len() as u64);
            out.extend_from_slice(b);
        }
        CborValue::Text(s) => {
            write_cbor_head(out, 3, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        CborValue::Array(items) => {
            write_cbor_head(out, 4, items.len() as u64);
            for item in items {
                encode_cbor_into(item, out);
            }
        }
        CborValue::Map(entries) => {
            write_cbor_head(out, 5, entries.len() as u64);
            for (k, v) in entries {
                encode_cbor_into(k, out);
                encode_cbor_into(v, out);
            }
        }
        CborValue::Tag(tag, inner) => {
            write_cbor_head(out, 6, *tag);
            encode_cbor_into(inner, out);
        }
        CborValue::Bool(false) => out.push(0xF4),
        CborValue::Bool(true) => out.push(0xF5),
        CborValue::Null => out.push(0xF6),
        CborValue::Float(f) => {
            out.push(0xFB);
            out.extend_from_slice(&f.to_be_bytes());
        }
    }
}

/// Parses an ISO/IEC 18013-5 `.mdoc` credential from hex or base64/base64url text.
pub fn parse_mdoc_text(
    input: &str,
    issuer_jwk_json: Option<&str>,
) -> Result<ParsedCredential, String> {
    parse_mdoc_bytes(&decode_mdoc_text(input)?, issuer_jwk_json)
}

/// Reads a single mdoc or single-document DeviceResponse for diagnostic comparisons.
pub fn parse_single_mdoc_text(
    input: &str,
    issuer_jwk_json: Option<&str>,
) -> Result<ParsedCredential, String> {
    parse_single_mdoc_bytes(&decode_mdoc_text(input)?, issuer_jwk_json)
}

/// Reads one binary mdoc without silently ignoring further presented documents.
pub fn parse_single_mdoc_bytes(
    bytes: &[u8],
    issuer_jwk_json: Option<&str>,
) -> Result<ParsedCredential, String> {
    let root = decode_cbor(bytes)?;
    if let Some(documents) = root.map_get("documents")
        && !documents.as_array().is_some_and(|docs| docs.len() == 1)
    {
        return Err("expected exactly one mDOC document".into());
    }
    let document = root.map_get("documents").and_then(CborValue::as_array)
        .map_or(&root, |docs| &docs[0]);
    if document.map_get("docType").and_then(CborValue::as_text).is_none() {
        return Err("missing mDOC docType".into());
    }
    parse_mdoc_document(&root, issuer_jwk_json)
}

fn decode_mdoc_text(input: &str) -> Result<Vec<u8>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty mDOC input".to_string());
    }
    if trimmed.bytes().all(|b| b.is_ascii_hexdigit() || b.is_ascii_whitespace()) {
        let hex_clean: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        decode_hex(&hex_clean).ok_or_else(|| "invalid hex in mDOC input".to_string())
    } else {
        base64url_decode(trimmed)
            .or_else(|| base64_std_decode(trimmed))
            .ok_or_else(|| "mDOC input must be hex or base64/base64url".to_string())
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16)? as u8;
        let lo = (pair[1] as char).to_digit(16)? as u8;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn base64_std_decode(input: &str) -> Option<Vec<u8>> {
    let normalized = input
        .trim()
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_string();
    base64url_decode(&normalized)
}

/// Parses and verifies an ISO/IEC 18013-5 `.mdoc` (`DeviceResponse` or `Document`) from raw CBOR bytes.
pub fn parse_mdoc_bytes(
    cbor_bytes: &[u8],
    issuer_jwk_json: Option<&str>,
) -> Result<ParsedCredential, String> {
    parse_mdoc_document(&decode_cbor(cbor_bytes)?, issuer_jwk_json)
}

fn parse_mdoc_document(
    root: &CborValue,
    issuer_jwk_json: Option<&str>,
) -> Result<ParsedCredential, String> {
    let doc = if let Some(docs) = root.map_get("documents").and_then(CborValue::as_array) {
        docs.first()
            .ok_or_else(|| "mDOC DeviceResponse contains no documents".to_string())?
    } else {
        root
    };

    let doc_type = doc
        .map_get("docType")
        .and_then(CborValue::as_text)
        .unwrap_or("org.iso.18013.5.1.mDL")
        .to_string();

    let issuer_signed = doc
        .map_get("issuerSigned")
        .ok_or_else(|| "missing issuerSigned in mDOC document".to_string())?;

    // Extract MSO valueDigests and verify COSE_Sign1 issuerAuth if present
    let mut mso_digests: BTreeMap<String, BTreeMap<i128, Vec<u8>>> = BTreeMap::new();
    let mut signature_valid: Option<bool> = None;

    if let Some(issuer_auth) = issuer_signed.map_get("issuerAuth").and_then(CborValue::as_array)
        && issuer_auth.len() == 4
    {
        let protected = issuer_auth[0].as_bytes().unwrap_or(&[]);
        let payload_bytes = issuer_auth[2].as_bytes();
        let sig_bytes = issuer_auth[3].as_bytes();

        if let Some(pb) = payload_bytes {
            // MSO may be directly CBOR-encoded or wrapped in Tag(24, Bytes)
            if let Ok(mso_raw) = decode_cbor(pb) {
                let mso = match &mso_raw {
                    CborValue::Tag(24, inner) => inner
                        .as_bytes()
                        .and_then(|b| decode_cbor(b).ok())
                        .unwrap_or(mso_raw),
                    _ => mso_raw,
                };
                if let Some(CborValue::Map(ns_digests)) = mso.map_get("valueDigests") {
                    for (ns_k, ns_v) in ns_digests {
                        if let (Some(ns_name), Some(CborValue::Map(id_map))) = (ns_k.as_text(), Some(ns_v)) {
                            let mut inner_map = BTreeMap::new();
                            for (id_k, digest_v) in id_map {
                                let id_int = match id_k {
                                    CborValue::Unsigned(u) => Some(*u as i128),
                                    CborValue::Negative(n) => Some(*n),
                                    _ => None,
                                };
                                if let (Some(id), Some(db)) = (id_int, digest_v.as_bytes()) {
                                    inner_map.insert(id, db.to_vec());
                                }
                            }
                            mso_digests.insert(ns_name.to_string(), inner_map);
                        }
                    }
                }
            }

            if let Some(sb) = sig_bytes
                && sb.len() == 64
            {
                let sig_structure = CborValue::Array(vec![
                    CborValue::Text("Signature1".into()),
                    CborValue::Bytes(protected.to_vec()),
                    CborValue::Bytes(Vec::new()),
                    CborValue::Bytes(pb.to_vec()),
                ]);
                let tbs = encode_cbor(&sig_structure);
                let mut sig = [0u8; 64];
                sig.copy_from_slice(sb);

                if let Some((px, py)) = issuer_jwk_json.and_then(parse_p256_public_coords_from_jwk) {
                    signature_valid = Some(verify_es256(&px, &py, &tbs, &sig));
                } else {
                    let demo = Es256KeyPair::demo_issuer_key();
                    if verify_es256(&demo.x, &demo.y, &tbs, &sig) {
                        signature_valid = Some(true);
                    }
                }
            }
        }
    }

    let name_spaces = issuer_signed
        .map_get("nameSpaces")
        .ok_or_else(|| "missing nameSpaces in mDOC issuerSigned".to_string())?;
    let CborValue::Map(ns_entries) = name_spaces else {
        return Err("mDOC nameSpaces must be a CBOR map".to_string());
    };

    let mut claims = Vec::new();
    let mut digests_verified = 0usize;
    let mut digests_total = 0usize;
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();

    for (ns_key, items_val) in ns_entries {
        let Some(ns_name) = ns_key.as_text() else {
            continue;
        };
        let Some(items) = items_val.as_array() else {
            continue;
        };
        for tagged_item in items {
            let (item_bytes_for_hash, item_cbor) = match tagged_item {
                CborValue::Tag(24, inner) => {
                    let raw = inner
                        .as_bytes()
                        .ok_or_else(|| "Tag(24) IssuerSignedItem must wrap bytes".to_string())?;
                    let decoded = decode_cbor(raw)?;
                    (encode_cbor(tagged_item), decoded)
                }
                other => (encode_cbor(other), other.clone()),
            };

            let elem_id = item_cbor
                .map_get("elementIdentifier")
                .and_then(CborValue::as_text)
                .ok_or_else(|| "IssuerSignedItem missing elementIdentifier".to_string())?;
            let elem_val = item_cbor
                .map_get("elementValue")
                .ok_or_else(|| "IssuerSignedItem missing elementValue".to_string())?;

            let digest_verified = if let Some(ns_map) = mso_digests.get(ns_name)
                && let Some(digest_id) = item_cbor.map_get("digestID").and_then(|v| match v {
                    CborValue::Unsigned(u) => Some(*u as i128),
                    CborValue::Negative(n) => Some(*n),
                    _ => None,
                })
                && let Some(expected_digest) = ns_map.get(&digest_id)
            {
                digests_total += 1;
                let actual = sha256(&item_bytes_for_hash);
                let ok = actual.as_slice() == expected_digest.as_slice();
                if ok {
                    digests_verified += 1;
                }
                Some(ok)
            } else {
                None
            };

            // Use clean elementIdentifier when unique across namespaces; prefix namespace if collision occurs
            let path = if seen_ids.insert(elem_id.to_string()) {
                elem_id.to_string()
            } else {
                format!("{ns_name}.{elem_id}")
            };

            claims.push(ParsedClaim {
                mdoc_element: Some((ns_name.to_string(), elem_id.to_string())),
                sd_jwt_path: None,
                path,
                value: elem_val.to_claim_value(),
                selective: true,
                disclose: true,
                digest_verified,
            });
        }
    }

    Ok(ParsedCredential {
        format: ParsedCredentialFormat::Mdoc,
        vct_or_doctype: doc_type,
        issuer: None,
        signature_valid,
        kb_sd_hash_valid: None,
        digests_verified,
        digests_total,
        claims,
    })
}

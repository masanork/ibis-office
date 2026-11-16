// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! ISO/IEC 18013-5 mDOC (CBOR / COSE_Sign1) credential issuance and selective presentation.

use crate::model::{Presentation, SlideId};
use crate::sd_jwt::{Es256KeyPair, base64url_encode, sha256};
use crate::sd_jwt_binding::{ClaimValue, SheetClaimBindings, extract_sheet_claims};
use crate::sd_jwt_import::{CborValue, encode_cbor};

/// Options controlling ISO/IEC 18013-5 mDOC issuance.
#[derive(Clone, Debug)]
pub struct MdocOptions {
    /// Document type (`docType`), e.g. `"org.iso.18013.5.1.mDL"`.
    pub doc_type: String,
    /// Primary namespace for worksheet claims, e.g. `"org.iso.18013.5.1"`.
    pub name_space: String,
}

impl Default for MdocOptions {
    fn default() -> Self {
        Self {
            doc_type: "org.iso.18013.5.1.mDL".to_string(),
            name_space: "org.iso.18013.5.1".to_string(),
        }
    }
}

/// Complete ISO/IEC 18013-5 mDOC issuance and selective-presentation artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdocOutput {
    pub doc_type: String,
    pub name_space: String,
    pub total_items: usize,
    pub disclosed_items: usize,
    /// Full `DeviceResponse` CBOR bytes containing all `IssuerSignedItem`s.
    pub device_response_all: Vec<u8>,
    /// Selective `DeviceResponse` CBOR bytes containing only disclosed `IssuerSignedItem`s.
    pub device_response_presentation: Vec<u8>,
    /// Base64url-encoded `device_response_all`.
    pub base64url_all: String,
    /// Base64url-encoded `device_response_presentation`.
    pub base64url_presentation: String,
    /// Lowercase hex-encoded `device_response_presentation`.
    pub hex_presentation: String,
}

fn claim_value_to_cbor(val: &ClaimValue) -> CborValue {
    match val {
        ClaimValue::Null => CborValue::Null,
        ClaimValue::Bool(b) => CborValue::Bool(*b),
        ClaimValue::Int(i) => {
            if *i >= 0 {
                CborValue::Unsigned(*i as u64)
            } else {
                CborValue::Negative(*i as i128)
            }
        }
        ClaimValue::Float(f) => CborValue::Float(*f),
        ClaimValue::String(s) => CborValue::Text(s.clone()),
        ClaimValue::Array(items) => {
            CborValue::Array(items.iter().map(claim_value_to_cbor).collect())
        }
        ClaimValue::Object(map) => CborValue::Map(
            map.iter()
                .map(|(k, v)| (CborValue::Text(k.clone()), claim_value_to_cbor(v)))
                .collect(),
        ),
    }
}

fn item_random_bytes(path_str: &str, digest_id: u64) -> Vec<u8> {
    let mut seed = Vec::from(path_str.as_bytes());
    seed.extend_from_slice(&digest_id.to_be_bytes());
    for i in 0..2u32 {
        let r = ibis_calc::random::next().unwrap_or(0.123456789 * (i as f64 + 1.0));
        seed.extend_from_slice(&r.to_bits().to_le_bytes());
    }
    sha256(&seed)[..16].to_vec()
}

fn resolve_doc_type(bindings: &SheetClaimBindings, fallback: &str) -> String {
    bindings
        .claims
        .iter()
        .find(|c| c.path_str == "vct" || c.path_str == "docType")
        .and_then(|c| match &c.value {
            ClaimValue::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn build_device_response(
    doc_type: &str,
    name_space: &str,
    items: Vec<CborValue>,
    issuer_auth: &CborValue,
) -> Vec<u8> {
    let ns_map = CborValue::Map(vec![(
        CborValue::Text(name_space.to_string()),
        CborValue::Array(items),
    )]);
    let issuer_signed = CborValue::Map(vec![
        (CborValue::Text("nameSpaces".to_string()), ns_map),
        (CborValue::Text("issuerAuth".to_string()), issuer_auth.clone()),
    ]);
    let document = CborValue::Map(vec![
        (
            CborValue::Text("docType".to_string()),
            CborValue::Text(doc_type.to_string()),
        ),
        (CborValue::Text("issuerSigned".to_string()), issuer_signed),
    ]);
    let response = CborValue::Map(vec![
        (
            CborValue::Text("version".to_string()),
            CborValue::Text("1.0".to_string()),
        ),
        (
            CborValue::Text("documents".to_string()),
            CborValue::Array(vec![document]),
        ),
        (CborValue::Text("status".to_string()), CborValue::Unsigned(0)),
    ]);
    encode_cbor(&response)
}

fn to_hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Issues an ISO/IEC 18013-5 mDOC `DeviceResponse` (and selective presentation) from `bindings`.
pub fn issue_mdoc(
    bindings: &SheetClaimBindings,
    options: &MdocOptions,
    issuer_key: &Es256KeyPair,
) -> MdocOutput {
    let doc_type = resolve_doc_type(bindings, &options.doc_type);
    let name_space = if options.name_space.trim().is_empty() {
        "org.iso.18013.5.1".to_string()
    } else {
        options.name_space.clone()
    };

    let mut all_items: Vec<CborValue> = Vec::new();
    let mut disclosed_items_vec: Vec<CborValue> = Vec::new();
    let mut digest_entries: Vec<(CborValue, CborValue)> = Vec::new();

    for (idx, claim) in bindings.claims.iter().enumerate() {
        if matches!(claim.path_str.as_str(), "vct" | "iss" | "docType") {
            continue;
        }
        let digest_id = idx as u64;
        let item_map = CborValue::Map(vec![
            (
                CborValue::Text("digestID".to_string()),
                CborValue::Unsigned(digest_id),
            ),
            (
                CborValue::Text("random".to_string()),
                CborValue::Bytes(item_random_bytes(&claim.path_str, digest_id)),
            ),
            (
                CborValue::Text("elementIdentifier".to_string()),
                CborValue::Text(claim.path_str.clone()),
            ),
            (
                CborValue::Text("elementValue".to_string()),
                claim_value_to_cbor(&claim.value),
            ),
        ]);
        let item_bytes = encode_cbor(&item_map);
        let tagged = CborValue::Tag(24, Box::new(CborValue::Bytes(item_bytes)));
        let digest = sha256(&encode_cbor(&tagged));
        digest_entries.push((CborValue::Unsigned(digest_id), CborValue::Bytes(digest.to_vec())));

        if !claim.selective || claim.disclose {
            disclosed_items_vec.push(tagged.clone());
        }
        all_items.push(tagged);
    }

    let value_digests = CborValue::Map(vec![(
        CborValue::Text(name_space.clone()),
        CborValue::Map(digest_entries),
    )]);
    let mso = CborValue::Map(vec![
        (
            CborValue::Text("version".to_string()),
            CborValue::Text("1.0".to_string()),
        ),
        (
            CborValue::Text("digestAlgorithm".to_string()),
            CborValue::Text("SHA-256".to_string()),
        ),
        (CborValue::Text("valueDigests".to_string()), value_digests),
        (
            CborValue::Text("docType".to_string()),
            CborValue::Text(doc_type.clone()),
        ),
    ]);
    let mso_tagged = CborValue::Tag(24, Box::new(CborValue::Bytes(encode_cbor(&mso))));
    let payload_bytes = encode_cbor(&mso_tagged);

    // COSE_Sign1 protected header: { 1 (alg): -7 (ES256) }
    let protected_bytes = encode_cbor(&CborValue::Map(vec![(
        CborValue::Unsigned(1),
        CborValue::Negative(-7),
    )]));
    let sig_structure = CborValue::Array(vec![
        CborValue::Text("Signature1".to_string()),
        CborValue::Bytes(protected_bytes.clone()),
        CborValue::Bytes(Vec::new()),
        CborValue::Bytes(payload_bytes.clone()),
    ]);
    let sig = issuer_key.sign_es256(&encode_cbor(&sig_structure));

    let issuer_auth = CborValue::Array(vec![
        CborValue::Bytes(protected_bytes),
        CborValue::Map(Vec::new()),
        CborValue::Bytes(payload_bytes),
        CborValue::Bytes(sig.to_vec()),
    ]);

    let total_items = all_items.len();
    let disclosed_items = disclosed_items_vec.len();
    let device_response_all =
        build_device_response(&doc_type, &name_space, all_items, &issuer_auth);
    let device_response_presentation =
        build_device_response(&doc_type, &name_space, disclosed_items_vec, &issuer_auth);

    let base64url_all = base64url_encode(&device_response_all);
    let base64url_presentation = base64url_encode(&device_response_presentation);
    let hex_presentation = to_hex_lower(&device_response_presentation);

    MdocOutput {
        doc_type,
        name_space,
        total_items,
        disclosed_items,
        device_response_all,
        device_response_presentation,
        base64url_all,
        base64url_presentation,
        hex_presentation,
    }
}

/// Extracts convention-based bindings from `slide_id` and issues an ISO/IEC 18013-5 mDOC.
pub fn issue_sheet_mdoc(
    presentation: &Presentation,
    slide_id: SlideId,
    options: &MdocOptions,
    issuer_key: &Es256KeyPair,
) -> Option<MdocOutput> {
    let bindings = extract_sheet_claims(presentation, slide_id)?;
    Some(issue_mdoc(&bindings, options, issuer_key))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use super::*;
    use crate::sd_jwt_binding::{BindingSource, BoundClaim};
    use crate::sd_jwt_import::{ParsedCredentialFormat, parse_mdoc_bytes};

    #[test]
    fn issue_and_parse_mdoc_roundtrip() {
        let bindings = SheetClaimBindings {
            slide: SlideId(1),
            claims: vec![
                BoundClaim {
                    path: vec!["vct".into()],
                    path_str: "vct".into(),
                    selective: false,
                    value: ClaimValue::String("org.iso.18013.5.1.mDL".into()),
                    disclose: true,
                    source: BindingSource::Cell { row: 0, column: 1 },
                },
                BoundClaim {
                    path: vec!["family_name".into()],
                    path_str: "family_name".into(),
                    selective: true,
                    value: ClaimValue::String("Yamada".into()),
                    disclose: true,
                    source: BindingSource::Cell { row: 1, column: 1 },
                },
                BoundClaim {
                    path: vec!["birth_date".into()],
                    path_str: "birth_date".into(),
                    selective: true,
                    value: ClaimValue::String("1990-01-01".into()),
                    disclose: false,
                    source: BindingSource::Cell { row: 2, column: 1 },
                },
            ],
            disclosures: BTreeMap::new(),
        };

        let issuer = Es256KeyPair::demo_issuer_key();
        let out = issue_mdoc(&bindings, &MdocOptions::default(), &issuer);
        assert_eq!(out.total_items, 2);
        assert_eq!(out.disclosed_items, 1);

        let parsed = parse_mdoc_bytes(&out.device_response_presentation, None).unwrap();
        assert_eq!(parsed.format, ParsedCredentialFormat::Mdoc);
        assert_eq!(parsed.vct_or_doctype, "org.iso.18013.5.1.mDL");
        assert_eq!(parsed.signature_valid, Some(true));
        assert_eq!(parsed.digests_verified, 1);
        assert_eq!(parsed.digests_total, 1);
        assert_eq!(parsed.claims.len(), 1);
        assert_eq!(parsed.claims[0].path, "family_name");
    }
}

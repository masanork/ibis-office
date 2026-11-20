// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::BTreeSet;

use ibis_doc_model::sd_jwt::{base64url_decode, verify_es256};
use ibis_doc_model::sd_jwt_import::{parse_single_mdoc_text, parse_sd_jwt};
use serde_json::{Value, json};

use crate::catalog::{Access, Tool, object, string};
use crate::service::Args;
use crate::{Service, ToolError};

pub(crate) fn tools() -> [Tool; 1] {
    [Tool {
        name: "presentation_compare",
        description: "Compare one SD-JWT or mdoc presentation against expected attribute paths. Returns diagnostics, not a trust decision or claim values.",
        access: Access::Read,
        returns_content: true,
        schema: || {
            object(
                &[
                    ("format", json!({"enum": ["sd-jwt", "mdoc"]})),
                    ("token", string("Compact SD-JWT or hex/base64url mdoc.")),
                    (
                        "expected_paths",
                        json!({"type": "array", "items": {"type": "string"}}),
                    ),
                    ("issuer_jwk", json!({"type": "object"})),
                    ("nonce", string("Expected SD-JWT key binding nonce.")),
                    ("audience", string("Expected SD-JWT key binding audience.")),
                ],
                &["format", "token", "expected_paths"],
            )
        },
        run: |_, args| compare(args),
    }]
}

fn decode_segment(segment: &str) -> Option<Value> {
    serde_json::from_slice(&base64url_decode(segment)?).ok()
}

fn public_coords(jwk: &Value) -> Option<([u8; 32], [u8; 32])> {
    if jwk["kty"] != "EC" || jwk["crv"] != "P-256" || jwk.get("d").is_some() {
        return None;
    }
    let x = base64url_decode(jwk["x"].as_str()?)?.try_into().ok()?;
    let y = base64url_decode(jwk["y"].as_str()?)?.try_into().ok()?;
    Some((x, y))
}

fn key_binding(token: &str) -> Option<(Value, bool)> {
    let (issuer, rest) = token.split_once('~')?;
    let kb = rest.rsplit('~').next()?;
    let (input, signature) = kb.rsplit_once('.')?;
    let (header, payload) = input.split_once('.')?;
    let header = decode_segment(header)?;
    let payload = decode_segment(payload)?;
    let issuer_payload = decode_segment(issuer.split('.').nth(1)?)?;
    let jwk = &issuer_payload["cnf"]["jwk"];
    let valid = (|| {
        let (x, y) = public_coords(jwk)?;
        let signature = base64url_decode(signature)?.try_into().ok()?;
        Some(
            header["alg"] == "ES256"
                && header["typ"] == "kb+jwt"
                && verify_es256(&x, &y, input.as_bytes(), &signature),
        )
    })()
    .unwrap_or(false);
    Some((payload, valid))
}

fn compare(args: &Value) -> Result<Value, ToolError> {
    let token = args.str_arg("token")?.trim();
    if token.len() > 1024 * 1024 {
        return Err(ToolError::invalid("presentation exceeds 1 MiB"));
    }
    let expected = args["expected_paths"]
        .as_array()
        .ok_or_else(|| ToolError::invalid("expected_paths must be an array"))?
        .iter()
        .map(|path| {
            path.as_str()
                .ok_or_else(|| ToolError::invalid("expected paths must be strings"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let nonce = args.opt_str("nonce")?;
    let audience = args.opt_str("audience")?;
    let jwk = match args.get("issuer_jwk") {
        Some(key) if public_coords(key).is_some() => Some(key.to_string()),
        Some(_) => return Err(ToolError::invalid("issuer_jwk must be a public P-256 JWK")),
        None => None,
    };
    let format = args.str_arg("format")?;
    let parsed = match format {
        "sd-jwt" => parse_sd_jwt(token, jwk.as_deref()),
        "mdoc" => parse_single_mdoc_text(token, jwk.as_deref()),
        _ => return Err(ToolError::invalid("unsupported presentation format")),
    }
    .map_err(|_| ToolError::invalid("could not parse presentation"))?;
    let actual: BTreeSet<_> = parsed
        .claims
        .iter()
        .filter(|claim| format == "mdoc" || !matches!(claim.path.as_str(), "iss" | "vct"))
        .map(|claim| claim.path.as_str())
        .collect();
    let kb = if format == "sd-jwt" {
        key_binding(token)
    } else {
        None
    };
    let check = |expected: Option<&str>, field: &str| {
        expected.and_then(|value| {
            kb.as_ref()
                .map(|(payload, _)| payload[field].as_str() == Some(value))
        })
    };
    let supplied_digests = if format == "sd-jwt" {
        token
            .split('~')
            .skip(1)
            .filter(|part| !part.is_empty() && !part.contains('.'))
            .count()
    } else {
        parsed.claims.len()
    };
    Ok(json!({
        "format": format,
        "disclosed_paths": actual,
        "missing_paths": expected.difference(&actual).collect::<Vec<_>>(),
        "unexpected_paths": actual.difference(&expected).collect::<Vec<_>>(),
        "issuer_signature_valid": jwk.as_ref().and(parsed.signature_valid),
        "disclosure_digests_verified": parsed.digests_verified,
        "supplied_disclosure_digests_match": supplied_digests == parsed.digests_verified,
        "disclosure_digests_total": parsed.digests_total,
        "holder_signature_valid": kb.as_ref().map(|(_, valid)| valid),
        "holder_sd_hash_valid": parsed.kb_sd_hash_valid,
        "nonce_matches": check(nonce, "nonce"),
        "audience_matches": check(audience, "aud"),
        "issuer_trust": "not_checked",
        "device_authentication": if format == "mdoc" { "not_checked" } else { "not_applicable" },
    }))
}

pub fn run_cli(args: &Value) -> Result<Value, ToolError> {
    let mut service = Service::new(&[], true, Default::default());
    service.call("presentation_compare", args)
}

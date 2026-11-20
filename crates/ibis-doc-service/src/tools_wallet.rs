// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::BTreeSet;

use ibis_doc_model::sd_jwt::{base64url_decode, verify_es256};
use ibis_doc_model::sd_jwt_import::{parse_sd_jwt, parse_single_mdoc_text};
use serde_json::{Value, json};

use crate::catalog::{Access, Tool, object, string};
use crate::service::Args;
use crate::{Service, ToolError};

pub(crate) fn tools() -> [Tool; 1] {
    [Tool {
        name: "presentation_compare",
        description: "Compare one SD-JWT or mdoc presentation against explicit attribute paths or a supported DCQL query. Returns diagnostics, not a trust decision or claim values.",
        access: Access::Read,
        returns_content: true,
        schema: || {
            let mut schema = object(
                &[
                    ("format", json!({"enum": ["sd-jwt", "mdoc"]})),
                    ("token", string("Compact SD-JWT or hex/base64url mdoc.")),
                    (
                        "vp_token",
                        json!({
                            "type": "object",
                            "description": "OpenID4VP JSON object keyed by the single DCQL credential id.",
                            "minProperties": 1, "maxProperties": 1,
                            "additionalProperties": {
                                "type": "array", "minItems": 1, "maxItems": 1,
                                "items": {"type": "string", "minLength": 1}
                            }
                        }),
                    ),
                    (
                        "expected_paths",
                        json!({"type": "array", "items": {"type": "string"}}),
                    ),
                    ("dcql_query", json!({"type": "object"})),
                    ("issuer_jwk", json!({"type": "object"})),
                    ("nonce", string("Expected SD-JWT key binding nonce.")),
                    ("audience", string("Expected SD-JWT key binding audience.")),
                ],
                &["format"],
            );
            schema["oneOf"] = json!([
                {"required": ["token", "expected_paths"], "not": {"anyOf": [
                    {"required": ["vp_token"]}, {"required": ["dcql_query"]}
                ]}},
                {"required": ["token", "dcql_query"], "not": {"anyOf": [
                    {"required": ["vp_token"]}, {"required": ["expected_paths"]}
                ]}},
                {"required": ["vp_token", "dcql_query"], "not": {"anyOf": [
                    {"required": ["token"]}, {"required": ["expected_paths"]}
                ]}}
            ]);
            schema
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

fn presentation_input(args: &Value) -> Result<(&str, Option<&str>), ToolError> {
    match (args.get("token"), args.get("vp_token")) {
        (Some(_), None) => Ok((args.str_arg("token")?, None)),
        (None, Some(vp_token)) => {
            if args.get("expected_paths").is_some() {
                return Err(ToolError::invalid(
                    "vp_token requires dcql_query instead of expected_paths",
                ));
            }
            let (_, id) = crate::wallet_dcql::single_credential(&args["dcql_query"])?;
            let presentations = vp_token
                .as_object()
                .filter(|map| map.len() == 1)
                .ok_or_else(|| ToolError::invalid("vp_token must contain one credential id"))?;
            let entry = presentations
                .get(id)
                .ok_or_else(|| ToolError::invalid("vp_token credential id does not match DCQL"))?;
            let token = entry
                .as_array()
                .filter(|tokens| tokens.len() == 1)
                .and_then(|tokens| tokens[0].as_str())
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(|| {
                    ToolError::invalid("vp_token entry must contain one presentation string")
                })?;
            Ok((token, Some(id)))
        }
        _ => Err(ToolError::invalid(
            "provide exactly one of token or vp_token",
        )),
    }
}

fn compare(args: &Value) -> Result<Value, ToolError> {
    let (token, credential_id) = presentation_input(args)?;
    let token = token.trim();
    if token.len() > 1024 * 1024 {
        return Err(ToolError::invalid("presentation exceeds 1 MiB"));
    }
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
    let (expected, actual, dcql) = match (args.get("expected_paths"), args.get("dcql_query")) {
        (Some(paths), None) => {
            let expected = paths
                .as_array()
                .ok_or_else(|| ToolError::invalid("expected_paths must be an array"))?
                .iter()
                .map(|path| {
                    path.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| ToolError::invalid("expected paths must be strings"))
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            let actual = parsed
                .claims
                .iter()
                .filter(|claim| format == "mdoc" || !matches!(claim.path.as_str(), "iss" | "vct"))
                .map(|claim| claim.path.clone())
                .collect();
            (expected, actual, Value::Null)
        }
        (None, Some(query)) => {
            let issuer_payload = if format == "sd-jwt" {
                token.split('.').nth(1).and_then(decode_segment)
            } else {
                None
            };
            let credential_type = if format == "mdoc" {
                Some(parsed.vct_or_doctype.as_str())
            } else {
                issuer_payload
                    .as_ref()
                    .and_then(|payload| payload["vct"].as_str())
            };
            let result =
                crate::wallet_dcql::compare(query, format, credential_type, &parsed.claims)?;
            let diagnostics = json!({
                "credential_type_matches": result.credential_type_matches,
                "value_mismatch_paths": result.value_mismatches,
            });
            (result.expected, result.actual, diagnostics)
        }
        _ => {
            return Err(ToolError::invalid(
                "provide exactly one of expected_paths or dcql_query",
            ));
        }
    };
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
        "dcql": dcql,
        "vp_token_credential_id": credential_id,
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

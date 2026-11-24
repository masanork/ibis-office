// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::{BTreeMap, BTreeSet};

use ibis_doc_model::sd_jwt_import::ParsedClaim;
use serde_json::Value;

use crate::ToolError;

pub(crate) struct Comparison {
    pub expected: BTreeSet<String>,
    pub actual: BTreeSet<String>,
    pub value_mismatches: BTreeSet<String>,
    pub credential_type_matches: bool,
}

fn object(value: &Value, allowed: &[&str]) -> Result<(), ToolError> {
    let fields = value
        .as_object()
        .ok_or_else(|| ToolError::invalid("DCQL expected an object"))?;
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ToolError::invalid("unsupported DCQL property"));
    }
    Ok(())
}

fn nonempty(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty())
}

fn mdoc_path(namespace: &str, element: &str) -> String {
    // JSON Pointer escaping keeps the namespace and element unambiguous.
    let escape = |s: &str| s.replace('~', "~0").replace('/', "~1");
    format!("/{}/{}", escape(namespace), escape(element))
}

fn sd_jwt_path(segments: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    // Keep familiar dotted output for ordinary names, but distinguish literal dots.
    segments
        .into_iter()
        .map(|segment| segment.as_ref().replace('\\', "\\\\").replace('.', "\\."))
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn single_credential(query: &Value) -> Result<(&Value, &str), ToolError> {
    object(query, &["credentials"])?;
    let credentials = query["credentials"]
        .as_array()
        .filter(|list| list.len() == 1)
        .ok_or_else(|| ToolError::invalid("DCQL requires one credential query"))?;
    let credential = &credentials[0];
    object(
        credential,
        &[
            "id",
            "format",
            "meta",
            "claims",
            "require_cryptographic_holder_binding",
        ],
    )?;
    if credential
        .get("require_cryptographic_holder_binding")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(ToolError::invalid(
            "DCQL holder binding requirement must be a boolean",
        ));
    }
    let id = nonempty(&credential["id"])
        .filter(|id| {
            id.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
        .ok_or_else(|| ToolError::invalid("invalid DCQL credential id"))?;
    Ok((credential, id))
}

pub(crate) fn compare(
    query: &Value,
    format: &str,
    credential_type: Option<&str>,
    claims: &[ParsedClaim],
) -> Result<Comparison, ToolError> {
    let (credential, _) = single_credential(query)?;
    let mdoc = format == "mdoc";
    if credential["format"] != if mdoc { "mso_mdoc" } else { "dc+sd-jwt" } {
        return Err(ToolError::invalid(
            "DCQL format does not match presentation format",
        ));
    }
    let meta = &credential["meta"];
    let credential_type_matches = if mdoc {
        object(meta, &["doctype_value"])?;
        let expected = nonempty(&meta["doctype_value"])
            .ok_or_else(|| ToolError::invalid("DCQL requires doctype_value"))?;
        credential_type == Some(expected)
    } else {
        object(meta, &["vct_values"])?;
        let expected = meta["vct_values"]
            .as_array()
            .filter(|xs| !xs.is_empty())
            .ok_or_else(|| ToolError::invalid("DCQL requires vct_values"))?;
        if expected.iter().any(|value| nonempty(value).is_none()) {
            return Err(ToolError::invalid("invalid DCQL vct_values"));
        }
        expected
            .iter()
            .any(|value| value.as_str() == credential_type)
    };
    let mut actual = BTreeMap::new();
    for claim in claims {
        let path = if let Some((namespace, element)) = &claim.mdoc_element {
            mdoc_path(namespace, element)
        } else {
            let segments = claim
                .sd_jwt_path
                .as_deref()
                .ok_or_else(|| ToolError::invalid("missing structured SD-JWT claim path"))?;
            if matches!(segments, [field] if matches!(field.as_str(), "iss" | "vct")) {
                continue;
            }
            sd_jwt_path(segments)
        };
        if actual.insert(path, &claim.value).is_some() {
            return Err(ToolError::invalid("ambiguous disclosed claim path"));
        }
    }
    let requests = credential["claims"]
        .as_array()
        .filter(|list| !list.is_empty())
        .ok_or_else(|| ToolError::invalid("DCQL requires a nonempty claim list"))?;
    let mut expected = BTreeSet::new();
    let mut value_mismatches = BTreeSet::new();
    for request in requests {
        if mdoc {
            object(request, &["path", "values", "intent_to_retain"])?;
            if request
                .get("intent_to_retain")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(ToolError::invalid(
                    "DCQL mdoc intent_to_retain must be a boolean",
                ));
            }
        } else {
            object(request, &["path", "values"])?;
        }
        let path = match request["path"].as_array().map(Vec::as_slice) {
            Some([namespace, element]) if mdoc => match (nonempty(namespace), nonempty(element)) {
                (Some(namespace), Some(element)) => mdoc_path(namespace, element),
                _ => {
                    return Err(ToolError::invalid(
                        "DCQL mdoc path requires namespace and element strings",
                    ));
                }
            },
            Some(segments) if !mdoc && !segments.is_empty() => {
                let segments = segments
                    .iter()
                    .map(nonempty)
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| {
                        ToolError::invalid("DCQL SD-JWT paths require object member names")
                    })?;
                sd_jwt_path(segments)
            }
            _ => return Err(ToolError::invalid("unsupported DCQL claim path")),
        };
        if !expected.insert(path.clone()) {
            return Err(ToolError::invalid("duplicate DCQL claim path"));
        }
        if let Some(values) = request.get("values") {
            let values = values
                .as_array()
                .filter(|xs| {
                    !xs.is_empty()
                        && xs
                            .iter()
                            .all(|value| value.is_string() || value.is_boolean() || value.is_i64())
                })
                .ok_or_else(|| {
                    ToolError::invalid(
                        "DCQL values must be nonempty strings, booleans or signed integers",
                    )
                })?;
            if let Some(value) = actual.get(&path) {
                let value: Value = serde_json::from_str(&value.to_json())
                    .map_err(|_| ToolError::invalid("could not read disclosed claim value"))?;
                if !values.contains(&value) {
                    value_mismatches.insert(path);
                }
            }
        }
    }
    Ok(Comparison {
        expected,
        actual: actual.into_keys().collect(),
        value_mismatches,
        credential_type_matches,
    })
}

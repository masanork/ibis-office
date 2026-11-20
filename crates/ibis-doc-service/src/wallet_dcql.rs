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

pub(crate) fn compare(
    query: &Value,
    format: &str,
    credential_type: Option<&str>,
    claims: &[ParsedClaim],
) -> Result<Comparison, ToolError> {
    object(query, &["credentials"])?;
    let credentials = query["credentials"]
        .as_array()
        .filter(|list| list.len() == 1)
        .ok_or_else(|| ToolError::invalid("DCQL requires one credential query"))?;
    let credential = &credentials[0];
    object(credential, &["id", "format", "meta", "claims"])?;
    if nonempty(&credential["id"]).is_none() {
        return Err(ToolError::invalid("DCQL requires a credential id"));
    }
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
        } else if matches!(claim.path.as_str(), "iss" | "vct") {
            continue;
        } else {
            claim.path.clone()
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
        object(request, &["path", "values"])?;
        let path = match request["path"].as_array().map(Vec::as_slice) {
            Some([namespace, element]) if mdoc => match (nonempty(namespace), nonempty(element)) {
                (Some(namespace), Some(element)) => mdoc_path(namespace, element),
                _ => {
                    return Err(ToolError::invalid(
                        "DCQL mdoc path requires namespace and element strings",
                    ));
                }
            },
            Some([field]) if !mdoc => nonempty(field)
                .filter(|field| !field.contains('.'))
                .ok_or_else(|| ToolError::invalid("DCQL supports flat SD-JWT claim paths"))?
                .to_owned(),
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

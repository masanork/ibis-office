// SPDX-License-Identifier: AGPL-3.0-only
//! Read-only inspection of foreign structured data. Values are never evaluated.

use std::collections::BTreeMap;

use ibis_doc_model::sd_jwt_import::{
    ParsedCredential, parse_sd_jwt, parse_single_mdoc_bytes, parse_single_mdoc_text,
};
use serde_json::Value;

pub mod visual;
pub mod xml_view;

pub use visual::{CalloutTone, PreviewBlock, PreviewSummary, TableCellValue, ValueKind};

const MAX_BYTES: usize = 1024 * 1024;
pub const SIZE_LIMIT_ERROR: &str = "Structured preview accepts files up to 1 MiB";
const MAX_FIELDS: usize = 1_000;
const MAX_DEPTH: usize = 32;

pub struct Field {
    pub name: String,
    pub path: String,
    pub value: String,
}

pub struct Preview {
    pub format: &'static str,
    pub fields: Vec<Field>,
    pub credential: Option<CredentialDetails>,
    pub summary: PreviewSummary,
    pub blocks: Vec<PreviewBlock>,
}

pub struct CredentialDetails {
    pub credential_type: Option<String>,
    pub issuer: Option<String>,
    pub matching_digests: usize,
}

pub fn supports_extension(extension: &str) -> bool {
    ["json", "yaml", "yml", "xml", "sd-jwt", "mdoc"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

pub fn read(extension: &str, bytes: &[u8]) -> Result<Preview, String> {
    let raw_len = bytes.len();
    if raw_len > MAX_BYTES {
        return Err(SIZE_LIMIT_ERROR.into());
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text =
        || std::str::from_utf8(bytes).map_err(|_| "The preview requires UTF-8 text".to_string());
    let mut fields = Vec::new();
    let mut blocks = Vec::new();
    let format = match extension.to_ascii_lowercase().as_str() {
        "json" => {
            let data: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid JSON")?;
            json_fields(&data, "", "/", 0, &mut fields)?;
            visual::build_json_blocks(&data, "", "", 0, &mut blocks)?;
            "JSON"
        }
        "yaml" | "yml" => {
            let src = text()?;
            let data: Value = serde_saphyr::from_str(src).map_err(|_| "Invalid YAML")?;
            json_fields(&data, "", "/", 0, &mut fields)?;
            visual::build_json_blocks(&data, "", "", 0, &mut blocks)?;
            "YAML"
        }
        "xml" => {
            let doc = parse_xml(bytes)?;
            let root = doc.root_element();
            let root_path = format!(
                "/{}",
                xml_name(root.tag_name().namespace(), root.tag_name().name())
            );
            xml_fields(root, &root_path, 0, &mut fields)?;
            visual::build_xml_blocks(root, &root_path, 0, &mut blocks)?;
            "XML"
        }
        "sd-jwt" => return credential("SD-JWT", raw_len, parse_sd_jwt(text()?, None)),
        "mdoc" => {
            return credential(
                "mdoc",
                raw_len,
                match std::str::from_utf8(bytes) {
                    Ok(text)
                        if text
                            .trim()
                            .bytes()
                            .all(|b| b.is_ascii_graphic() || b.is_ascii_whitespace()) =>
                    {
                        parse_single_mdoc_text(text, None)
                    }
                    _ => parse_single_mdoc_bytes(bytes, None),
                },
            );
        }
        _ => return Err("Unsupported structured preview format".into()),
    };
    let summary = visual::summarize_blocks(raw_len, fields.len(), &blocks, None, Vec::new());
    Ok(Preview {
        format,
        fields,
        credential: None,
        summary,
        blocks,
    })
}

fn credential(
    format: &'static str,
    byte_size: usize,
    parsed: Result<ParsedCredential, String>,
) -> Result<Preview, String> {
    let parsed = parsed.map_err(|_| "Invalid or unsupported credential data")?;
    let mut fields = Vec::new();
    for claim in parsed.claims {
        if format == "SD-JWT" && matches!(claim.path.as_str(), "iss" | "vct") {
            continue;
        }
        let (name, path) = if let Some((namespace, element)) = claim.mdoc_element {
            let path = format!(
                "/{}/{}",
                pointer_segment(&namespace),
                pointer_segment(&element)
            );
            (element, path)
        } else {
            let name = claim
                .path
                .rsplit('.')
                .next()
                .unwrap_or(&claim.path)
                .to_owned();
            (name, claim.path)
        };
        push(&mut fields, &path, &name, claim.value.to_json())?;
    }
    let blocks = visual::blocks_from_fields(&fields);
    let summary = visual::summarize_blocks(byte_size, fields.len(), &blocks, None, Vec::new());
    Ok(Preview {
        format,
        fields,
        credential: Some(CredentialDetails {
            credential_type: match format {
                "SD-JWT" if parsed.vct_or_doctype == "urn:example:sd-jwt:1" => None,
                _ => Some(parsed.vct_or_doctype),
            },
            issuer: parsed.issuer,
            matching_digests: parsed.digests_verified,
        }),
        summary,
        blocks,
    })
}

fn pointer_segment(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn push(fields: &mut Vec<Field>, path: &str, name: &str, value: String) -> Result<(), String> {
    if fields.len() == MAX_FIELDS || path.len() > 4_096 {
        return Err("Structured preview is limited to 1,000 fields and 4 KiB paths".into());
    }
    fields.push(Field {
        name: name.into(),
        path: if path.is_empty() {
            "/".into()
        } else {
            path.into()
        },
        value,
    });
    Ok(())
}

fn check_depth(depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("Structured preview is limited to 32 levels".into());
    }
    Ok(())
}

fn json_fields(
    value: &Value,
    path: &str,
    name: &str,
    depth: usize,
    fields: &mut Vec<Field>,
) -> Result<(), String> {
    check_depth(depth)?;
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, value) in map {
                json_fields(
                    value,
                    &format!("{path}/{}", pointer_segment(key)),
                    key,
                    depth + 1,
                    fields,
                )?;
            }
        }
        Value::Array(list) if !list.is_empty() => {
            for (index, value) in list.iter().enumerate() {
                json_fields(
                    value,
                    &format!("{path}/{index}"),
                    &format!("[{index}]"),
                    depth + 1,
                    fields,
                )?;
            }
        }
        value => push(fields, path, name, value.to_string())?,
    }
    Ok(())
}

fn parse_xml(bytes: &[u8]) -> Result<roxmltree::Document<'_>, String> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| "The preview requires UTF-8 text")?;
    let options = roxmltree::ParsingOptions {
        nodes_limit: 8_000,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(text, options)
        .map_err(|_| "Invalid or unsupported XML".into())
}

fn xml_name(namespace: Option<&str>, local: &str) -> String {
    namespace.map_or_else(|| local.to_owned(), |ns| format!("{{{ns}}}{local}"))
}

fn xml_fields(
    node: roxmltree::Node<'_, '_>,
    path: &str,
    depth: usize,
    fields: &mut Vec<Field>,
) -> Result<(), String> {
    check_depth(depth)?;
    for attr in node.attributes() {
        let name = xml_name(attr.namespace(), attr.name());
        push(
            fields,
            &format!("{path}/@{name}"),
            attr.name(),
            attr.value().into(),
        )?;
    }
    let mut siblings = BTreeMap::<String, usize>::new();
    let mut text_index = 0;
    let mut has_elements = false;
    for child in node.children() {
        if child.is_element() {
            has_elements = true;
            let name = xml_name(child.tag_name().namespace(), child.tag_name().name());
            let index = siblings.entry(name.clone()).or_default();
            *index += 1;
            xml_fields(child, &format!("{path}/{name}[{index}]"), depth + 1, fields)?;
        } else if let Some(text) = child
            .text()
            .filter(|text| child.is_text() && !text.trim().is_empty())
        {
            text_index += 1;
            push(
                fields,
                &format!("{path}/#text[{text_index}]"),
                node.tag_name().name(),
                text.into(),
            )?;
        }
    }
    if !has_elements && text_index == 0 {
        push(fields, path, node.tag_name().name(), String::new())?;
    }
    Ok(())
}

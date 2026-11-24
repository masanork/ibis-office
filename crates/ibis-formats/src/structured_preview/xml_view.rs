// SPDX-License-Identifier: AGPL-3.0-only

//! Local display definitions for namespace-aware XML previews.

use std::collections::BTreeMap;

use roxmltree::Node;
use serde::Deserialize;

use super::{Field, MAX_BYTES, Preview, SIZE_LIMIT_ERROR, check_depth, push, xml_name};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    version: u32,
    title: String,
    root: Root,
    #[serde(default)]
    namespaces: BTreeMap<String, String>,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Root {
    namespace: String,
    name: String,
    #[serde(default)]
    attributes: BTreeMap<String, String>,
    #[serde(default)]
    elements: Vec<ElementSelector>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementSelector {
    path: Vec<String>,
    value: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Entry {
    Field {
        path: Vec<String>,
        label: String,
        #[serde(default)]
        unit: String,
        #[serde(default)]
        group_digits: bool,
    },
    Group {
        path: Vec<String>,
        label: String,
        entries: Vec<Entry>,
    },
}

pub const MAX_DEFINITION_BYTES: usize = MAX_BYTES;

pub fn read(bytes: &[u8], definition: &[u8]) -> Result<Preview, String> {
    if bytes.len() > MAX_BYTES {
        return Err(SIZE_LIMIT_ERROR.into());
    }
    let definition = Definition::parse(definition)?;
    let doc = super::parse_xml(bytes)?;
    let root = doc.root_element();
    if !definition.matches(root)? {
        return Err("XML does not match the selectors in the view definition".into());
    }
    definition.preview(root)
}

#[derive(Debug)]
pub enum SharedError {
    Invalid(String),
    Conflict(Vec<String>),
}

/// Select a unique shared definition for the document. Invalid definitions
/// and definitions without an attribute or element selector are skipped.
pub fn select_shared(
    bytes: &[u8],
    definitions: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Result<Option<Preview>, SharedError> {
    let mut definitions = definitions.into_iter().peekable();
    if definitions.peek().is_none() {
        return Ok(None);
    }
    if bytes.len() > MAX_BYTES {
        return Err(SharedError::Invalid(SIZE_LIMIT_ERROR.into()));
    }
    let doc = super::parse_xml(bytes).map_err(SharedError::Invalid)?;
    let root = doc.root_element();
    let mut matches = definitions.filter_map(|(name, bytes)| {
        let definition = Definition::parse(&bytes).ok()?;
        ((!definition.root.attributes.is_empty() || !definition.root.elements.is_empty())
            && definition.matches(root).ok()?)
        .then_some((name, definition))
    });
    let Some((name, definition)) = matches.next() else {
        return Ok(None);
    };
    let mut names = vec![name];
    names.extend(matches.map(|(name, _)| name));
    if names.len() > 1 {
        names.sort_unstable();
        return Err(SharedError::Conflict(names));
    }
    definition
        .preview(root)
        .map(Some)
        .map_err(SharedError::Invalid)
}

impl Root {
    fn matches(&self, root: Node<'_, '_>) -> bool {
        let tag = root.tag_name();
        tag.name() == self.name
            && tag.namespace().unwrap_or_default() == self.namespace
            && self
                .attributes
                .iter()
                .all(|(key, value)| root.attribute(key.as_str()) == Some(value.as_str()))
    }
}

impl Definition {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_DEFINITION_BYTES {
            return Err(SIZE_LIMIT_ERROR.into());
        }
        let definition: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("Invalid XML view definition: {error}"))?;
        if definition.version != 1 || definition.title.trim().is_empty() {
            return Err("Unsupported XML view definition".into());
        }
        Ok(definition)
    }

    fn matches(&self, root: Node<'_, '_>) -> Result<bool, String> {
        if !self.root.matches(root) {
            return Ok(false);
        }
        for selector in &self.root.elements {
            let nodes = self.select(root, &selector.path)?;
            let [node] = nodes.as_slice() else {
                return Ok(false);
            };
            if leaf_text(*node).is_none_or(|value| value.trim() != selector.value) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn preview(&self, root: Node<'_, '_>) -> Result<Preview, String> {
        let mut fields = Vec::new();
        push(&mut fields, "", &self.title, String::new())?;
        self.render(root, &self.entries, 0, &mut fields)?;
        Ok(Preview {
            format: "XML",
            fields,
            credential: None,
        })
    }

    fn select<'a, 'input>(
        &self,
        root: Node<'a, 'input>,
        path: &[String],
    ) -> Result<Vec<Node<'a, 'input>>, String> {
        if path.is_empty() || path.len() > super::MAX_DEPTH {
            return Err("Invalid XML view path length".into());
        }
        let mut matches = vec![root];
        for segment in path {
            let (namespace, local) = match segment.split_once(':') {
                Some((prefix, local)) => (
                    self.namespaces
                        .get(prefix)
                        .ok_or("Unknown XML view namespace prefix")?
                        .as_str(),
                    local,
                ),
                None => (self.root.namespace.as_str(), segment.as_str()),
            };
            if local.is_empty() || local.contains([':', '/', '*', '[', ']']) {
                return Err("Invalid XML view element name".into());
            }
            matches = matches
                .into_iter()
                .flat_map(|node| node.children())
                .filter(|child| {
                    child.is_element()
                        && child.tag_name().name() == local
                        && child.tag_name().namespace().unwrap_or_default() == namespace
                })
                .collect();
        }
        Ok(matches)
    }

    fn render(
        &self,
        root: Node<'_, '_>,
        entries: &[Entry],
        depth: usize,
        fields: &mut Vec<Field>,
    ) -> Result<(), String> {
        check_depth(depth)?;
        for entry in entries {
            match entry {
                Entry::Field {
                    path,
                    label,
                    unit,
                    group_digits,
                } => {
                    for node in self.select(root, path)? {
                        let text = leaf_text(node)
                            .ok_or("An XML view field must refer to a leaf element")?;
                        let mut value = if *group_digits {
                            group_integer(text.trim())
                        } else {
                            text
                        };
                        if !unit.is_empty() && !value.is_empty() {
                            value.push(' ');
                            value.push_str(unit);
                        }
                        push(fields, &source_path(node, self)?, label, value)?;
                    }
                }
                Entry::Group {
                    path,
                    label,
                    entries,
                } => {
                    for (index, node) in self.select(root, path)?.into_iter().enumerate() {
                        let heading = format!("{label} {}", index + 1);
                        push(fields, &source_path(node, self)?, &heading, String::new())?;
                        self.render(node, entries, depth + 1, fields)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn leaf_text(node: Node<'_, '_>) -> Option<String> {
    if node.children().any(|child| child.is_element()) {
        return None;
    }
    Some(
        node.children()
            .filter(|child| child.is_text())
            .filter_map(|child| child.text())
            .collect(),
    )
}

fn source_path(node: Node<'_, '_>, definition: &Definition) -> Result<String, String> {
    let mut segments = Vec::new();
    for node in node.ancestors().filter(Node::is_element) {
        check_depth(segments.len())?;
        let tag = node.tag_name();
        let namespace = tag.namespace().unwrap_or_default();
        let name = if namespace == definition.root.namespace {
            tag.name().to_owned()
        } else if let Some((prefix, _)) = definition
            .namespaces
            .iter()
            .find(|(_, uri)| uri.as_str() == namespace)
        {
            format!("{prefix}:{}", tag.name())
        } else {
            xml_name(tag.namespace(), tag.name())
        };
        if node.parent_element().is_some() {
            let index = node
                .prev_siblings()
                .filter(|sibling| sibling.is_element() && sibling.tag_name() == node.tag_name())
                .count();
            segments.push(format!("{name}[{index}]"));
        } else {
            segments.push(name);
        }
    }
    segments.reverse();
    Ok(format!("/{}", segments.join("/")))
}

#[cfg(test)]
mod tests {
    use super::read;
    const XML: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test/fixtures/structured-preview/etax/life-insurance.xml"
    ));
    const DEFINITION: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test/fixtures/structured-preview/etax/life-insurance.view.json"
    ));

    #[test]
    fn shared_jma_definitions_match_the_unique_values_of_both_header_fields() {
        const PHENOLOGY: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test/fixtures/structured-preview/jma/phenology.view.json"
        ));
        const WEATHER: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test/fixtures/structured-preview/jma/special-weather.view.json"
        ));
        let select = |header: &str| {
            let xml = format!(
                r#"<Report xmlns="http://xml.kishou.go.jp/jmaxml1/" xmlns:i="http://xml.kishou.go.jp/jmaxml1/informationBasis1/"><i:Head>{header}</i:Head></Report>"#
            );
            super::select_shared(
                xml.as_bytes(),
                [
                    ("weather".into(), WEATHER.to_vec()),
                    ("phenology".into(), PHENOLOGY.to_vec()),
                ],
            )
            .unwrap()
        };
        for (kind, title) in [
            ("生物季節観測報告気象報", "生物季節観測の項目"),
            ("特殊気象報", "特殊気象報の項目"),
        ] {
            let header = format!(
                "<i:InfoKind> {kind} </i:InfoKind><i:InfoKindVersion>1.0_0</i:InfoKindVersion>"
            );
            assert_eq!(select(&header).unwrap().fields[0].name, title);
            assert!(select(&header.replace("1.0_0", "2.0_0")).is_none());
            assert!(select(&header.replace("i:InfoKindVersion", "InfoKindVersion")).is_none());
            assert!(select(&(header + "<i:InfoKindVersion>1.0_0</i:InfoKindVersion>")).is_none());
        }
    }

    #[test]
    fn shared_element_and_attribute_selectors_are_combined() {
        let select =
            |profile: Vec<u8>| super::select_shared(XML, [("life".into(), profile)]).unwrap();
        assert!(select(DEFINITION.to_vec()).is_some());

        let mut profile: serde_json::Value = serde_json::from_slice(DEFINITION).unwrap();
        profile["root"]["elements"] = serde_json::json!([
            {"path": ["WCA00000"], "value": "架空保険株式会社"}
        ]);
        assert!(select(serde_json::to_vec(&profile).unwrap()).is_some());
        profile["root"]["attributes"]["VR"] = serde_json::json!("2.0");
        assert!(select(serde_json::to_vec(&profile).unwrap()).is_none());

        profile["root"]["attributes"]["VR"] = serde_json::json!("1.1");
        profile["root"]["elements"][0]["value"] = serde_json::json!("別の会社");
        assert!(select(serde_json::to_vec(&profile).unwrap()).is_none());
    }

    #[test]
    fn official_schema_fixture_uses_local_order_and_keeps_repeated_details() {
        let preview = read(XML, DEFINITION).unwrap();
        assert_eq!(preview.fields[1].name, "保険会社名");
        assert_eq!(preview.fields[1].value, "架空保険株式会社");
        assert_eq!(preview.fields[2].name, "契約者");
        for (record, amount) in [(1, "48,000 円"), (2, "126,000 円")] {
            assert!(
                preview
                    .fields
                    .iter()
                    .any(|field| field.name == format!("明細 {record}"))
            );
            assert!(preview.fields.iter().any(|field| {
                field.name == "保険料"
                    && field.value == amount
                    && field.path.contains(&format!("/WCE00000[{record}]/"))
            }));
        }
        assert!(preview.fields.iter().any(|field| field.name == "証明年"
            && field.value == "2026"
            && field.path.ends_with("/g:yyyy[1]")));
        let mut definition: serde_json::Value = serde_json::from_slice(DEFINITION).unwrap();
        definition["entries"].as_array_mut().unwrap().swap(0, 1);
        definition["entries"][2]["entries"]
            .as_array_mut()
            .unwrap()
            .swap(0, 3);
        let reordered = read(XML, &serde_json::to_vec(&definition).unwrap()).unwrap();
        assert_eq!(reordered.fields[1].name, "契約者");
        assert_eq!(reordered.fields[4].name, "保険料");
        assert_eq!(reordered.fields[4].value, "48,000 円");
        assert_eq!(reordered.fields.len(), preview.fields.len());
    }

    #[test]
    fn xml_view_uses_namespace_uris_and_checks_the_advertised_version() {
        let xml = std::str::from_utf8(XML)
            .unwrap()
            .replace("xmlns:g=", "xmlns:general=")
            .replace("<g:", "<general:")
            .replace("</g:", "</general:");
        assert!(read(xml.as_bytes(), DEFINITION).is_ok());
        assert!(
            read(
                xml.replace("VR=\"1.1\"", "VR=\"2.0\"").as_bytes(),
                DEFINITION
            )
            .is_err()
        );
        let xml = br#"<!DOCTYPE TEG800 [<!ENTITY text "forbidden">]><TEG800>&text;</TEG800>"#;
        assert!(read(xml, DEFINITION).is_err());
    }
}

fn group_integer(text: &str) -> String {
    let digits = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('+'))
        .unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return text.into();
    }
    let mut result = text[..text.len() - digits.len()].to_owned();
    for (index, digit) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(char::from(digit));
    }
    result
}

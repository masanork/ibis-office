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

pub fn read(bytes: &[u8], definition: &[u8]) -> Result<Preview, String> {
    if bytes.len() > MAX_BYTES || definition.len() > MAX_BYTES {
        return Err(SIZE_LIMIT_ERROR.into());
    }
    let definition: Definition = serde_json::from_slice(definition)
        .map_err(|error| format!("Invalid XML view definition: {error}"))?;
    if definition.version != 1 || definition.title.trim().is_empty() {
        return Err("Unsupported XML view definition".into());
    }
    let doc = super::parse_xml(bytes)?;
    let root = doc.root_element();
    let actual = root.tag_name();
    if actual.name() != definition.root.name
        || actual.namespace().unwrap_or_default() != definition.root.namespace
        || definition
            .root
            .attributes
            .iter()
            .any(|(key, value)| root.attribute(key.as_str()) != Some(value))
    {
        return Err("XML does not match the root element or version in the view definition".into());
    }

    let mut fields = Vec::new();
    push(&mut fields, "", &definition.title, String::new())?;
    definition.render(root, &definition.entries, 0, &mut fields)?;
    Ok(Preview {
        format: "XML",
        fields,
        credential: None,
    })
}

impl Definition {
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
                        if node.children().any(|child| child.is_element()) {
                            return Err("An XML view field must refer to a leaf element".into());
                        }
                        let text: String = node
                            .children()
                            .filter(|child| child.is_text())
                            .filter_map(|child| child.text())
                            .collect();
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

// SPDX-License-Identifier: AGPL-3.0-only

use std::{collections::HashMap, ops::Range};

use serde::Deserialize;
use serde_json::value::RawValue;

use super::{Fields, Source, excerpt, parse_fields};

#[derive(Deserialize)]
struct Header {
    #[serde(rename = "bomFormat")]
    format: String,
    #[serde(rename = "specVersion")]
    version: String,
    metadata: Option<Metadata>,
}

#[derive(Deserialize)]
struct Metadata {
    component: Option<Subject>,
}

#[derive(Deserialize)]
struct Subject {
    name: Option<String>,
    version: Option<String>,
}

pub(super) struct Target {
    pub span: Range<usize>,
    pub label: String,
}

pub(super) struct Inventory {
    pub version: String,
    pub subject: String,
    pub components: usize,
    pub crypto_assets: usize,
    pub services: usize,
    pub dependency_entries: usize,
    references: HashMap<String, Option<Target>>,
}

#[derive(Clone, Copy)]
enum Entry {
    Document,
    Metadata,
    Component,
    Service,
    Dependency,
    Other,
}

impl Inventory {
    pub fn read(source: &Source) -> Option<Self> {
        let bytes = &source.bytes[source.records.first()?.clone()];
        let header: Header = serde_json::from_slice(bytes).ok()?;
        if header.format != "CycloneDX" || header.version.is_empty() {
            return None;
        }
        let subject = header.metadata.and_then(|metadata| metadata.component);
        let subject = subject
            .and_then(|subject| {
                let mut name = subject.name?;
                if let Some(version) = subject.version.filter(|version| !version.is_empty()) {
                    name.push(' ');
                    name.push_str(&version);
                }
                Some(name)
            })
            .unwrap_or_default();
        let mut inventory = Self {
            version: excerpt(&header.version, 32),
            subject: excerpt(&subject, 160),
            components: 0,
            crypto_assets: 0,
            services: 0,
            dependency_entries: 0,
            references: HashMap::new(),
        };
        let Ok(root) = serde_json::from_slice::<&RawValue>(bytes) else {
            return None;
        };
        let mut pending = vec![(root, Entry::Document)];
        while let Some((raw, entry)) = pending.pop() {
            let text = raw.get();
            let fields = parse_fields(
                text.as_bytes(),
                Fields {
                    offset: 0,
                    limit: usize::MAX,
                },
            );
            if text.starts_with('[') {
                pending.extend(fields.into_iter().filter_map(|(_, child)| {
                    matches!(child.get().as_bytes().first(), Some(b'{' | b'['))
                        .then_some((child, entry))
                }));
                continue;
            }
            let value = |key| {
                fields
                    .iter()
                    .find(|(name, _)| name == key)
                    .and_then(|(_, raw)| serde_json::from_str::<String>(raw.get()).ok())
            };
            match entry {
                Entry::Component => {
                    inventory.components += 1;
                    if value("type").as_deref() == Some("cryptographic-asset") {
                        inventory.crypto_assets += 1;
                    }
                }
                Entry::Service => inventory.services += 1,
                Entry::Dependency => inventory.dependency_entries += 1,
                _ => {}
            }
            if let Some(reference) = value("bom-ref").filter(|value| !value.is_empty()) {
                let target = Target {
                    span: source.span(raw),
                    label: excerpt(&value("name").unwrap_or_else(|| reference.clone()), 120),
                };
                inventory
                    .references
                    .entry(reference)
                    .and_modify(|target| *target = None)
                    .or_insert(Some(target));
            }
            pending.extend(fields.into_iter().filter_map(|(key, child)| {
                let kind = child.get().as_bytes().first()?;
                let entry = match (entry, key.as_str(), kind) {
                    (Entry::Document, "metadata", b'{') => Entry::Metadata,
                    (Entry::Metadata, "component", b'{') => Entry::Component,
                    (Entry::Document | Entry::Component, "components", b'[') => Entry::Component,
                    (Entry::Document | Entry::Service, "services", b'[') => Entry::Service,
                    (Entry::Document, "dependencies", b'[') => Entry::Dependency,
                    (_, _, b'{' | b'[') => Entry::Other,
                    _ => return None,
                };
                Some((child, entry))
            }));
        }
        Some(inventory)
    }

    pub fn target(&self, field: &str, parent: Option<&str>, reference: &str) -> Option<&Target> {
        let refers_to_object = field == "ref"
            || field.ends_with("Ref")
            || (field.starts_with('[')
                && parent.is_some_and(|name| {
                    matches!(name, "dependsOn" | "provides") || name.ends_with("Refs")
                }));
        if refers_to_object {
            self.references.get(reference)?.as_ref()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Inventory;
    use crate::structured_view::Source;

    #[test]
    fn summarizes_and_resolves_nested_cbom_assets() {
        let source = Source::new(
            include_bytes!("../../../../test/fixtures/cbom/payment-demo.cdx.json").to_vec(),
            false,
        );
        let inventory = Inventory::read(&source).unwrap();
        assert_eq!(inventory.subject, "Demo payment service");
        assert_eq!(inventory.crypto_assets, 5);
        let rsa = inventory.target("[0]", Some("dependsOn"), "rsa").unwrap();
        assert_eq!(rsa.label, "RSA-2048");
        let raw: serde_json::Value =
            serde_json::from_slice(&source.bytes[rsa.span.clone()]).unwrap();
        assert_eq!(raw["bom-ref"], "rsa");
        assert!(
            inventory
                .target("signatureAlgorithmRef", None, "rsa")
                .is_some()
        );
        assert!(inventory.target("name", None, "rsa").is_none());
    }

    #[test]
    fn ambiguous_and_external_references_are_not_resolved() {
        let source = Source::new(
            br#"{
            "bomFormat":"CycloneDX","specVersion":"1.6",
            "components":[{"bom-ref":"same"},{"bom-ref":"same"}],
            "services":[{"bom-ref":"service","name":"Catalog"}]
        }"#
            .to_vec(),
            false,
        );
        let inventory = Inventory::read(&source).unwrap();
        assert_eq!(inventory.components, 2);
        assert_eq!(inventory.services, 1);
        assert!(inventory.target("ref", None, "same").is_none());
        assert!(
            inventory
                .target("[0]", Some("provides"), "urn:cdx:example")
                .is_none()
        );
        assert_eq!(
            inventory.target("ref", None, "service").unwrap().label,
            "Catalog"
        );
    }
}

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_ooxml::Package;
use sha2::{Digest, Sha256};

use crate::Error;

const NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/bibliography";
const STRICT_NS: &str = "http://purl.oclc.org/ooxml/officeDocument/bibliography";
const CUSTOM_XML: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";
const CUSTOM_XML_PROPS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps";

pub(crate) fn is_custom_xml(kind: &str) -> bool {
    matches!(
        kind,
        CUSTOM_XML | "http://purl.oclc.org/ooxml/officeDocument/relationships/customXml"
    )
}

fn sources_namespace(xml: &str) -> Option<&'static str> {
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    match (root.tag_name().name(), root.tag_name().namespace()) {
        ("Sources", Some(NS)) => Some(NS),
        ("Sources", Some(STRICT_NS)) => Some(STRICT_NS),
        _ => None,
    }
}

pub(crate) fn is_sources(xml: &str) -> bool {
    sources_namespace(xml).is_some()
}

pub(crate) fn write_sources(
    package: &mut Package,
    main: &str,
    sources: &[String],
) -> Result<(), Error> {
    let mut old = Vec::new();
    for rel in package
        .relationships(main)?
        .into_iter()
        .filter(|rel| is_custom_xml(&rel.kind))
    {
        if let Some(part) = rel.target_part(main) {
            if package
                .read(&part)
                .ok()
                .and_then(|data| String::from_utf8(data).ok())
                .is_some_and(|xml| is_sources(&xml))
            {
                old.push((part, rel.id));
            }
        }
    }
    for ((part, _), xml) in old.iter().zip(sources) {
        if !is_sources(xml) {
            return Err(Error::new("Invalid Word bibliography Sources XML"));
        }
        if package.read(part)?.as_slice() != xml.as_bytes() {
            package.set_bytes(part, xml.as_bytes().to_vec());
        }
    }
    for (_, id) in old.iter().skip(sources.len()) {
        package.remove_relationship(main, id)?;
    }
    let mut candidates = 1..;
    for xml in sources.iter().skip(old.len()) {
        let namespace = sources_namespace(xml)
            .ok_or_else(|| Error::new("Invalid Word bibliography Sources XML"))?;
        let (part, props, props_name) = candidates
            .find_map(|n| {
                let part = format!("customXml/item{n}.xml");
                let props_name = format!("itemProps{n}.xml");
                let props = format!("customXml/{props_name}");
                (!package.contains(&part) && !package.contains(&props))
                    .then_some((part, props, props_name))
            })
            .expect("an unused custom XML part name");

        let digest = format!(
            "{:x}",
            Sha256::digest([part.as_bytes(), xml.as_bytes()].concat())
        );
        let id = format!(
            "{{{}-{}-{}-{}-{}}}",
            &digest[..8],
            &digest[8..12],
            &digest[12..16],
            &digest[16..20],
            &digest[20..32]
        );
        let props_xml = format!(
            r#"<ds:datastoreItem ds:itemID="{id}" xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"><ds:schemaRefs><ds:schemaRef ds:uri="{namespace}"/></ds:schemaRefs></ds:datastoreItem>"#
        );

        package.set_bytes(&part, xml.as_bytes().to_vec());
        package.set_bytes(&props, props_xml.into_bytes());
        package.set_override(&part, "application/xml")?;
        package.set_override(
            &props,
            "application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
        )?;
        package.add_relationship(main, CUSTOM_XML, &format!("/{part}"), false)?;
        package.add_relationship(&part, CUSTOM_XML_PROPS, &props_name, false)?;
    }
    Ok(())
}

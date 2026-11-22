// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/235)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! PDF/A identification from document-level XMP.

use crate::{CosObj, Error, PdfContainer, Result, xref::decode_stream_bytes_with_limit};

const ID: &str = "http://www.aiim.org/pdfa/ns/id/";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const METADATA_LIMIT: usize = 1024 * 1024;

/// A claim supplied by the file's author. No conformance checks have been run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Identification {
    pub part: Option<String>,
    pub conformance: Option<String>,
    pub revision: Option<String>,
}

impl Identification {
    pub fn declared_profile(&self) -> Option<&'static str> {
        match (self.part.as_deref()?, self.conformance.as_deref()) {
            ("1", Some("A")) => Some("PDF/A-1a"),
            ("1", Some("B")) => Some("PDF/A-1b"),
            ("2", Some("A")) => Some("PDF/A-2a"),
            ("2", Some("B")) => Some("PDF/A-2b"),
            ("2", Some("U")) => Some("PDF/A-2u"),
            ("3", Some("A")) => Some("PDF/A-3a"),
            ("3", Some("B")) => Some("PDF/A-3b"),
            ("3", Some("U")) => Some("PDF/A-3u"),
            ("4", None) => Some("PDF/A-4"),
            ("4", Some("E")) => Some("PDF/A-4e"),
            ("4", Some("F")) => Some("PDF/A-4f"),
            _ => None,
        }
    }
}

pub fn inspect(pdf: &mut PdfContainer) -> Result<Identification> {
    let (_, catalog) = pdf.catalog()?;
    let Some(metadata) = catalog.get(b"Metadata") else {
        return Ok(Identification::default());
    };
    let CosObj::Stream { dict, data } = pdf.deref(metadata)? else {
        return Err(Error::new("catalog /Metadata is not a stream"));
    };
    let bytes = decode_stream_bytes_with_limit(&dict, &data, pdf.raw_bytes(), METADATA_LIMIT)?;
    let xml = std::str::from_utf8(&bytes).map_err(|_| Error::new("XMP must be UTF-8"))?;
    parse_xmp(xml)
}

fn parse_xmp(xml: &str) -> Result<Identification> {
    let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}'))
        .map_err(|err| Error::new(format!("invalid XMP: {err}")))?;
    let root = document.root_element();
    let rdf = if root.has_tag_name((RDF, "RDF")) {
        root
    } else {
        let mut packets = root
            .children()
            .filter(|node| node.has_tag_name((RDF, "RDF")));
        let rdf = packets
            .next()
            .ok_or_else(|| Error::new("XMP is missing rdf:RDF"))?;
        if packets.next().is_some() {
            return Err(Error::new("multiple rdf:RDF elements in XMP"));
        }
        rdf
    };

    let mut result = Identification::default();
    for subject in rdf.children().filter(|node| {
        node.is_element()
            && node.attribute((RDF, "about")).unwrap_or("").is_empty()
            && node.attribute((RDF, "nodeID")).is_none()
            && node.attribute((RDF, "ID")).is_none()
    }) {
        for (name, value) in [
            ("part", &mut result.part),
            ("conformance", &mut result.conformance),
            ("rev", &mut result.revision),
        ] {
            if let Some(attribute) = subject.attribute((ID, name)) {
                set_property(value, name, attribute.trim().into())?;
            }
            for child in subject
                .children()
                .filter(|node| node.has_tag_name((ID, name)))
            {
                if child.children().any(|node| node.is_element())
                    || child.attribute((RDF, "resource")).is_some()
                    || child.attribute((RDF, "nodeID")).is_some()
                    || child.attribute((RDF, "parseType")).is_some()
                {
                    return Err(Error::new(format!(
                        "XMP pdfaid:{name} is not a simple value"
                    )));
                }
                let text: String = child
                    .children()
                    .filter(|node| node.is_text())
                    .filter_map(|node| node.text())
                    .collect();
                set_property(value, name, text.trim().into())?;
            }
        }
    }
    Ok(result)
}

fn set_property(slot: &mut Option<String>, name: &str, value: String) -> Result<()> {
    if slot.as_ref().is_some_and(|previous| previous != &value) {
        return Err(Error::new(format!("conflicting XMP pdfaid:{name} values")));
    }
    *slot = Some(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(description: &str) -> String {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="{RDF}"
            xmlns:archive="{ID}">{description}</rdf:RDF></x:xmpmeta>"#
        )
    }

    #[test]
    fn reads_attributes_and_elements_by_namespace() {
        let id = parse_xmp(&packet(
            r#"<rdf:Description rdf:about="" archive:part="4"
            archive:rev="2020"><archive:conformance> F </archive:conformance></rdf:Description>"#,
        ))
        .unwrap();
        assert_eq!(id.declared_profile(), Some("PDF/A-4f"));
        assert_eq!(id.revision.as_deref(), Some("2020"));
    }

    #[test]
    fn ignores_other_namespaces_and_rdf_subjects() {
        let id = parse_xmp(&packet(
            r#"<rdf:Description xmlns:pdfaid="urn:other" pdfaid:part="2"/>
            <rdf:Description rdf:about="elsewhere" archive:part="4"/>"#,
        ))
        .unwrap();
        assert_eq!(id, Identification::default());
    }

    #[test]
    fn never_guesses_from_unknown_combinations() {
        for (part, conformance, expected) in [
            ("1", Some("U"), None),
            ("2", None, None),
            ("3", Some("U"), Some("PDF/A-3u")),
            ("4", None, Some("PDF/A-4")),
            ("4", Some(""), None),
            ("5", Some("B"), None),
        ] {
            let id = Identification {
                part: Some(part.into()),
                conformance: conformance.map(String::from),
                revision: None,
            };
            assert_eq!(id.declared_profile(), expected);
        }
    }

    #[test]
    fn rejects_conflicting_or_malformed_xmp() {
        assert!(
            parse_xmp(&packet(
                r#"<rdf:Description archive:part="2">
            <archive:part>3</archive:part></rdf:Description>"#
            ))
            .is_err()
        );
        assert!(parse_xmp(&packet("<rdf:Description>")).is_err());
    }
}

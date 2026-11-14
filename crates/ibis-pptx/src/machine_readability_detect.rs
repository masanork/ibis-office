// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
//! Content evidence for machine-readability input; no model or runtime lookup.
//!
//! CSV and TSV overlap with each other and other text formats. Text therefore
//! deliberately does not claim a delimiter or reject JSON, XML, or one-column
//! tables. The caller selects the delimiter and the CSV parser diagnoses it.
use crate::{machine_readability_csv::decode_csv_bytes, zip::Archive};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    Workbook,
    Zip,
    Text,
    Binary,
    Unsupported,
}

const METADATA_LIMIT: usize = 1024 * 1024;
const OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

/// Confirm workbook metadata when it is small enough to inspect. An unknown ZIP
/// remains `Zip` and is handed to the real workbook parser, not presumed valid.
fn workbook(archive: &Archive<'_>) -> Option<()> {
    let rels = archive.read_bounded("_rels/.rels", METADATA_LIMIT).ok()??;
    let types = archive
        .read_bounded("[Content_Types].xml", METADATA_LIMIT)
        .ok()??;
    let rels = roxmltree::Document::parse(std::str::from_utf8(&rels).ok()?).ok()?;
    let node = rels.descendants().find(|n| {
        n.is_element()
            && n.tag_name().name() == "Relationship"
            && n.attribute("Type") == Some(OFFICE_DOCUMENT)
            && n.attribute("TargetMode") != Some("External")
    })?;
    let part = node.attribute("Target")?.trim_start_matches('/');
    if part.split('/').any(|s| s == "..") || !archive.contains(part) {
        return None;
    }
    let types = roxmltree::Document::parse(std::str::from_utf8(&types).ok()?).ok()?;
    types
        .descendants()
        .any(|n| {
            n.is_element()
                && n.tag_name().name() == "Override"
                && n.attribute("PartName").map(|s| s.trim_start_matches('/')) == Some(part)
                && matches!(
                    n.attribute("ContentType"),
                    Some(
                        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
                            | "application/vnd.ms-excel.sheet.macroEnabled.main+xml"
                    )
                )
        })
        .then_some(())
}

pub fn detect(bytes: &[u8]) -> Content {
    if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        return match Archive::new(bytes) {
            Ok(archive) if workbook(&archive).is_some() => Content::Workbook,
            Ok(_) => Content::Zip,
            Err(_) => Content::Binary,
        };
    }
    // Strong signatures are evidence of binary input; they are not a general
    // file-type registry. Avoid scanning inside text (e.g. quoted "%PDF-").
    if bytes.starts_with(b"%PDF-")
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1")
        || bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(b"\x1f\x8b\x08")
    {
        return Content::Binary;
    }
    if bytes.starts_with(b"\xff\xfe") || bytes.starts_with(b"\xfe\xff") || bytes.contains(&0) {
        return Content::Unsupported;
    }
    if decode_csv_bytes(bytes).is_ok() {
        Content::Text
    } else {
        Content::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_is_not_forced_into_one_format() {
        for bytes in [
            b"\"alpha\tbeta\"\n\"gamma\tdelta\"\n".as_slice(),
            b"[1,2,3]\n",
            b"<?xml version=\"1.0\"?><data/>",
            b"a,b\n1,2\n",
            b"",
        ] {
            assert_eq!(detect(bytes), Content::Text);
        }
        assert_eq!(
            detect(&[0x93, 0xfa, 0x96, 0x7b, b',', b'1', b'\n']),
            Content::Text
        );
        assert_eq!(detect(&[0xff, 0xfe, b'a', 0]), Content::Unsupported);
    }
    #[test]
    fn signatures_and_broken_zip() {
        assert_eq!(detect(b"GIF89a\0\0"), Content::Binary);
        assert_eq!(detect(b"%PDF-1.7\n"), Content::Binary);
        assert_eq!(detect(b"PK\x03\x04broken"), Content::Binary);
        assert_eq!(detect(b"\"the %PDF- marker\""), Content::Text);
    }
    #[test]
    fn spreadsheet_fixture_corpus() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test/fixtures/machine-readability");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "xlsx") {
                assert_eq!(
                    detect(&std::fs::read(&path).unwrap()),
                    Content::Workbook,
                    "{}",
                    path.display()
                );
                count += 1;
            }
        }
        assert!(count >= 20);
    }

    #[test]
    fn real_office_packages() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test");
        for (name, expected) in [
            (
                "fixtures/machine-readability/structure.xlsx",
                Content::Workbook,
            ),
            (
                "benchmark/securities-registration-statement.docx",
                Content::Zip,
            ),
            ("benchmark/earnings-presentation.pptx", Content::Zip),
        ] {
            assert_eq!(
                detect(&std::fs::read(root.join(name)).unwrap()),
                expected,
                "{name}"
            );
        }
    }
}

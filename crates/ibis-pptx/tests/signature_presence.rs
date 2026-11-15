// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_pptx::ibis_ooxml::{Package, SaveOptions};

#[test]
fn opened_source_reports_signature_presence_and_malformed_origin() {
    let original = include_bytes!("../../../test/corpus-report/docx/list-justify-cell-synthetic.docx");
    let opened = ibis_pptx::open(original.as_slice()).unwrap();
    assert_eq!(opened.source.has_package_signatures(), Ok(false));
    let mut pkg = Package::open(original.as_slice()).unwrap();
    let prefix = "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/";
    pkg.set_bytes("_xmlsignatures/origin.sigs", Vec::new());
    pkg.set_bytes("_xmlsignatures/sig1.xml", b"<not-verified/>".to_vec());
    pkg.add_relationship("", &format!("{prefix}origin"), "_xmlsignatures/origin.sigs", false).unwrap();
    pkg.add_relationship("_xmlsignatures/origin.sigs", &format!("{prefix}signature"), "sig1.xml", false).unwrap();
    let opened = ibis_pptx::open(pkg.save(SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(opened.source.has_package_signatures(), Ok(true));
    pkg.remove("_xmlsignatures/sig1.xml");
    let opened = ibis_pptx::open(pkg.save(SaveOptions::default()).unwrap()).unwrap();
    assert!(opened.source.has_package_signatures().is_err());
}

// SPDX-License-Identifier: AGPL-3.0-only
// The fixture is synthetic and contains no real document content.
use ibis_pptx::jtd::decode::decode;

#[test]
fn decodes_synthetic_document_text_stream() {
    let file = include_bytes!("../../../test/fixtures/jtd/synthetic.jtd");
    let stream = ibis_ooxml::cfb::stream(file, "DocumentText").unwrap();
    let decoded = decode(&stream).unwrap();
    assert!(String::from_utf16_lossy(&decoded.units).contains("日本語 test"));
}

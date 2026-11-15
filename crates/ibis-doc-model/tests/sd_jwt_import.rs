// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use ibis_doc_model::{
    CborValue, Document, EmuRect, Es256KeyPair, ParsedCredentialFormat, Presentation,
    SdJwtOptions, SdJwtTemplateKind, Shape, ShapeId, ShapeKind, Sheet, SheetState, Slide, SlideId,
    Workbook, WorkbookSheet, base64url_encode, build_sd_jwt_template_commands, encode_cbor,
    extract_sheet_claims, issue_sheet_sd_jwt, parse_mdoc_text, parse_sd_jwt, sha256,
};

fn blank_worksheet() -> (Presentation, SlideId) {
    let slide_id = SlideId(1);
    let mut slide = Slide::new(slide_id);
    slide.shapes.push(Shape::new(
        ShapeId(1),
        EmuRect::new(0, 0, 1_000_000, 1_000_000),
        ShapeKind::Sheet(Arc::new(Sheet::default())),
    ));
    let mut wb = Workbook::default();
    wb.sheets.push(WorkbookSheet {
        name: "Sheet1".to_string(),
        state: SheetState::Visible,
        slide: Some(slide_id),
        hidden: None,
        other: false,
    });
    let mut pres = Presentation::new();
    pres.slides = vec![slide];
    pres.workbook = Some(Arc::new(wb));
    (pres, slide_id)
}

#[test]
fn parse_sd_jwt_verifies_signature_digests_kb_hash_and_imports_to_sheet() {
    let (pres, slide) = blank_worksheet();
    let mut doc = Document::new(pres);
    let pid_cmds = build_sd_jwt_template_commands(
        doc.presentation(),
        slide,
        SdJwtTemplateKind::Pid,
    );
    doc.execute(pid_cmds).unwrap();

    let issuer_key = Es256KeyPair::demo_issuer_key();
    let holder_key = Es256KeyPair::demo_holder_key();
    let mut opts = SdJwtOptions::default();
    opts.holder_public_jwk = Some(holder_key.to_public_jwk());

    let (_bindings, sd_out) =
        issue_sheet_sd_jwt(doc.presentation(), slide, &opts, &issuer_key).unwrap();
    let kb_out = sd_out.attach_key_binding(
        "https://verifier.example.com",
        "nonce-123",
        1_700_000_000,
        &holder_key,
    );

    let parsed = parse_sd_jwt(&kb_out.vp_token, Some(&issuer_key.to_public_jwk().to_json()))
        .expect("parse SD-JWT with KB-JWT");
    assert_eq!(parsed.format, ParsedCredentialFormat::SdJwt);
    assert_eq!(parsed.vct_or_doctype, "urn:eudi:pid:jp:1");
    assert_eq!(parsed.signature_valid, Some(true));
    assert_eq!(parsed.kb_sd_hash_valid, Some(true));
    assert!(parsed.digests_total >= 4);
    assert!(parsed.digests_verified >= 4 && parsed.digests_verified <= parsed.digests_total);

    // Import into a fresh worksheet and verify bindings roundtrip
    let (pres2, slide2) = blank_worksheet();
    let mut doc2 = Document::new(pres2);
    let import_cmds = parsed.build_sheet_commands(doc2.presentation(), slide2);
    doc2.execute(import_cmds).unwrap();

    let extracted = extract_sheet_claims(doc2.presentation(), slide2).expect("extracted claims");
    assert!(extracted.claim("family_name").is_some());
    assert!(extracted.claim("address.country").is_some());
    assert!(extracted.claim("address.region").is_some());
    assert!(extracted.claim("address.locality").is_none());
    assert!(extracted.claim("nationalities").is_some());
}

#[test]
fn parse_mdoc_device_response_verifies_cose_mso_and_imports_to_sheet() {
    let issuer_key = Es256KeyPair::demo_issuer_key();
    let ns = "org.iso.18013.5.1";

    let make_item = |digest_id: u64, id: &str, val: CborValue| -> (CborValue, Vec<u8>) {
        let item = CborValue::Map(vec![
            (CborValue::Text("digestID".into()), CborValue::Unsigned(digest_id)),
            (CborValue::Text("random".into()), CborValue::Bytes(vec![0xAA; 16])),
            (CborValue::Text("elementIdentifier".into()), CborValue::Text(id.into())),
            (CborValue::Text("elementValue".into()), val),
        ]);
        let tagged = CborValue::Tag(24, Box::new(CborValue::Bytes(encode_cbor(&item))));
        let digest = sha256(&encode_cbor(&tagged)).to_vec();
        (tagged, digest)
    };

    let (item0, d0) = make_item(0, "family_name", CborValue::Text("YAMADA".into()));
    let (item1, d1) = make_item(
        1,
        "birth_date",
        CborValue::Tag(1004, Box::new(CborValue::Text("1990-05-15".into()))),
    );
    let (item2, d2) = make_item(
        2,
        "driving_privileges",
        CborValue::Array(vec![CborValue::Map(vec![
            (CborValue::Text("vehicle_category_code".into()), CborValue::Text("B".into())),
            (CborValue::Text("issue_date".into()), CborValue::Text("2020-04-01".into())),
        ])]),
    );

    let mso = CborValue::Map(vec![
        (CborValue::Text("version".into()), CborValue::Text("1.0".into())),
        (CborValue::Text("digestAlgorithm".into()), CborValue::Text("SHA-256".into())),
        (CborValue::Text("docType".into()), CborValue::Text("org.iso.18013.5.1.mDL".into())),
        (
            CborValue::Text("valueDigests".into()),
            CborValue::Map(vec![(
                CborValue::Text(ns.into()),
                CborValue::Map(vec![
                    (CborValue::Unsigned(0), CborValue::Bytes(d0)),
                    (CborValue::Unsigned(1), CborValue::Bytes(d1)),
                    (CborValue::Unsigned(2), CborValue::Bytes(d2)),
                ]),
            )]),
        ),
    ]);
    let mso_bytes = encode_cbor(&CborValue::Tag(24, Box::new(CborValue::Bytes(encode_cbor(&mso)))));
    let protected = encode_cbor(&CborValue::Map(vec![(
        CborValue::Unsigned(1),
        CborValue::Negative(-7), // ES256
    )]));
    let sig_structure = CborValue::Array(vec![
        CborValue::Text("Signature1".into()),
        CborValue::Bytes(protected.clone()),
        CborValue::Bytes(Vec::new()),
        CborValue::Bytes(mso_bytes.clone()),
    ]);
    let sig = issuer_key.sign_es256(&encode_cbor(&sig_structure));

    let cose_sign1 = CborValue::Array(vec![
        CborValue::Bytes(protected),
        CborValue::Map(vec![]),
        CborValue::Bytes(mso_bytes),
        CborValue::Bytes(sig.to_vec()),
    ]);

    let device_response = CborValue::Map(vec![
        (CborValue::Text("version".into()), CborValue::Text("1.0".into())),
        (
            CborValue::Text("documents".into()),
            CborValue::Array(vec![CborValue::Map(vec![
                (CborValue::Text("docType".into()), CborValue::Text("org.iso.18013.5.1.mDL".into())),
                (
                    CborValue::Text("issuerSigned".into()),
                    CborValue::Map(vec![
                        (
                            CborValue::Text("nameSpaces".into()),
                            CborValue::Map(vec![(
                                CborValue::Text(ns.into()),
                                CborValue::Array(vec![item0, item1, item2]),
                            )]),
                        ),
                        (CborValue::Text("issuerAuth".into()), cose_sign1),
                    ]),
                ),
            ])]),
        ),
    ]);

    let b64 = base64url_encode(&encode_cbor(&device_response));
    let parsed = parse_mdoc_text(&b64, Some(&issuer_key.to_public_jwk().to_json()))
        .expect("parse mDOC DeviceResponse");
    assert_eq!(parsed.format, ParsedCredentialFormat::Mdoc);
    assert_eq!(parsed.vct_or_doctype, "org.iso.18013.5.1.mDL");
    assert_eq!(parsed.signature_valid, Some(true));
    assert_eq!(parsed.digests_total, 3);
    assert_eq!(parsed.digests_verified, 3);

    // Import into worksheet and verify driving_privileges 2D table roundtrip
    let (pres, slide) = blank_worksheet();
    let mut doc = Document::new(pres);
    let cmds = parsed.build_sheet_commands(doc.presentation(), slide);
    doc.execute(cmds).unwrap();

    let extracted = extract_sheet_claims(doc.presentation(), slide).expect("extracted mDOC claims");
    assert_eq!(
        extracted.claim("family_name").unwrap().value.as_str(),
        Some("YAMADA")
    );
    assert_eq!(
        extracted.claim("birth_date").unwrap().value.as_str(),
        Some("1990-05-15")
    );
    assert!(extracted.claim("driving_privileges").is_some());
}

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::sync::Arc;

use ibis_doc_model::{
    CellKind, CellStyle, CellValue, ClaimValue, ControlKind, DefinedName, Document, EmuRect,
    FormControl, Presentation, Shape, ShapeId, ShapeKind, Sheet, SheetCell, Slide, SlideId,
    Workbook, WorkbookSheet, extract_sheet_claims,
};

fn put_cell(sheet: &mut Sheet, row: u32, column: u32, value: CellValue, text: &str, style: u32) {
    let mut cells = (*sheet.cells).clone();
    cells.push(SheetCell {
        row,
        column,
        kind: CellKind::of(&value),
        value,
        text: text.to_string(),
        formula: None,
        style,
        phonetic: None,
        runs: None,
    });
    cells.sort_by_key(|c| (c.row, c.column));
    sheet.cells = cells.into();
}

fn sample_presentation() -> (Presentation, SlideId, ShapeId) {
    let slide_id = SlideId(1);
    let mut sheet = Sheet::default();
    let general_style = CellStyle::default();
    let mut date_style = CellStyle::default();
    date_style.number_format = "yyyy-mm-dd".to_string();
    let mut checkbox_style = CellStyle::default();
    checkbox_style.checkbox = true;
    sheet.styles = vec![general_style, date_style, checkbox_style];

    put_cell(
        &mut sheet,
        0,
        1,
        CellValue::Text("https://credentials.example.com/identity_credential".into()),
        "https://credentials.example.com/identity_credential",
        0,
    );
    put_cell(
        &mut sheet,
        1,
        1,
        CellValue::Text("https://issuer.example.com".into()),
        "https://issuer.example.com",
        0,
    );
    put_cell(&mut sheet, 2, 1, CellValue::Number(1_700_000_000.0), "1700000000", 1);
    put_cell(&mut sheet, 3, 1, CellValue::Text("山田".into()), "山田", 0);
    put_cell(&mut sheet, 4, 1, CellValue::Text("太郎".into()), "太郎", 0);
    let serial_1990_05_15 = ibis_calc::eval::convert::serial_from_date(1990, 5, 15, false) as f64;
    put_cell(&mut sheet, 5, 1, CellValue::Number(serial_1990_05_15), "1990-05-15", 1);
    put_cell(&mut sheet, 6, 1, CellValue::Text("JP".into()), "JP", 0);
    put_cell(&mut sheet, 7, 1, CellValue::Number(2.0), "2", 0);
    put_cell(&mut sheet, 8, 1, CellValue::Bool(true), "TRUE", 2);
    put_cell(&mut sheet, 9, 1, CellValue::Empty, "", 0);
    put_cell(&mut sheet, 4, 2, CellValue::Bool(false), "FALSE", 0);

    put_cell(&mut sheet, 0, 5, CellValue::Text("東京都".into()), "東京都", 0);
    put_cell(&mut sheet, 1, 5, CellValue::Text("大阪府".into()), "大阪府", 0);
    put_cell(&mut sheet, 2, 5, CellValue::Text("北海道".into()), "北海道", 0);

    let mut slide = Slide::new(slide_id);
    slide.shapes.push(Shape::new(
        ShapeId(1),
        EmuRect::new(0, 0, 1_000_000, 1_000_000),
        ShapeKind::Sheet(Arc::new(sheet)),
    ));

    let mut drop = FormControl::new(ControlKind::Drop, "sd.address.region");
    drop.link = Some("$B$8".into());
    drop.range = Some("$F$1:$F$3".into());
    drop.value = 2;
    slide.shapes.push(Shape::new(
        ShapeId(10),
        EmuRect::new(0, 0, 500_000, 200_000),
        ShapeKind::Control(Arc::new(drop)),
    ));

    let mut radio1 = FormControl::new(ControlKind::Radio, "sd.gender");
    radio1.first_button = true;
    radio1.text = "male".into();
    radio1.checked = 0;
    slide.shapes.push(Shape::new(
        ShapeId(11),
        EmuRect::new(0, 0, 400_000, 200_000),
        ShapeKind::Control(Arc::new(radio1)),
    ));
    let mut radio2 = FormControl::new(ControlKind::Radio, "Radio Button 2");
    radio2.first_button = false;
    radio2.text = "female".into();
    radio2.checked = 1;
    slide.shapes.push(Shape::new(
        ShapeId(12),
        EmuRect::new(0, 0, 400_000, 200_000),
        ShapeKind::Control(Arc::new(radio2)),
    ));

    let disclose_gn_id = ShapeId(20);
    let mut chk = FormControl::new(ControlKind::CheckBox, "disclose.given_name");
    chk.text = "名を開示".into();
    chk.link = Some("$C$5".into());
    chk.checked = 0;
    slide.shapes.push(Shape::new(
        disclose_gn_id,
        EmuRect::new(0, 0, 400_000, 200_000),
        ShapeKind::Control(Arc::new(chk)),
    ));

    let mut wb = Workbook::default();
    wb.sheets.push(WorkbookSheet {
        name: "Sheet1".into(),
        state: ibis_doc_model::SheetState::Visible,
        slide: Some(slide_id),
        hidden: None,
        other: false,
    });
    wb.names = vec![
        DefinedName { name: "claim.vct".into(), sheet: None, formula: "Sheet1!$B$1".into(), hidden: false },
        DefinedName { name: "claim.iss".into(), sheet: None, formula: "Sheet1!$B$2".into(), hidden: false },
        DefinedName { name: "claim.iat".into(), sheet: None, formula: "Sheet1!$B$3".into(), hidden: false },
        DefinedName { name: "sd.family_name".into(), sheet: None, formula: "Sheet1!$B$4".into(), hidden: false },
        DefinedName { name: "sd.given_name".into(), sheet: None, formula: "Sheet1!$B$5".into(), hidden: false },
        DefinedName { name: "sd.birthdate".into(), sheet: None, formula: "Sheet1!$B$6".into(), hidden: false },
        DefinedName { name: "sd.address.country".into(), sheet: None, formula: "Sheet1!$B$7".into(), hidden: false },
        DefinedName { name: "sd.prefecture_from_cell".into(), sheet: None, formula: "Sheet1!$B$8".into(), hidden: false },
        DefinedName { name: "sd.age_over_18".into(), sheet: None, formula: "Sheet1!$B$9".into(), hidden: false },
        DefinedName { name: "sd.middle_name".into(), sheet: None, formula: "Sheet1!$B$10".into(), hidden: false },
    ];

    let mut pres = Presentation::new();
    pres.slides = vec![slide];
    pres.workbook = Some(Arc::new(wb));
    (pres, slide_id, disclose_gn_id)
}

#[test]
fn extracts_claims_nested_objects_dates_controls_and_disclosure_toggles() {
    let (pres, slide_id, _chk_id) = sample_presentation();
    let bindings = extract_sheet_claims(&pres, slide_id).expect("extract bindings");

    assert!(bindings.claim("middle_name").is_none());

    let vct = bindings.claim("vct").unwrap();
    assert!(!vct.selective);
    assert_eq!(vct.value.as_str(), Some("https://credentials.example.com/identity_credential"));
    let iat = bindings.claim("iat").unwrap();
    assert!(!iat.selective);
    assert_eq!(iat.value, ClaimValue::Int(1_700_000_000));

    let family_name = bindings.claim("family_name").unwrap();
    assert!(family_name.selective);
    assert!(family_name.disclose);
    assert_eq!(family_name.value.as_str(), Some("山田"));

    let given_name = bindings.claim("given_name").unwrap();
    assert!(given_name.selective);
    assert!(!given_name.disclose);
    assert_eq!(given_name.value.as_str(), Some("太郎"));

    let birthdate = bindings.claim("birthdate").unwrap();
    assert_eq!(birthdate.value.as_str(), Some("1990-05-15"));

    let region = bindings.claim("address.region").unwrap();
    assert_eq!(region.value.as_str(), Some("大阪府"));
    let pref = bindings.claim("prefecture_from_cell").unwrap();
    assert_eq!(pref.value.as_str(), Some("大阪府"));

    let gender = bindings.claim("gender").unwrap();
    assert_eq!(gender.value.as_str(), Some("female"));

    let age_over_18 = bindings.claim("age_over_18").unwrap();
    assert_eq!(age_over_18.value, ClaimValue::Bool(true));

    let plain = bindings.to_plain_claims_json();
    let disclosed = bindings.to_disclosed_claims_json();
    assert!(plain.as_object().unwrap().contains_key("given_name"));
    assert!(!disclosed.as_object().unwrap().contains_key("given_name"));

    let addr = disclosed.as_object().unwrap().get("address").unwrap().as_object().unwrap();
    assert_eq!(addr.get("country").and_then(ClaimValue::as_str), Some("JP"));
    assert_eq!(addr.get("region").and_then(ClaimValue::as_str), Some("大阪府"));
    assert!(disclosed.to_json().contains(r#""address":{"country":"JP","region":"大阪府"}"#));
}

#[test]
fn set_disclose_commands_updates_checkbox_control_and_linked_cell() {
    let (pres, slide_id, chk_id) = sample_presentation();
    let bindings = extract_sheet_claims(&pres, slide_id).unwrap();
    assert!(!bindings.claim("given_name").unwrap().disclose);

    let cmds = bindings.set_disclose_commands(&pres, &["given_name"], false);
    assert_eq!(cmds.len(), 2);

    let mut doc = Document::new(pres);
    doc.execute(cmds).unwrap();

    let ShapeKind::Control(c) = &doc.presentation().slide(slide_id).unwrap().shape(chk_id).unwrap().kind else {
        panic!("expected control");
    };
    assert_eq!(c.checked, 1);

    let updated = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
    assert!(updated.claim("given_name").unwrap().disclose);
}

#[test]
fn rfc7515_es256_jwk_and_signature_verification() {
    use ibis_doc_model::{Es256KeyPair, base64url_decode, verify_es256};

    // RFC 7515 Appendix A.3.1 private JWK
    let rfc7515_jwk = r#"{
        "kty":"EC",
        "crv":"P-256",
        "x":"f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU",
        "y":"x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0",
        "d":"jpsQnnGQmL-YBIffH1136cspYG6-0iY7X1fCE9-E9LI"
    }"#;
    let kp = Es256KeyPair::from_jwk_json(rfc7515_jwk).expect("RFC 7515 A.3 JWK should parse and match (x, y) = d * G");

    // Verify RFC 7515 Appendix A.3.2 signature with our verifier
    let signing_input = b"eyJhbGciOiJFUzI1NiJ9.eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ";
    let rfc_sig_b64 = "DtEhU3ljbEg8L38VWAfUAqOyKAM6-Xx-F4GawxaepmXFCgfTjDxw5djxLa8ISlSApmWQxfKTUJqPP3-Kg6NU1Q";
    let rfc_sig: [u8; 64] = base64url_decode(rfc_sig_b64).unwrap().try_into().unwrap();
    assert!(verify_es256(&kp.x, &kp.y, signing_input, &rfc_sig));

    // Also sign with RFC 6979 deterministic ES256 and verify
    let our_sig = kp.sign_es256(signing_input);
    assert!(verify_es256(&kp.x, &kp.y, signing_input, &our_sig));
}

#[test]
fn issues_rfc9901_sd_jwt_with_nested_digests_and_selective_presentation() {
    use ibis_doc_model::{
        Es256KeyPair, SdJwtOptions, base64url_decode, issue_sd_jwt, issue_sheet_sd_jwt,
        verify_es256,
    };

    let (pres, slide_id, _chk_id) = sample_presentation();
    let issuer_key = Es256KeyPair::demo_issuer_key();
    let holder_key = Es256KeyPair::demo_holder_key();

    let mut opts = SdJwtOptions::default();
    opts.holder_public_jwk = Some(holder_key.to_public_jwk());
    opts.salt_overrides
        .insert("family_name".into(), "salt_family_1234".into());
    opts.salt_overrides
        .insert("given_name".into(), "salt_given_5678".into());

    let (bindings, out) = issue_sheet_sd_jwt(&pres, slide_id, &opts, &issuer_key).expect("issue SD-JWT");

    // 1. Verify ES256 signature on issuer_jwt
    let jwt_parts: Vec<&str> = out.issuer_jwt.split('.').collect();
    assert_eq!(jwt_parts.len(), 3);
    let signing_input = format!("{}.{}", jwt_parts[0], jwt_parts[1]);
    let sig_bytes: [u8; 64] = base64url_decode(jwt_parts[2]).unwrap().try_into().unwrap();
    assert!(verify_es256(
        &issuer_key.x,
        &issuer_key.y,
        signing_input.as_bytes(),
        &sig_bytes
    ));

    // 2. Payload contains `_sd_alg: "sha-256"`, top-level `_sd`, nested `address._sd`, and `cnf.jwk`
    let payload_obj = out.payload.as_object().unwrap();
    assert_eq!(payload_obj.get("_sd_alg").and_then(ClaimValue::as_str), Some("sha-256"));
    assert!(payload_obj.contains_key("cnf"));
    let top_sd = match payload_obj.get("_sd").unwrap() {
        ClaimValue::Array(a) => a,
        _ => panic!("expected top-level _sd array"),
    };
    // Top-level selective claims: family_name, given_name, birthdate, prefecture_from_cell, age_over_18, gender (6 items)
    assert_eq!(top_sd.len(), 6);

    let addr_obj = payload_obj.get("address").unwrap().as_object().unwrap();
    let addr_sd = match addr_obj.get("_sd").unwrap() {
        ClaimValue::Array(a) => a,
        _ => panic!("expected nested address._sd array"),
    };
    // Nested selective claims under `address`: country, region (2 items)
    assert_eq!(addr_sd.len(), 2);

    // 3. Presentation SD-JWT excludes `given_name` disclosure while `disclose.given_name` is false
    let gn_disclosure = out
        .disclosures
        .iter()
        .find(|d| d.path_str == "given_name")
        .unwrap();
    assert!(!gn_disclosure.disclose);
    assert!(out.sd_jwt_all.contains(&gn_disclosure.encoded));
    assert!(!out.sd_jwt_presentation.contains(&gn_disclosure.encoded));

    // 4. After enabling `given_name` via `set_disclose_commands`, `sd_jwt_presentation` includes it
    let mut doc = Document::new(pres);
    let cmds = bindings.set_disclose_commands(doc.presentation(), &["given_name"], false);
    doc.execute(cmds).unwrap();
    let updated_bindings = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
    let updated_out = issue_sd_jwt(&updated_bindings, &opts, &issuer_key);
    assert!(updated_out.sd_jwt_presentation.contains(&gn_disclosure.encoded));
}

#[test]
fn oid4vp_request_synchronizes_sheet_checkboxes_and_issues_kb_jwt() {
    use ibis_doc_model::{
        Es256KeyPair, Oid4vpRequest, SdJwtOptions, base64url_decode, base64url_encode,
        issue_sheet_sd_jwt, sha256, verify_es256,
    };

    // 1. Parse OID4VP URI with URL-encoded DCQL query
    let dcql_uri = "openid4vp://?client_id=https%3A%2F%2Fverifier.example.org%2Fcb&nonce=nonce-xyz-987&dcql_query=%7B%22credentials%22%3A%5B%7B%22id%22%3A%22pid%22%2C%22claims%22%3A%5B%7B%22path%22%3A%5B%22given_name%22%5D%7D%2C%7B%22path%22%3A%5B%22address%22%2C%22region%22%5D%7D%5D%7D%5D%7D";
    let req = Oid4vpRequest::parse(dcql_uri).expect("parse OID4VP DCQL URI");
    assert_eq!(req.client_id, "https://verifier.example.org/cb");
    assert_eq!(req.nonce, "nonce-xyz-987");
    assert_eq!(
        req.requested_claims,
        vec!["given_name".to_string(), "address.region".to_string()]
    );

    // 2. Also verify DIF Presentation Exchange JSON parsing
    let pe_json = r#"{
        "client_id": "https://pe-verifier.example.com",
        "nonce": "pe-nonce-111",
        "presentation_definition": {
            "id": "vp_req",
            "input_descriptors": [{
                "id": "id_card",
                "constraints": {
                    "fields": [
                        { "path": ["$.family_name", "$.address.country"] }
                    ]
                }
            }]
        }
    }"#;
    let pe_req = Oid4vpRequest::parse(pe_json).expect("parse OID4VP PE JSON");
    assert_eq!(pe_req.client_id, "https://pe-verifier.example.com");
    assert_eq!(pe_req.nonce, "pe-nonce-111");
    assert_eq!(
        pe_req.requested_claims,
        vec!["family_name".to_string(), "address.country".to_string()]
    );

    // 3. Synchronize worksheet checkboxes from `req` and issue SD-JWT + Key Binding JWT (`kb+jwt`)
    let (pres, slide_id, chk_id) = sample_presentation();
    let initial_bindings = extract_sheet_claims(&pres, slide_id).unwrap();
    let req_refs: Vec<&str> = req.requested_claims.iter().map(String::as_str).collect();
    let cmds = initial_bindings.set_disclose_commands(&pres, &req_refs, true);
    let mut doc = Document::new(pres);
    doc.execute(cmds).unwrap();

    // `disclose.given_name` checkbox on the sheet was turned ON automatically
    let ShapeKind::Control(c) = &doc.presentation().slide(slide_id).unwrap().shape(chk_id).unwrap().kind else {
        panic!("expected CheckBox control");
    };
    assert_eq!(c.checked, 1);

    let issuer_key = Es256KeyPair::demo_issuer_key();
    let holder_key = Es256KeyPair::demo_holder_key();
    let mut opts = SdJwtOptions::default();
    opts.holder_public_jwk = Some(holder_key.to_public_jwk());

    let (_bindings, sd_jwt_out) =
        issue_sheet_sd_jwt(doc.presentation(), slide_id, &opts, &issuer_key).unwrap();
    let kb_out = sd_jwt_out.attach_key_binding(
        &req.client_id,
        &req.nonce,
        1_700_000_123,
        &holder_key,
    );

    // Verify `sd_hash` matches SHA-256 of `sd_jwt_presentation`
    let expected_sd_hash = base64url_encode(&sha256(sd_jwt_out.sd_jwt_presentation.as_bytes()));
    assert_eq!(kb_out.sd_hash, expected_sd_hash);
    assert_eq!(
        kb_out.vp_token,
        format!("{}{}", sd_jwt_out.sd_jwt_presentation, kb_out.kb_jwt)
    );

    // Verify Holder ES256 signature on `kb_jwt` and its header/payload contents
    let kb_parts: Vec<&str> = kb_out.kb_jwt.split('.').collect();
    assert_eq!(kb_parts.len(), 3);
    let kb_header = String::from_utf8(base64url_decode(kb_parts[0]).unwrap()).unwrap();
    assert!(kb_header.contains("\"typ\":\"kb+jwt\""));
    let kb_payload = String::from_utf8(base64url_decode(kb_parts[1]).unwrap()).unwrap();
    assert!(kb_payload.contains("\"aud\":\"https://verifier.example.org/cb\""));
    assert!(kb_payload.contains("\"nonce\":\"nonce-xyz-987\""));
    assert!(kb_payload.contains(&format!("\"sd_hash\":\"{}\"", expected_sd_hash)));

    let kb_signing_input = format!("{}.{}", kb_parts[0], kb_parts[1]);
    let kb_sig: [u8; 64] = base64url_decode(kb_parts[2]).unwrap().try_into().unwrap();
    assert!(verify_es256(
        &holder_key.x,
        &holder_key.y,
        kb_signing_input.as_bytes(),
        &kb_sig
    ));
}

#[test]
fn pid_and_iso18013_mdl_templates_and_range_bindings_roundtrip() {
    use ibis_doc_model::{
        CellRange, Es256KeyPair, SdJwtOptions, SdJwtTemplateKind, bind_range_as_sd_claim_commands,
        build_sd_jwt_template_commands, issue_sheet_sd_jwt,
    };

    let (pres, slide_id, _) = sample_presentation();
    let mut doc = Document::new(pres);

    // 1. Apply EUDI PID template onto worksheet
    let pid_cmds = build_sd_jwt_template_commands(
        doc.presentation(),
        slide_id,
        SdJwtTemplateKind::Pid,
    );
    doc.execute(pid_cmds).unwrap();

    let pid_bindings = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
    assert_eq!(
        pid_bindings.claim("vct").and_then(|c| c.value.as_str()),
        Some("urn:eudi:pid:jp:1")
    );
    // 1D range B12:B13 -> `["JP", "US"]`
    assert_eq!(
        pid_bindings.claim("nationalities").map(|c| &c.value),
        Some(&ClaimValue::Array(vec![
            ClaimValue::String("JP".into()),
            ClaimValue::String("US".into()),
        ]))
    );
    // 1-level nested `address.region`
    assert_eq!(
        pid_bindings
            .claim("address.region")
            .and_then(|c| c.value.as_str()),
        Some("東京都")
    );

    // Interactive toggle of `family_name` disclosure (`true` -> `false`)
    let toggle_cmds = pid_bindings.toggle_claim_disclose_commands(doc.presentation(), "family_name");
    doc.execute(toggle_cmds).unwrap();
    let after_toggle = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
    assert!(!after_toggle.claim("family_name").unwrap().disclose);

    // 2. Apply ISO/IEC 18013-5 mDL template onto worksheet
    let mdl_cmds = build_sd_jwt_template_commands(
        doc.presentation(),
        slide_id,
        SdJwtTemplateKind::Iso18013Mdl,
    );
    doc.execute(mdl_cmds).unwrap();

    let mdl_bindings = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
    assert_eq!(
        mdl_bindings.claim("vct").and_then(|c| c.value.as_str()),
        Some("org.iso.18013.5.1.mDL")
    );
    // `@`-formatted `document_number` preserves leading zeros as string `"001234567890"`
    assert_eq!(
        mdl_bindings
            .claim("document_number")
            .and_then(|c| c.value.as_str()),
        Some("001234567890")
    );
    // 2D table range `$A$14:$C$16` -> `driving_privileges` array of 2 objects
    let dp = mdl_bindings
        .claim("driving_privileges")
        .expect("driving_privileges 2D table claim");
    let ClaimValue::Array(dp_rows) = &dp.value else {
        panic!("expected Array for driving_privileges");
    };
    assert_eq!(dp_rows.len(), 2);
    let first_priv = dp_rows[0].as_object().unwrap();
    assert_eq!(
        first_priv
            .get("vehicle_category_code")
            .and_then(ClaimValue::as_str),
        Some("普通")
    );
    assert_eq!(
        first_priv.get("issue_date").and_then(ClaimValue::as_str),
        Some("2018-04-01")
    );
    assert_eq!(
        first_priv.get("expiry_date").and_then(ClaimValue::as_str),
        Some("2029-05-15")
    );

    // Issue SD-JWT from the ISO 18013-5 mDL sheet
    let (_b, sd_out) = issue_sheet_sd_jwt(
        doc.presentation(),
        slide_id,
        &SdJwtOptions::default(),
        &Es256KeyPair::demo_issuer_key(),
    )
    .unwrap();
    assert!(
        sd_out
            .disclosures
            .iter()
            .any(|d| d.path_str == "driving_privileges" && d.disclose)
    );

    // 3. One-click bind of an arbitrary cell range
    let (bound_name, bind_cmds) = bind_range_as_sd_claim_commands(
        doc.presentation(),
        slide_id,
        CellRange {
            first_row: 3,
            first_column: 1,
            last_row: 3,
            last_column: 1,
        },
        "custom_authority",
        true,
    )
    .unwrap();
    assert_eq!(bound_name, "sd.custom_authority");
    doc.execute(bind_cmds).unwrap();
    let final_bindings = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
    assert_eq!(
        final_bindings
            .claim("custom_authority")
            .and_then(|c| c.value.as_str()),
        Some("東京都公安委員会")
    );
}

#[test]
fn japanese_business_presets_roundtrip() {
    use ibis_doc_model::{
        Es256KeyPair, SdJwtOptions, SdJwtTemplateKind, build_sd_jwt_template_commands,
        issue_sheet_sd_jwt,
    };

    let (pres, slide_id, _) = sample_presentation();
    let mut doc = Document::new(pres);

    let cases = [
        (
            SdJwtTemplateKind::EmployeeId,
            "urn:jp:business:employee_id:1",
            "employee_id",
            "EMP-001234",
        ),
        (
            SdJwtTemplateKind::EnrollmentCert,
            "urn:jp:edu:enrollment_cert:1",
            "student_id",
            "2023-1042",
        ),
        (
            SdJwtTemplateKind::CompletionCert,
            "urn:jp:edu:completion_cert:1",
            "certificate_id",
            "CERT-2025-0881",
        ),
    ];

    for (kind, expected_vct, key_claim, expected_val) in cases {
        let cmds = build_sd_jwt_template_commands(doc.presentation(), slide_id, kind);
        doc.execute(cmds).unwrap();

        let bindings = extract_sheet_claims(doc.presentation(), slide_id).unwrap();
        assert_eq!(
            bindings.claim("vct").and_then(|c| c.value.as_str()),
            Some(expected_vct)
        );
        assert_eq!(
            bindings.claim(key_claim).and_then(|c| c.value.as_str()),
            Some(expected_val)
        );

        let (_b, sd_out) = issue_sheet_sd_jwt(
            doc.presentation(),
            slide_id,
            &SdJwtOptions::default(),
            &Es256KeyPair::demo_issuer_key(),
        )
        .unwrap();
        assert!(!sd_out.disclosures.is_empty());
    }
}

#[test]
fn present_existing_sd_jwt_and_direct_post_body_roundtrip() {
    use ibis_doc_model::{
        Es256KeyPair, Oid4vpRequest, SdJwtOptions, issue_sheet_sd_jwt,
        list_sd_jwt_disclosure_paths, parse_sd_jwt, present_existing_sd_jwt,
    };

    let (pres, slide_id, _) = sample_presentation();
    let issuer_key = Es256KeyPair::demo_issuer_key();
    let holder_key = Es256KeyPair::demo_holder_key();
    assert_eq!(holder_key.thumbprint_short().len(), 12);
    assert_eq!(
        Es256KeyPair::from_jwk_json(&holder_key.to_private_jwk_json()),
        Some(holder_key.clone())
    );

    let mut opts = SdJwtOptions::default();
    opts.holder_public_jwk = Some(holder_key.to_public_jwk());
    let (_bindings, issued) = issue_sheet_sd_jwt(&pres, slide_id, &opts, &issuer_key).unwrap();

    // Verify all disclosures are listed from the raw issued `.sd-jwt`
    let paths = list_sd_jwt_disclosure_paths(&issued.sd_jwt_all).unwrap();
    let path_names: Vec<&str> = paths.iter().map(|(p, _)| p.as_str()).collect();
    assert!(path_names.contains(&"given_name"));
    assert!(path_names.contains(&"address.region"));

    // Parse an OID4VP DCQL request with state & response_mode=direct_post
    let uri = "openid4vp://?client_id=https%3A%2F%2Fverifier.example.org&response_mode=direct_post&response_uri=https%3A%2F%2Fverifier.example.org%2Fpost&nonce=nonce-abc-123&state=st-42&dcql_query=%7B%22credentials%22%3A%5B%7B%22id%22%3A%22cred_pid%22%2C%22claims%22%3A%5B%7B%22path%22%3A%5B%22address%22%2C%22region%22%5D%7D%5D%7D%5D%7D";
    let req = Oid4vpRequest::parse(uri).expect("parse DCQL request");
    assert_eq!(req.response_mode.as_deref(), Some("direct_post"));
    assert_eq!(req.state.as_deref(), Some("st-42"));
    assert_eq!(req.dcql_credential_id.as_deref(), Some("cred_pid"));
    assert_eq!(req.requested_claims, vec!["address.region".to_string()]);

    // Re-present the existing issued `.sd-jwt` disclosing ONLY `address.region`
    let presented = present_existing_sd_jwt(
        &issued.sd_jwt_all,
        Some(&req.requested_claims),
        &req.client_id,
        &req.nonce,
        1_750_000_000,
        &holder_key,
    )
    .expect("re-present existing SD-JWT");

    assert_eq!(presented.holder_cnf_matches, Some(true));
    assert_eq!(presented.disclosed_paths, vec!["address.region".to_string()]);
    assert!(presented.omitted_paths.iter().any(|p| p == "given_name"));

    // Re-parse the resulting `vp_token` and ensure Issuer signature + KB sd_hash + selective digests all verify
    let parsed = parse_sd_jwt(&presented.kb.vp_token, Some(&issuer_key.to_public_jwk_json()))
        .expect("parse re-presented vp_token");
    assert_eq!(parsed.signature_valid, Some(true));
    assert_eq!(parsed.kb_sd_hash_valid, Some(true));
    assert!(parsed.claims.iter().any(|c| c.path == "address.region"));
    assert!(!parsed.claims.iter().any(|c| c.path == "given_name"));

    // Build OpenID4VP 1.0 `direct_post` body
    let body = req.build_direct_post_body(&presented.kb.vp_token);
    assert!(body.starts_with("vp_token="));
    assert!(body.contains("&state=st-42"));
}

#[test]
fn external_signer_kb_jwt_matches_issue_kb_jwt() {
    use ibis_doc_model::{
        Es256KeyPair, SdJwtOptions, assemble_kb_jwt_from_raw_sig, build_kb_jwt_signing_input,
        issue_kb_jwt, issue_sheet_sd_jwt, parse_sd_jwt,
    };

    let (pres, slide_id, _) = sample_presentation();
    let issuer = Es256KeyPair::demo_issuer_key();
    let holder = Es256KeyPair::demo_holder_key();

    let mut opts = SdJwtOptions::default();
    opts.holder_public_jwk = Some(holder.to_public_jwk());
    let (_bindings, issued) = issue_sheet_sd_jwt(&pres, slide_id, &opts, &issuer).unwrap();

    let aud = "https://verifier.example/cb";
    let nonce = "nonce-webcrypto-test";
    let iat = 1_735_000_123;

    // Direct issue_kb_jwt
    let direct = issue_kb_jwt(&issued.sd_jwt_all, aud, nonce, iat, &holder);

    // Split external-signer flow (simulating WebCrypto SubtleCrypto.sign)
    let (normalized, sd_hash, signing_input) =
        build_kb_jwt_signing_input(&issued.sd_jwt_all, aud, nonce, iat);
    let raw_sig = holder.sign_es256(signing_input.as_bytes());
    let assembled =
        assemble_kb_jwt_from_raw_sig(&normalized, sd_hash, &signing_input, &raw_sig);

    assert_eq!(assembled.kb_jwt, direct.kb_jwt);
    assert_eq!(assembled.vp_token, direct.vp_token);
    assert_eq!(assembled.sd_hash, direct.sd_hash);

    let parsed = parse_sd_jwt(&assembled.vp_token, Some(&issuer.to_public_jwk_json()))
        .expect("assembled kb+jwt should parse and verify");
    assert_eq!(parsed.signature_valid, Some(true));
    assert_eq!(parsed.kb_sd_hash_valid, Some(true));
}

#[test]
fn external_signer_re_presents_existing_sd_jwt() {
    use ibis_doc_model::{
        Es256KeyPair, SdJwtOptions, assemble_kb_jwt_from_raw_sig, build_kb_jwt_signing_input,
        issue_sheet_sd_jwt, parse_sd_jwt, prepare_existing_sd_jwt_for_external_signer,
        present_existing_sd_jwt,
    };

    let (pres, slide_id, _) = sample_presentation();
    let issuer = Es256KeyPair::demo_issuer_key();
    let holder = Es256KeyPair::demo_holder_key();

    let mut opts = SdJwtOptions::default();
    opts.holder_public_jwk = Some(holder.to_public_jwk());
    let (_bindings, issued) = issue_sheet_sd_jwt(&pres, slide_id, &opts, &issuer).unwrap();

    let selected = vec!["address.region".to_string()];
    let aud = "https://verifier.example.jp/cb";
    let nonce = "n-ext-re-present";
    let iat = 1_750_000_000;

    let direct = present_existing_sd_jwt(
        &issued.sd_jwt_all,
        Some(&selected),
        aud,
        nonce,
        iat,
        &holder,
    )
    .unwrap();

    let prep = prepare_existing_sd_jwt_for_external_signer(
        &issued.sd_jwt_all,
        Some(&selected),
        Some(&holder.to_public_jwk()),
    )
    .unwrap();
    assert_eq!(prep.holder_cnf_matches, Some(true));
    assert_eq!(prep.disclosed_paths, direct.disclosed_paths);
    assert_eq!(prep.sd_jwt_without_kb, direct.sd_jwt_without_kb);

    let (normalized, sd_hash, signing_input) =
        build_kb_jwt_signing_input(&prep.sd_jwt_without_kb, aud, nonce, iat);
    let sig = holder.sign_es256(signing_input.as_bytes());
    let assembled = assemble_kb_jwt_from_raw_sig(&normalized, sd_hash, &signing_input, &sig);

    assert_eq!(assembled.vp_token, direct.kb.vp_token);
    let parsed = parse_sd_jwt(&assembled.vp_token, Some(&issuer.to_public_jwk_json())).unwrap();
    assert_eq!(parsed.signature_valid, Some(true));
    assert_eq!(parsed.kb_sd_hash_valid, Some(true));
    assert!(parsed.claims.iter().any(|c| c.path == "address.region"));
    assert!(!parsed.claims.iter().any(|c| c.path == "given_name"));
}

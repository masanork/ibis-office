// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::{path::PathBuf, process::Command};
use ibis_ooxml::{Package, SaveOptions};

fn run(path: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(path).output().unwrap()
}

#[test]
fn reports_untrusted_signature_and_tampered_part() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/synthetic-signed.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("matches embedded untrusted certificate"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
    assert!(text.contains("not that a signature is valid"));
    assert!(text.contains("digest matches; content type matches"));

    let mut package = Package::open(std::fs::read(&fixture).unwrap()).unwrap();
    package.set_bytes("word/document.xml", b"altered".to_vec());
    let path = std::env::temp_dir().join(format!("ibis-signature-cli-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("digest differs; content type matches"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn missing_file_is_an_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr).unwrap().contains("usage:"));
}

#[test]
fn explicit_time_and_anchor_produce_only_candidate_observations() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/synthetic-signed.docx");
    let package = Package::open(std::fs::read(&fixture).unwrap()).unwrap();
    let cert = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, "_xmlsignatures/sig1.xml").unwrap();
    let path = std::env::temp_dir().join(format!("ibis-signature-cli-anchor-{}.der", std::process::id()));
    std::fs::write(&path, cert).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect"))
        .arg(&fixture).args(["--at", "0", "--anchor"]).arg(&path).output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Certificate time at 0: BeforeValidity"));
    assert!(text.contains("identical to supplied anchor: true"));
    assert!(text.contains("Candidate paths: 1 (truncated: false); NOT validated"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
    let bad = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&fixture).args(["--at", "wrong"]).output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn supplied_crl_is_reported_without_a_revocation_verdict() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let mut package = Package::open(std::fs::read(fixtures.join("synthetic-signed.docx")).unwrap()).unwrap();
    let signature = "_xmlsignatures/sig1.xml";
    let previous = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, signature).unwrap();
    let leaf = std::fs::read(fixtures.join("crl-leaf.der")).unwrap();
    let xml = String::from_utf8(package.read(signature).unwrap()).unwrap();
    let changed = xml.replace(&ibis_ooxml::base64::encode(&previous), &ibis_ooxml::base64::encode(&leaf));
    assert_ne!(xml, changed);
    package.set_bytes(signature, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-signature-cli-crl-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .args(["--at", "0", "--crl"]).arg(fixtures.join("revoked.crl.der"))
        .arg("--crl-issuer").arg(fixtures.join("crl-issuer.der")).output().unwrap();
    let incomplete = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .arg("--crl").arg(fixtures.join("revoked.crl.der")).output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("CRL 1 observations (NOT a revocation verdict)"));
    assert!(text.contains("Signature Matches; issuer name exact true; certificate issuer name exact true; cRLSign Some(true)"));
    assert!(text.contains("Time NotYetIssued"));
    assert!(text.contains("CRL number NonCritical"));
    assert!(text.contains("CRL authority key observations (NOT issuer authorization)"));
    assert!(text.contains("listed revocation time Some("));
    assert!(text.contains("CRL applicability and issuer trust: NOT EVALUATED"));
    assert!(text.contains("Signature value: does not match"));
    assert_eq!(incomplete.status.code(), Some(2));
}

#[test]
fn legacy_diagnostics_are_explicit_and_detect_altered_content() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/legacy-signed.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("legacy weak RSA-SHA1 (1024 bits): matches; certificate is untrusted"));
    assert!(text.contains("Package object digest: matches (weak legacy SHA-1)"));
    assert!(text.contains("digest matches (weak legacy SHA-1); content type matches"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
    let mut package = Package::open(std::fs::read(&fixture).unwrap()).unwrap();
    package.set_bytes("word/document.xml", b"altered".to_vec());
    let path = std::env::temp_dir().join(format!("ibis-legacy-signature-cli-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("digest differs (weak legacy SHA-1); content type matches"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn reports_additional_signed_info_references_as_unchecked() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/synthetic-signed.docx");
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let path = "_xmlsignatures/sig1.xml";
    let xml = String::from_utf8(package.read(path).unwrap()).unwrap();
    let modified = xml.replace("</ds:SignedInfo>", "<ds:Reference URI=\"#properties\"/><ds:Reference URI=\"https://example.invalid/external\"/><ds:Reference/></ds:SignedInfo>");
    assert_ne!(xml, modified);
    package.set_bytes(path, modified.into_bytes());
    let temporary = std::env::temp_dir().join(format!("ibis-additional-references-{}.docx", std::process::id()));
    std::fs::write(&temporary, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&temporary);
    std::fs::remove_file(temporary).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Additional SignedInfo reference \"#properties\": NOT CHECKED"));
    assert!(text.contains("Additional SignedInfo reference \"https://example.invalid/external\": NOT CHECKED"));
    assert!(text.contains("Additional SignedInfo reference without URI: NOT CHECKED"));
    assert!(text.contains("Signature value: does not match"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn reports_additional_object_digest_and_tampering() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/synthetic-signed.docx");
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let signature = "_xmlsignatures/sig1.xml";
    let xml = String::from_utf8(package.read(signature).unwrap()).unwrap();
    let reference = r##"<ds:Reference URI="#office" Type="http://www.w3.org/2000/09/xmldsig#Object"><ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><ds:DigestValue>8WlCwdJlKpViT32rmxo5VNWoPWFQ7ASC2XkvZOzB2Sk=</ds:DigestValue></ds:Reference>"##;
    let modified = xml.replace("</ds:SignedInfo>", &format!("{reference}</ds:SignedInfo>"))
        .replace("</ds:Signature>", "<ds:Object Id=\"office\"><ds:Data>original</ds:Data></ds:Object></ds:Signature>");
    let temporary = std::env::temp_dir().join(format!("ibis-additional-digest-{}.docx", std::process::id()));
    for (content, result) in [(modified.clone(), "digest matches"), (modified.replace("original", "changed"), "digest differs")] {
        package.set_bytes(signature, content.into_bytes());
        std::fs::write(&temporary, package.save(SaveOptions::default()).unwrap()).unwrap();
        let output = run(&temporary);
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(&format!("Additional SignedInfo reference \"#office\": {result}")));
        assert!(text.contains("Certificate trust: NOT EVALUATED"));
    }
    std::fs::remove_file(temporary).unwrap();
}

#[test]
fn signed_xades_properties_digest_detects_tampering_without_changing_signed_info() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/xades-signed-synthetic.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Signature value: matches embedded untrusted certificate"));
    assert!(text.contains("Additional SignedInfo reference \"#props\": digest matches"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let signature = "_xmlsignatures/sig1.xml";
    let xml = String::from_utf8(package.read(signature).unwrap()).unwrap();
    let changed = xml.replace("2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z");
    assert_ne!(xml, changed);
    package.set_bytes(signature, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-xades-properties-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Signature value: matches embedded untrusted certificate"));
    assert!(text.contains("Additional SignedInfo reference \"#props\": digest differs"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn xades_certificate_digest_is_reported_as_unauthenticated() {
    use sha2::{Digest, Sha256};
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/xades-cert-synthetic.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Signature value: matches embedded untrusted certificate"));
    assert!(text.contains("Additional SignedInfo reference \"#props\": digest matches"));
    assert!(text.contains("XAdES SigningCertificateV2 digest: matches embedded certificate; NOT AUTHENTICATED"));
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let cert = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, part).unwrap();
    let expected = ibis_ooxml::base64::encode(&Sha256::digest(&cert));
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let changed = xml.replace(&expected, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    assert_ne!(xml, changed);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-xades-cert-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Signature value: matches embedded untrusted certificate"));
    assert!(text.contains("Additional SignedInfo reference \"#props\": digest differs"));
    assert!(text.contains("XAdES SigningCertificateV2 digest: differs from embedded certificate; NOT AUTHENTICATED"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn ecdsa_p256_is_reported_without_trust_and_detects_tampering() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/ecdsa-p256-signed.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("ECDSA P-256 SHA-256: matches; certificate is untrusted"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let signature = "_xmlsignatures/sig1.xml";
    let xml = String::from_utf8(package.read(signature).unwrap()).unwrap();
    let start_tag = "<ds:SignatureValue>"; let end_tag = "</ds:SignatureValue>";
    let start = xml.find(start_tag).unwrap() + start_tag.len(); let end = xml[start..].find(end_tag).unwrap() + start;
    let mut value = ibis_ooxml::base64::decode(&xml[start..end]).unwrap(); value[0] ^= 1;
    let changed = format!("{}{}{}", &xml[..start], ibis_ooxml::base64::encode(&value), &xml[end..]);
    package.set_bytes(signature, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-ecdsa-signature-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("ECDSA P-256 SHA-256: differs; certificate is untrusted"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn ec_chain_and_crl_are_reported_without_trust_verdicts() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let mut package = Package::open(std::fs::read(fixtures.join("ecdsa-p256-signed.docx")).unwrap()).unwrap();
    let signature = "_xmlsignatures/sig1.xml";
    let previous = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, signature).unwrap();
    let leaf = std::fs::read(fixtures.join("ec-leaf.der")).unwrap();
    let xml = String::from_utf8(package.read(signature).unwrap()).unwrap();
    let changed = xml.replace(&ibis_ooxml::base64::encode(&previous), &ibis_ooxml::base64::encode(&leaf));
    assert_ne!(xml, changed);
    package.set_bytes(signature, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-signature-ec-chain-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .args(["--at", "0", "--anchor"]).arg(fixtures.join("ec-ca.der"))
        .arg("--crl").arg(fixtures.join("ec-revoked.crl.der"))
        .arg("--crl-issuer").arg(fixtures.join("ec-ca.der")).output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Candidate paths: 1 (truncated: false); NOT validated"));
    assert!(text.contains("Link 1: signature Matches, CA Some(true), keyCertSign Some(true)"));
    assert!(text.contains("Signature Matches; issuer name exact true; certificate issuer name exact true; cRLSign Some(true)"));
    assert!(text.contains("listed revocation time Some("));
    assert!(text.contains("CRL applicability and issuer trust: NOT EVALUATED"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn xades_issuer_serial_exact_and_mismatch_are_not_trust_verdicts() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let fixture = fixtures.join("xades-issuer-serial-synthetic.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("XAdES IssuerSerialV2: issuer DER identical true; serial equal true; NOT AUTHENTICATED; semantic name matching NOT CHECKED"));
    assert!(text.contains("XAdES SigningCertificateV2 digest: matches embedded certificate; NOT AUTHENTICATED"));
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let start_tag = "<xades:IssuerSerialV2>"; let end_tag = "</xades:IssuerSerialV2>";
    let start = xml.find(start_tag).unwrap() + start_tag.len(); let end = xml[start..].find(end_tag).unwrap() + start;
    let wrong = std::fs::read(fixtures.join("xades-wrong-serial.der")).unwrap();
    let changed = format!("{}{}{}", &xml[..start], ibis_ooxml::base64::encode(&wrong), &xml[end..]);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-xades-issuer-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("issuer DER identical true; serial equal false; NOT AUTHENTICATED"));
    assert!(text.contains("Additional SignedInfo reference \"#props\": digest differs"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn rsa_sha384_and_sha512_signature_and_crl_are_reported_without_trust() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    for bits in [384, 512] {
        let fixture = fixtures.join(format!("rsa-sha{bits}-signed.docx"));
        let output = run(&fixture);
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Signature value: matches embedded untrusted certificate"));
        assert!(text.contains("Certificate trust: NOT EVALUATED"));
        let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
        let part = "_xmlsignatures/sig1.xml";
        let old = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, part).unwrap();
        let leaf = std::fs::read(fixtures.join(format!("rsa-sha{bits}-leaf.der"))).unwrap();
        let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
        let changed = xml.replace(&ibis_ooxml::base64::encode(&old), &ibis_ooxml::base64::encode(&leaf));
        assert_ne!(xml, changed);
        package.set_bytes(part, changed.into_bytes());
        let path = std::env::temp_dir().join(format!("ibis-rsa-sha{bits}-{}.docx", std::process::id()));
        std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
            .args(["--at", "0", "--anchor"]).arg(fixtures.join("rsa-sha2-ca.der"))
            .arg("--crl").arg(fixtures.join(format!("rsa-sha{bits}.crl.der")))
            .arg("--crl-issuer").arg(fixtures.join("rsa-sha2-ca.der")).output().unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Link 1: signature Matches, CA Some(true), keyCertSign Some(true)"));
        assert!(text.contains("Signature Matches; issuer name exact true; certificate issuer name exact true; cRLSign Some(true)"));
        assert!(text.contains("CRL applicability and issuer trust: NOT EVALUATED"));
        assert!(text.contains("Certificate trust: NOT EVALUATED"));
    }
}

#[test]
fn rsa_pss_signature_is_reported_without_trust_and_detects_wrong_salt() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let fixture = fixtures.join("rsa-pss-sha256-signed.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("RSA-PSS SHA-256: matches; certificate is untrusted"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let start_tag = "<ds:SignatureValue>"; let end_tag = "</ds:SignatureValue>";
    let start = xml.find(start_tag).unwrap() + start_tag.len(); let end = xml[start..].find(end_tag).unwrap() + start;
    let wrong_salt = std::fs::read(fixtures.join("rsa-pss-salt16.sig")).unwrap();
    let changed = format!("{}{}{}", &xml[..start], ibis_ooxml::base64::encode(&wrong_salt), &xml[end..]);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-rsa-pss-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("RSA-PSS SHA-256: differs; certificate is untrusted"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn rsa_pss_certificate_and_crl_candidates_are_not_trust_verdicts() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let mut package = Package::open(std::fs::read(fixtures.join("rsa-pss-sha256-signed.docx")).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let old = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, part).unwrap();
    let leaf = std::fs::read(fixtures.join("pss-normal-leaf.der")).unwrap();
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let changed = xml.replace(&ibis_ooxml::base64::encode(&old), &ibis_ooxml::base64::encode(&leaf));
    assert_ne!(xml, changed);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-pss-chain-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .args(["--at", "0", "--anchor"]).arg(fixtures.join("pss-ca.der"))
        .arg("--crl").arg(fixtures.join("pss-normal.crl.der"))
        .arg("--crl-issuer").arg(fixtures.join("pss-ca.der")).output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Candidate paths: 1 (truncated: false); NOT validated"));
    assert!(text.contains("Link 1: signature Matches, CA Some(true), keyCertSign Some(true)"));
    assert!(text.contains("Signature Matches; issuer name exact true; certificate issuer name exact true; cRLSign Some(true)"));
    assert!(text.contains("CRL applicability and issuer trust: NOT EVALUATED"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn parameterized_rsa_pss_is_reported_without_trust() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    for kind in ["default", "empty", "explicit"] {
        let output = run(&fixtures.join(format!("rsa-pss-param-{kind}.docx")));
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("RSA-PSS SHA-256: matches; certificate is untrusted"));
        assert!(text.contains("Certificate trust: NOT EVALUATED"));
    }
    let mut package = Package::open(std::fs::read(fixtures.join("rsa-pss-param-explicit.docx")).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let changed = xml.replace("<pss:SaltLength>32</pss:SaltLength>", "<pss:SaltLength>16</pss:SaltLength>");
    assert_ne!(xml, changed);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-pss-param-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Signature value: not checked (unsupported)"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn p384_document_chain_and_crl_are_reported_without_trust() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let fixture = fixtures.join("p384-signed-synthetic.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("ECDSA P-384 SHA-384: matches; certificate is untrusted"));
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let old = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, part).unwrap();
    let leaf = std::fs::read(fixtures.join("p384-normal-leaf.der")).unwrap();
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let changed = xml.replace(&ibis_ooxml::base64::encode(&old), &ibis_ooxml::base64::encode(&leaf));
    assert_ne!(xml, changed);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-p384-chain-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .args(["--at", "0", "--anchor"]).arg(fixtures.join("p384-ca.der"))
        .arg("--crl").arg(fixtures.join("p384-normal.crl.der"))
        .arg("--crl-issuer").arg(fixtures.join("p384-ca.der")).output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Link 1: signature Matches, CA Some(true), keyCertSign Some(true)"));
    assert!(text.contains("Signature Matches; issuer name exact true; certificate issuer name exact true; cRLSign Some(true)"));
    assert!(text.contains("CRL applicability and issuer trust: NOT EVALUATED"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn manifest_inventory_reports_unlisted_parts_without_claiming_coverage() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/synthetic-signed.docx");
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    package.set_bytes("word/added.xml", b"added".to_vec());
    let path = std::env::temp_dir().join(format!("ibis-signature-inventory-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Signature value: matches embedded untrusted certificate"));
    assert!(text.contains("7 package entries; 6 not literally listed; 0 ambiguous entry names; 0 unresolved reference URIs"));
    assert!(text.contains("Not literally listed: \"word/added.xml\""));
    assert!(text.contains("does NOT establish coverage or unsigned status"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn relationship_selection_shows_partial_transform_and_tampering() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let fixture = fixtures.join("relationship-selection-signed.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Signature value: matches embedded untrusted certificate"));
    assert!(text.contains("digest matches; content type matches\n    Current relationship selection: 1 selected, 1 not selected"));
    assert!(text.contains("Selected ID: \"rSigned\""));
    assert!(text.contains("Not selected ID: \"rOther\""));
    assert!(text.contains("NOT AUTHENTICATED"));
    let bytes = std::fs::read(fixture).unwrap();
    for (old, new, digest) in [("other.xml", "other-changed.xml", "digest matches"), ("signed.xml", "signed-changed.xml", "digest differs")] {
        let mut package = Package::open(bytes.clone()).unwrap();
        let part = "word/_rels/document.xml.rels";
        let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
        let changed = xml.replace(old, new);
        assert_ne!(xml, changed);
        package.set_bytes(part, changed.into_bytes());
        let path = std::env::temp_dir().join(format!("ibis-relationship-{old}-{}.docx", std::process::id()));
        std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
        let output = run(&path);
        std::fs::remove_file(path).unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Signature value: matches embedded untrusted certificate"));
        assert!(text.contains(&format!("Reference \"/word/_rels/document.xml.rels?ContentType=application/vnd.openxmlformats-package.relationships+xml\": {digest}; content type matches")));
        assert!(text.contains("Current relationship selection: 1 selected, 1 not selected"));
        assert!(text.contains("Certificate trust: NOT EVALUATED"));
    }
}

#[test]
fn certificate_location_hints_are_reported_as_untrusted() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/location-hints-signed.docx");
    let output = run(&fixture);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Certificate location hints (UNTRUSTED; NOT FETCHED)"));
    assert!(text.contains("OCSP: 1 URI hints, 0 omitted, 1 non-URI names"));
    assert!(text.contains("http://ocsp.example.invalid/status"));
    assert!(text.contains("CA issuer: 1 URI hints, 0 omitted, 0 non-URI names"));
    assert!(text.contains("CRL: 2 URI hints, 0 omitted, 1 non-URI names"));
    assert!(text.contains("Other AIA methods 1; CRL points with reason restrictions 1; alternate issuers 1; other point forms 1. Applicability NOT EVALUATED"));
    assert!(text.contains("Signature value: matches embedded untrusted certificate"));
    assert!(text.contains("Certificate trust: NOT EVALUATED"));
}

#[test]
fn certificate_location_hints_do_not_trigger_loopback_fetches() {
    use x509_cert::{Certificate, der::{Decode, Encode, asn1::{Ia5String, OctetString}}, ext::pkix::{AuthorityInfoAccessSyntax, name::GeneralName}};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let uri = format!("http://{}/ocsp", listener.local_addr().unwrap());
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures/location-hints-signed.docx");
    let mut package = Package::open(std::fs::read(fixture).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let old = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, part).unwrap();
    let mut cert = Certificate::from_der(&old).unwrap();
    let (_, mut aia) = cert.tbs_certificate.get::<AuthorityInfoAccessSyntax>().unwrap().unwrap();
    aia.0[0].access_location = GeneralName::UniformResourceIdentifier(Ia5String::new(&uri).unwrap());
    let ext = cert.tbs_certificate.extensions.as_mut().unwrap().iter_mut().find(|e| e.extn_id.to_string() == "1.3.6.1.5.5.7.1.1").unwrap();
    ext.extn_value = OctetString::new(aia.to_der().unwrap()).unwrap();
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let changed = xml.replace(&ibis_ooxml::base64::encode(&old), &ibis_ooxml::base64::encode(&cert.to_der().unwrap()));
    assert_ne!(xml, changed);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-certificate-hints-loopback-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(&uri));
    assert!(text.contains("UNTRUSTED; NOT FETCHED"));
    let error = listener.accept().expect_err("inspector must not connect to the supplied endpoint");
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
}

#[test]
fn supplied_ocsp_is_reported_without_a_revocation_verdict() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let mut package = Package::open(std::fs::read(fixtures.join("synthetic-signed.docx")).unwrap()).unwrap();
    let signature = "_xmlsignatures/sig1.xml";
    let previous = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, signature).unwrap();
    let leaf = std::fs::read(fixtures.join("ocsp-leaf.der")).unwrap();
    let xml = String::from_utf8(package.read(signature).unwrap()).unwrap();
    let changed = xml.replace(&ibis_ooxml::base64::encode(&previous), &ibis_ooxml::base64::encode(&leaf));
    assert_ne!(xml, changed);
    package.set_bytes(signature, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-signature-cli-ocsp-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let nonce_path = std::env::temp_dir().join(format!("ibis-signature-cli-nonce-{}.bin", std::process::id()));
    std::fs::write(&nonce_path, b"synthetic-nonce").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .args(["--at", "0", "--ocsp"]).arg(fixtures.join("ocsp-good.der"))
        .arg("--ocsp").arg(fixtures.join("ocsp-revoked.der"))
        .arg("--ocsp-issuer").arg(fixtures.join("ocsp-issuer.der"))
        .arg("--ocsp-responder").arg(fixtures.join("ocsp-responder.der"))
        .arg("--ocsp-nonce").arg(&nonce_path).output().unwrap();
    let request_output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .args(["--at", "0", "--ocsp"]).arg(fixtures.join("ocsp-revoked.der"))
        .arg("--ocsp-issuer").arg(fixtures.join("ocsp-issuer.der"))
        .arg("--ocsp-responder").arg(fixtures.join("ocsp-responder.der"))
        .arg("--ocsp-request").arg(fixtures.join("ocsp-request.der")).output().unwrap();
    let conflicting = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .args(["--at", "0", "--ocsp"]).arg(fixtures.join("ocsp-revoked.der"))
        .arg("--ocsp-issuer").arg(fixtures.join("ocsp-issuer.der"))
        .arg("--ocsp-responder").arg(fixtures.join("ocsp-responder.der"))
        .arg("--ocsp-request").arg(fixtures.join("ocsp-request.der"))
        .arg("--ocsp-nonce").arg(&nonce_path).output().unwrap();
    let incomplete = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .arg("--ocsp").arg(fixtures.join("ocsp-good.der")).output().unwrap();
    let nonce_alone = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .arg("--ocsp-nonce").arg(&nonce_path).output().unwrap();
    std::fs::write(&nonce_path, []).unwrap();
    let empty = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .arg("--ocsp-nonce").arg(&nonce_path).output().unwrap();
    std::fs::write(&nonce_path, [0u8; 33]).unwrap();
    let oversized = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&path)
        .arg("--ocsp-nonce").arg(&nonce_path).output().unwrap();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(nonce_path).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("OCSP 1 observations (NOT a revocation verdict)"));
    assert!(text.contains("Signature Matches; responder ID ByKeySha1(true)"));
    assert!(text.contains("issuer_name_hash_equal: true, issuer_key_hash_equal: true, serial_equal: true"));
    assert!(text.contains("Responder certificate observations (NOT authorization)"));
    assert!(text.contains("ocsp_signing_eku: Absent"));
    assert!(text.contains("Nonce observation (UNAUTHENTICATED; NOT freshness proof): Absent"));
    assert!(text.contains("Response extension inventory (UNTRUSTED; NOT interpreted)"));
    assert!(text.contains("Single-response extension inventory (UNTRUSTED; NOT interpreted)"));
    assert!(text.contains("comparison: Matches"));
    assert!(text.contains("UNAUTHENTICATED claim Good"));
    assert!(text.contains("UNAUTHENTICATED claim Revoked"));
    assert!(text.contains("reason: Some(KeyCompromise)"));
    assert!(text.contains("Extended-revoke response extension (UNAUTHENTICATED): Absent"));
    assert!(text.contains("extended-revoke at single-response scope Absent"));
    assert!(text.contains("response extensions present true"));
    assert!(text.contains("Responder authorization, issuer trust, nonce policy, other extension semantics and revocation: NOT EVALUATED"));
    assert_eq!(incomplete.status.code(), Some(2));
    assert!(request_output.status.success(), "{}", String::from_utf8_lossy(&request_output.stderr));
    let request_text = String::from_utf8(request_output.stdout).unwrap();
    assert!(request_text.contains("Request/response observations (UNTRUSTED; NOT acceptance)"));
    assert!(request_text.contains("comparison: Matches"));
    assert!(request_text.contains("exact_response_cert_id_counts: [1]"));
    assert!(request_text.contains("request_extension_inventory"));
    assert!(request_text.contains("single_request_extension_inventories"));
    assert_eq!(conflicting.status.code(), Some(2));
    assert_eq!(nonce_alone.status.code(), Some(2));
    assert_eq!(empty.status.code(), Some(2));
    assert_eq!(oversized.status.code(), Some(2));
}

#[test]
fn certificate_policy_claims_are_not_policy_validation() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let mut package = Package::open(std::fs::read(fixtures.join("synthetic-signed.docx")).unwrap()).unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let old = ibis_ooxml::signatures::extract_single_embedded_certificate(&package, part).unwrap();
    let cert = std::fs::read(fixtures.join("certificate-policies.der")).unwrap();
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let changed = xml.replace(&ibis_ooxml::base64::encode(&old), &ibis_ooxml::base64::encode(&cert));
    assert_ne!(xml, changed);
    package.set_bytes(part, changed.into_bytes());
    let path = std::env::temp_dir().join(format!("ibis-policy-cli-{}.docx", std::process::id()));
    std::fs::write(&path, package.save(SaveOptions::default()).unwrap()).unwrap();
    let output = run(&path);
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Certificate policies (UNTRUSTED; NOT path or jurisdiction validation)"));
    assert!(text.contains("critical: true, total: 2"));
    assert!(text.contains("any_policy_inspected: true"));
    assert!(text.contains("1.2.3.4.5"));
    assert!(text.contains("Signature value: does not match"));
    assert!(!text.contains("cps.example.invalid"));
}

#[test]
fn supplied_certificate_policies_are_labeled_by_role_and_fingerprint() {
    use sha2::{Digest, Sha256};
    use x509_cert::{Certificate, der::{Decode, Encode, asn1::OctetString}};
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signatures");
    let document = fixtures.join("synthetic-signed.docx");
    let policy = fixtures.join("certificate-policies.der");
    let intermediate = fixtures.join("ocsp-leaf.der");
    let controls = fixtures.join("policy-controls.der");
    let mut malformed = Certificate::from_der(&std::fs::read(&policy).unwrap()).unwrap();
    malformed.tbs_certificate.extensions.as_mut().unwrap()[0].extn_value = OctetString::new(vec![0xff]).unwrap();
    let malformed_path = std::env::temp_dir().join(format!("ibis-policy-malformed-{}.der", std::process::id()));
    std::fs::write(&malformed_path, malformed.to_der().unwrap()).unwrap();
    let malformed_hash = Sha256::digest(std::fs::read(&malformed_path).unwrap()).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let fingerprint = |path: &std::path::Path| Sha256::digest(std::fs::read(path).unwrap()).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let output = Command::new(env!("CARGO_BIN_EXE_ibis-signature-inspect")).arg(&document)
        .arg("--anchor").arg(&policy).arg("--anchor").arg(&policy)
        .arg("--anchor").arg(&malformed_path)
        .arg("--anchor").arg(&controls)
        .arg("--intermediate").arg(&intermediate).output().unwrap();
    std::fs::remove_file(malformed_path).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Supplied certificate policy observations (UNTRUSTED; NOT path or jurisdiction validation)"));
    assert!(text.contains(&format!("Intermediate 1 SHA-256: {}; policies: Absent", fingerprint(&intermediate))));
    assert!(text.contains(&format!("Anchor candidate 1 SHA-256: {}; policies: Present", fingerprint(&policy))));
    assert!(text.contains(&format!("Anchor candidate 2 SHA-256: {}; policies: Present", fingerprint(&policy))));
    assert!(text.contains(&format!("Anchor candidate 3 SHA-256: {malformed_hash}; policies: InvalidOrDuplicateExtension")));
    assert!(text.contains(&format!("Anchor candidate 4 SHA-256: {}; policies: Absent", fingerprint(&controls))));
    assert!(text.contains("Policy control observations (UNTRUSTED; NOT applied)"));
    assert!(text.contains("require_explicit_policy: Some(2), inhibit_policy_mapping: Some(3)"));
    assert!(text.contains("inhibit_any_policy: Present { critical: true, value: 4 }"));
    assert!(text.contains("(\"1.2.3.4\", \"1.2.3.5\")"));
    assert!(text.contains("1.2.3.4.5"));
    assert!(text.contains("Candidate paths: 0"));
    assert!(text.contains("they do not establish path membership or anchor trust"));
    assert!(text.contains("policy tree: NOT EVALUATED"));
    assert!(!text.contains("cps.example.invalid"));
}

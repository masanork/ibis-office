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

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::{
    path::PathBuf,
    process::{Command, Output},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}
fn command(document: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ibis-certval-path-probe"));
    cmd.arg(document)
        .args([
            "--signature",
            "_xmlsignatures/sig1.xml",
            "--at",
            "1796083200",
        ])
        .arg("--anchor")
        .arg(root().join("document-path-root.der"))
        .arg("--intermediate")
        .arg(root().join("document-path-intermediate.der"));
    cmd
}
fn text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn cli_reports_signatures_references_and_path_independently() {
    let document = root().join("document-path-signed.docx");
    let output = command(&document).arg("--policy-any").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = text(&output);
    assert!(result.contains("Exit status 0 means observation completed, NOT that the document or any certificate is valid"));
    assert!(result.contains(
        "Offline only: no network, system trust store, OCSP, CRL or trusted timestamp validation"
    ));
    assert!(result.contains("explicit policy required false"));
    assert!(result.contains(
        "XML signature against untrusted embedded certificate: MatchesUntrustedCertificate"
    ));
    assert!(result.contains("Signed package object digest: Matches"));
    assert!(result.contains("digest Matches; content type Matches"));
    assert!(
        result.contains(
            "Path engine: PASS under supplied inputs; revocation and trust NOT established"
        )
    );
    assert!(result.contains(
        "Edge observations (anchor to signer; NOT path or trust verdict): [Matches, Matches]"
    ));
    let mut package = ibis_ooxml::Package::open(std::fs::read(&document).unwrap()).unwrap();
    package.set_bytes("word/document.xml", b"modified".to_vec());
    let changed = std::env::temp_dir().join(format!(
        "ibis-path-probe-tampered-{}.docx",
        std::process::id()
    ));
    std::fs::write(
        &changed,
        package.save(ibis_ooxml::SaveOptions::default()).unwrap(),
    )
    .unwrap();
    let output = command(&changed).arg("--policy-any").output().unwrap();
    std::fs::remove_file(changed).unwrap();
    assert!(output.status.success());
    let result = text(&output);
    assert!(result.contains(
        "XML signature against untrusted embedded certificate: MatchesUntrustedCertificate"
    ));
    assert!(result.contains("digest Differs; content type Matches"));
    assert!(
        result.contains(
            "Path engine: PASS under supplied inputs; revocation and trust NOT established"
        )
    );
}

#[test]
fn cli_requires_explicit_inputs_and_reports_policy_failure() {
    let document = root().join("document-path-signed.docx");
    let no_anchor = Command::new(env!("CARGO_BIN_EXE_ibis-certval-path-probe"))
        .arg(&document)
        .args([
            "--signature",
            "_xmlsignatures/sig1.xml",
            "--at",
            "1796083200",
            "--policy-any",
        ])
        .output()
        .unwrap();
    assert_eq!(no_anchor.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&no_anchor.stderr).contains("--anchor is required"));
    let no_time = Command::new(env!("CARGO_BIN_EXE_ibis-certval-path-probe"))
        .arg(&document)
        .args(["--signature", "_xmlsignatures/sig1.xml", "--policy-any"])
        .arg("--anchor")
        .arg(root().join("document-path-root.der"))
        .output()
        .unwrap();
    assert_eq!(no_time.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&no_time.stderr).contains("--at is required"));
    let no_policy = command(&document).output().unwrap();
    assert_eq!(no_policy.status.code(), Some(2));
    let both = command(&document)
        .args(["--policy-any", "--policy", "1.2.3.4"])
        .output()
        .unwrap();
    assert_eq!(both.status.code(), Some(2));
    let duplicate = command(&document)
        .args(["--policy", "1.2.3.4", "--policy", "1.2.3.4"])
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(2));
    let any_oid = command(&document)
        .args(["--policy", "2.5.29.32.0"])
        .output()
        .unwrap();
    assert_eq!(any_oid.status.code(), Some(2));
    let bad_oid = command(&document)
        .args(["--policy", "not-an-oid"])
        .output()
        .unwrap();
    assert_eq!(bad_oid.status.code(), Some(2));
    let mut zero = Command::new(env!("CARGO_BIN_EXE_ibis-certval-path-probe"));
    let zero = zero
        .arg(&document)
        .args([
            "--signature",
            "_xmlsignatures/sig1.xml",
            "--at",
            "0",
            "--policy-any",
        ])
        .arg("--anchor")
        .arg(root().join("document-path-root.der"))
        .output()
        .unwrap();
    assert_eq!(zero.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&zero.stderr).contains("zero disables"));
    let nonexistent = Command::new(env!("CARGO_BIN_EXE_ibis-certval-path-probe"))
        .arg(&document)
        .args([
            "--signature",
            "_xmlsignatures/other.xml",
            "--at",
            "1796083200",
            "--policy-any",
        ])
        .arg("--anchor")
        .arg(root().join("document-path-root.der"))
        .output()
        .unwrap();
    assert_eq!(nonexistent.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&nonexistent.stderr).contains("UnregisteredSignature"));
    let no_candidate = Command::new(env!("CARGO_BIN_EXE_ibis-certval-path-probe"))
        .arg(&document)
        .args([
            "--signature",
            "_xmlsignatures/sig1.xml",
            "--at",
            "1796083200",
            "--policy-any",
        ])
        .arg("--anchor")
        .arg(root().join("root.der"))
        .output()
        .unwrap();
    assert!(no_candidate.status.success());
    assert!(text(&no_candidate).contains("Candidate paths: 0"));
    assert!(text(&no_candidate).contains("does not prove that no valid path exists"));
    let explicit = command(&document)
        .args(["--policy", "1.2.3.4", "--inhibit-any", "--inhibit-mapping"])
        .output()
        .unwrap();
    assert!(explicit.status.success());
    let result = text(&explicit);
    assert!(
        result.contains(
            "explicit policy required true; inhibit anyPolicy true; inhibit mapping true"
        )
    );
    assert!(result.contains("Path engine: DID NOT PASS OR INDETERMINATE"));
    assert!(result.contains("NullPolicySet"));
    assert!(!result.contains("Path engine: PASS"));
}

#[test]
fn cli_rejects_oversized_declared_zip_entries() {
    let document = root().join("document-path-signed.docx");
    let mut bytes = std::fs::read(document).unwrap();
    let central = bytes
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    bytes[central + 24..central + 28].copy_from_slice(&(16u32 * 1024 * 1024 + 1).to_le_bytes());
    let path =
        std::env::temp_dir().join(format!("ibis-path-probe-big-{}.docx", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let output = command(&path).arg("--policy-any").output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("ZIP entry exceeds 16 MiB expanded size")
    );
}

#[test]
fn cli_reports_extra_signed_info_and_unlisted_parts_without_fetching() {
    use std::{io::ErrorKind, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let uri = format!("http://{}/not-fetched", listener.local_addr().unwrap());
    let mut package =
        ibis_ooxml::Package::open(std::fs::read(root().join("document-path-signed.docx")).unwrap())
            .unwrap();
    let part = "_xmlsignatures/sig1.xml";
    let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
    let added = format!(r#"<ds:Reference URI="{uri}"/>"#);
    let changed = xml.replace("</ds:SignedInfo>", &format!("{added}</ds:SignedInfo>"));
    assert_ne!(xml, changed);
    package.set_bytes(part, changed.into_bytes());
    package.set_bytes("word/media/unlisted.bin", b"unlisted".to_vec());
    let path =
        std::env::temp_dir().join(format!("ibis-path-probe-extra-{}.docx", std::process::id()));
    std::fs::write(
        &path,
        package.save(ibis_ooxml::SaveOptions::default()).unwrap(),
    )
    .unwrap();
    let output = command(&path).arg("--policy-any").output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = text(&output);
    assert!(result.contains("Additional SignedInfo references: 1"));
    assert!(result.contains(&uri));
    assert!(result.contains("digest Unsupported"));
    assert!(result.contains("Manifest inventory (UNAUTHENTICATED; NOT coverage)"));
    assert!(result.contains("word/media/unlisted.bin"));
    assert!(result.contains("NOT a coverage verdict"));
    assert!(result.contains("XML signature against untrusted embedded certificate: Differs"));
    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
}

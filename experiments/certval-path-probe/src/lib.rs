// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Isolated experiment only; not used by ibisOffice or a trust decision.
#![allow(dead_code)]

use certval::{
    CertificationPath, CertificationPathResults, CertificationPathSettings, DeferDecodeSigned,
    Error, PDVCertificate, PDVTrustAnchorChoice, PathValidationStatus, PkiEnvironment,
    TimeOfInterest,
};
/// Single-edge diagnostic only. This cannot validate a certificate path,
/// identify the issuer, authorize a key, or establish certificate trust.
#[derive(Debug, PartialEq, Eq)]
enum EdgeDiagnostic {
    Matches,
    UnsupportedOrUnrecognized,
    /// The backend reported a failure; other unsupported inputs may also map here.
    BackendReportedSignatureFailure,
    InvalidInput,
    OtherFailure,
}

fn inspect_edge_signature(child_der: &[u8], proposed_issuer_der: &[u8]) -> EdgeDiagnostic {
    use der::Decode;
    if child_der.len() > 1024 * 1024 || proposed_issuer_der.len() > 1024 * 1024 {
        return EdgeDiagnostic::InvalidInput;
    }
    let Ok(child) = PDVCertificate::try_from(child_der) else {
        return EdgeDiagnostic::InvalidInput;
    };
    let Ok(issuer) = PDVCertificate::try_from(proposed_issuer_der) else {
        return EdgeDiagnostic::InvalidInput;
    };
    let Ok(signed) = DeferDecodeSigned::from_der(child_der) else {
        return EdgeDiagnostic::InvalidInput;
    };
    if child.decoded().signature_algorithm() != child.decoded().tbs_certificate().signature() {
        return EdgeDiagnostic::InvalidInput;
    }
    let mut env = PkiEnvironment::default();
    env.populate_5280_pki_environment();
    let result = env.verify_signature_message(
        &env,
        &signed.tbs_field,
        child.decoded().signature(),
        child.decoded().tbs_certificate().signature(),
        issuer.decoded().tbs_certificate().subject_public_key_info(),
    );
    match result {
        Ok(()) => EdgeDiagnostic::Matches,
        Err(Error::Unrecognized) => EdgeDiagnostic::UnsupportedOrUnrecognized,
        Err(Error::PathValidation(PathValidationStatus::SignatureVerificationFailure)) => {
            EdgeDiagnostic::BackendReportedSignatureFailure
        }
        Err(_) => EdgeDiagnostic::OtherFailure,
    }
}

/// Independent results for one caller-supplied path; neither result is a
/// document signature or revocation verdict.
#[derive(Debug, PartialEq, Eq)]
struct PathProbeReport {
    /// Raw engine outcome; Err can include unsupported or indeterminate cases.
    engine_result: Result<(), String>,
    /// Anchor-to-target order, one signature observation per proposed edge.
    edge_observations: Vec<EdgeDiagnostic>,
}

#[derive(Debug, PartialEq, Eq)]
enum PathProbeInputError {
    TooManyIntermediates,
    InputTooLarge,
    InvalidTime,
}

/// Examine a caller-selected anchor, target and anchor-to-target intermediates.
/// This does not build paths, authenticate inputs or infer a trust decision.
fn inspect_explicit_path(
    leaf: &[u8],
    anchor: &[u8],
    intermediates: &[&[u8]],
    time: u64,
    settings: CertificationPathSettings,
) -> Result<PathProbeReport, PathProbeInputError> {
    if intermediates.len() > 7 {
        return Err(PathProbeInputError::TooManyIntermediates);
    }
    if TimeOfInterest::from_unix_secs(time).map_or(true, |t| t.is_disabled()) {
        return Err(PathProbeInputError::InvalidTime);
    }
    let inputs = std::iter::once(anchor)
        .chain(intermediates.iter().copied())
        .chain(std::iter::once(leaf));
    let mut size = 0usize;
    for input in inputs {
        if input.len() > 1024 * 1024 {
            return Err(PathProbeInputError::InputTooLarge);
        }
        size = size
            .checked_add(input.len())
            .ok_or(PathProbeInputError::InputTooLarge)?;
    }
    if size > 4 * 1024 * 1024 {
        return Err(PathProbeInputError::InputTooLarge);
    }
    let mut edges = Vec::with_capacity(intermediates.len() + 1);
    let mut issuer = anchor;
    for child in intermediates.iter().copied().chain(std::iter::once(leaf)) {
        edges.push(inspect_edge_signature(child, issuer));
        issuer = child;
    }
    Ok(PathProbeReport {
        engine_result: validate_with_settings(leaf, anchor, intermediates, time, settings),
        edge_observations: edges,
    })
}

/// A single discovered signature's independent XML and candidate-path facts.
/// No field is an overall document-validity or trust verdict.
#[derive(Debug, PartialEq, Eq)]
struct DocumentPathProbe {
    xml_signature: ibis_ooxml::signatures::SignatureValueCheck,
    package_object_digest: ibis_ooxml::signatures::ObjectDigest,
    manifest_references: Vec<ibis_ooxml::signatures::ManifestReference>,
    signer_fingerprint: [u8; 32],
    candidate_search_truncated: bool,
    candidates: Vec<DocumentPathCandidate>,
}

#[derive(Debug, PartialEq, Eq)]
struct DocumentPathCandidate {
    /// Fingerprints from the signer toward the explicitly supplied anchor.
    fingerprints: Vec<[u8; 32]>,
    result: DocumentCandidateResult,
}

#[derive(Debug, PartialEq, Eq)]
enum DocumentCandidateResult {
    Evaluated(PathProbeReport),
    /// The supplied anchor is byte-identical to the embedded signer. Direct
    /// anchor-as-target treatment has not been designed or evaluated here.
    AnchorIsSignerNotEvaluated,
}

#[derive(Debug, PartialEq, Eq)]
enum DocumentProbeError {
    UnregisteredSignature,
    Package(String),
    AmbiguousFingerprint,
    InvalidCandidate,
    PathInput(PathProbeInputError),
}

/// Look up only a uniquely identified certificate, allowing repeated copies
/// of identical bytes but refusing any fingerprint collision across inputs.
fn resolve_fingerprint<'a>(
    fingerprint: &[u8; 32],
    inputs: &[([u8; 32], &'a [u8])],
) -> Result<&'a [u8], DocumentProbeError> {
    let mut found: Option<&[u8]> = None;
    for (_, bytes) in inputs.iter().filter(|(hash, _)| hash == fingerprint) {
        if found.is_some_and(|previous| previous != *bytes) {
            return Err(DocumentProbeError::AmbiguousFingerprint);
        }
        found = Some(bytes);
    }
    found.ok_or(DocumentProbeError::InvalidCandidate)
}

/// Experimental bridge for one relationship-discovered signature. Trust
/// anchors and intermediates are explicitly supplied, with no network access.
/// Candidates use the existing exact-name builder and may omit valid paths.
fn inspect_document_candidate_paths(
    package: &ibis_ooxml::Package,
    signature_part: &str,
    anchors: &[&[u8]],
    intermediates: &[&[u8]],
    time: u64,
    settings: CertificationPathSettings,
) -> Result<DocumentPathProbe, DocumentProbeError> {
    use ibis_ooxml::signatures;
    use sha2::{Digest, Sha256};
    if TimeOfInterest::from_unix_secs(time).map_or(true, |t| t.is_disabled()) {
        return Err(DocumentProbeError::PathInput(
            PathProbeInputError::InvalidTime,
        ));
    }
    let parts = signatures::signature_parts(package)
        .map_err(|e| DocumentProbeError::Package(e.to_string()))?;
    if !parts.iter().any(|part| part == signature_part) {
        return Err(DocumentProbeError::UnregisteredSignature);
    }
    let leaf = signatures::extract_single_embedded_certificate(package, signature_part)
        .map_err(|e| DocumentProbeError::Package(e.to_string()))?;
    let xml_signature = signatures::inspect_signature_value(package, signature_part)
        .map_err(|e| DocumentProbeError::Package(e.to_string()))?;
    let package_object_digest = signatures::inspect_package_object_digest(package, signature_part)
        .map_err(|e| DocumentProbeError::Package(e.to_string()))?;
    let manifest_references = signatures::inspect_manifest_digests(package, signature_part)
        .map_err(|e| DocumentProbeError::Package(e.to_string()))?;
    let found = signatures::find_candidate_certificate_paths(&leaf, intermediates, anchors)
        .map_err(|e| DocumentProbeError::Package(e.to_string()))?;
    let signer_fingerprint: [u8; 32] = Sha256::digest(&leaf).into();
    let inputs: Vec<([u8; 32], &[u8])> = std::iter::once(leaf.as_slice())
        .chain(intermediates.iter().copied())
        .chain(anchors.iter().copied())
        .map(|bytes| (Sha256::digest(bytes).into(), bytes))
        .collect();
    let mut candidates = Vec::with_capacity(found.paths.len());
    for candidate in found.paths {
        let certs: Vec<&[u8]> = candidate
            .fingerprints_sha256
            .iter()
            .map(|fingerprint| resolve_fingerprint(fingerprint, &inputs))
            .collect::<Result<_, _>>()?;
        if certs.first().copied() != Some(leaf.as_slice())
            || !certs.last().is_some_and(|cert| anchors.contains(cert))
        {
            return Err(DocumentProbeError::InvalidCandidate);
        }
        let result = if certs.len() == 1 {
            DocumentCandidateResult::AnchorIsSignerNotEvaluated
        } else {
            let intermediate: Vec<&[u8]> =
                certs[1..certs.len() - 1].iter().rev().copied().collect();
            DocumentCandidateResult::Evaluated(
                inspect_explicit_path(
                    &leaf,
                    certs[certs.len() - 1],
                    &intermediate,
                    time,
                    settings.clone(),
                )
                .map_err(DocumentProbeError::PathInput)?,
            )
        };
        candidates.push(DocumentPathCandidate {
            fingerprints: candidate.fingerprints_sha256,
            result,
        });
    }
    Ok(DocumentPathProbe {
        xml_signature,
        package_object_digest,
        manifest_references,
        signer_fingerprint,
        candidate_search_truncated: found.truncated,
        candidates,
    })
}

fn validate(leaf: &[u8], anchor: &[u8], time: u64) -> Result<(), String> {
    validate_with_intermediates(leaf, anchor, &[], time)
}
fn validate_with_intermediates(
    leaf: &[u8],
    anchor: &[u8],
    intermediates: &[&[u8]],
    time: u64,
) -> Result<(), String> {
    let settings = CertificationPathSettings::new();
    validate_with_settings(leaf, anchor, intermediates, time, settings)
}
fn validate_with_settings(
    leaf: &[u8],
    anchor: &[u8],
    intermediates: &[&[u8]],
    time: u64,
    mut settings: CertificationPathSettings,
) -> Result<(), String> {
    let ta = PDVTrustAnchorChoice::try_from(anchor).map_err(|e| format!("anchor: {e:?}"))?;
    let target = PDVCertificate::try_from(leaf).map_err(|e| format!("target: {e:?}"))?;
    let chain = intermediates
        .iter()
        .map(|der| PDVCertificate::try_from(*der).map_err(|e| format!("intermediate: {e:?}")))
        .collect::<Result<Vec<_>, _>>()?;
    let path = CertificationPath::new(ta, chain, target);
    let mut env = PkiEnvironment::default();
    env.populate_5280_pki_environment();
    settings.set_require_ta_store(false);
    settings.set_time_of_interest(
        TimeOfInterest::from_unix_secs(time).map_err(|e| format!("time: {e:?}"))?,
    );
    let mut result = CertificationPathResults::new();
    env.validate_path(&env, &settings, &path, &mut result)
        .map_err(|e| format!("{e:?}; {:?}", result.get_validation_status()))
}
#[cfg(test)]
mod tests {
    use super::*;
    const LEAF: &[u8] =
        include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/rsa-sha384-leaf.der");
    const ANCHOR: &[u8] =
        include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/rsa-sha2-ca.der");
    const WRONG: &[u8] =
        include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/ocsp-issuer.der");
    #[test]
    fn behavior() {
        let good = validate(LEAF, ANCHOR, 1796083200);
        println!("good: {good:?}");
        assert!(good.is_ok());
        let wrong = validate(LEAF, WRONG, 1796083200);
        println!("wrong: {wrong:?}");
        assert!(wrong.unwrap_err().contains("NameChainingFailure"));
        let expired = validate(LEAF, ANCHOR, 1893456000);
        println!("expired: {expired:?}");
        assert!(expired.unwrap_err().contains("InvalidNotAfterDate"));
        let mut tampered = LEAF.to_vec();
        let n = tampered.len();
        tampered[n - 1] ^= 1;
        let damaged = validate(&tampered, ANCHOR, 1796083200);
        println!("tampered: {damaged:?}");
        assert!(
            damaged
                .unwrap_err()
                .contains("SignatureVerificationFailure")
        );
    }
    #[test]
    fn generated_chain_cases() {
        let root = include_bytes!("../data/root.der");
        let wrong = include_bytes!("../data/same-name-wrong.der");
        let leaf = include_bytes!("../data/leaf.der");
        let critical = include_bytes!("../data/unknown-critical.der");
        let intermediate = include_bytes!("../data/intermediate.der");
        let intermediate_leaf = include_bytes!("../data/intermediate-leaf.der");
        let nonca = include_bytes!("../data/bad-intermediate-nonca.der");
        let no_key_cert_sign = include_bytes!("../data/bad-intermediate-key-usage.der");
        for (label, result, expected) in [
            ("direct", validate(leaf, root, 1796083200), None),
            (
                "same-name-wrong-key",
                validate(leaf, wrong, 1796083200),
                Some("SignatureVerificationFailure"),
            ),
            (
                "unknown-critical",
                validate(critical, root, 1796083200),
                Some("UnprocessedCriticalExtension"),
            ),
            (
                "missing-intermediate",
                validate(intermediate_leaf, root, 1796083200),
                Some("NameChainingFailure"),
            ),
            (
                "explicit-intermediate",
                validate_with_intermediates(intermediate_leaf, root, &[intermediate], 1796083200),
                None,
            ),
            (
                "intermediate-not-ca",
                validate_with_intermediates(intermediate_leaf, root, &[nonca], 1796083200),
                Some("InvalidBasicConstraints"),
            ),
            (
                "intermediate-key-usage",
                validate_with_intermediates(
                    intermediate_leaf,
                    root,
                    &[no_key_cert_sign],
                    1796083200,
                ),
                Some("InvalidKeyUsage"),
            ),
        ] {
            println!("{label}: {result:?}");
            match expected {
                None => assert!(result.is_ok(), "{label}: {result:?}"),
                Some(reason) => assert!(result.unwrap_err().contains(reason), "{label}"),
            }
        }
    }
    #[test]
    fn edge_diagnostic_preserves_unrecognized_without_claiming_trust() {
        let pss_ca =
            include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-ca.der");
        let other_ca =
            include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-other.der");
        let normal = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-normal-leaf.der"
        );
        let mixed = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-mgf384-leaf.der"
        );
        assert_eq!(
            inspect_edge_signature(normal, pss_ca),
            EdgeDiagnostic::Matches
        );
        assert_eq!(
            inspect_edge_signature(mixed, pss_ca),
            EdgeDiagnostic::UnsupportedOrUnrecognized
        );
        let mut damaged = normal.to_vec();
        let end = damaged.len();
        damaged[end - 1] ^= 1;
        assert_eq!(
            inspect_edge_signature(&damaged, pss_ca),
            EdgeDiagnostic::BackendReportedSignatureFailure
        );
        assert_eq!(
            inspect_edge_signature(normal, other_ca),
            EdgeDiagnostic::BackendReportedSignatureFailure
        );
        let mut damaged_unsupported = mixed.to_vec();
        let end = damaged_unsupported.len();
        damaged_unsupported[end - 1] ^= 1;
        assert_eq!(
            inspect_edge_signature(&damaged_unsupported, pss_ca),
            EdgeDiagnostic::UnsupportedOrUnrecognized
        );
        assert_eq!(
            inspect_edge_signature(b"bad", pss_ca),
            EdgeDiagnostic::InvalidInput
        );
        assert_eq!(
            inspect_edge_signature(normal, b"bad"),
            EdgeDiagnostic::InvalidInput
        );
        assert_eq!(
            inspect_edge_signature(&vec![0; 1024 * 1024 + 1], pss_ca),
            EdgeDiagnostic::InvalidInput
        );
        let leaf = include_bytes!("../data/intermediate-leaf.der");
        let nonca = include_bytes!("../data/bad-intermediate-nonca.der");
        let root = include_bytes!("../data/root.der");
        // The key matches the signature even though the proposed issuer is not a CA.
        assert_eq!(inspect_edge_signature(leaf, nonca), EdgeDiagnostic::Matches);
        assert!(
            validate_with_intermediates(leaf, root, &[nonca], 1796083200)
                .unwrap_err()
                .contains("InvalidBasicConstraints")
        );
    }

    fn package_with_signing_certificate(cert: &[u8]) -> ibis_ooxml::Package {
        use ibis_ooxml::{Package, base64, signatures::extract_single_embedded_certificate};
        let mut package = Package::open(
            include_bytes!(
                "../../../crates/ibis-ooxml/tests/fixtures/signatures/synthetic-signed.docx"
            )
            .as_slice(),
        )
        .unwrap();
        let part = "_xmlsignatures/sig1.xml";
        let previous = extract_single_embedded_certificate(&package, part).unwrap();
        let xml = String::from_utf8(package.read(part).unwrap()).unwrap();
        let changed = xml.replace(&base64::encode(&previous), &base64::encode(cert));
        assert_ne!(xml, changed);
        package.set_bytes(part, changed.into_bytes());
        package
    }

    #[test]
    fn document_bridge_keeps_xml_and_each_candidate_separate() {
        use ibis_ooxml::signatures::SignatureValueCheck;
        let root = include_bytes!("../data/root.der");
        let wrong_root = include_bytes!("../data/same-name-wrong.der");
        let intermediate = include_bytes!("../data/intermediate.der");
        let nonca = include_bytes!("../data/bad-intermediate-nonca.der");
        let leaf = include_bytes!("../data/intermediate-leaf.der");
        let package = package_with_signing_certificate(leaf);
        let report = inspect_document_candidate_paths(
            &package,
            "_xmlsignatures/sig1.xml",
            &[root, wrong_root],
            &[intermediate, nonca],
            1796083200,
            CertificationPathSettings::new(),
        )
        .unwrap();
        assert_eq!(report.xml_signature, SignatureValueCheck::Differs);
        assert!(!report.candidate_search_truncated);
        assert!(report.candidates.len() >= 2);
        assert!(report.candidates.iter().any(|candidate| matches!(&candidate.result, DocumentCandidateResult::Evaluated(path) if path.engine_result.is_ok())));
        assert!(report.candidates.iter().any(|candidate| matches!(&candidate.result, DocumentCandidateResult::Evaluated(path) if path.engine_result.as_ref().is_err_and(|e| e.contains("InvalidBasicConstraints")))));
        assert!(report.candidates.iter().any(|candidate| matches!(&candidate.result, DocumentCandidateResult::Evaluated(path) if path.engine_result.as_ref().is_err_and(|e| e.contains("SignatureVerificationFailure")))));
        let no_intermediate = inspect_document_candidate_paths(
            &package,
            "_xmlsignatures/sig1.xml",
            &[root],
            &[],
            1796083200,
            CertificationPathSettings::new(),
        )
        .unwrap();
        assert!(no_intermediate.candidates.is_empty());
        assert!(!no_intermediate.candidate_search_truncated);
        assert_eq!(
            inspect_document_candidate_paths(
                &package,
                "_xmlsignatures/other.xml",
                &[root],
                &[intermediate],
                1796083200,
                CertificationPathSettings::new()
            ),
            Err(DocumentProbeError::UnregisteredSignature)
        );
        assert_eq!(
            inspect_document_candidate_paths(
                &package,
                "_xmlsignatures/sig1.xml",
                &[],
                &[],
                0,
                CertificationPathSettings::new()
            ),
            Err(DocumentProbeError::PathInput(
                PathProbeInputError::InvalidTime
            ))
        );
        assert_eq!(
            inspect_document_candidate_paths(
                &package,
                "_xmlsignatures/sig1.xml",
                &[root],
                &[intermediate],
                u64::MAX,
                CertificationPathSettings::new()
            ),
            Err(DocumentProbeError::PathInput(
                PathProbeInputError::InvalidTime
            ))
        );
        let original = ibis_ooxml::Package::open(
            include_bytes!(
                "../../../crates/ibis-ooxml/tests/fixtures/signatures/synthetic-signed.docx"
            )
            .as_slice(),
        )
        .unwrap();
        let signer = ibis_ooxml::signatures::extract_single_embedded_certificate(
            &original,
            "_xmlsignatures/sig1.xml",
        )
        .unwrap();
        let anchor_is_signer = inspect_document_candidate_paths(
            &original,
            "_xmlsignatures/sig1.xml",
            &[&signer],
            &[],
            1796083200,
            CertificationPathSettings::new(),
        )
        .unwrap();
        assert_eq!(
            anchor_is_signer.xml_signature,
            SignatureValueCheck::MatchesUntrustedCertificate
        );
        assert_eq!(anchor_is_signer.candidates.len(), 1);
        assert_eq!(
            anchor_is_signer.candidates[0].result,
            DocumentCandidateResult::AnchorIsSignerNotEvaluated
        );
    }

    #[test]
    fn signed_document_path_and_manifest_remain_distinct() {
        use ibis_ooxml::signatures::{ManifestDigest, ObjectDigest, SignatureValueCheck};
        let data = include_bytes!("../data/document-path-signed.docx");
        let root = include_bytes!("../data/document-path-root.der");
        let intermediate = include_bytes!("../data/document-path-intermediate.der");
        let part = "_xmlsignatures/sig1.xml";
        let mut package = ibis_ooxml::Package::open(data.as_slice()).unwrap();
        let inspect = |package: &ibis_ooxml::Package| {
            inspect_document_candidate_paths(
                package,
                part,
                &[root],
                &[intermediate],
                1796083200,
                CertificationPathSettings::new(),
            )
            .unwrap()
        };
        let report = inspect(&package);
        assert_eq!(
            report.xml_signature,
            SignatureValueCheck::MatchesUntrustedCertificate
        );
        assert_eq!(report.package_object_digest, ObjectDigest::Matches);
        assert_eq!(report.manifest_references.len(), 1);
        assert_eq!(
            report.manifest_references[0].digest,
            ManifestDigest::Matches
        );
        assert_eq!(report.candidates.len(), 1);
        assert!(
            matches!(&report.candidates[0].result, DocumentCandidateResult::Evaluated(path) if path.engine_result.is_ok())
        );
        package.set_bytes("word/document.xml", b"modified document".to_vec());
        let changed = inspect(&package);
        assert_eq!(
            changed.xml_signature,
            SignatureValueCheck::MatchesUntrustedCertificate
        );
        assert_eq!(changed.package_object_digest, ObjectDigest::Matches);
        assert_eq!(
            changed.manifest_references[0].digest,
            ManifestDigest::Differs
        );
        assert!(
            matches!(&changed.candidates[0].result, DocumentCandidateResult::Evaluated(path) if path.engine_result.is_ok())
        );
    }

    #[test]
    fn fingerprint_lookup_rejects_ambiguity() {
        let hash = [42u8; 32];
        let a = b"first".as_slice();
        let b = b"second".as_slice();
        assert_eq!(resolve_fingerprint(&hash, &[(hash, a), (hash, a)]), Ok(a));
        assert_eq!(
            resolve_fingerprint(&hash, &[(hash, a), (hash, b)]),
            Err(DocumentProbeError::AmbiguousFingerprint)
        );
        assert_eq!(
            resolve_fingerprint(&[0u8; 32], &[(hash, a)]),
            Err(DocumentProbeError::InvalidCandidate)
        );
    }

    #[test]
    fn path_report_keeps_engine_and_edge_observations_separate() {
        let settings = CertificationPathSettings::new;
        let root = include_bytes!("../data/root.der");
        let intermediate = include_bytes!("../data/intermediate.der");
        let nonca = include_bytes!("../data/bad-intermediate-nonca.der");
        let leaf = include_bytes!("../data/intermediate-leaf.der");
        let good =
            inspect_explicit_path(leaf, root, &[intermediate], 1796083200, settings()).unwrap();
        assert!(good.engine_result.is_ok());
        assert_eq!(
            good.edge_observations,
            vec![EdgeDiagnostic::Matches, EdgeDiagnostic::Matches]
        );
        let invalid_ca =
            inspect_explicit_path(leaf, root, &[nonca], 1796083200, settings()).unwrap();
        assert!(
            invalid_ca
                .engine_result
                .unwrap_err()
                .contains("InvalidBasicConstraints")
        );
        assert_eq!(
            invalid_ca.edge_observations,
            vec![EdgeDiagnostic::Matches, EdgeDiagnostic::Matches]
        );
        let critical = include_bytes!("../data/unknown-critical.der");
        let critical_report =
            inspect_explicit_path(critical, root, &[], 1796083200, settings()).unwrap();
        assert!(
            critical_report
                .engine_result
                .unwrap_err()
                .contains("UnprocessedCriticalExtension")
        );
        assert_eq!(
            critical_report.edge_observations,
            vec![EdgeDiagnostic::Matches]
        );
        let pss_ca =
            include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-ca.der");
        let pss = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-normal-leaf.der"
        );
        let mixed = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-mgf384-leaf.der"
        );
        let unsupported =
            inspect_explicit_path(mixed, pss_ca, &[], 1796083200, settings()).unwrap();
        let mut damaged = pss.to_vec();
        let n = damaged.len();
        damaged[n - 1] ^= 1;
        let bad_signature =
            inspect_explicit_path(&damaged, pss_ca, &[], 1796083200, settings()).unwrap();
        assert_eq!(unsupported.engine_result, bad_signature.engine_result);
        assert_eq!(
            unsupported.edge_observations,
            vec![EdgeDiagnostic::UnsupportedOrUnrecognized]
        );
        assert_eq!(
            bad_signature.edge_observations,
            vec![EdgeDiagnostic::BackendReportedSignatureFailure]
        );
        let input = inspect_explicit_path(b"bad", root, &[], 1796083200, settings()).unwrap();
        assert!(input.engine_result.is_err());
        assert_eq!(input.edge_observations, vec![EdgeDiagnostic::InvalidInput]);
        assert_eq!(
            inspect_explicit_path(
                leaf,
                root,
                &[intermediate.as_slice(); 8],
                1796083200,
                settings()
            ),
            Err(PathProbeInputError::TooManyIntermediates)
        );
        assert_eq!(
            inspect_explicit_path(leaf, root, &[], 0, settings()),
            Err(PathProbeInputError::InvalidTime)
        );
        assert_eq!(
            inspect_explicit_path(leaf, root, &[], u64::MAX, settings()),
            Err(PathProbeInputError::InvalidTime)
        );
        let oversize = vec![0; 1024 * 1024 + 1];
        assert_eq!(
            inspect_explicit_path(&oversize, root, &[], 1796083200, settings()),
            Err(PathProbeInputError::InputTooLarge)
        );
        let chunk = vec![0; 1024 * 1024];
        assert_eq!(
            inspect_explicit_path(
                &chunk,
                &chunk,
                &[&chunk, &chunk, &chunk],
                1796083200,
                settings()
            ),
            Err(PathProbeInputError::InputTooLarge)
        );
    }

    #[test]
    fn signature_algorithm_observations() {
        const P256_ROOT: &[u8] =
            include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/ec-ca.der");
        const P256_LEAF: &[u8] =
            include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/ec-leaf.der");
        const P384_ROOT: &[u8] =
            include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/p384-ca.der");
        const P384_LEAF: &[u8] = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/p384-normal-leaf.der"
        );
        const P384_SHA256: &[u8] = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/p384-sha256-leaf.der"
        );
        const PSS_ROOT: &[u8] =
            include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-ca.der");
        const PSS_OTHER: &[u8] =
            include_bytes!("../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-other.der");
        const PSS_NORMAL: &[u8] = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-normal-leaf.der"
        );
        const PSS_SALT20: &[u8] = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-salt20-leaf.der"
        );
        const PSS_MGF384: &[u8] = include_bytes!(
            "../../../crates/ibis-ooxml/tests/fixtures/signatures/pss-mgf384-leaf.der"
        );
        for (name, leaf, issuer) in [
            ("p256-sha256", P256_LEAF, P256_ROOT),
            ("p384-sha384", P384_LEAF, P384_ROOT),
            ("p384-sha256", P384_SHA256, P384_ROOT),
            ("pss-sha256-salt32", PSS_NORMAL, PSS_ROOT),
            ("pss-salt20", PSS_SALT20, PSS_ROOT),
        ] {
            let result = validate(leaf, issuer, 1796083200);
            println!("{name}: {result:?}");
            assert!(result.is_ok(), "{name}: {result:?}");
        }
        let unsupported = validate(PSS_MGF384, PSS_ROOT, 1796083200);
        let wrong_issuer = validate(PSS_NORMAL, PSS_OTHER, 1796083200);
        let mut damaged = PSS_NORMAL.to_vec();
        let end = damaged.len();
        damaged[end - 1] ^= 1;
        let invalid = validate(&damaged, PSS_ROOT, 1796083200);
        println!(
            "pss-mgf384: {unsupported:?}; wrong issuer: {wrong_issuer:?}; damaged: {invalid:?}"
        );
        // certval 0.2.0-rc.3's path API returns the same status for the
        // independently verified but unsupported MGF and a bad signature.
        assert_eq!(unsupported, invalid);
        assert_eq!(wrong_issuer, invalid);
        assert!(
            invalid
                .unwrap_err()
                .contains("SignatureVerificationFailure")
        );
    }

    #[test]
    fn policy_inputs_change_path_results() {
        let root = include_bytes!("../data/policy-root.der");
        let ca = include_bytes!("../data/policy-intermediate-a.der");
        let mapped_ca = include_bytes!("../data/policy-intermediate-mapped.der");
        let policy_a = include_bytes!("../data/policy-leaf-a.der");
        let policy_b = include_bytes!("../data/policy-leaf-b.der");
        let policy_any = include_bytes!("../data/policy-leaf-any.der");
        let no_policy = include_bytes!("../data/policy-leaf-absent.der");
        let evaluate = |leaf: &[u8],
                        issuer: &[u8],
                        explicit: bool,
                        inhibit_any: bool,
                        inhibit_mapping: bool| {
            let mut settings = CertificationPathSettings::new();
            settings.set_initial_policy_set(vec!["1.2.3.4".to_string()]);
            settings.set_initial_explicit_policy_indicator(explicit);
            settings.set_initial_inhibit_any_policy_indicator(inhibit_any);
            settings.set_initial_policy_mapping_inhibit_indicator(inhibit_mapping);
            validate_with_settings(leaf, root, &[issuer], 1796083200, settings)
        };
        for (label, result, rejected) in [
            (
                "matching-explicit",
                evaluate(policy_a, ca, true, false, false),
                false,
            ),
            (
                "mismatch-explicit",
                evaluate(policy_b, ca, true, false, false),
                true,
            ),
            (
                "mismatch-not-explicit",
                evaluate(policy_b, ca, false, false, false),
                false,
            ),
            (
                "absent-explicit",
                evaluate(no_policy, ca, true, false, false),
                true,
            ),
            (
                "absent-not-explicit",
                evaluate(no_policy, ca, false, false, false),
                false,
            ),
            (
                "any-allowed",
                evaluate(policy_any, ca, true, false, false),
                false,
            ),
            (
                "any-inhibited",
                evaluate(policy_any, ca, true, true, false),
                true,
            ),
            (
                "mapping-allowed",
                evaluate(policy_b, mapped_ca, true, false, false),
                false,
            ),
            (
                "mapping-inhibited",
                evaluate(policy_b, mapped_ca, true, false, true),
                true,
            ),
        ] {
            println!("{label}: {result:?}");
            if rejected {
                assert!(result.unwrap_err().contains("NullPolicySet"), "{label}");
            } else {
                assert!(result.is_ok(), "{label}: {result:?}");
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod cli;

#[cfg(not(target_arch = "wasm32"))]
pub fn cli_main() -> std::process::ExitCode {
    cli::main()
}

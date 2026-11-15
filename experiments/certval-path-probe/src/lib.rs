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

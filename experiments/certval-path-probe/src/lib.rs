// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Isolated experiment only; not used by ibisOffice or a trust decision.
#![allow(dead_code)]

use certval::{
    CertificationPath, CertificationPathResults, CertificationPathSettings, PDVCertificate,
    PDVTrustAnchorChoice, PkiEnvironment, TimeOfInterest,
};
fn validate(leaf: &[u8], anchor: &[u8], time: u64) -> Result<(), String> {
    validate_with_intermediates(leaf, anchor, &[], time)
}
fn validate_with_intermediates(
    leaf: &[u8],
    anchor: &[u8],
    intermediates: &[&[u8]],
    time: u64,
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
    let mut settings = CertificationPathSettings::new();
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
}

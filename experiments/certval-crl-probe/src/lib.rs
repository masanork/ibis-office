//! Offline backend experiment. Engine results are observations, never revocation decisions.
use certval::{
    CertificationPath, CertificationPathResults, CertificationPathSettings, Error, PDVCertificate,
    PDVTrustAnchorChoice, PathValidationStatus, PkiEnvironment, TimeOfInterest,
    check_revocation_local,
};

#[derive(Debug, PartialEq, Eq)]
pub enum RawCrlResult {
    NotRequested,
    PathFailed,
    EngineOk,
    EngineRevoked,
    EngineUndetermined,
    EngineError(String),
}
#[derive(Debug)]
pub struct Observation {
    pub path: Result<(), String>,
    pub crl: RawCrlResult,
}

/// The anchor, target and optional CRL are supplied by the caller; no stores or network are used.
/// EngineOk is not a finding of good revocation status, even when a path succeeds.
pub fn observe(
    anchor: &[u8],
    leaf: &[u8],
    crl: Option<&[u8]>,
    time: u64,
    requested: bool,
) -> Result<Observation, String> {
    if time == 0 {
        return Err("zero time disables backend time checks".into());
    }
    if anchor.is_empty()
        || leaf.is_empty()
        || anchor.len() > 1_048_576
        || leaf.len() > 1_048_576
        || crl.is_some_and(|c| c.len() > 16_777_216)
    {
        return Err("input size outside experiment bounds".into());
    }
    if anchor == leaf {
        return Err("anchor-as-target is outside the experiment".into());
    }
    let ta = PDVTrustAnchorChoice::try_from(anchor).map_err(|e| format!("anchor: {e:?}"))?;
    let target = PDVCertificate::try_from(leaf).map_err(|e| format!("target: {e:?}"))?;
    let mut path = CertificationPath::new(ta, vec![], target);
    path.crls[0] = crl.map(<[u8]>::to_vec);
    let mut env = PkiEnvironment::default();
    env.populate_5280_pki_environment();
    let mut settings = CertificationPathSettings::new();
    settings.set_require_ta_store(false);
    settings.set_time_of_interest(
        TimeOfInterest::from_unix_secs(time).map_err(|e| format!("time: {e:?}"))?,
    );
    settings.set_check_revocation_status(true);
    settings.set_check_crls(true);
    let mut results = CertificationPathResults::new();
    let validation = env
        .validate_path(&env, &settings, &path, &mut results)
        .map_err(|e| format!("{e:?}; {:?}", results.get_validation_status()));
    if validation.is_err() {
        return Ok(Observation {
            path: validation,
            crl: RawCrlResult::PathFailed,
        });
    }
    if !requested {
        return Ok(Observation {
            path: validation,
            crl: RawCrlResult::NotRequested,
        });
    }
    let outcome = match check_revocation_local(&env, &settings, &path, &mut results) {
        Ok(()) => RawCrlResult::EngineOk,
        Err(Error::PathValidation(PathValidationStatus::CertificateRevokedEndEntity)) => {
            RawCrlResult::EngineRevoked
        }
        Err(Error::PathValidation(PathValidationStatus::RevocationStatusNotDetermined)) => {
            RawCrlResult::EngineUndetermined
        }
        Err(e) => RawCrlResult::EngineError(format!("{e:?}")),
    };
    Ok(Observation {
        path: validation,
        crl: outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const ROOT: &[u8] = include_bytes!("../data/root.der");
    const LEAF: &[u8] = include_bytes!("../data/leaf.der");
    const T: u64 = 1796083200;
    fn run(crl: Option<&[u8]>) -> RawCrlResult {
        let observed = observe(ROOT, LEAF, crl, T, true).unwrap();
        assert_eq!(observed.path, Ok(()));
        observed.crl
    }
    #[test]
    fn ordinary_crls() {
        assert_eq!(
            run(Some(include_bytes!("../data/empty.der"))),
            RawCrlResult::EngineOk
        );
        assert_eq!(
            run(Some(include_bytes!("../data/revoked.der"))),
            RawCrlResult::EngineRevoked
        );
        for crl in [
            None,
            Some(&include_bytes!("../data/wrong-key.der")[..]),
            Some(&include_bytes!("../data/future.der")[..]),
            Some(&include_bytes!("../data/expired.der")[..]),
            Some(&include_bytes!("../data/critical-unknown.der")[..]),
            Some(&b"bad"[..]),
        ] {
            assert_eq!(run(crl), RawCrlResult::EngineUndetermined);
        }
    }
    #[test]
    fn documented_profile_gaps_are_only_raw_backend_observations() {
        for crl in [
            &include_bytes!("../data/missing-number.der")[..],
            &include_bytes!("../data/critical-number.der")[..],
            &include_bytes!("../data/duplicate-unrelated.der")[..],
        ] {
            assert_eq!(run(Some(crl)), RawCrlResult::EngineOk);
        }
    }
    #[test]
    fn gates_do_not_treat_skipped_checks_as_success() {
        assert_eq!(
            observe(ROOT, LEAF, None, T, false).unwrap().crl,
            RawCrlResult::NotRequested
        );
        assert!(observe(ROOT, LEAF, None, 0, true).is_err());
        assert!(observe(ROOT, ROOT, None, T, true).is_err());
        let wrong = include_bytes!("../../certval-path-probe/data/same-name-wrong.der");
        let o = observe(wrong, LEAF, None, T, true).unwrap();
        assert!(o.path.is_err());
        assert_eq!(o.crl, RawCrlResult::PathFailed);
    }
}

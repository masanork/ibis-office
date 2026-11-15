// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Offline OCSP observations. Nothing here establishes revocation or trust.

use crate::{
    Error,
    signatures::{
        CertificateSignature, inspect_ecdsa_x509_signature, inspect_rsa_pss_x509_signature,
        rsa_x509_hash, verify_rsa_sha2,
    },
};
use sha2::{Digest, Sha256};
use x509_cert::{
    Certificate,
    der::{Decode, Encode, asn1::Any},
};
use x509_ocsp::{BasicOcspResponse, CertStatus, OcspResponse, OcspResponseStatus, ResponderId};

#[derive(Debug, PartialEq, Eq)]
pub struct OcspInspection {
    /// Protocol status, not certificate status.
    pub response_status: OcspResponseStatus,
    /// OID of the response body, if present.
    pub response_type: Option<String>,
    /// Unsupported response types and unsuccessful responses have no basic facts.
    pub basic: Option<OcspBasicFacts>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct OcspBasicFacts {
    pub signature: CertificateSignature,
    pub responder_id: OcspResponderId,
    pub issuer_name_exact: bool,
    pub produced_at: u64,
    pub produced_after_reference_time: bool,
    pub response_extensions_present: bool,
    pub embedded_certificates: usize,
    pub total_responses: usize,
    pub responses_truncated: bool,
    pub responses: Vec<OcspSingleFacts>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OcspResponderId {
    ByNameExact(bool),
    ByKeySha1(bool),
}

#[derive(Debug, PartialEq, Eq)]
pub struct OcspSingleFacts {
    pub cert_id: OcspCertId,
    /// This is an unauthenticated claim from the response, never a verdict.
    pub claimed_status: OcspClaim,
    pub this_update: u64,
    pub next_update: Option<u64>,
    pub time_window: OcspTimeWindow,
    pub extensions_present: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OcspCertId {
    Compared {
        issuer_name_hash_equal: bool,
        issuer_key_hash_equal: bool,
        serial_equal: bool,
    },
    Unsupported,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OcspClaim {
    Good,
    Revoked { revocation_time: u64 },
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OcspTimeWindow {
    NotYetValid,
    WithinWindow,
    Expired,
    NoNextUpdate,
    InvalidWindow,
}

fn signature_observation(
    algorithm: &x509_cert::spki::AlgorithmIdentifierOwned,
    responder: &Certificate,
    message: &[u8],
    signature: &[u8],
) -> Result<CertificateSignature, Error> {
    use rsa::{RsaPublicKey, pkcs1::DecodeRsaPublicKey, pkcs1v15, traits::PublicKeyParts};
    let spki = &responder.tbs_certificate.subject_public_key_info;
    let oid = algorithm.oid.to_string();
    if oid == "1.2.840.113549.1.1.10" {
        return inspect_rsa_pss_x509_signature(algorithm, spki, message, signature);
    }
    if matches!(oid.as_str(), "1.2.840.10045.4.3.2" | "1.2.840.10045.4.3.3") {
        return inspect_ecdsa_x509_signature(algorithm, spki, message, signature);
    }
    let null_or_absent = |p: &Option<Any>| {
        p.as_ref()
            .is_none_or(|v| v.to_der().is_ok_and(|v| v == [0x05, 0x00]))
    };
    let Some(hash) = rsa_x509_hash(&oid) else {
        return Ok(CertificateSignature::Unsupported);
    };
    if !null_or_absent(&algorithm.parameters)
        || spki.algorithm.oid.to_string() != "1.2.840.113549.1.1.1"
        || !null_or_absent(&spki.algorithm.parameters)
    {
        return Ok(CertificateSignature::Unsupported);
    }
    let raw = spki
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| Error::new("Invalid OCSP responder public key"))?;
    let key = RsaPublicKey::from_pkcs1_der(raw)
        .map_err(|_| Error::new("Invalid OCSP responder RSA key"))?;
    if !(2048..=8192).contains(&key.n().bits()) {
        return Ok(CertificateSignature::Unsupported);
    }
    Ok(match pkcs1v15::Signature::try_from(signature) {
        Ok(sig) if verify_rsa_sha2(hash, key, message, &sig) => CertificateSignature::Matches,
        _ => CertificateSignature::Differs,
    })
}

/// Inspect a caller-provided OCSP response and certificates at an explicit time.
/// Does not fetch data, authorize responders, validate chains, enforce nonce or
/// extension semantics, or decide whether a certificate is revoked.
pub fn inspect_ocsp(
    response_der: &[u8],
    certificate_der: &[u8],
    issuer_der: &[u8],
    responder_der: &[u8],
    reference_time: u64,
) -> Result<OcspInspection, Error> {
    if response_der.len() > 8 * 1024 * 1024
        || [certificate_der, issuer_der, responder_der]
            .iter()
            .any(|c| c.len() > 1024 * 1024)
    {
        return Err(Error::new("OCSP inputs are too large"));
    }
    let response =
        OcspResponse::from_der(response_der).map_err(|_| Error::new("Invalid OCSP response"))?;
    let mut result = OcspInspection {
        response_status: response.response_status,
        response_type: response
            .response_bytes
            .as_ref()
            .map(|v| v.response_type.to_string()),
        basic: None,
    };
    if response.response_status != OcspResponseStatus::Successful {
        if response.response_bytes.is_some() {
            return Err(Error::new("Unexpected OCSP response body"));
        }
        return Ok(result);
    }
    let bytes = response
        .response_bytes
        .ok_or_else(|| Error::new("Missing OCSP response body"))?;
    if bytes.response_type.to_string() != "1.3.6.1.5.5.7.48.1.1" {
        return Ok(result);
    }
    let basic = BasicOcspResponse::from_der(bytes.response.as_bytes())
        .map_err(|_| Error::new("Invalid basic OCSP response"))?;
    let certificate = Certificate::from_der(certificate_der)
        .map_err(|_| Error::new("Invalid target certificate"))?;
    let issuer = Certificate::from_der(issuer_der)
        .map_err(|_| Error::new("Invalid OCSP issuer certificate"))?;
    let responder = Certificate::from_der(responder_der)
        .map_err(|_| Error::new("Invalid OCSP responder certificate"))?;
    let issuer_key = issuer
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| Error::new("Invalid OCSP issuer public key"))?;
    let responder_key = responder
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| Error::new("Invalid OCSP responder public key"))?;
    let issuer_name = certificate
        .tbs_certificate
        .issuer
        .to_der()
        .map_err(|_| Error::new("Invalid certificate issuer name"))?;
    let responder_id = match &basic.tbs_response_data.responder_id {
        ResponderId::ByName(name) => {
            OcspResponderId::ByNameExact(name == &responder.tbs_certificate.subject)
        }
        ResponderId::ByKey(hash) => OcspResponderId::ByKeySha1(
            hash.as_bytes() == sha1::Sha1::digest(responder_key).as_slice(),
        ),
    };
    let signed = basic
        .tbs_response_data
        .to_der()
        .map_err(|_| Error::new("Invalid OCSP response data"))?;
    let signature = basic
        .signature
        .as_bytes()
        .ok_or_else(|| Error::new("Invalid OCSP signature"))?;
    let signature =
        signature_observation(&basic.signature_algorithm, &responder, &signed, signature)?;
    let produced_at = basic
        .tbs_response_data
        .produced_at
        .0
        .to_unix_duration()
        .as_secs();
    let total_responses = basic.tbs_response_data.responses.len();
    let mut facts = OcspBasicFacts {
        signature,
        responder_id,
        issuer_name_exact: certificate.tbs_certificate.issuer == issuer.tbs_certificate.subject,
        produced_at,
        produced_after_reference_time: produced_at > reference_time,
        response_extensions_present: basic.tbs_response_data.response_extensions.is_some(),
        embedded_certificates: basic.certs.as_ref().map_or(0, Vec::len),
        total_responses,
        responses_truncated: total_responses > 64,
        responses: Vec::new(),
    };
    for entry in basic.tbs_response_data.responses.iter().take(64) {
        let algorithm = &entry.cert_id.hash_algorithm;
        let valid_params = algorithm
            .parameters
            .as_ref()
            .is_none_or(|v| v.to_der().is_ok_and(|v| v == [0x05, 0x00]));
        let hashes = if !valid_params {
            None
        } else {
            match algorithm.oid.to_string().as_str() {
                "1.3.14.3.2.26" => Some((
                    sha1::Sha1::digest(&issuer_name).to_vec(),
                    sha1::Sha1::digest(issuer_key).to_vec(),
                )),
                "2.16.840.1.101.3.4.2.1" => Some((
                    Sha256::digest(&issuer_name).to_vec(),
                    Sha256::digest(issuer_key).to_vec(),
                )),
                _ => None,
            }
        };
        let cert_id = match hashes {
            Some((name, key)) => OcspCertId::Compared {
                issuer_name_hash_equal: entry.cert_id.issuer_name_hash.as_bytes() == name,
                issuer_key_hash_equal: entry.cert_id.issuer_key_hash.as_bytes() == key,
                serial_equal: entry.cert_id.serial_number
                    == certificate.tbs_certificate.serial_number,
            },
            None => OcspCertId::Unsupported,
        };
        let claimed_status = match entry.cert_status {
            CertStatus::Good(_) => OcspClaim::Good,
            CertStatus::Unknown(_) => OcspClaim::Unknown,
            CertStatus::Revoked(revoked) => OcspClaim::Revoked {
                revocation_time: revoked.revocation_time.0.to_unix_duration().as_secs(),
            },
        };
        let this_update = entry.this_update.0.to_unix_duration().as_secs();
        let next_update = entry.next_update.map(|t| t.0.to_unix_duration().as_secs());
        let time_window = if next_update.is_some_and(|n| n < this_update) {
            OcspTimeWindow::InvalidWindow
        } else if reference_time < this_update {
            OcspTimeWindow::NotYetValid
        } else {
            match next_update {
                Some(end) if reference_time >= end => OcspTimeWindow::Expired,
                Some(_) => OcspTimeWindow::WithinWindow,
                None => OcspTimeWindow::NoNextUpdate,
            }
        };
        facts.responses.push(OcspSingleFacts {
            cert_id,
            claimed_status,
            this_update,
            next_update,
            time_window,
            extensions_present: entry.single_extensions.is_some(),
        });
    }
    result.basic = Some(facts);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use x509_cert::der::asn1::{Any, BitString, GeneralizedTime, ObjectIdentifier, OctetString};
    use x509_ocsp::{OcspGeneralizedTime, OcspResponse};

    const LEAF: &[u8] = include_bytes!("../tests/fixtures/signatures/ocsp-leaf.der");
    const ISSUER: &[u8] = include_bytes!("../tests/fixtures/signatures/ocsp-issuer.der");
    const OTHER: &[u8] = include_bytes!("../tests/fixtures/signatures/ocsp-other-issuer.der");
    const RESPONDER: &[u8] = include_bytes!("../tests/fixtures/signatures/ocsp-responder.der");
    const GOOD: &[u8] = include_bytes!("../tests/fixtures/signatures/ocsp-good.der");
    const REVOKED: &[u8] = include_bytes!("../tests/fixtures/signatures/ocsp-revoked.der");
    const UNKNOWN: &[u8] = include_bytes!("../tests/fixtures/signatures/ocsp-unknown.der");

    fn modify(response: &[u8], edit: impl FnOnce(&mut BasicOcspResponse)) -> Vec<u8> {
        let mut outer = OcspResponse::from_der(response).unwrap();
        let bytes = outer.response_bytes.as_mut().unwrap();
        let mut basic = BasicOcspResponse::from_der(bytes.response.as_bytes()).unwrap();
        edit(&mut basic);
        bytes.response = OctetString::new(basic.to_der().unwrap()).unwrap();
        outer.to_der().unwrap()
    }

    #[test]
    fn ocsp_reports_claims_ids_signatures_and_time_without_trust() {
        let initial = inspect_ocsp(GOOD, LEAF, ISSUER, RESPONDER, 0).unwrap();
        assert_eq!(initial.response_status, OcspResponseStatus::Successful);
        let basic = initial.basic.unwrap();
        assert_eq!(basic.signature, CertificateSignature::Matches);
        assert_eq!(basic.responder_id, OcspResponderId::ByKeySha1(true));
        assert!(basic.issuer_name_exact);
        assert!(basic.produced_after_reference_time);
        assert_eq!(basic.embedded_certificates, 1);
        assert_eq!(
            basic.responses[0].cert_id,
            OcspCertId::Compared {
                issuer_name_hash_equal: true,
                issuer_key_hash_equal: true,
                serial_equal: true
            }
        );
        assert_eq!(basic.responses[0].claimed_status, OcspClaim::Good);
        assert_eq!(basic.responses[0].time_window, OcspTimeWindow::NotYetValid);
        let at_start = basic.responses[0].this_update;
        let next = basic.responses[0].next_update.unwrap();
        assert_eq!(
            inspect_ocsp(GOOD, LEAF, ISSUER, RESPONDER, at_start)
                .unwrap()
                .basic
                .unwrap()
                .responses[0]
                .time_window,
            OcspTimeWindow::WithinWindow
        );
        assert_eq!(
            inspect_ocsp(GOOD, LEAF, ISSUER, RESPONDER, next)
                .unwrap()
                .basic
                .unwrap()
                .responses[0]
                .time_window,
            OcspTimeWindow::Expired
        );
        let wrong_issuer = inspect_ocsp(GOOD, LEAF, OTHER, RESPONDER, 0)
            .unwrap()
            .basic
            .unwrap();
        assert!(wrong_issuer.issuer_name_exact);
        assert_eq!(
            wrong_issuer.responses[0].cert_id,
            OcspCertId::Compared {
                issuer_name_hash_equal: true,
                issuer_key_hash_equal: false,
                serial_equal: true
            }
        );
        let wrong_responder = inspect_ocsp(GOOD, LEAF, ISSUER, OTHER, 0)
            .unwrap()
            .basic
            .unwrap();
        assert_eq!(wrong_responder.signature, CertificateSignature::Differs);
        assert_eq!(
            wrong_responder.responder_id,
            OcspResponderId::ByKeySha1(false)
        );
        let revoked = inspect_ocsp(REVOKED, LEAF, ISSUER, RESPONDER, 0)
            .unwrap()
            .basic
            .unwrap();
        assert_eq!(revoked.signature, CertificateSignature::Matches);
        assert_eq!(revoked.responder_id, OcspResponderId::ByNameExact(true));
        assert!(revoked.response_extensions_present);
        assert!(matches!(
            revoked.responses[0].claimed_status,
            OcspClaim::Revoked { .. }
        ));
        assert_eq!(
            revoked.responses[0].cert_id,
            OcspCertId::Compared {
                issuer_name_hash_equal: true,
                issuer_key_hash_equal: true,
                serial_equal: true
            }
        );
        let unknown = inspect_ocsp(UNKNOWN, LEAF, ISSUER, RESPONDER, u64::MAX)
            .unwrap()
            .basic
            .unwrap();
        assert_eq!(unknown.responses[0].claimed_status, OcspClaim::Unknown);
        assert_eq!(
            unknown.responses[0].time_window,
            OcspTimeWindow::NoNextUpdate
        );
        let unsuccessful = inspect_ocsp(
            include_bytes!("../tests/fixtures/signatures/ocsp-unauthorized.der"),
            LEAF,
            ISSUER,
            RESPONDER,
            0,
        )
        .unwrap();
        assert_eq!(
            unsuccessful.response_status,
            OcspResponseStatus::Unauthorized
        );
        assert!(unsuccessful.basic.is_none());
        assert!(inspect_ocsp(b"bad", LEAF, ISSUER, RESPONDER, 0).is_err());
    }

    #[test]
    fn ocsp_rejects_unsupported_ids_and_reports_conflicts_and_limits() {
        let invalid_hash = modify(GOOD, |b| {
            b.tbs_response_data.responses[0].cert_id.hash_algorithm.oid =
                ObjectIdentifier::new_unwrap("1.2.3.4")
        });
        let basic = inspect_ocsp(&invalid_hash, LEAF, ISSUER, RESPONDER, 0)
            .unwrap()
            .basic
            .unwrap();
        assert_eq!(basic.signature, CertificateSignature::Differs);
        assert_eq!(basic.responses[0].cert_id, OcspCertId::Unsupported);
        let invalid_params = modify(GOOD, |b| {
            b.tbs_response_data.responses[0]
                .cert_id
                .hash_algorithm
                .parameters = Some(Any::new(x509_cert::der::Tag::Integer, vec![1]).unwrap())
        });
        assert_eq!(
            inspect_ocsp(&invalid_params, LEAF, ISSUER, RESPONDER, 0)
                .unwrap()
                .basic
                .unwrap()
                .responses[0]
                .cert_id,
            OcspCertId::Unsupported
        );
        let damaged = modify(GOOD, |b| {
            let mut sig = b.signature.as_bytes().unwrap().to_vec();
            sig[0] ^= 1;
            b.signature = BitString::from_bytes(&sig).unwrap();
        });
        assert_eq!(
            inspect_ocsp(&damaged, LEAF, ISSUER, RESPONDER, 0)
                .unwrap()
                .basic
                .unwrap()
                .signature,
            CertificateSignature::Differs
        );
        let conflict = modify(GOOD, |b| {
            let mut entry = b.tbs_response_data.responses[0].clone();
            entry.cert_status = CertStatus::Unknown(x509_cert::der::asn1::Null);
            b.tbs_response_data.responses.push(entry.clone());
            for _ in 0..63 {
                b.tbs_response_data.responses.push(entry.clone());
            }
        });
        let basic = inspect_ocsp(&conflict, LEAF, ISSUER, RESPONDER, 0)
            .unwrap()
            .basic
            .unwrap();
        assert_eq!(basic.signature, CertificateSignature::Differs);
        assert_eq!(basic.total_responses, 65);
        assert!(basic.responses_truncated);
        assert_eq!(basic.responses.len(), 64);
        assert_eq!(basic.responses[0].claimed_status, OcspClaim::Good);
        assert_eq!(basic.responses[1].claimed_status, OcspClaim::Unknown);
        let bad_window = modify(GOOD, |b| {
            let entry = &mut b.tbs_response_data.responses[0];
            let earlier = entry.this_update.0.to_unix_duration().as_secs() - 1;
            entry.next_update = Some(OcspGeneralizedTime(
                GeneralizedTime::from_unix_duration(std::time::Duration::from_secs(earlier))
                    .unwrap(),
            ));
        });
        assert_eq!(
            inspect_ocsp(&bad_window, LEAF, ISSUER, RESPONDER, 0)
                .unwrap()
                .basic
                .unwrap()
                .responses[0]
                .time_window,
            OcspTimeWindow::InvalidWindow
        );
    }
}

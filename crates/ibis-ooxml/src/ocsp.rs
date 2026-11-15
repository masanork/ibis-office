// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Offline OCSP observations. Nothing here establishes revocation or trust.

use crate::{
    Error,
    signatures::{
        CertificateLinkFacts, CertificateSignature, inspect_certificate_link,
        inspect_ecdsa_x509_signature, inspect_rsa_pss_x509_signature, rsa_x509_hash,
        verify_rsa_sha2,
    },
};
use sha2::{Digest, Sha256};
use x509_cert::{
    Certificate,
    der::{Decode, Encode, asn1::Any},
};
use x509_ocsp::{
    BasicOcspResponse, CertStatus, OcspRequest, OcspResponse, OcspResponseStatus, ResponderId,
};

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
    pub responder_certificate: OcspResponderCertificateFacts,
    pub issuer_name_exact: bool,
    pub produced_at: u64,
    pub produced_after_reference_time: bool,
    pub response_extensions_present: bool,
    /// Response nonce observation; not proof of freshness or authenticity.
    pub nonce: OcspNonce,
    pub embedded_certificates: usize,
    pub total_responses: usize,
    pub responses_truncated: bool,
    pub responses: Vec<OcspSingleFacts>,
}

/// Observations about the caller-supplied responder certificate. These do not
/// establish authorization, delegation, certificate validity, or issuer trust.
#[derive(Debug, PartialEq, Eq)]
pub struct OcspResponderCertificateFacts {
    pub identical_to_issuer: bool,
    /// None means the proposed certificate link could not be inspected.
    pub proposed_issuer_link: Option<CertificateLinkFacts>,
    pub not_before: u64,
    pub not_after: u64,
    pub at_reference_time: OcspCertificateTime,
    pub at_produced_at: OcspCertificateTime,
    pub key_usage: OcspUsage,
    pub ocsp_signing_eku: OcspUsage,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OcspCertificateTime {
    Before,
    Within,
    After,
    InvalidInterval,
}

/// Presence and relevant bit or OID; invalid includes duplicate extensions.
#[derive(Debug, PartialEq, Eq)]
pub enum OcspUsage {
    Absent,
    Present(bool),
    InvalidOrDuplicate,
}

fn responder_certificate_facts(
    responder: &Certificate,
    responder_der: &[u8],
    issuer_der: &[u8],
    reference_time: u64,
    produced_at: u64,
) -> OcspResponderCertificateFacts {
    use x509_cert::ext::pkix::{ExtendedKeyUsage, KeyUsage};
    let tbs = &responder.tbs_certificate;
    let not_before = tbs.validity.not_before.to_unix_duration().as_secs();
    let not_after = tbs.validity.not_after.to_unix_duration().as_secs();
    let at = |time| {
        if not_before > not_after {
            OcspCertificateTime::InvalidInterval
        } else if time < not_before {
            OcspCertificateTime::Before
        } else if time > not_after {
            OcspCertificateTime::After
        } else {
            OcspCertificateTime::Within
        }
    };
    let key_usage = match tbs.get::<KeyUsage>() {
        Ok(None) => OcspUsage::Absent,
        Ok(Some((_, usage))) => OcspUsage::Present(usage.digital_signature()),
        Err(_) => OcspUsage::InvalidOrDuplicate,
    };
    let ocsp_signing_eku = match tbs.get::<ExtendedKeyUsage>() {
        Ok(None) => OcspUsage::Absent,
        Ok(Some((_, usage))) if !usage.0.is_empty() => OcspUsage::Present(
            usage
                .0
                .iter()
                .any(|oid| oid.to_string() == "1.3.6.1.5.5.7.3.9"),
        ),
        _ => OcspUsage::InvalidOrDuplicate,
    };
    OcspResponderCertificateFacts {
        identical_to_issuer: responder_der == issuer_der,
        proposed_issuer_link: inspect_certificate_link(responder_der, issuer_der).ok(),
        not_before,
        not_after,
        at_reference_time: at(reference_time),
        at_produced_at: at(produced_at),
        key_usage,
        ocsp_signing_eku,
    }
}

/// Unauthenticated response nonce observation, separate from any policy decision.
#[derive(Debug, PartialEq, Eq)]
pub enum OcspNonce {
    Absent,
    Duplicate,
    InvalidEncoding,
    InvalidLength {
        length: usize,
    },
    Present {
        length: usize,
        critical: bool,
        comparison: OcspNonceComparison,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum OcspNonceComparison {
    NotProvided,
    Matches,
    Differs,
}

fn observe_nonce(
    extensions: Option<&x509_cert::ext::Extensions>,
    expected: Option<&[u8]>,
) -> OcspNonce {
    use x509_cert::der::asn1::OctetString;
    let mut found = extensions
        .into_iter()
        .flatten()
        .filter(|ext| ext.extn_id.to_string() == "1.3.6.1.5.5.7.48.1.2");
    let Some(ext) = found.next() else {
        return OcspNonce::Absent;
    };
    if found.next().is_some() {
        return OcspNonce::Duplicate;
    }
    let Ok(nonce) = OctetString::from_der(ext.extn_value.as_bytes()) else {
        return OcspNonce::InvalidEncoding;
    };
    let bytes = nonce.as_bytes();
    if !(1..=32).contains(&bytes.len()) {
        return OcspNonce::InvalidLength {
            length: bytes.len(),
        };
    }
    let comparison = match expected {
        None => OcspNonceComparison::NotProvided,
        Some(value) if value == bytes => OcspNonceComparison::Matches,
        Some(_) => OcspNonceComparison::Differs,
    };
    OcspNonce::Present {
        length: bytes.len(),
        critical: ext.critical,
        comparison,
    }
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

/// Bounded comparisons of untrusted request and response data, not request
/// authentication, response acceptance, or a revocation decision.
#[derive(Debug, PartialEq, Eq)]
pub struct OcspRequestObservations {
    pub signature_present: bool,
    pub requestor_name_present: bool,
    pub request_extensions_present: bool,
    pub nonce: OcspNonce,
    pub total_requests: usize,
    pub requests_truncated: bool,
    pub response_status: OcspResponseStatus,
    /// None means no supported BasicOCSPResponse was available for comparison.
    pub response: Option<OcspRequestResponseObservations>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct OcspRequestResponseObservations {
    pub nonce: OcspNonce,
    pub total_responses: usize,
    pub responses_truncated: bool,
    /// One count per first 64 request entries; counts use only first 64 responses.
    /// Matching is exact ASN.1 structure equality, not semantic equivalence.
    pub exact_response_cert_id_counts: Vec<usize>,
    pub single_request_extensions_present: Vec<bool>,
}

/// Compare a caller-provided request with a response without trusting either.
/// Neither request signature nor identity, request provenance, response
/// authorization, freshness, extension semantics, or status is validated.
pub fn inspect_ocsp_request(
    request_der: &[u8],
    response_der: &[u8],
) -> Result<OcspRequestObservations, Error> {
    use x509_cert::der::asn1::OctetString;
    if request_der.len() > 1024 * 1024 || response_der.len() > 8 * 1024 * 1024 {
        return Err(Error::new("OCSP request or response is too large"));
    }
    let request =
        OcspRequest::from_der(request_der).map_err(|_| Error::new("Invalid OCSP request"))?;
    let response =
        OcspResponse::from_der(response_der).map_err(|_| Error::new("Invalid OCSP response"))?;
    let request_nonce = observe_nonce(request.tbs_request.request_extensions.as_ref(), None);
    // Only use a uniquely present, well-formed nonce of permitted length.
    let expected = if matches!(request_nonce, OcspNonce::Present { .. }) {
        request
            .tbs_request
            .request_extensions
            .as_ref()
            .and_then(|extensions| {
                extensions
                    .iter()
                    .find(|ext| ext.extn_id.to_string() == "1.3.6.1.5.5.7.48.1.2")
            })
            .and_then(|ext| OctetString::from_der(ext.extn_value.as_bytes()).ok())
    } else {
        None
    };
    let mut result = OcspRequestObservations {
        signature_present: request.optional_signature.is_some(),
        requestor_name_present: request.tbs_request.requestor_name.is_some(),
        request_extensions_present: request.tbs_request.request_extensions.is_some(),
        nonce: request_nonce,
        total_requests: request.tbs_request.request_list.len(),
        requests_truncated: request.tbs_request.request_list.len() > 64,
        response_status: response.response_status,
        response: None,
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
    let requests = request.tbs_request.request_list.iter().take(64);
    let counts = requests
        .clone()
        .map(|entry| {
            basic
                .tbs_response_data
                .responses
                .iter()
                .take(64)
                .filter(|response| response.cert_id == entry.req_cert)
                .count()
        })
        .collect();
    result.response = Some(OcspRequestResponseObservations {
        nonce: observe_nonce(
            basic.tbs_response_data.response_extensions.as_ref(),
            expected.as_ref().map(|nonce| nonce.as_bytes()),
        ),
        total_responses: basic.tbs_response_data.responses.len(),
        responses_truncated: basic.tbs_response_data.responses.len() > 64,
        exact_response_cert_id_counts: counts,
        single_request_extensions_present: requests
            .map(|entry| entry.single_request_extensions.is_some())
            .collect(),
    });
    Ok(result)
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
    inspect_ocsp_with_nonce(
        response_der,
        certificate_der,
        issuer_der,
        responder_der,
        reference_time,
        None,
    )
}

/// Inspect with optional expected raw nonce bytes, without enforcing a nonce
/// policy or proving that the expected bytes came from a particular request.
pub fn inspect_ocsp_with_nonce(
    response_der: &[u8],
    certificate_der: &[u8],
    issuer_der: &[u8],
    responder_der: &[u8],
    reference_time: u64,
    expected_nonce: Option<&[u8]>,
) -> Result<OcspInspection, Error> {
    if expected_nonce.is_some_and(|nonce| !(1..=32).contains(&nonce.len())) {
        return Err(Error::new("Expected OCSP nonce must contain 1 to 32 bytes"));
    }
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
        responder_certificate: responder_certificate_facts(
            &responder,
            responder_der,
            issuer_der,
            reference_time,
            produced_at,
        ),
        issuer_name_exact: certificate.tbs_certificate.issuer == issuer.tbs_certificate.subject,
        produced_at,
        produced_after_reference_time: produced_at > reference_time,
        response_extensions_present: basic.tbs_response_data.response_extensions.is_some(),
        nonce: observe_nonce(
            basic.tbs_response_data.response_extensions.as_ref(),
            expected_nonce,
        ),
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
    fn request_response_comparison_is_bounded_and_untrusted() {
        let request = include_bytes!("../tests/fixtures/signatures/ocsp-request.der");
        let facts = inspect_ocsp_request(request, REVOKED).unwrap();
        assert!(!facts.signature_present);
        assert_eq!(facts.total_requests, 1);
        assert_eq!(
            facts.nonce,
            OcspNonce::Present {
                length: 15,
                critical: false,
                comparison: OcspNonceComparison::NotProvided
            }
        );
        let response = facts.response.unwrap();
        assert_eq!(
            response.nonce,
            OcspNonce::Present {
                length: 15,
                critical: false,
                comparison: OcspNonceComparison::Matches
            }
        );
        assert_eq!(response.exact_response_cert_id_counts, vec![1]);
        assert_eq!(response.single_request_extensions_present, vec![false]);
        let different = inspect_ocsp_request(request, GOOD)
            .unwrap()
            .response
            .unwrap();
        assert_eq!(different.exact_response_cert_id_counts, vec![0]);
        assert_eq!(different.nonce, OcspNonce::Absent);
        assert!(inspect_ocsp_request(b"bad", REVOKED).is_err());
        assert!(inspect_ocsp_request(request, b"bad").is_err());
        assert!(inspect_ocsp_request(&vec![0; 1024 * 1024 + 1], REVOKED).is_err());
        let unsigned_status = inspect_ocsp_request(
            request,
            include_bytes!("../tests/fixtures/signatures/ocsp-unauthorized.der"),
        )
        .unwrap();
        assert_eq!(
            unsigned_status.response_status,
            OcspResponseStatus::Unauthorized
        );
        assert!(unsigned_status.response.is_none());

        let mut parsed = OcspRequest::from_der(request).unwrap();
        parsed.tbs_request.request_extensions = None;
        let unsigned = OcspResponse::from_der(REVOKED)
            .unwrap()
            .response_bytes
            .unwrap();
        let basic = BasicOcspResponse::from_der(unsigned.response.as_bytes()).unwrap();
        parsed.optional_signature = Some(x509_ocsp::Signature {
            signature_algorithm: basic.signature_algorithm,
            signature: basic.signature,
            certs: None,
        });
        let with_signature = inspect_ocsp_request(&parsed.to_der().unwrap(), REVOKED).unwrap();
        assert!(with_signature.signature_present);
        assert_eq!(with_signature.nonce, OcspNonce::Absent);
        assert!(matches!(
            with_signature.response.unwrap().nonce,
            OcspNonce::Present {
                comparison: OcspNonceComparison::NotProvided,
                ..
            }
        ));
        let mut parsed = OcspRequest::from_der(request).unwrap();
        let first = parsed.tbs_request.request_list[0].clone();
        for _ in 0..64 {
            parsed.tbs_request.request_list.push(first.clone());
        }
        let mut duplicate_nonce =
            parsed.tbs_request.request_extensions.as_ref().unwrap()[0].clone();
        parsed
            .tbs_request
            .request_extensions
            .as_mut()
            .unwrap()
            .push(duplicate_nonce.clone());
        let many = inspect_ocsp_request(&parsed.to_der().unwrap(), REVOKED).unwrap();
        assert!(many.requests_truncated);
        assert_eq!(many.total_requests, 65);
        assert_eq!(many.nonce, OcspNonce::Duplicate);
        let response = many.response.unwrap();
        assert_eq!(response.exact_response_cert_id_counts.len(), 64);
        assert!(
            response
                .exact_response_cert_id_counts
                .iter()
                .all(|count| *count == 1)
        );
        assert!(matches!(
            response.nonce,
            OcspNonce::Present {
                comparison: OcspNonceComparison::NotProvided,
                ..
            }
        ));
        parsed
            .tbs_request
            .request_extensions
            .as_mut()
            .unwrap()
            .pop();
        duplicate_nonce.extn_value = OctetString::new(vec![0xff]).unwrap();
        parsed.tbs_request.request_extensions.as_mut().unwrap()[0] = duplicate_nonce;
        let malformed = inspect_ocsp_request(&parsed.to_der().unwrap(), REVOKED).unwrap();
        assert_eq!(malformed.nonce, OcspNonce::InvalidEncoding);
        assert!(matches!(
            malformed.response.unwrap().nonce,
            OcspNonce::Present {
                comparison: OcspNonceComparison::NotProvided,
                ..
            }
        ));
        let many_responses = modify(REVOKED, |basic| {
            let entry = basic.tbs_response_data.responses[0].clone();
            for _ in 0..64 {
                basic.tbs_response_data.responses.push(entry.clone());
            }
        });
        let counts = inspect_ocsp_request(request, &many_responses)
            .unwrap()
            .response
            .unwrap();
        assert!(counts.responses_truncated);
        assert_eq!(counts.total_responses, 65);
        assert_eq!(counts.exact_response_cert_id_counts, vec![64]);
    }

    #[test]
    fn nonce_comparison_reports_absence_mismatch_and_malformed_values() {
        let inspect = |bytes: &[u8], expected: Option<&[u8]>| {
            inspect_ocsp_with_nonce(bytes, LEAF, ISSUER, RESPONDER, 0, expected)
                .unwrap()
                .basic
                .unwrap()
        };
        let good = inspect(GOOD, Some(b"synthetic-nonce"));
        assert_eq!(good.nonce, OcspNonce::Absent);
        let expected = b"synthetic-nonce";
        let matching = inspect(REVOKED, Some(expected));
        assert_eq!(matching.signature, CertificateSignature::Matches);
        assert_eq!(
            matching.nonce,
            OcspNonce::Present {
                length: expected.len(),
                critical: false,
                comparison: OcspNonceComparison::Matches
            }
        );
        assert_eq!(
            inspect(REVOKED, None).nonce,
            OcspNonce::Present {
                length: expected.len(),
                critical: false,
                comparison: OcspNonceComparison::NotProvided
            }
        );
        assert_eq!(
            inspect(REVOKED, Some(b"other")).nonce,
            OcspNonce::Present {
                length: expected.len(),
                critical: false,
                comparison: OcspNonceComparison::Differs
            }
        );
        assert!(inspect_ocsp_with_nonce(GOOD, LEAF, ISSUER, RESPONDER, 0, Some(&[])).is_err());
        assert!(inspect_ocsp_with_nonce(GOOD, LEAF, ISSUER, RESPONDER, 0, Some(&[1; 33])).is_err());
        let duplicate = modify(REVOKED, |basic| {
            let extensions = basic
                .tbs_response_data
                .response_extensions
                .as_mut()
                .unwrap();
            extensions.push(extensions[0].clone());
        });
        assert_eq!(
            inspect(&duplicate, Some(expected)).nonce,
            OcspNonce::Duplicate
        );
        let malformed = modify(REVOKED, |basic| {
            basic
                .tbs_response_data
                .response_extensions
                .as_mut()
                .unwrap()[0]
                .extn_value = OctetString::new(vec![0xff]).unwrap()
        });
        let observation = inspect(&malformed, Some(expected));
        assert_eq!(observation.nonce, OcspNonce::InvalidEncoding);
        assert_eq!(observation.signature, CertificateSignature::Differs);
        for len in [0, 33] {
            let invalid = modify(REVOKED, |basic| {
                let encoded = OctetString::new(vec![0u8; len]).unwrap().to_der().unwrap();
                basic
                    .tbs_response_data
                    .response_extensions
                    .as_mut()
                    .unwrap()[0]
                    .extn_value = OctetString::new(encoded).unwrap();
            });
            assert_eq!(
                inspect(&invalid, Some(expected)).nonce,
                OcspNonce::InvalidLength { length: len }
            );
        }
        let critical = modify(REVOKED, |basic| {
            basic
                .tbs_response_data
                .response_extensions
                .as_mut()
                .unwrap()[0]
                .critical = true
        });
        assert_eq!(
            inspect(&critical, Some(expected)).nonce,
            OcspNonce::Present {
                length: expected.len(),
                critical: true,
                comparison: OcspNonceComparison::Matches
            }
        );
        assert_eq!(
            inspect(&critical, Some(expected)).signature,
            CertificateSignature::Differs
        );
    }

    #[test]
    fn responder_certificate_observations_do_not_authorize() {
        use x509_cert::{der::asn1::OctetString, ext::Extension};
        let basic = inspect_ocsp(GOOD, LEAF, ISSUER, RESPONDER, 0)
            .unwrap()
            .basic
            .unwrap();
        assert_eq!(basic.signature, CertificateSignature::Matches);
        let facts = basic.responder_certificate;
        assert!(!facts.identical_to_issuer);
        assert_eq!(facts.ocsp_signing_eku, OcspUsage::Absent);
        assert_eq!(facts.at_reference_time, OcspCertificateTime::Before);
        assert!(!facts.proposed_issuer_link.unwrap().issuer_name_exact);

        let issuer = Certificate::from_der(ISSUER).unwrap();
        let same = responder_certificate_facts(&issuer, ISSUER, ISSUER, 0, 0);
        assert!(same.identical_to_issuer);
        let leaf = Certificate::from_der(LEAF).unwrap();
        let proposed = responder_certificate_facts(&leaf, LEAF, ISSUER, 0, 0);
        let link = proposed.proposed_issuer_link.unwrap();
        assert!(link.issuer_name_exact);
        assert_eq!(link.child_signature, CertificateSignature::Matches);
        assert_eq!(proposed.ocsp_signing_eku, OcspUsage::Absent);
        assert_eq!(proposed.at_reference_time, OcspCertificateTime::Before);
        let within = responder_certificate_facts(
            &leaf,
            LEAF,
            ISSUER,
            proposed.not_before,
            proposed.not_after,
        );
        assert_eq!(within.at_reference_time, OcspCertificateTime::Within);
        assert_eq!(within.at_produced_at, OcspCertificateTime::Within);
        let after = responder_certificate_facts(&leaf, LEAF, ISSUER, proposed.not_after + 1, 0);
        assert_eq!(after.at_reference_time, OcspCertificateTime::After);

        for (der, expected) in [
            (
                include_bytes!("../tests/fixtures/signatures/digital-signature.der").as_slice(),
                true,
            ),
            (
                include_bytes!("../tests/fixtures/signatures/encipherment-only.der").as_slice(),
                false,
            ),
        ] {
            let cert = Certificate::from_der(der).unwrap();
            assert_eq!(
                responder_certificate_facts(&cert, der, ISSUER, 0, 0).key_usage,
                OcspUsage::Present(expected)
            );
        }
        let mut modified = leaf.clone();
        let usage = x509_cert::ext::pkix::ExtendedKeyUsage(vec![ObjectIdentifier::new_unwrap(
            "1.3.6.1.5.5.7.3.9",
        )]);
        let ext = Extension {
            extn_id: ObjectIdentifier::new_unwrap("2.5.29.37"),
            critical: false,
            extn_value: OctetString::new(usage.to_der().unwrap()).unwrap(),
        };
        modified
            .tbs_certificate
            .extensions
            .get_or_insert_default()
            .push(ext.clone());
        let facts = responder_certificate_facts(&modified, LEAF, ISSUER, 0, 0);
        assert_eq!(facts.ocsp_signing_eku, OcspUsage::Present(true));
        modified
            .tbs_certificate
            .extensions
            .as_mut()
            .unwrap()
            .push(ext);
        assert_eq!(
            responder_certificate_facts(&modified, LEAF, ISSUER, 0, 0).ocsp_signing_eku,
            OcspUsage::InvalidOrDuplicate
        );
        let extension = modified
            .tbs_certificate
            .extensions
            .as_mut()
            .unwrap()
            .pop()
            .unwrap();
        modified.tbs_certificate.extensions.as_mut().unwrap().pop();
        let mut malformed = extension;
        malformed.extn_value = OctetString::new(vec![0xff]).unwrap();
        modified
            .tbs_certificate
            .extensions
            .as_mut()
            .unwrap()
            .push(malformed);
        assert_eq!(
            responder_certificate_facts(&modified, LEAF, ISSUER, 0, 0).ocsp_signing_eku,
            OcspUsage::InvalidOrDuplicate
        );
        modified.tbs_certificate.validity.not_after = issuer.tbs_certificate.validity.not_before;
        modified.tbs_certificate.validity.not_before = leaf.tbs_certificate.validity.not_after;
        assert_eq!(
            responder_certificate_facts(&modified, LEAF, ISSUER, 0, 0).at_reference_time,
            OcspCertificateTime::InvalidInterval
        );
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

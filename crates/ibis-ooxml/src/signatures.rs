// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Discovery of OPC digital signature parts.
//!
//! Discovery is not signature verification. In particular, callers must not
//! infer integrity, certificate trust, or revocation status from this result.

use crate::{Error, Package};
use sha2::{Digest, Sha256, Sha384, Sha512};

const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const ORIGIN: &str = "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/origin";
const SIGNATURE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/signature";
const DS: &str = "http://www.w3.org/2000/09/xmldsig#";
const OPC_DS: &str = "http://schemas.openxmlformats.org/package/2006/digital-signature";
const REL_TRANSFORM: &str = "http://schemas.openxmlformats.org/package/2005/06/RelationshipTransform";
const REL_TRANSFORM_2006: &str = "http://schemas.openxmlformats.org/package/2006/RelationshipTransform";
const C14N: &str = "http://www.w3.org/TR/2001/REC-xml-c14n-20010315";

/// A comparison with a digest claimed in a signature's manifest.
/// This is unauthenticated until the signature and all reference chains have
/// themselves been verified; matching digests do not establish integrity.
#[derive(Debug, PartialEq, Eq)]
pub enum ManifestDigest {
    /// Weak SHA-1 diagnostic only; not evidence of security.
    LegacySha1 { matches: bool },
    Matches,
    Differs,
    Unsupported,
    MissingPart,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ManifestReference {
    pub uri: String,
    pub digest: ManifestDigest,
    /// Comparison against the package's declared content type. This is not an
    /// authenticated result before the signature chain has been checked.
    pub content_type: ContentTypeCheck,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AdditionalReference {
    pub uri: Option<String>,
    pub digest: ObjectDigest,
    /// Unauthenticated SigningCertificateV2 digest comparison.
    pub xades_certificate_digest: Option<ObjectDigest>,
    /// Exact DER name and serial comparisons only; not an identity verdict.
    pub xades_issuer_serial: Option<XadesIssuerSerial>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum XadesIssuerSerial {
    Absent,
    Compared { issuer_der_equal: bool, serial_equal: bool },
    Unsupported,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ContentTypeCheck { Matches, Differs, Missing, Unsupported }

/// Individual observations, deliberately without an overall valid verdict.
#[derive(Debug, PartialEq, Eq)]
pub struct SignatureInspection {
    pub signature_part: String,
    pub signature_value: SignatureValueCheck,
    pub package_object: ObjectDigest,
    /// Additional SignedInfo reference observations, not authenticated results.
    pub additional_signed_info_references: Vec<AdditionalReference>,
    pub manifest: Vec<ManifestReference>,
    /// Certificate path, revocation, timestamps, and complete reference coverage
    /// are not evaluated by this inspection.
    pub trust: TrustCheck,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TrustCheck { NotEvaluated }

/// Certificate facts at an explicitly supplied Unix time. These observations
/// do not validate the certificate's signature, path, or revocation status.
#[derive(Debug, PartialEq, Eq)]
pub struct SignerCertificateFacts {
    pub sha256: [u8; 32],
    pub not_before: u64,
    pub not_after: u64,
    pub time_status: CertificateTime,
    pub key_usage: CertificateKeyUsage,
    /// True only for byte-for-byte equality with an explicitly supplied
    /// anchor certificate. This is not proof of a certificate path.
    pub identical_to_supplied_anchor: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CertificateTime { BeforeValidity, WithinValidity, AfterValidity }

#[derive(Debug, PartialEq, Eq)]
pub enum CertificateKeyUsage {
    Absent,
    Present { digital_signature: bool, content_commitment: bool },
}

/// Observations for one proposed issuer/child pair, not a validated path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateLinkFacts {
    /// Exact ASN.1 name equality; RFC 5280 name matching is broader.
    pub issuer_name_exact: bool,
    pub issuer_ca: Option<bool>,
    pub issuer_path_length: Option<u8>,
    pub issuer_key_cert_sign: Option<bool>,
    pub child_signature: CertificateSignature,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CertificateSignature { Matches, Differs, Unsupported }

/// Compare an X.509 ECDSA-SHA256 signature, encoded as DER (unlike XMLDSig).
fn inspect_p256_x509_signature(
    algorithm: &x509_cert::spki::AlgorithmIdentifierOwned,
    issuer: &x509_cert::spki::SubjectPublicKeyInfoOwned,
    message: &[u8], signature: &[u8],
) -> Result<CertificateSignature, Error> {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use x509_cert::der::asn1::ObjectIdentifier;
    let curve = issuer.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
    if algorithm.oid.to_string() != "1.2.840.10045.4.3.2" || algorithm.parameters.is_some()
        || issuer.algorithm.oid.to_string() != "1.2.840.10045.2.1"
        || curve.is_none_or(|oid| oid.to_string() != "1.2.840.10045.3.1.7") {
        return Ok(CertificateSignature::Unsupported);
    }
    let bytes = issuer.subject_public_key.as_bytes().ok_or_else(|| Error::new("Invalid EC issuer public key"))?;
    let key = VerifyingKey::from_sec1_bytes(bytes).map_err(|_| Error::new("Invalid P-256 issuer public key"))?;
    Ok(if Signature::from_der(signature).is_ok_and(|sig| key.verify(message, &sig).is_ok()) { CertificateSignature::Matches } else { CertificateSignature::Differs })
}

/// Facts observed in a caller-supplied CRL. They are not a revocation verdict:
/// issuer path, distribution point applicability, and trust remain unchecked.
#[derive(Debug, PartialEq, Eq)]
pub struct CrlFacts {
    pub issuer_name_exact: bool,
    pub certificate_issuer_name_exact: bool,
    pub issuer_crl_sign: Option<bool>,
    pub signature: CertificateSignature,
    pub this_update: u64,
    pub next_update: Option<u64>,
    pub time_status: CrlTime,
    pub scope: CrlScope,
    /// A serial number match only. Meaning depends on scope, issuer and time.
    pub listed_revocation_time: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CrlTime { NotYetIssued, WithinWindow, Expired, NoNextUpdate }

#[derive(Debug, PartialEq, Eq)]
pub enum CrlScope { NoScopeExtensionsObserved, UnsupportedExtensions }

/// Inspect a supplied CRL at an explicit time without network or trust lookup.
/// Delta, indirect, and scoped CRLs are not interpreted; an absent serial is
/// never presented as a positive certificate-status decision.
pub fn inspect_crl(crl_der: &[u8], cert_der: &[u8], issuer_der: &[u8], reference_unix_seconds: u64) -> Result<CrlFacts, Error> {
    use rsa::{pkcs1::DecodeRsaPublicKey, pkcs1v15, signature::Verifier, traits::PublicKeyParts, RsaPublicKey};
    use x509_cert::{crl::CertificateList, der::{Decode, Encode}, ext::pkix::KeyUsage, Certificate};
    if crl_der.len() > 8 * 1024 * 1024 || cert_der.len() > 1024 * 1024 || issuer_der.len() > 1024 * 1024 { return Err(Error::new("CRL inputs are too large")); }
    let crl = CertificateList::from_der(crl_der).map_err(|_| Error::new("Invalid CRL"))?;
    let cert = Certificate::from_der(cert_der).map_err(|_| Error::new("Invalid target certificate"))?;
    let issuer = Certificate::from_der(issuer_der).map_err(|_| Error::new("Invalid CRL issuer certificate"))?;
    let issuer_usage = issuer.tbs_certificate.get::<KeyUsage>().map_err(|_| Error::new("Invalid or duplicate KeyUsage extension"))?.map(|(_, k)| k.crl_sign());
    let start = crl.tbs_cert_list.this_update.to_unix_duration().as_secs();
    let end = crl.tbs_cert_list.next_update.map(|t| t.to_unix_duration().as_secs());
    if end.is_some_and(|t| t < start) { return Err(Error::new("Invalid CRL validity interval")); }
    let time_status = if reference_unix_seconds < start { CrlTime::NotYetIssued } else { match end { Some(end) if reference_unix_seconds >= end => CrlTime::Expired, Some(_) => CrlTime::WithinWindow, None => CrlTime::NoNextUpdate } };
    let mut scope = CrlScope::NoScopeExtensionsObserved;
    let mut extension_ids = std::collections::HashSet::new();
    for ext in crl.tbs_cert_list.crl_extensions.as_deref().unwrap_or(&[]) {
        let oid = ext.extn_id.to_string();
        if !extension_ids.insert(oid.clone()) { return Err(Error::new("Duplicate CRL extension")); }
        if ext.critical || !matches!(oid.as_str(), "2.5.29.20" | "2.5.29.35") { scope = CrlScope::UnsupportedExtensions; }
    }
    let mut listed = None;
    let mut serials = std::collections::HashSet::new();
    for entry in crl.tbs_cert_list.revoked_certificates.as_deref().unwrap_or(&[]) {
        if !serials.insert(entry.serial_number.as_bytes()) { return Err(Error::new("Duplicate serial number in CRL")); }
        if entry.crl_entry_extensions.as_ref().is_some_and(|ext| !ext.is_empty()) { scope = CrlScope::UnsupportedExtensions; }
        if entry.serial_number == cert.tbs_certificate.serial_number { listed = Some(entry.revocation_date.to_unix_duration().as_secs()); }
    }
    let algorithm = &crl.signature_algorithm;
    let spki = &issuer.tbs_certificate.subject_public_key_info;
    let null_or_absent = |p: &Option<x509_cert::der::asn1::Any>| p.as_ref().is_none_or(|v| v.to_der().is_ok_and(|b| b == [0x05, 0x00]));
    let supported = algorithm == &crl.tbs_cert_list.signature && algorithm.oid.to_string() == "1.2.840.113549.1.1.11" && null_or_absent(&algorithm.parameters)
        && spki.algorithm.oid.to_string() == "1.2.840.113549.1.1.1" && null_or_absent(&spki.algorithm.parameters);
    let signature = if algorithm == &crl.tbs_cert_list.signature && algorithm.oid.to_string() == "1.2.840.10045.4.3.2" {
        let tbs = crl.tbs_cert_list.to_der().map_err(|_| Error::new("Invalid CRL body"))?;
        let raw = crl.signature.as_bytes().ok_or_else(|| Error::new("Invalid CRL signature"))?;
        inspect_p256_x509_signature(algorithm, spki, &tbs, raw)?
    } else if !supported { CertificateSignature::Unsupported } else {
        let key_bytes = spki.subject_public_key.as_bytes().ok_or_else(|| Error::new("Invalid CRL issuer public key"))?;
        let key = RsaPublicKey::from_pkcs1_der(key_bytes).map_err(|_| Error::new("Invalid CRL issuer RSA key"))?;
        if !(2048..=8192).contains(&key.n().bits()) { CertificateSignature::Unsupported } else {
            let tbs = crl.tbs_cert_list.to_der().map_err(|_| Error::new("Invalid CRL body"))?;
            let raw = crl.signature.as_bytes().ok_or_else(|| Error::new("Invalid CRL signature"))?;
            match pkcs1v15::Signature::try_from(raw) {
                Ok(sig) if pkcs1v15::VerifyingKey::<Sha256>::new(key).verify(&tbs, &sig).is_ok() => CertificateSignature::Matches,
                _ => CertificateSignature::Differs,
            }
        }
    };
    Ok(CrlFacts { issuer_name_exact: crl.tbs_cert_list.issuer == issuer.tbs_certificate.subject,
        certificate_issuer_name_exact: cert.tbs_certificate.issuer == crl.tbs_cert_list.issuer,
        issuer_crl_sign: issuer_usage, signature, this_update: start, next_update: end, time_status, scope, listed_revocation_time: listed })
}

/// A proposed chain from the signer to a supplied anchor. Edges can have
/// mismatching or unsupported signatures; no trust or validity is implied.
#[derive(Debug, PartialEq, Eq)]
pub struct CandidateCertificatePath {
    pub fingerprints_sha256: Vec<[u8; 32]>,
    pub links: Vec<CertificateLinkFacts>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CandidatePaths {
    pub paths: Vec<CandidateCertificatePath>,
    /// Search stopped at a depth, edge, or result limit. Even when false,
    /// RFC 5280's more general distinguished-name matching is not evaluated.
    pub truncated: bool,
}

/// Enumerate bounded candidate chains using exact ASN.1 issuer/subject names.
/// Inputs are caller-supplied DER; no network or system trust store is used.
/// These are diagnostic candidates, not validated paths. They must not be used
/// to infer signature validity, revocation status, or policy conformance.
pub fn find_candidate_certificate_paths(leaf: &[u8], intermediates: &[&[u8]], anchors: &[&[u8]]) -> Result<CandidatePaths, Error> {
    use x509_cert::{der::Decode, Certificate};
    const MAX_CERTS: usize = 64;
    const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
    const MAX_DEPTH: usize = 8;
    const MAX_EDGES: usize = 512;
    const MAX_PATHS: usize = 32;
    if intermediates.len().saturating_add(anchors.len()).saturating_add(1) > MAX_CERTS {
        return Err(Error::new("Too many candidate certificates"));
    }
    let mut raw = Vec::new();
    raw.push(leaf);
    for &cert in intermediates.iter().chain(anchors.iter()) {
        if !raw.contains(&cert) { raw.push(cert); }
    }
    let total = raw.iter().try_fold(0usize, |n, cert| n.checked_add(cert.len())).ok_or_else(|| Error::new("Certificate inputs are too large"))?;
    if total > MAX_TOTAL_BYTES || raw.iter().any(|cert| cert.len() > 1024 * 1024) { return Err(Error::new("Certificate inputs are too large")); }
    let certs: Vec<_> = raw.iter().map(|der| Certificate::from_der(der).map_err(|_| Error::new("Invalid candidate certificate"))).collect::<Result<_, _>>()?;
    let hashes: Vec<[u8; 32]> = raw.iter().map(|der| Sha256::digest(der).into()).collect();
    let mut pending = vec![(vec![0usize], Vec::<CertificateLinkFacts>::new())];
    let mut paths = Vec::new();
    let mut tested_edges = 0usize;
    let mut truncated = false;
    while let Some((path, links)) = pending.pop() {
        let current = *path.last().ok_or_else(|| Error::new("Empty candidate path"))?;
        if anchors.contains(&raw[current]) {
            paths.push(CandidateCertificatePath { fingerprints_sha256: path.iter().map(|&i| hashes[i]).collect(), links });
            if paths.len() == MAX_PATHS { truncated |= !pending.is_empty(); break; }
            continue;
        }
        if path.len() >= MAX_DEPTH { truncated = true; continue; }
        for i in (0..raw.len()).rev() {
            if path.contains(&i) || certs[current].tbs_certificate.issuer != certs[i].tbs_certificate.subject { continue; }
            if tested_edges == MAX_EDGES { truncated = true; break; }
            tested_edges += 1;
            let facts = inspect_certificate_link(raw[current], raw[i])?;
            let mut next_path = path.clone(); next_path.push(i);
            let mut next_links = links.clone(); next_links.push(facts);
            pending.push((next_path, next_links));
        }
        if tested_edges == MAX_EDGES { truncated = true; break; }
    }
    Ok(CandidatePaths { paths, truncated })
}

/// Inspect one child and proposed issuer certificate, supplied as DER.
/// This checks RSA-SHA256 and ECDSA P-256/SHA-256 certificate signatures. It does not evaluate
/// time, revocation, policies, name constraints, anchor trust, or a path.
pub fn inspect_certificate_link(child_der: &[u8], issuer_der: &[u8]) -> Result<CertificateLinkFacts, Error> {
    use rsa::{pkcs1::DecodeRsaPublicKey, pkcs1v15, signature::Verifier, traits::PublicKeyParts, RsaPublicKey};
    use x509_cert::{der::{Decode, Encode}, ext::pkix::{BasicConstraints, KeyUsage}, Certificate};
    if child_der.len() > 1024 * 1024 || issuer_der.len() > 1024 * 1024 { return Err(Error::new("Certificate is too large")); }
    let child = Certificate::from_der(child_der).map_err(|_| Error::new("Invalid child certificate"))?;
    let issuer = Certificate::from_der(issuer_der).map_err(|_| Error::new("Invalid issuer certificate"))?;
    let constraints = issuer.tbs_certificate.get::<BasicConstraints>().map_err(|_| Error::new("Invalid or duplicate BasicConstraints extension"))?;
    let usage = issuer.tbs_certificate.get::<KeyUsage>().map_err(|_| Error::new("Invalid or duplicate KeyUsage extension"))?;
    let spki = &issuer.tbs_certificate.subject_public_key_info;
    let algorithm = &child.signature_algorithm;
    let null_or_absent = |params: &Option<x509_cert::der::asn1::Any>| params.as_ref().is_none_or(|p| p.to_der().is_ok_and(|v| v == [0x05, 0x00]));
    let supported = algorithm == &child.tbs_certificate.signature
        && algorithm.oid.to_string() == "1.2.840.113549.1.1.11" && null_or_absent(&algorithm.parameters)
        && spki.algorithm.oid.to_string() == "1.2.840.113549.1.1.1" && null_or_absent(&spki.algorithm.parameters);
    let child_signature = if algorithm == &child.tbs_certificate.signature && algorithm.oid.to_string() == "1.2.840.10045.4.3.2" {
        let tbs = child.tbs_certificate.to_der().map_err(|_| Error::new("Invalid certificate body"))?;
        let raw = child.signature.as_bytes().ok_or_else(|| Error::new("Invalid certificate signature"))?;
        inspect_p256_x509_signature(algorithm, spki, &tbs, raw)?
    } else if !supported { CertificateSignature::Unsupported } else {
        let key_bytes = spki.subject_public_key.as_bytes().ok_or_else(|| Error::new("Invalid issuer public key"))?;
        let key = RsaPublicKey::from_pkcs1_der(key_bytes).map_err(|_| Error::new("Invalid issuer RSA key"))?;
        if !(2048..=8192).contains(&key.n().bits()) { CertificateSignature::Unsupported } else {
            let tbs = child.tbs_certificate.to_der().map_err(|_| Error::new("Invalid certificate body"))?;
            let raw = child.signature.as_bytes().ok_or_else(|| Error::new("Invalid certificate signature"))?;
            match pkcs1v15::Signature::try_from(raw) {
                Ok(sig) if pkcs1v15::VerifyingKey::<Sha256>::new(key).verify(&tbs, &sig).is_ok() => CertificateSignature::Matches,
                _ => CertificateSignature::Differs,
            }
        }
    };
    Ok(CertificateLinkFacts {
        issuer_name_exact: child.tbs_certificate.issuer == issuer.tbs_certificate.subject,
        issuer_ca: constraints.as_ref().map(|(_, c)| c.ca),
        issuer_path_length: constraints.and_then(|(_, c)| c.path_len_constraint),
        issuer_key_cert_sign: usage.map(|(_, u)| u.key_cert_sign()),
        child_signature,
    })
}

/// Extract one embedded certificate. Its identity and trust are not verified.
pub fn extract_single_embedded_certificate(package: &Package, signature_part: &str) -> Result<Vec<u8>, Error> {
    use ua_cryptography_xml_c14n::DocumentText;
    let bytes = package.read(signature_part)?;
    if bytes.len() > 8 * 1024 * 1024 { return Err(Error::new("Signature XML is too large")); }
    let text = DocumentText::decode(&bytes).map_err(|_| Error::new("Unsupported signature XML encoding"))?;
    let doc = text.parse().map_err(|_| Error::new("Invalid signature XML"))?;
    let root = doc.root_element();
    if root.tag_name().namespace() != Some(DS) || root.tag_name().name() != "Signature" { return Err(Error::new("Invalid signature root")); }
    let keyinfo = ds_children(root, "KeyInfo");
    if keyinfo.len() != 1 { return Err(Error::new("Expected one KeyInfo")); }
    let data = ds_children(keyinfo[0], "X509Data");
    if data.len() != 1 { return Err(Error::new("Expected one X509Data")); }
    let certs = ds_children(data[0], "X509Certificate");
    if certs.len() != 1 || certs[0].children().any(|n| n.is_element()) { return Err(Error::new("Expected one signing certificate")); }
    let encoded: String = certs[0].children().filter(|n| n.is_text()).filter_map(|n| n.text()).flat_map(str::chars).filter(|c| !matches!(c, '\t' | '\n' | '\r' | ' ')).collect();
    crate::base64::decode(&encoded).filter(|v| !v.is_empty() && v.len() <= 1024 * 1024 && crate::base64::encode(v) == encoded).ok_or_else(|| Error::new("Invalid certificate encoding"))
}

/// Inspect one embedded certificate at the caller's explicit reference time.
/// No system clock, network access, path building, or revocation check is used.
pub fn inspect_signer_certificate(package: &Package, signature_part: &str, reference_unix_seconds: u64, anchor_der: &[&[u8]]) -> Result<SignerCertificateFacts, Error> {
    use x509_cert::{der::Decode, ext::pkix::KeyUsage, Certificate};
    let der = extract_single_embedded_certificate(package, signature_part)?;
    let cert = Certificate::from_der(&der).map_err(|_| Error::new("Invalid signing certificate"))?;
    let before = cert.tbs_certificate.validity.not_before.to_unix_duration().as_secs();
    let after = cert.tbs_certificate.validity.not_after.to_unix_duration().as_secs();
    if before > after { return Err(Error::new("Invalid certificate validity interval")); }
    let time_status = if reference_unix_seconds < before { CertificateTime::BeforeValidity } else if reference_unix_seconds > after { CertificateTime::AfterValidity } else { CertificateTime::WithinValidity };
    let usage = cert.tbs_certificate.get::<KeyUsage>().map_err(|_| Error::new("Invalid or duplicate KeyUsage extension"))?;
    let key_usage = match usage {
        None => CertificateKeyUsage::Absent,
        Some((_, value)) => CertificateKeyUsage::Present { digital_signature: value.digital_signature(), content_commitment: value.non_repudiation() },
    };
    Ok(SignerCertificateFacts {
        sha256: Sha256::digest(&der).into(), not_before: before, not_after: after, time_status, key_usage,
        identical_to_supplied_anchor: anchor_der.contains(&der.as_slice()),
    })
}

/// Inspect the signatures linked from the package. The resulting observations
/// do not constitute signature verification or a trust decision.
pub fn inspect_signatures(package: &Package) -> Result<Vec<SignatureInspection>, Error> {
    let mut reports = Vec::new();
    for part in signature_parts(package)? {
        let binding = inspect_package_object_binding(package, &part)?;
        let object_id = match binding {
            PackageObjectBinding::Declared { object_id } => object_id,
            PackageObjectBinding::Unsupported => return Err(Error::new("Unsupported package object binding")),
        };
        reports.push(SignatureInspection {
            signature_value: inspect_signature_value(package, &part)?,
            package_object: inspect_package_object_digest(package, &part)?,
            additional_signed_info_references: additional_signed_info_references(package, &part, &object_id)?,
            manifest: inspect_manifest_digests(package, &part)?,
            trust: TrustCheck::NotEvaluated,
            signature_part: part,
        });
    }
    Ok(reports)
}

/// Compare a bounded subset of additional references: direct ds:Object nodes
/// and XAdES v1.3.2 SignedProperties under a signature-bound container.
fn additional_signed_info_references(package: &Package, signature_part: &str, object_id: &str) -> Result<Vec<AdditionalReference>, Error> {
    use ua_cryptography_xml_c14n::{canonicalize, Algorithm, DocumentText};
    let bytes = package.read(signature_part)?;
    if bytes.len() > 8 * 1024 * 1024 { return Err(Error::new("Signature XML is too large")); }
    let text = DocumentText::decode(&bytes).map_err(|_| Error::new("Unsupported signature XML encoding"))?;
    let doc = text.parse().map_err(|_| Error::new("Invalid signature XML"))?;
    let root = doc.root_element();
    if root.tag_name().namespace() != Some(DS) || root.tag_name().name() != "Signature" { return Err(Error::new("Invalid signature root")); }
    let infos = ds_children(root, "SignedInfo");
    if infos.len() != 1 { return Err(Error::new("Expected one SignedInfo")); }
    let mut seen = std::collections::HashSet::new();
    for node in root.descendants().filter(|n| n.is_element()) {
        for attr in node.attributes() {
            if attr.name().eq_ignore_ascii_case("id") && (attr.value().is_empty() || !seen.insert(attr.value())) {
                return Err(Error::new("Ambiguous signature identifier"));
            }
        }
    }
    let package_uri = format!("#{object_id}");
    let mut additional = Vec::new();
    let mut processed_bytes = 0usize;
    let mut signer_certificate: Option<Option<Vec<u8>>> = None;
    for reference in ds_children(infos[0], "Reference") {
        let uri = reference.attribute("URI");
        if uri == Some(package_uri.as_str()) { continue; }
        if additional.len() == 256 || uri.is_some_and(|u| u.len() > 4096) { return Err(Error::new("Too many or oversized SignedInfo references")); }
        let mut digest = ObjectDigest::Unsupported;
        let is_xades = reference.attribute("Type") == Some("http://uri.etsi.org/01903#SignedProperties");
        let mut xades_certificate_digest = is_xades.then_some(ObjectDigest::Unsupported);
        let mut xades_issuer_serial = is_xades.then_some(XadesIssuerSerial::Unsupported);
        let id = uri.and_then(|u| u.strip_prefix('#')).filter(|id| id.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')));
        if let Some(id) = id {
            const XADES: &str = "http://uri.etsi.org/01903/v1.3.2#";
            let targets: Vec<_> = match reference.attribute("Type") {
                Some("http://www.w3.org/2000/09/xmldsig#Object") => ds_children(root, "Object").into_iter().filter(|n| n.attribute("Id") == Some(id)).collect(),
                Some("http://uri.etsi.org/01903#SignedProperties") => {
                    let signature_uri = root.attribute("Id").filter(|id| !id.is_empty()).map(|id| format!("#{id}"));
                    ds_children(root, "Object").into_iter().flat_map(|object| object.children())
                        .filter(|n| n.is_element() && n.tag_name().namespace() == Some(XADES) && n.tag_name().name() == "QualifyingProperties" && signature_uri.as_deref().is_some_and(|uri| n.attribute("Target") == Some(uri)))
                        .flat_map(|qualifying| qualifying.children())
                        .filter(|n| n.is_element() && n.tag_name().namespace() == Some(XADES) && n.tag_name().name() == "SignedProperties" && n.attribute("Id") == Some(id)).collect()
                }
                _ => Vec::new(),
            };
            let children: Vec<_> = reference.children().filter(|n| n.is_element()).collect();
            let is_ds = |n: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>, name: &str| n.tag_name().namespace() == Some(DS) && n.tag_name().name() == name;
            let parts = match children.as_slice() {
                [method, value] if is_ds(*method, "DigestMethod") && is_ds(*value, "DigestValue") => Some((*method, *value)),
                [transforms, method, value] if is_ds(*transforms, "Transforms") && is_ds(*method, "DigestMethod") && is_ds(*value, "DigestValue") => {
                    let steps = ds_children(*transforms, "Transform");
                    if steps.len() == 1 && transforms.children().filter(|n| n.is_element()).count() == 1 && steps[0].attribute("Algorithm") == Some(C14N) && !steps[0].children().any(|n| n.is_element()) { Some((*method, *value)) } else { None }
                }
                _ => None,
            };
            if let ([target], Some((method, value))) = (targets.as_slice(), parts) {
                processed_bytes = processed_bytes.saturating_add(target.range().len());
                if processed_bytes <= 16 * 1024 * 1024 && !method.children().any(|n| n.is_element()) && !value.children().any(|n| n.is_element()) {
                    if is_xades {
                        let cert = signer_certificate.get_or_insert_with(|| extract_single_embedded_certificate(package, signature_part).ok());
                        xades_certificate_digest = Some(cert.as_deref().map_or(ObjectDigest::Unsupported, |cert| inspect_xades_certificate_digest(*target, cert)));
                        xades_issuer_serial = Some(cert.as_deref().map_or(XadesIssuerSerial::Unsupported, |cert| inspect_xades_issuer_serial(*target, cert)));
                    }
                    let encoded: String = value.children().filter(|n| n.is_text()).filter_map(|n| n.text()).flat_map(str::chars).filter(|c| !matches!(c, '\t' | '\n' | '\r' | ' ')).collect();
                    if let Some(expected) = crate::base64::decode(&encoded).filter(|v| !v.is_empty() && crate::base64::encode(v) == encoded) {
                        let canonical = canonicalize(*target, Algorithm::Inclusive, &[], None);
                        digest = match method.attribute("Algorithm") {
                            Some("http://www.w3.org/2000/09/xmldsig#sha1") => ObjectDigest::LegacySha1 { matches: sha1::Sha1::digest(&canonical).as_slice() == expected },
                            Some("http://www.w3.org/2001/04/xmlenc#sha256") => if Sha256::digest(&canonical).as_slice() == expected { ObjectDigest::Matches } else { ObjectDigest::Differs },
                            Some("http://www.w3.org/2001/04/xmldsig-more#sha384") => if Sha384::digest(&canonical).as_slice() == expected { ObjectDigest::Matches } else { ObjectDigest::Differs },
                            Some("http://www.w3.org/2001/04/xmlenc#sha512") => if Sha512::digest(&canonical).as_slice() == expected { ObjectDigest::Matches } else { ObjectDigest::Differs },
                            _ => ObjectDigest::Unsupported,
                        };
                    }
                }
            }
        }
        additional.push(AdditionalReference { uri: uri.map(str::to_owned), digest, xades_certificate_digest, xades_issuer_serial });
    }
    Ok(additional)
}

/// Compare the first XAdES SigningCertificateV2 CertDigest to the embedded
/// certificate bytes. Neither the property nor the certificate is trusted.
fn inspect_xades_certificate_digest(properties: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>, certificate: &[u8]) -> ObjectDigest {
    const XADES: &str = "http://uri.etsi.org/01903/v1.3.2#";
    fn children<'a, 'input>(node: ua_cryptography_xml_c14n::roxmltree::Node<'a, 'input>, name: &str) -> Vec<ua_cryptography_xml_c14n::roxmltree::Node<'a, 'input>> {
        node.children().filter(|n| n.is_element() && n.tag_name().namespace() == Some(XADES) && n.tag_name().name() == name).collect()
    }
    let signed = children(properties, "SignedSignatureProperties");
    if signed.len() != 1 { return ObjectDigest::Unsupported; }
    let signing = children(signed[0], "SigningCertificateV2");
    if signing.len() != 1 { return ObjectDigest::Unsupported; }
    let certs: Vec<_> = signing[0].children().filter(|n| n.is_element()).collect();
    if certs.is_empty() || certs.iter().any(|n| n.tag_name().namespace() != Some(XADES) || n.tag_name().name() != "Cert") { return ObjectDigest::Unsupported; }
    let digests = children(certs[0], "CertDigest");
    if digests.len() != 1 { return ObjectDigest::Unsupported; }
    let fields: Vec<_> = digests[0].children().filter(|n| n.is_element()).collect();
    if fields.len() != 2 || fields[0].tag_name().namespace() != Some(DS) || fields[0].tag_name().name() != "DigestMethod" || fields[1].tag_name().namespace() != Some(DS) || fields[1].tag_name().name() != "DigestValue" || fields.iter().any(|n| n.children().any(|c| c.is_element())) { return ObjectDigest::Unsupported; }
    let encoded: String = fields[1].children().filter(|n| n.is_text()).filter_map(|n| n.text()).flat_map(str::chars).filter(|c| !matches!(c, '\t' | '\n' | '\r' | ' ')).collect();
    let Some(expected) = crate::base64::decode(&encoded).filter(|v| !v.is_empty() && crate::base64::encode(v) == encoded) else { return ObjectDigest::Unsupported };
    match fields[0].attribute("Algorithm") {
        Some("http://www.w3.org/2000/09/xmldsig#sha1") => ObjectDigest::LegacySha1 { matches: sha1::Sha1::digest(certificate).as_slice() == expected },
        Some("http://www.w3.org/2001/04/xmlenc#sha256") => if Sha256::digest(certificate).as_slice() == expected { ObjectDigest::Matches } else { ObjectDigest::Differs },
        Some("http://www.w3.org/2001/04/xmldsig-more#sha384") => if Sha384::digest(certificate).as_slice() == expected { ObjectDigest::Matches } else { ObjectDigest::Differs },
        Some("http://www.w3.org/2001/04/xmlenc#sha512") => if Sha512::digest(certificate).as_slice() == expected { ObjectDigest::Matches } else { ObjectDigest::Differs },
        _ => ObjectDigest::Unsupported,
    }
}

/// Compare the first XAdES IssuerSerialV2 against the embedded certificate.
/// Name comparison is byte-exact DER, not RFC 5280 name matching.
fn inspect_xades_issuer_serial(properties: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>, certificate: &[u8]) -> XadesIssuerSerial {
    use x509_cert::{Certificate, der::{Decode, Encode, Reader, SliceReader}, ext::pkix::name::{GeneralName, GeneralNames}, serial_number::SerialNumber};
    const XADES: &str = "http://uri.etsi.org/01903/v1.3.2#";
    fn children<'a, 'input>(node: ua_cryptography_xml_c14n::roxmltree::Node<'a, 'input>, name: &str) -> Vec<ua_cryptography_xml_c14n::roxmltree::Node<'a, 'input>> {
        node.children().filter(|n| n.is_element() && n.tag_name().namespace() == Some(XADES) && n.tag_name().name() == name).collect()
    }
    let signed = children(properties, "SignedSignatureProperties");
    if signed.len() != 1 { return XadesIssuerSerial::Unsupported; }
    let signing = children(signed[0], "SigningCertificateV2");
    if signing.len() != 1 { return XadesIssuerSerial::Unsupported; }
    let certs: Vec<_> = signing[0].children().filter(|n| n.is_element()).collect();
    if certs.is_empty() || certs.iter().any(|n| n.tag_name().namespace() != Some(XADES) || n.tag_name().name() != "Cert") { return XadesIssuerSerial::Unsupported; }
    let fields = children(certs[0], "IssuerSerialV2");
    if fields.is_empty() { return XadesIssuerSerial::Absent; }
    if fields.len() != 1 || fields[0].children().any(|n| n.is_element()) { return XadesIssuerSerial::Unsupported; }
    let encoded: String = fields[0].children().filter(|n| n.is_text()).filter_map(|n| n.text()).flat_map(str::chars).filter(|c| !matches!(c, '\t' | '\n' | '\r' | ' ')).collect();
    if encoded.len() > (64 * 1024_usize).div_ceil(3) * 4 { return XadesIssuerSerial::Unsupported; }
    let Some(bytes) = crate::base64::decode(&encoded).filter(|v| !v.is_empty() && v.len() <= 64 * 1024 && crate::base64::encode(v) == encoded) else { return XadesIssuerSerial::Unsupported };
    let parsed = (|| -> x509_cert::der::Result<_> {
        let mut reader = SliceReader::new(&bytes)?;
        let fields = reader.sequence(|r| Ok((GeneralNames::decode(r)?, SerialNumber::decode(r)?)))?;
        reader.finish(fields)
    })();
    let Ok((names, serial)) = parsed else { return XadesIssuerSerial::Unsupported };
    let [GeneralName::DirectoryName(name)] = names.as_slice() else { return XadesIssuerSerial::Unsupported };
    let Ok(cert) = Certificate::from_der(certificate) else { return XadesIssuerSerial::Unsupported };
    let (Ok(expected), Ok(actual)) = (name.to_der(), cert.tbs_certificate.issuer.to_der()) else { return XadesIssuerSerial::Unsupported };
    XadesIssuerSerial::Compared { issuer_der_equal: expected == actual, serial_equal: serial == cert.tbs_certificate.serial_number }
}

/// A structurally declared binding, not an authenticated signature.
#[derive(Debug, PartialEq, Eq)]
pub enum PackageObjectBinding {
    /// The single package object is named by a SignedInfo reference. Neither
    /// the reference digest nor the cryptographic signature has been checked.
    Declared { object_id: String },
    /// The structure is outside the subset understood by this inspector.
    Unsupported,
}

/// Comparison of the declared package-object digest. Even a match is not an
/// authenticated result until SignedInfo's signature and signer are checked.
#[derive(Debug, PartialEq, Eq)]
pub enum ObjectDigest {
    /// Weak SHA-1 diagnostic only; not evidence of security.
    LegacySha1 { matches: bool },
    Matches,
    Differs,
    Unsupported,
}

/// Signature-value comparison using an embedded, untrusted certificate.
/// This alone does not validate referenced data, identity, or certificate trust.
#[derive(Debug, PartialEq, Eq)]
pub enum SignatureValueCheck {
    /// Weak legacy signature checked against an untrusted certificate.
    LegacySha1 { key_bits: usize, matches: bool },
    /// ECDSA comparison against an untrusted P-256 certificate.
    EcdsaP256 { matches: bool },
    MatchesUntrustedCertificate,
    Differs,
    Unsupported,
}

fn ds_children<'a, 'input>(parent: ua_cryptography_xml_c14n::roxmltree::Node<'a, 'input>, name: &str) -> Vec<ua_cryptography_xml_c14n::roxmltree::Node<'a, 'input>> {
    parent.children().filter(|n| n.is_element() && n.tag_name().namespace() == Some(DS) && n.tag_name().name() == name).collect()
}

/// Compare RSA-SHA256, legacy RSA-SHA1, or ECDSA-P256-SHA256 over SignedInfo against exactly one
/// embedded certificate. The certificate is not chain- or revocation-checked.
pub fn inspect_signature_value(package: &Package, signature_part: &str) -> Result<SignatureValueCheck, Error> {
    use rsa::{pkcs1::DecodeRsaPublicKey, pkcs1v15, signature::Verifier, traits::PublicKeyParts, RsaPublicKey};
    use ua_cryptography_xml_c14n::{canonicalize, Algorithm, DocumentText};
    use x509_cert::{der::Decode, Certificate};
    let bytes = package.read(signature_part)?;
    if bytes.len() > 8 * 1024 * 1024 { return Err(Error::new("Signature XML is too large")); }
    let text = DocumentText::decode(&bytes).map_err(|_| Error::new("Unsupported signature XML encoding"))?;
    let doc = text.parse().map_err(|_| Error::new("Invalid signature XML"))?;
    let root = doc.root_element();
    let is_ds = |n: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>, name: &str| n.is_element() && n.tag_name().namespace() == Some(DS) && n.tag_name().name() == name;
    if !is_ds(root, "Signature") { return Err(Error::new("Invalid signature root")); }
    let infos = ds_children(root, "SignedInfo");
    let values = ds_children(root, "SignatureValue");
    let keyinfos = ds_children(root, "KeyInfo");
    if infos.len() != 1 || values.len() != 1 || keyinfos.len() != 1 { return Ok(SignatureValueCheck::Unsupported); }
    let c14n = ds_children(infos[0], "CanonicalizationMethod");
    let method = ds_children(infos[0], "SignatureMethod");
    if c14n.len() != 1 || method.len() != 1 || c14n[0].attribute("Algorithm") != Some(C14N)
        || c14n[0].children().any(|n| n.is_element()) || method[0].children().any(|n| n.is_element()) {
        return Ok(SignatureValueCheck::Unsupported);
    }
    let (legacy, ecdsa) = match method[0].attribute("Algorithm") {
        Some("http://www.w3.org/2000/09/xmldsig#rsa-sha1") => (true, false),
        Some("http://www.w3.org/2001/04/xmldsig-more#rsa-sha256") => (false, false),
        Some("http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256") => (false, true),
        _ => return Ok(SignatureValueCheck::Unsupported),
    };
    let x509data = ds_children(keyinfos[0], "X509Data");
    if x509data.len() != 1 { return Ok(SignatureValueCheck::Unsupported); }
    let certs = ds_children(x509data[0], "X509Certificate");
    if certs.len() != 1 || values[0].children().any(|n| n.is_element()) || certs[0].children().any(|n| n.is_element()) { return Ok(SignatureValueCheck::Unsupported); }
    let decode = |node: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>| -> Result<Vec<u8>, Error> {
        let value: String = node.children().filter(|n| n.is_text()).filter_map(|n| n.text()).flat_map(str::chars).filter(|c| !matches!(c, '\t' | '\n' | '\r' | ' ')).collect();
        crate::base64::decode(&value).filter(|v| !v.is_empty() && crate::base64::encode(v) == value).ok_or_else(|| Error::new("Invalid base64 in signature"))
    };
    let cert = Certificate::from_der(&decode(certs[0])?).map_err(|_| Error::new("Invalid signing certificate"))?;
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let Some(public_key) = spki.subject_public_key.as_bytes() else { return Err(Error::new("Invalid public key")) };
    if ecdsa {
        use x509_cert::der::asn1::ObjectIdentifier;
        let curve = spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
        if spki.algorithm.oid.to_string() != "1.2.840.10045.2.1" || curve.is_none_or(|oid| oid.to_string() != "1.2.840.10045.3.1.7") {
            return Ok(SignatureValueCheck::Unsupported);
        }
        let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(public_key).map_err(|_| Error::new("Invalid P-256 public key"))?;
        let raw_signature = decode(values[0])?;
        let canonical = canonicalize(infos[0], Algorithm::Inclusive, &[], None);
        let matches = p256::ecdsa::Signature::from_slice(&raw_signature).is_ok_and(|signature| key.verify(&canonical, &signature).is_ok());
        return Ok(SignatureValueCheck::EcdsaP256 { matches });
    }
    if spki.algorithm.oid.to_string() != "1.2.840.113549.1.1.1" { return Ok(SignatureValueCheck::Unsupported); }
    let key = RsaPublicKey::from_pkcs1_der(public_key).map_err(|_| Error::new("Invalid RSA public key"))?;
    let key_bits = key.n().bits();
    if !((if legacy { 1024 } else { 2048 })..=8192).contains(&key_bits) { return Ok(SignatureValueCheck::Unsupported); }
    let raw_signature = decode(values[0])?;
    let canonical = canonicalize(infos[0], Algorithm::Inclusive, &[], None);
    let signature = pkcs1v15::Signature::try_from(raw_signature.as_slice());
    if legacy {
        let matches = signature.is_ok_and(|signature| pkcs1v15::VerifyingKey::<sha1::Sha1>::new(key).verify(&canonical, &signature).is_ok());
        Ok(SignatureValueCheck::LegacySha1 { key_bits, matches })
    } else {
        let matches = signature.is_ok_and(|signature| pkcs1v15::VerifyingKey::<Sha256>::new(key).verify(&canonical, &signature).is_ok());
        Ok(if matches { SignatureValueCheck::MatchesUntrustedCertificate } else { SignatureValueCheck::Differs })
    }
}

/// Canonicalize the uniquely referenced package object and compare its digest.
/// This does not validate the signature over SignedInfo or certificate trust.
pub fn inspect_package_object_digest(package: &Package, signature_part: &str) -> Result<ObjectDigest, Error> {
    use ua_cryptography_xml_c14n::{canonicalize, Algorithm, DocumentText};
    let bytes = package.read(signature_part)?;
    if bytes.len() > 8 * 1024 * 1024 { return Err(Error::new("Signature XML is too large")); }
    let source = DocumentText::decode(&bytes).map_err(|_| Error::new("Unsupported signature XML encoding"))?;
    let doc = source.parse().map_err(|_| Error::new("Invalid signature XML"))?;
    let root = doc.root_element();
    if root.tag_name().namespace() != Some(DS) || root.tag_name().name() != "Signature" {
        return Err(Error::new("Invalid signature root"));
    }
    let mut ids = std::collections::HashSet::new();
    for node in root.descendants().filter(|n| n.is_element()) {
        for attr in node.attributes() {
            if attr.name().eq_ignore_ascii_case("id") && (attr.value().is_empty() || !ids.insert(attr.value())) {
                return Err(Error::new("Ambiguous signature identifier"));
            }
        }
    }
    let is_ds = |node: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>, name: &str| node.is_element() && node.tag_name().namespace() == Some(DS) && node.tag_name().name() == name;
    let infos: Vec<_> = root.children().filter(|n| is_ds(*n, "SignedInfo")).collect();
    let objects: Vec<_> = root.children().filter(|n| is_ds(*n, "Object") && n.children().any(|c| is_ds(c, "Manifest"))).collect();
    if infos.len() != 1 || objects.len() != 1 { return Err(Error::new("Ambiguous package object")); }
    let object = objects[0];
    if object.children().filter(|n| is_ds(*n, "Manifest")).count() != 1 { return Err(Error::new("Ambiguous manifest")); }
    let id = object.attribute("Id").filter(|s| !s.is_empty()).ok_or_else(|| Error::new("Package object has no Id"))?;
    let uri = format!("#{id}");
    let refs: Vec<_> = infos[0].children().filter(|n| is_ds(*n, "Reference") && n.attribute("URI") == Some(uri.as_str())).collect();
    if refs.len() != 1 { return Err(Error::new("Package object reference is missing or ambiguous")); }
    let reference = refs[0];
    if reference.attribute("Type") != Some("http://www.w3.org/2000/09/xmldsig#Object") { return Ok(ObjectDigest::Unsupported); }
    let children: Vec<_> = reference.children().filter(|n| n.is_element()).collect();
    let (method, value) = match children.as_slice() {
        [method, value] if is_ds(*method, "DigestMethod") && is_ds(*value, "DigestValue") => (*method, *value),
        [transforms, method, value] if is_ds(*transforms, "Transforms") && is_ds(*method, "DigestMethod") && is_ds(*value, "DigestValue") => {
            let steps: Vec<_> = transforms.children().filter(|n| n.is_element()).collect();
            if steps.len() != 1 || !is_ds(steps[0], "Transform") || steps[0].attribute("Algorithm") != Some(C14N)
                || steps[0].children().any(|n| n.is_element()) { return Ok(ObjectDigest::Unsupported); }
            (*method, *value)
        }
        _ => return Ok(ObjectDigest::Unsupported),
    };
    if method.children().any(|n| n.is_element()) || value.children().any(|n| n.is_element()) { return Ok(ObjectDigest::Unsupported); }
    let encoded: String = value.children().filter_map(|n| n.text()).flat_map(str::chars).filter(|c| !matches!(c, '\t' | '\n' | '\r' | ' ')).collect();
    let expected = crate::base64::decode(&encoded).filter(|v| crate::base64::encode(v) == encoded).ok_or_else(|| Error::new("Invalid digest encoding"))?;
    let canonical = canonicalize(object, Algorithm::Inclusive, &[], None);
    let algorithm = method.attribute("Algorithm");
    if algorithm == Some("http://www.w3.org/2000/09/xmldsig#sha1") {
        return Ok(ObjectDigest::LegacySha1 { matches: sha1::Sha1::digest(&canonical).as_slice() == expected });
    }
    let actual = match algorithm {
        Some("http://www.w3.org/2001/04/xmlenc#sha256") => Sha256::digest(&canonical).to_vec(),
        Some("http://www.w3.org/2001/04/xmldsig-more#sha384") => Sha384::digest(&canonical).to_vec(),
        Some("http://www.w3.org/2001/04/xmlenc#sha512") => Sha512::digest(&canonical).to_vec(),
        _ => return Ok(ObjectDigest::Unsupported),
    };
    Ok(if actual == expected { ObjectDigest::Matches } else { ObjectDigest::Differs })
}

/// Inspect the declared binding between SignedInfo and the package object.
/// Duplicate ID-like values are rejected to avoid ambiguous fragment lookup.
/// A declared binding says nothing about the signature's validity or trust.
pub fn inspect_package_object_binding(package: &Package, signature_part: &str) -> Result<PackageObjectBinding, Error> {
    let doc = package.xml(signature_part)?;
    let root = doc.root_element().ok_or_else(|| Error::new("Empty signature"))?;
    if doc.local_name(root) != "Signature" || doc.namespace(root) != Some(DS) {
        return Err(Error::new("Invalid signature root"));
    }
    if doc.elements(crate::XmlDocument::ROOT).count() != 1
        || doc.children(crate::XmlDocument::ROOT).iter().any(|&n| matches!(doc.kind(n), crate::xml::NodeKind::Doctype | crate::xml::NodeKind::ProcessingInstruction)) {
        return Ok(PackageObjectBinding::Unsupported);
    }
    let mut seen = std::collections::HashSet::new();
    for node in doc.descendants(root).into_iter().filter(|&n| doc.is_element(n)) {
        for attribute in doc.attributes(node) {
            if !is_namespace_attr(&attribute.name) && attribute.name.rsplit(':').next().is_some_and(|s| s.eq_ignore_ascii_case("id"))
                && (attribute.value.is_empty() || !seen.insert(attribute.value.as_str())) {
                return Err(Error::new("Ambiguous signature identifier"));
            }
        }
    }
    let children: Vec<_> = doc.elements(root).collect();
    let named = |local| children.iter().copied().filter(|&n| doc.namespace(n) == Some(DS) && doc.local_name(n) == local).collect::<Vec<_>>();
    let signed_info = named("SignedInfo");
    if signed_info.len() != 1 { return Err(Error::new("Expected one SignedInfo")); }
    let mut objects = Vec::new();
    for object in named("Object") {
        let manifests = doc.elements(object).filter(|&n| doc.namespace(n) == Some(DS) && doc.local_name(n) == "Manifest").count();
        if manifests > 1 { return Err(Error::new("Multiple manifests in package object")); }
        if manifests == 1 { objects.push(object); }
    }
    if objects.len() != 1 { return Err(Error::new("Expected one package object")); }
    let object = objects[0];
    let Some(id) = doc.attr(object, "Id").filter(|s| !s.is_empty()) else { return Err(Error::new("Package object has no Id")) };
    let fragment = format!("#{id}");
    let references: Vec<_> = doc.elements(signed_info[0]).filter(|&n| doc.namespace(n) == Some(DS) && doc.local_name(n) == "Reference").collect();
    let matching: Vec<_> = references.iter().copied().filter(|&n| doc.attr(n, "URI") == Some(fragment.as_str())).collect();
    if matching.len() != 1 { return Err(Error::new("Package object reference is missing or ambiguous")); }
    if doc.attr(matching[0], "Type") != Some("http://www.w3.org/2000/09/xmldsig#Object") {
        return Ok(PackageObjectBinding::Unsupported);
    }
    Ok(PackageObjectBinding::Declared { object_id: id.to_string() })
}

/// Compare supported manifest references with package bytes.
///
/// This diagnostic does not authenticate the manifest, signature, signer, or
/// trust chain. Unsupported transformations are reported as unsupported. The
/// caller must never treat this result as verification.
pub fn inspect_manifest_digests(package: &Package, signature_part: &str) -> Result<Vec<ManifestReference>, Error> {
    let doc = package.xml(signature_part)?;
    let root = doc.root_element().ok_or_else(|| Error::new("Empty signature"))?;
    if doc.local_name(root) != "Signature" || doc.namespace(root) != Some(DS) {
        return Err(Error::new("Invalid signature root"));
    }
    let mut result = Vec::new();
    for object in doc.elements(root).filter(|&n| doc.namespace(n) == Some(DS) && doc.local_name(n) == "Object") {
        for manifest in doc.elements(object).filter(|&n| doc.namespace(n) == Some(DS) && doc.local_name(n) == "Manifest") {
            for reference in doc.elements(manifest) {
                if doc.namespace(reference) != Some(DS) || doc.local_name(reference) != "Reference" {
                    return Err(Error::new("Invalid manifest element"));
                }
                let uri = doc.attr(reference, "URI").ok_or_else(|| Error::new("Missing reference URI"))?.to_string();
                let children: Vec<_> = doc.elements(reference).collect();
                let find = |name| children.iter().copied().filter(|&n| doc.namespace(n) == Some(DS) && doc.local_name(n) == name).collect::<Vec<_>>();
                let methods = find("DigestMethod");
                let values = find("DigestValue");
                if methods.len() != 1 || values.len() != 1 { return Err(Error::new("Invalid manifest digest")); }
                let algorithm = doc.attr(methods[0], "Algorithm").ok_or_else(|| Error::new("Missing digest algorithm"))?;
                let encoded: String = doc.text(values[0]).chars().filter(|c| !matches!(c, '\t' | '\n' | '\r' | ' ')).collect();
                let expected = crate::base64::decode(&encoded).filter(|bytes| crate::base64::encode(bytes) == encoded)
                    .ok_or_else(|| Error::new("Invalid digest encoding"))?;
                let part_ref = uri.strip_prefix('/').and_then(|u| u.split_once("?ContentType="));
                let path = part_ref.map(|(path, _)| path);
                let content_type = match part_ref {
                    Some((path, kind)) if !path.is_empty() && !kind.is_empty() && !path.contains(['%', '\\', '#', ':']) && !kind.contains(['%', '&', '#']) => {
                        match package.content_type(path)? { Some(actual) if actual == kind => ContentTypeCheck::Matches, Some(_) => ContentTypeCheck::Differs, None => ContentTypeCheck::Missing }
                    }
                    _ => ContentTypeCheck::Unsupported,
                };
                let transforms = find("Transforms");
                let digest = if transforms.len() > 1 || path.is_none_or(|p| p.is_empty() || p.contains(['%', '\\', '#', ':']) || p.split('/').any(|s| s.is_empty() || s == "." || s == "..")) {
                    ManifestDigest::Unsupported
                } else {
                    let path = path.unwrap();
                    if !package.contains(path) { ManifestDigest::MissingPart } else {
                        let bytes = if transforms.is_empty() { Some(package.read(path)?) }
                            else { relationship_transform(package, path, &doc, transforms[0])? };
                        let actual = bytes.and_then(|bytes| match algorithm {
                            "http://www.w3.org/2000/09/xmldsig#sha1" => Some(sha1::Sha1::digest(&bytes).to_vec()),
                            "http://www.w3.org/2001/04/xmlenc#sha256" => Some(Sha256::digest(&bytes).to_vec()),
                            "http://www.w3.org/2001/04/xmldsig-more#sha384" => Some(Sha384::digest(&bytes).to_vec()),
                            "http://www.w3.org/2001/04/xmlenc#sha512" => Some(Sha512::digest(&bytes).to_vec()),
                            _ => None,
                        });
                        match actual {
                            Some(actual) if algorithm == "http://www.w3.org/2000/09/xmldsig#sha1" => ManifestDigest::LegacySha1 { matches: actual == expected },
                            Some(actual) if actual == expected => ManifestDigest::Matches,
                            Some(_) => ManifestDigest::Differs,
                            None => ManifestDigest::Unsupported,
                        }
                    }
                };
                result.push(ManifestReference { uri, digest, content_type });
            }
        }
    }
    Ok(result)
}

// Implements only the strict, namespace-clean subset of OPC Relationship
// Transform followed by inclusive C14N 1.0. Markup Compatibility and other
// extensions are rejected rather than silently transformed incorrectly.
fn relationship_transform(package: &Package, path: &str, signature: &crate::XmlDocument, transforms: crate::NodeId) -> Result<Option<Vec<u8>>, Error> {
    if !path.ends_with(".rels") { return Ok(None) }
    let steps: Vec<_> = signature.elements(transforms).collect();
    if steps.len() != 2 || steps.iter().any(|&n| signature.local_name(n) != "Transform" || signature.namespace(n) != Some(DS))
        || !matches!(signature.attr(steps[0], "Algorithm"), Some(REL_TRANSFORM | REL_TRANSFORM_2006)) || signature.attr(steps[1], "Algorithm") != Some(C14N)
        || signature.elements(steps[1]).next().is_some() { return Ok(None) }
    let mut ids = Vec::new();
    let mut types = Vec::new();
    for selector in signature.elements(steps[0]) {
        if signature.namespace(selector) != Some(OPC_DS) || signature.elements(selector).next().is_some() { return Ok(None) }
        match signature.local_name(selector) {
            "RelationshipReference" => match signature.attr(selector, "SourceId") { Some(s) if !s.is_empty() => ids.push(s), _ => return Ok(None) },
            "RelationshipsGroupReference" => match signature.attr(selector, "SourceType") { Some(s) if !s.is_empty() => types.push(s), _ => return Ok(None) },
            _ => return Ok(None),
        }
    }
    if ids.is_empty() && types.is_empty() { return Ok(None) }
    let rels = package.xml(path)?;
    let root = rels.root_element().ok_or_else(|| Error::new("Empty relationships part"))?;
    if rels.local_name(root) != "Relationships" || rels.namespace(root) != Some(REL_NS)
        || rels.attributes(root).iter().any(|a| !is_namespace_attr(&a.name))
        || rels.children(crate::XmlDocument::ROOT).iter().any(|&n| matches!(rels.kind(n), crate::xml::NodeKind::Doctype | crate::xml::NodeKind::ProcessingInstruction)) {
        return Ok(None);
    }
    let mut seen = std::collections::HashSet::new();
    let mut selected = Vec::new();
    for n in rels.elements(root) {
        if rels.local_name(n) != "Relationship" || rels.namespace(n) != Some(REL_NS)
            || rels.attributes(n).iter().any(|a| !matches!(a.name.as_str(), "Id" | "Type" | "Target" | "TargetMode") && !is_namespace_attr(&a.name)) { return Ok(None) }
        let (Some(id), Some(kind), Some(target)) = (rels.attr(n, "Id"), rels.attr(n, "Type"), rels.attr(n, "Target")) else { return Ok(None) };
        if id.is_empty() || !seen.insert(id) { return Ok(None) }
        let mode = rels.attr(n, "TargetMode").unwrap_or("Internal");
        if mode != "Internal" && mode != "External" { return Ok(None) }
        if ids.contains(&id) || types.contains(&kind) { selected.push((id, kind, target, mode)); }
    }
    selected.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = format!("<Relationships xmlns=\"{REL_NS}\">");
    for (id, kind, target, mode) in selected {
        out.push_str("<Relationship Id=\""); escape_c14n_attr(&mut out, id);
        out.push_str("\" Target=\""); escape_c14n_attr(&mut out, target);
        out.push_str("\" TargetMode=\""); escape_c14n_attr(&mut out, mode);
        out.push_str("\" Type=\""); escape_c14n_attr(&mut out, kind);
        out.push_str("\"></Relationship>");
    }
    out.push_str("</Relationships>");
    Ok(Some(out.into_bytes()))
}

fn is_namespace_attr(name: &str) -> bool { name == "xmlns" || name.starts_with("xmlns:") }

fn escape_c14n_attr(out: &mut String, value: &str) {
    for c in value.chars() {
        match c { '&' => out.push_str("&amp;"), '<' => out.push_str("&lt;"), '"' => out.push_str("&quot;"), '\t' => out.push_str("&#x9;"), '\n' => out.push_str("&#xA;"), '\r' => out.push_str("&#xD;"), _ => out.push(c) }
    }
}

/// Locate the package's signature parts without asserting that they are valid.
/// Malformed relationships and missing targets are errors, not unsigned files.
pub fn signature_parts(package: &Package) -> Result<Vec<String>, Error> {
    let origins = targets(package, "", ORIGIN)?;
    if origins.len() > 1 {
        return Err(Error::new("Multiple digital signature origins"));
    }
    let Some(origin) = origins.first() else { return Ok(Vec::new()) };
    let signatures = targets(package, origin, SIGNATURE)?;
    if signatures.is_empty() {
        return Err(Error::new("Digital signature origin has no signatures"));
    }
    Ok(signatures)
}

fn targets(package: &Package, source: &str, wanted: &str) -> Result<Vec<String>, Error> {
    let rels = crate::package::relationships_part(source);
    if !package.contains(&rels) { return Ok(Vec::new()) }
    let doc = package.xml(&rels)?;
    let root = doc.root_element().ok_or_else(|| Error::new("Empty relationships part"))?;
    if doc.local_name(root) != "Relationships" || doc.namespace(root) != Some(REL_NS) {
        return Err(Error::new("Invalid relationships root"));
    }
    let mut ids = std::collections::HashSet::new();
    let mut result = Vec::new();
    for node in doc.elements(root) {
        if doc.local_name(node) != "Relationship" || doc.namespace(node) != Some(REL_NS) {
            return Err(Error::new("Invalid relationship element"));
        }
        let id = doc.attr(node, "Id").filter(|s| !s.is_empty()).ok_or_else(|| Error::new("Relationship has no Id"))?;
        if !ids.insert(id) { return Err(Error::new("Duplicate relationship Id")); }
        let kind = doc.attr(node, "Type").ok_or_else(|| Error::new("Relationship has no Type"))?;
        let target = doc.attr(node, "Target").ok_or_else(|| Error::new("Relationship has no Target"))?;
        if kind != wanted { continue }
        if doc.attr(node, "TargetMode").is_some_and(|mode| mode != "Internal")
            || target.is_empty() || target.contains(['?', '#', '\\', '%', ':'])
        { return Err(Error::new("Unsupported signature relationship target")); }
        let dir = source.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut parts: Vec<&str> = if target.starts_with('/') || dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
        for component in target.trim_start_matches('/').split('/') {
            match component {
                "" => return Err(Error::new("Invalid signature relationship target")),
                "." => {},
                ".." => { if parts.pop().is_none() { return Err(Error::new("Signature target escapes package")); } },
                _ => parts.push(component),
            }
        }
        let name = parts.join("/");
        if !package.contains(&name) { return Err(Error::new("Missing digital signature part")); }
        if result.contains(&name) { return Err(Error::new("Duplicate signature relationship target")); }
        result.push(name);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::{EntryMeta, Writer};

    fn package(root: &str, origin_rels: Option<&str>) -> Package {
        let mut zip = Writer::new();
        zip.add("[Content_Types].xml", EntryMeta::new(), b"<Types/>").unwrap();
        zip.add("_rels/.rels", EntryMeta::new(), root.as_bytes()).unwrap();
        if let Some(rels) = origin_rels {
            zip.add("_xmlsignatures/origin.sigs", EntryMeta::new(), b"").unwrap();
            zip.add("_xmlsignatures/_rels/origin.sigs.rels", EntryMeta::new(), rels.as_bytes()).unwrap();
            zip.add("_xmlsignatures/sig1.xml", EntryMeta::new(), b"<Signature/>").unwrap();
        }
        Package::open(zip.finish().unwrap()).unwrap()
    }

    fn rels(content: &str) -> String {
        format!("<Relationships xmlns=\"{REL_NS}\">{content}</Relationships>")
    }

    fn rel(id: &str, kind: &str, target: &str, mode: &str) -> String {
        format!("<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\" {mode}/>")
    }

    #[test]
    fn discovers_referenced_parts_without_claiming_validity() {
        let root = rels(&rel("r1", ORIGIN, "_xmlsignatures/origin.sigs", ""));
        let origin = rels(&rel("r1", SIGNATURE, "sig1.xml", ""));
        assert_eq!(signature_parts(&package(&root, Some(&origin))).unwrap(), ["_xmlsignatures/sig1.xml"]);
        assert!(signature_parts(&package(&rels(""), None)).unwrap().is_empty());
    }

    #[test]
    fn malformed_and_external_relationships_are_not_unsigned() {
        let root = rels(&rel("r1", ORIGIN, "_xmlsignatures/origin.sigs", ""));
        for target in ["../../sig1.xml", "https://example.org/sig1.xml", "missing.xml"] {
            let origin = rels(&rel("r1", SIGNATURE, target, ""));
            assert!(signature_parts(&package(&root, Some(&origin))).is_err());
        }
        let origin = rels(&rel("r1", SIGNATURE, "sig1.xml", "TargetMode=\"External\""));
        assert!(signature_parts(&package(&root, Some(&origin))).is_err());
        let duplicate = rels(&format!("{}{}", rel("r1", ORIGIN, "_xmlsignatures/origin.sigs", ""), rel("r1", ORIGIN, "_xmlsignatures/origin.sigs", "")));
        assert!(signature_parts(&package(&duplicate, Some(&origin))).is_err());
        assert!(signature_parts(&package("<Relationships/>", None)).is_err());
    }

    #[test]
    fn manifest_digest_comparisons_remain_unauthenticated() {
        let mut pkg = package(&rels(""), None);
        pkg.set_bytes("word/document.xml", b"hello".to_vec());
        let value = crate::base64::encode(&Sha256::digest(b"hello"));
        let xml = format!(r#"<Signature xmlns="{DS}"><Object><Manifest>
          <Reference URI="/word/document.xml?ContentType=application/xml"><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>{value}</DigestValue></Reference>
          <Reference URI="/word/missing.xml?ContentType=application/xml"><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>{value}</DigestValue></Reference>
          <Reference URI="/word/document.xml?ContentType=application/xml"><Transforms/><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>{value}</DigestValue></Reference>
        </Manifest></Object></Signature>"#);
        pkg.set_bytes("sig.xml", xml.into_bytes());
        let result = inspect_manifest_digests(&pkg, "sig.xml").unwrap();
        assert_eq!(result.iter().map(|r| &r.digest).collect::<Vec<_>>(), [&ManifestDigest::Matches, &ManifestDigest::MissingPart, &ManifestDigest::Unsupported]);
        pkg.set_bytes("word/document.xml", b"changed".to_vec());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::Differs);
    }

    #[test]
    fn manifest_rejects_ambiguous_digest_fields_and_bad_base64() {
        let mut pkg = package(&rels(""), None);
        for digest in ["YQ=", "YQ==<DigestValue>YQ==</DigestValue>"] {
            let xml = format!(r#"<Signature xmlns="{DS}"><Object><Manifest><Reference URI="/x?ContentType=text/plain"><DigestMethod Algorithm="sha"/><DigestValue>{digest}</DigestValue></Reference></Manifest></Object></Signature>"#);
            pkg.set_bytes("sig.xml", xml.into_bytes());
            assert!(inspect_manifest_digests(&pkg, "sig.xml").is_err());
        }
    }

    #[test]
    fn relationship_transform_selects_and_sorts_before_digesting() {
        let mut pkg = package(&rels(""), None);
        let rels_xml = format!(r#"<r:Relationships xmlns:r="{REL_NS}"><r:Relationship Id="z" Type="kind" Target="unused"/><r:Relationship Target="b&amp;c" Type="wanted" Id="b"/><r:Relationship Target="a" Id="a" Type="one" TargetMode="External"/></r:Relationships>"#);
        pkg.set_bytes("word/_rels/document.xml.rels", rels_xml.into_bytes());
        let canonical = format!(r#"<Relationships xmlns="{REL_NS}"><Relationship Id="a" Target="a" TargetMode="External" Type="one"></Relationship><Relationship Id="b" Target="b&amp;c" TargetMode="Internal" Type="wanted"></Relationship></Relationships>"#);
        let expected = crate::base64::encode(&Sha256::digest(canonical.as_bytes()));
        let xml = format!(r#"<Signature xmlns="{DS}" xmlns:p="{OPC_DS}"><Object><Manifest><Reference URI="/word/_rels/document.xml.rels?ContentType=application/vnd.openxmlformats-package.relationships+xml"><Transforms><Transform Algorithm="{REL_TRANSFORM}"><p:RelationshipReference SourceId="a"/><p:RelationshipsGroupReference SourceType="wanted"/></Transform><Transform Algorithm="{C14N}"/></Transforms><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>{expected}</DigestValue></Reference></Manifest></Object></Signature>"#);
        pkg.set_bytes("sig.xml", xml.clone().into_bytes());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::Matches);
        pkg.set_bytes("sig.xml", xml.replace(REL_TRANSFORM, REL_TRANSFORM_2006).into_bytes());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::Matches);
        let changed = format!(r#"<Relationships xmlns="{REL_NS}"><Relationship Id="b" Type="wanted" Target="changed"/><Relationship Id="a" Type="one" Target="a" TargetMode="External"/></Relationships>"#);
        pkg.set_bytes("word/_rels/document.xml.rels", changed.into_bytes());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::Differs);
    }

    #[test]
    fn relationship_transform_rejects_markup_compatibility() {
        let mut pkg = package(&rels(""), None);
        pkg.set_bytes("r.rels", format!(r#"<Relationships xmlns="{REL_NS}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x"/>"#).into_bytes());
        let xml = format!(r#"<Signature xmlns="{DS}" xmlns:p="{OPC_DS}"><Object><Manifest><Reference URI="/r.rels?ContentType=application/xml"><Transforms><Transform Algorithm="{REL_TRANSFORM}"><p:RelationshipReference SourceId="a"/></Transform><Transform Algorithm="{C14N}"/></Transforms><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>YQ==</DigestValue></Reference></Manifest></Object></Signature>"#);
        pkg.set_bytes("sig.xml", xml.into_bytes());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::Unsupported);
    }

    #[test]
    fn additional_signed_info_references_are_bounded_and_not_resolved() {
        let mut pkg = package(&rels(""), None);
        let xml = format!(r##"<Signature xmlns="{DS}"><SignedInfo><Reference URI="#package"/><Reference URI="#properties"/><Reference URI="https://example.invalid/external"/><Reference/><Reference URI=""/><x:Reference xmlns:x="urn:other" URI="#ignored"/></SignedInfo></Signature>"##);
        pkg.set_bytes("sig.xml", xml.into_bytes());
        let refs = additional_signed_info_references(&pkg, "sig.xml", "package").unwrap();
        assert_eq!(refs.iter().map(|r| r.uri.as_deref()).collect::<Vec<_>>(), [Some("#properties"), Some("https://example.invalid/external"), None, Some("")]);
        assert!(refs.iter().all(|r| r.digest == ObjectDigest::Unsupported));
        let many = format!(r#"<Signature xmlns="{DS}"><SignedInfo>{}</SignedInfo></Signature>"#, "<Reference/>".repeat(257));
        pkg.set_bytes("sig.xml", many.into_bytes());
        assert!(additional_signed_info_references(&pkg, "sig.xml", "package").is_err());
        let oversized = format!(r#"<Signature xmlns="{DS}"><SignedInfo><Reference URI="{}"/></SignedInfo></Signature>"#, "a".repeat(4097));
        pkg.set_bytes("sig.xml", oversized.into_bytes());
        assert!(additional_signed_info_references(&pkg, "sig.xml", "package").is_err());
    }

    #[test]
    fn additional_object_digest_checks_are_bounded_and_reject_wrapping() {
        let mut pkg = package(&rels(""), None);
        // lxml C14N without comments and Python hashlib, independently computed.
        let reference = r##"<Reference URI="#office" Type="http://www.w3.org/2000/09/xmldsig#Object"><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>9iPjWb5cX1Cux6C3nNLl7xkWHOI3aTA7MH9VZ/sTtJI=</DigestValue></Reference>"##;
        let object = r#"<Object Id="office"><Data>original</Data><!-- ignored --></Object>"#;
        let xml = format!(r#"<Signature xmlns="{DS}"><SignedInfo>{reference}</SignedInfo>{object}</Signature>"#);
        pkg.set_bytes("sig.xml", xml.clone().into_bytes());
        assert_eq!(additional_signed_info_references(&pkg, "sig.xml", "package").unwrap()[0].digest, ObjectDigest::Matches);
        let with_transform = xml.replace("<DigestMethod", &format!("<Transforms><Transform Algorithm=\"{C14N}\"/></Transforms><DigestMethod"));
        pkg.set_bytes("sig.xml", with_transform.into_bytes());
        assert_eq!(additional_signed_info_references(&pkg, "sig.xml", "package").unwrap()[0].digest, ObjectDigest::Matches);
        pkg.set_bytes("sig.xml", xml.replace("original", "changed").into_bytes());
        assert_eq!(additional_signed_info_references(&pkg, "sig.xml", "package").unwrap()[0].digest, ObjectDigest::Differs);
        let weak = xml.replace("http://www.w3.org/2001/04/xmlenc#sha256", "http://www.w3.org/2000/09/xmldsig#sha1").replace("9iPjWb5cX1Cux6C3nNLl7xkWHOI3aTA7MH9VZ/sTtJI=", "Ck8iwHJuGDEO/YxHq9wymKbop/4=");
        pkg.set_bytes("sig.xml", weak.into_bytes());
        assert_eq!(additional_signed_info_references(&pkg, "sig.xml", "package").unwrap()[0].digest, ObjectDigest::LegacySha1 { matches: true });
        for bad in [
            xml.replace("#office", "#xpointer(id('office'))"),
            xml.replace("#office", "#123").replace("Id=\"office\"", "Id=\"123\""),
            xml.replace("<DigestMethod", "<Transforms><Transform Algorithm=\"urn:unsupported\"/></Transforms><DigestMethod"),
            xml.replace("Type=\"http://www.w3.org/2000/09/xmldsig#Object\"", "Type=\"urn:other\""),
            xml.replace(object, &format!("<Object><Wrapper>{object}</Wrapper></Object>")),
        ] {
            pkg.set_bytes("sig.xml", bad.into_bytes());
            assert_eq!(additional_signed_info_references(&pkg, "sig.xml", "package").unwrap()[0].digest, ObjectDigest::Unsupported);
        }
        pkg.set_bytes("sig.xml", xml.replace("</Signature>", "<Object xmlns:x=\"urn:other\" x:Id=\"office\"/></Signature>").into_bytes());
        assert!(additional_signed_info_references(&pkg, "sig.xml", "package").is_err());
    }

    #[test]
    fn xades_signed_properties_digest_is_only_an_observation() {
        let mut pkg = package(&rels(""), None);
        // Digest independently computed with lxml inclusive C14N and hashlib.
        let xml = r##"<ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#" xmlns:xades="http://uri.etsi.org/01903/v1.3.2#" Id="sig"><ds:SignedInfo><ds:Reference URI="#props" Type="http://uri.etsi.org/01903#SignedProperties"><ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><ds:DigestValue>7e71dWswIJxgdZy+9MKRvCSLpkbScCerkeZcd9zSA2I=</ds:DigestValue></ds:Reference></ds:SignedInfo><ds:Object><xades:QualifyingProperties Target="#sig"><xades:SignedProperties Id="props"><xades:SignedSignatureProperties><xades:SigningTime>2026-01-01T00:00:00Z</xades:SigningTime></xades:SignedSignatureProperties></xades:SignedProperties></xades:QualifyingProperties></ds:Object></ds:Signature>"##;
        pkg.set_bytes("sig.xml", xml.as_bytes().to_vec());
        assert_eq!(additional_signed_info_references(&pkg, "sig.xml", "package").unwrap()[0].digest, ObjectDigest::Matches);
        pkg.set_bytes("sig.xml", xml.replace("2026-01-01", "2027-01-01").into_bytes());
        assert_eq!(additional_signed_info_references(&pkg, "sig.xml", "package").unwrap()[0].digest, ObjectDigest::Differs);
        for bad in [
            xml.replace("Target=\"#sig\"", "Target=\"#other\""),
            xml.replace(" Id=\"sig\"", ""),
            xml.replace("http://uri.etsi.org/01903/v1.3.2#", "urn:spoof"),
            xml.replace("http://uri.etsi.org/01903#SignedProperties", "urn:other"),
            xml.replace("<xades:SignedProperties Id=\"props\">", "<xades:Wrapper><xades:SignedProperties Id=\"props\">").replace("</xades:SignedProperties>", "</xades:SignedProperties></xades:Wrapper>"),
        ] {
            pkg.set_bytes("sig.xml", bad.into_bytes());
            assert_eq!(additional_signed_info_references(&pkg, "sig.xml", "package").unwrap()[0].digest, ObjectDigest::Unsupported);
        }
        let duplicate = xml.replace("</ds:Object>", "<ds:Object Id=\"props\"/></ds:Object>");
        pkg.set_bytes("sig.xml", duplicate.into_bytes());
        assert!(additional_signed_info_references(&pkg, "sig.xml", "package").is_err());
    }

    #[test]
    fn xades_certificate_digest_uses_first_v2_certificate_and_fails_closed() {
        use ua_cryptography_xml_c14n::DocumentText;
        let source = r#"<x:SignedProperties xmlns:x="http://uri.etsi.org/01903/v1.3.2#" xmlns:d="http://www.w3.org/2000/09/xmldsig#"><x:SignedSignatureProperties><x:SigningCertificateV2><x:Cert><x:CertDigest><d:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><d:DigestValue>A9Zt0Ig1wco/EozOrNHzGslBYwlrIPRFroQoW8CDLXI=</d:DigestValue></x:CertDigest></x:Cert><x:Cert><x:CertDigest/></x:Cert></x:SigningCertificateV2></x:SignedSignatureProperties></x:SignedProperties>"#;
        let inspect = |xml: &str, cert: &[u8]| {
            let text = DocumentText::decode(xml.as_bytes()).unwrap();
            let doc = text.parse().unwrap();
            inspect_xades_certificate_digest(doc.root_element(), cert)
        };
        assert_eq!(inspect(source, b"certificate"), ObjectDigest::Matches);
        assert_eq!(inspect(source, b"different"), ObjectDigest::Differs);
        let weak = source.replace("http://www.w3.org/2001/04/xmlenc#sha256", "http://www.w3.org/2000/09/xmldsig#sha1").replace("A9Zt0Ig1wco/EozOrNHzGslBYwlrIPRFroQoW8CDLXI=", "c1rVccGJ17qERkv0qfHSKAF1sSg=");
        assert_eq!(inspect(&weak, b"certificate"), ObjectDigest::LegacySha1 { matches: true });
        assert_eq!(inspect(&weak, b"different"), ObjectDigest::LegacySha1 { matches: false });
        for bad in [source.replace("SigningCertificateV2", "SigningCertificate"), source.replace("http://www.w3.org/2001/04/xmlenc#sha256", "urn:unknown"), source.replace("A9Zt0Ig1wco/EozOrNHzGslBYwlrIPRFroQoW8CDLXI=", "not-base64!"), source.replace("<x:CertDigest/>", "<d:CertDigest/>")] {
            // The last replacement changes a later certificate only, which is deliberately not checked.
            if bad.contains("<d:CertDigest/>") { assert_eq!(inspect(&bad, b"certificate"), ObjectDigest::Matches); }
            else { assert_eq!(inspect(&bad, b"certificate"), ObjectDigest::Unsupported); }
        }
        let duplicate = source.replace("<x:SigningCertificateV2>", "<x:SigningCertificateV2/><x:SigningCertificateV2>");
        assert_eq!(inspect(&duplicate, b"certificate"), ObjectDigest::Unsupported);
    }

    #[test]
    fn ecdsa_p256_checks_xml_signature_format_and_curve() {
        let bytes = include_bytes!("../tests/fixtures/signatures/ecdsa-p256-signed.docx");
        let mut pkg = Package::open(bytes.to_vec()).unwrap();
        let part = "_xmlsignatures/sig1.xml";
        let xml = String::from_utf8(pkg.read(part).unwrap()).unwrap();
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::EcdsaP256 { matches: true });
        pkg.set_bytes(part, xml.replace("#package", "#changed").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::EcdsaP256 { matches: false });
        let old_cert = extract_single_embedded_certificate(&pkg, part).unwrap();
        let p384 = include_bytes!("../tests/fixtures/signatures/ecdsa-p384-cert.der");
        let wrong_curve = xml.replace(&crate::base64::encode(&old_cert), &crate::base64::encode(p384));
        pkg.set_bytes(part, wrong_curve.into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::Unsupported);
        pkg.set_bytes(part, xml.replace("ecdsa-sha256", "rsa-sha256").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::Unsupported);
        let doc = ua_cryptography_xml_c14n::DocumentText::decode(xml.as_bytes()).unwrap();
        let parsed = doc.parse().unwrap();
        let value = ds_children(parsed.root_element(), "SignatureValue")[0].text().unwrap();
        let der = include_bytes!("../tests/fixtures/signatures/ecdsa-p256-signature.der");
        pkg.set_bytes(part, xml.replace(value, &crate::base64::encode(der)).into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::EcdsaP256 { matches: false });
    }

    #[test]
    fn xades_issuer_serial_compares_only_exact_der_name_and_serial() {
        use ua_cryptography_xml_c14n::DocumentText;
        let pkg = Package::open(include_bytes!("../tests/fixtures/signatures/xades-issuer-serial-synthetic.docx").to_vec()).unwrap();
        let part = "_xmlsignatures/sig1.xml";
        let xml = String::from_utf8(pkg.read(part).unwrap()).unwrap();
        let cert = extract_single_embedded_certificate(&pkg, part).unwrap();
        let inspect = |source: &str| {
            let text = DocumentText::decode(source.as_bytes()).unwrap();
            let doc = text.parse().unwrap();
            let props = doc.root_element().descendants().find(|n| n.is_element() && n.tag_name().namespace() == Some("http://uri.etsi.org/01903/v1.3.2#") && n.tag_name().name() == "SignedProperties").unwrap();
            inspect_xades_issuer_serial(props, &cert)
        };
        assert_eq!(inspect(&xml), XadesIssuerSerial::Compared { issuer_der_equal: true, serial_equal: true });
        let start_tag = "<xades:IssuerSerialV2>"; let end_tag = "</xades:IssuerSerialV2>";
        let start = xml.find(start_tag).unwrap() + start_tag.len(); let end = xml[start..].find(end_tag).unwrap() + start;
        for (bytes, expected) in [
            (include_bytes!("../tests/fixtures/signatures/xades-wrong-issuer.der").as_slice(), XadesIssuerSerial::Compared { issuer_der_equal: false, serial_equal: true }),
            (include_bytes!("../tests/fixtures/signatures/xades-wrong-serial.der").as_slice(), XadesIssuerSerial::Compared { issuer_der_equal: true, serial_equal: false }),
            (include_bytes!("../tests/fixtures/signatures/xades-multiple-names.der").as_slice(), XadesIssuerSerial::Unsupported),
            (b"bad".as_slice(), XadesIssuerSerial::Unsupported),
        ] {
            let changed = format!("{}{}{}", &xml[..start], crate::base64::encode(bytes), &xml[end..]);
            assert_eq!(inspect(&changed), expected);
        }
        let encoded = &xml[start..end];
        assert_eq!(inspect(&xml.replace(encoded, "not-base64!")), XadesIssuerSerial::Unsupported);
        assert_eq!(inspect(&xml.replace(encoded, &crate::base64::encode(&vec![0; 64 * 1024 + 1]))), XadesIssuerSerial::Unsupported);
        let mut trailing = crate::base64::decode(encoded).unwrap(); trailing.push(0);
        assert_eq!(inspect(&xml.replace(encoded, &crate::base64::encode(&trailing))), XadesIssuerSerial::Unsupported);
        let full = format!("{start_tag}{encoded}{end_tag}");
        assert_eq!(inspect(&xml.replace(&full, "")), XadesIssuerSerial::Absent);
        assert_eq!(inspect(&xml.replace(&full, &format!("{full}{full}"))), XadesIssuerSerial::Unsupported);
    }

    #[test]
    fn legacy_sha1_observations_are_distinct_and_detect_changes() {
        let mut pkg = package(&rels(""), None);
        let signature = include_str!("../tests/fixtures/signatures/rsa-sha1-synthetic.xml");
        pkg.set_bytes("sig.xml", signature.as_bytes().to_vec());
        assert_eq!(inspect_signature_value(&pkg, "sig.xml").unwrap(), SignatureValueCheck::LegacySha1 { key_bits: 1024, matches: true });
        pkg.set_bytes("sig.xml", signature.replace("#test", "#changed").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, "sig.xml").unwrap(), SignatureValueCheck::LegacySha1 { key_bits: 1024, matches: false });

        let part = b"synthetic part";
        pkg.set_bytes("document.xml", part.to_vec());
        let part_digest = crate::base64::encode(&sha1::Sha1::digest(part));
        let object = format!(r#"<Object xmlns="{DS}" Id="package"><Manifest><Reference URI="/document.xml?ContentType=application/xml"><DigestMethod Algorithm="{DS}sha1"></DigestMethod><DigestValue>{part_digest}</DigestValue></Reference></Manifest></Object>"#);
        let object_digest = crate::base64::encode(&sha1::Sha1::digest(object.as_bytes()));
        let xml = format!(r##"<Signature xmlns="{DS}"><SignedInfo><Reference URI="#package" Type="{DS}Object"><DigestMethod Algorithm="{DS}sha1"/><DigestValue>{object_digest}</DigestValue></Reference></SignedInfo>{object}</Signature>"##);
        pkg.set_bytes("sig.xml", xml.into_bytes());
        assert_eq!(inspect_package_object_digest(&pkg, "sig.xml").unwrap(), ObjectDigest::LegacySha1 { matches: true });
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::LegacySha1 { matches: true });
        pkg.set_bytes("document.xml", b"changed".to_vec());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::LegacySha1 { matches: false });
        let changed = String::from_utf8(pkg.read("sig.xml").unwrap()).unwrap().replace(&part_digest, "AAAAAAAAAAAAAAAAAAAAAAAAAAA=");
        pkg.set_bytes("sig.xml", changed.into_bytes());
        assert_eq!(inspect_package_object_digest(&pkg, "sig.xml").unwrap(), ObjectDigest::LegacySha1 { matches: false });
    }

    #[test]
    fn package_object_binding_is_declared_not_verified() {
        let mut pkg = package(&rels(""), None);
        let reference = r##"<Reference URI="#package" Type="http://www.w3.org/2000/09/xmldsig#Object"/>"##;
        let xml = format!(r#"<Signature xmlns="{DS}"><SignedInfo>{reference}</SignedInfo><Object Id="package"><Manifest/></Object></Signature>"#);
        pkg.set_bytes("sig.xml", xml.into_bytes());
        assert_eq!(inspect_package_object_binding(&pkg, "sig.xml").unwrap(), PackageObjectBinding::Declared { object_id: "package".into() });
        let missing = format!(r#"<Signature xmlns="{DS}"><SignedInfo/><Object Id="package"><Manifest/></Object></Signature>"#);
        pkg.set_bytes("sig.xml", missing.into_bytes());
        assert!(inspect_package_object_binding(&pkg, "sig.xml").is_err());
        let duplicate_reference = format!(r#"<Signature xmlns="{DS}"><SignedInfo>{reference}{reference}</SignedInfo><Object Id="package"><Manifest/></Object></Signature>"#);
        pkg.set_bytes("sig.xml", duplicate_reference.into_bytes());
        assert!(inspect_package_object_binding(&pkg, "sig.xml").is_err());
    }

    #[test]
    fn package_object_binding_rejects_id_wrapping() {
        let mut pkg = package(&rels(""), None);
        for extra in [r#"<Object xml:id="package"/>"#, r#"<Object ID="package"/>"#, r#"<Object xmlns:x="urn:example" x:Id="package"/>"#, r#"<Object Id="other"><Manifest/></Object>"#] {
            let xml = format!(r##"<Signature xmlns="{DS}"><SignedInfo><Reference URI="#package" Type="http://www.w3.org/2000/09/xmldsig#Object"/></SignedInfo><Object Id="package"><Manifest/></Object>{extra}</Signature>"##);
            pkg.set_bytes("sig.xml", xml.into_bytes());
            assert!(inspect_package_object_binding(&pkg, "sig.xml").is_err());
        }
    }

    #[test]
    fn package_object_digest_detects_modification_and_wrapping() {
        let mut pkg = package(&rels(""), None);
        let object = format!(r#"<Object xmlns="{DS}" Id="package"><Manifest></Manifest></Object>"#);
        let digest = crate::base64::encode(&Sha256::digest(object.as_bytes()));
        let xml = format!(r##"<Signature xmlns="{DS}"><SignedInfo><Reference URI="#package" Type="http://www.w3.org/2000/09/xmldsig#Object"><Transforms><Transform Algorithm="{C14N}"/></Transforms><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>{digest}</DigestValue></Reference></SignedInfo><Object Id="package"><Manifest/></Object></Signature>"##);
        pkg.set_bytes("sig.xml", xml.clone().into_bytes());
        assert_eq!(inspect_package_object_digest(&pkg, "sig.xml").unwrap(), ObjectDigest::Matches);
        let implicit = xml.replace(&format!("<Transforms><Transform Algorithm=\"{C14N}\"/></Transforms>"), "");
        pkg.set_bytes("sig.xml", implicit.into_bytes());
        assert_eq!(inspect_package_object_digest(&pkg, "sig.xml").unwrap(), ObjectDigest::Matches);
        pkg.set_bytes("sig.xml", xml.replace("<Manifest/>", "<Manifest><Reference/></Manifest>").into_bytes());
        assert_eq!(inspect_package_object_digest(&pkg, "sig.xml").unwrap(), ObjectDigest::Differs);
        pkg.set_bytes("sig.xml", xml.replace("</Signature>", "<Object xml:id=\"package\"/></Signature>").into_bytes());
        assert!(inspect_package_object_digest(&pkg, "sig.xml").is_err());
        pkg.set_bytes("sig.xml", xml.replace(C14N, "urn:unsupported-transform").into_bytes());
        assert_eq!(inspect_package_object_digest(&pkg, "sig.xml").unwrap(), ObjectDigest::Unsupported);
    }

    #[test]
    fn rsa_signature_value_is_checked_without_asserting_trust() {
        let mut pkg = package(&rels(""), None);
        let xml = include_str!("../tests/fixtures/signatures/rsa-sha256-synthetic.xml");
        pkg.set_bytes("sig.xml", xml.as_bytes().to_vec());
        assert_eq!(inspect_signature_value(&pkg, "sig.xml").unwrap(), SignatureValueCheck::MatchesUntrustedCertificate);
        pkg.set_bytes("sig.xml", xml.replace("#test", "#tampered").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, "sig.xml").unwrap(), SignatureValueCheck::Differs);
        pkg.set_bytes("sig.xml", xml.replace("rsa-sha256", "rsa-sha512").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, "sig.xml").unwrap(), SignatureValueCheck::Unsupported);
    }

    #[test]
    fn synthetic_signed_package_reports_each_stage_separately() {
        let data = include_bytes!("../tests/fixtures/signatures/synthetic-signed.docx");
        let mut pkg = Package::open(data.as_slice()).unwrap();
        let result = inspect_signatures(&pkg).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].signature_value, SignatureValueCheck::MatchesUntrustedCertificate);
        assert_eq!(result[0].package_object, ObjectDigest::Matches);
        assert_eq!(result[0].manifest[0].digest, ManifestDigest::Matches);
        assert_eq!(result[0].manifest[0].content_type, ContentTypeCheck::Matches);
        assert_eq!(result[0].trust, TrustCheck::NotEvaluated);
        pkg.set_bytes("word/document.xml", b"tampered".to_vec());
        let result = inspect_signatures(&pkg).unwrap();
        assert_eq!(result[0].signature_value, SignatureValueCheck::MatchesUntrustedCertificate);
        assert_eq!(result[0].package_object, ObjectDigest::Matches);
        assert_eq!(result[0].manifest[0].digest, ManifestDigest::Differs);
        pkg.set_override("word/document.xml", "application/other").unwrap();
        assert_eq!(inspect_signatures(&pkg).unwrap()[0].manifest[0].content_type, ContentTypeCheck::Differs);
    }

    #[test]
    fn certificate_facts_use_explicit_time_and_exact_anchor_bytes() {
        let mut pkg = package(&rels(""), None);
        let xml = include_str!("../tests/fixtures/signatures/rsa-sha256-synthetic.xml");
        pkg.set_bytes("sig.xml", xml.as_bytes().to_vec());
        let first = inspect_signer_certificate(&pkg, "sig.xml", 0, &[]).unwrap();
        assert_eq!(first.time_status, CertificateTime::BeforeValidity);
        assert!(!first.identical_to_supplied_anchor);
        let at_start = inspect_signer_certificate(&pkg, "sig.xml", first.not_before, &[]).unwrap();
        assert_eq!(at_start.time_status, CertificateTime::WithinValidity);
        assert_eq!(inspect_signer_certificate(&pkg, "sig.xml", first.not_after, &[]).unwrap().time_status, CertificateTime::WithinValidity);
        assert_eq!(inspect_signer_certificate(&pkg, "sig.xml", first.not_after + 1, &[]).unwrap().time_status, CertificateTime::AfterValidity);
        let doc = crate::XmlDocument::parse(xml.as_bytes()).unwrap();
        let root = doc.root_element().unwrap();
        let node = doc.descendants(root).into_iter().find(|&n| doc.local_name(n) == "X509Certificate").unwrap();
        let der = crate::base64::decode(&doc.text(node)).unwrap();
        let facts = inspect_signer_certificate(&pkg, "sig.xml", first.not_before, &[&der]).unwrap();
        assert!(facts.identical_to_supplied_anchor);
        assert_eq!(facts.sha256.as_slice(), Sha256::digest(&der).as_slice());
        let unrelated = b"not an anchor";
        assert!(!inspect_signer_certificate(&pkg, "sig.xml", first.not_before, &[unrelated]).unwrap().identical_to_supplied_anchor);
        for (cert, expected) in [
            (include_bytes!("../tests/fixtures/signatures/digital-signature.der").as_slice(), CertificateKeyUsage::Present { digital_signature: true, content_commitment: true }),
            (include_bytes!("../tests/fixtures/signatures/encipherment-only.der").as_slice(), CertificateKeyUsage::Present { digital_signature: false, content_commitment: false }),
        ] {
            let changed = xml.replace(&doc.text(node), &crate::base64::encode(cert));
            pkg.set_bytes("sig.xml", changed.into_bytes());
            assert_eq!(inspect_signer_certificate(&pkg, "sig.xml", 0, &[]).unwrap().key_usage, expected);
        }
    }

    #[test]
    fn certificate_link_does_not_conflate_matching_names_and_signatures() {
        let leaf = include_bytes!("../tests/fixtures/signatures/issuer-signed-leaf.der");
        let issuer = include_bytes!("../tests/fixtures/signatures/issuer.der");
        let other = include_bytes!("../tests/fixtures/signatures/other-issuer.der");
        let facts = inspect_certificate_link(leaf, issuer).unwrap();
        assert!(facts.issuer_name_exact);
        assert_eq!(facts.issuer_ca, Some(true));
        assert_eq!(facts.issuer_path_length, Some(0));
        assert_eq!(facts.issuer_key_cert_sign, Some(true));
        assert_eq!(facts.child_signature, CertificateSignature::Matches);
        let facts = inspect_certificate_link(leaf, other).unwrap();
        assert!(facts.issuer_name_exact);
        assert_eq!(facts.child_signature, CertificateSignature::Differs);
        let facts = inspect_certificate_link(leaf, leaf).unwrap();
        assert!(!facts.issuer_name_exact);
        assert_eq!(facts.issuer_ca, Some(false));
        assert_eq!(facts.issuer_key_cert_sign, Some(false));
        assert!(inspect_certificate_link(b"bad", issuer).is_err());
    }

    #[test]
    fn ecdsa_certificate_and_crl_signatures_remain_observations() {
        use x509_cert::{Certificate, crl::CertificateList, der::{Decode, Encode, asn1::Any}};
        let leaf = include_bytes!("../tests/fixtures/signatures/ec-leaf.der");
        let issuer = include_bytes!("../tests/fixtures/signatures/ec-ca.der");
        let other = include_bytes!("../tests/fixtures/signatures/ec-other.der");
        let p384 = include_bytes!("../tests/fixtures/signatures/ec-p384.der");
        let empty = include_bytes!("../tests/fixtures/signatures/ec-empty.crl.der");
        let revoked = include_bytes!("../tests/fixtures/signatures/ec-revoked.crl.der");
        let facts = inspect_certificate_link(leaf, issuer).unwrap();
        assert!(facts.issuer_name_exact);
        assert_eq!(facts.issuer_ca, Some(true));
        assert_eq!(facts.issuer_key_cert_sign, Some(true));
        assert_eq!(facts.child_signature, CertificateSignature::Matches);
        assert_eq!(inspect_certificate_link(leaf, other).unwrap().child_signature, CertificateSignature::Differs);
        assert_eq!(inspect_certificate_link(leaf, p384).unwrap().child_signature, CertificateSignature::Unsupported);
        let candidates = find_candidate_certificate_paths(leaf, &[], &[issuer, other]).unwrap();
        assert_eq!(candidates.paths.len(), 2);
        assert_eq!(candidates.paths[0].links[0].child_signature, CertificateSignature::Matches);
        assert_eq!(candidates.paths[1].links[0].child_signature, CertificateSignature::Differs);
        let mut cert = Certificate::from_der(leaf.as_slice()).unwrap();
        cert.signature_algorithm.parameters = Some(Any::null());
        assert_eq!(inspect_certificate_link(&cert.to_der().unwrap(), issuer).unwrap().child_signature, CertificateSignature::Unsupported);
        cert.tbs_certificate.signature.parameters = Some(Any::null());
        assert_eq!(inspect_certificate_link(&cert.to_der().unwrap(), issuer).unwrap().child_signature, CertificateSignature::Unsupported);
        let facts = inspect_crl(empty, leaf, issuer, 0).unwrap();
        assert_eq!(facts.signature, CertificateSignature::Matches);
        assert_eq!(facts.issuer_crl_sign, Some(true));
        assert!(facts.listed_revocation_time.is_none());
        assert!(inspect_crl(revoked, leaf, issuer, 0).unwrap().listed_revocation_time.is_some());
        assert_eq!(inspect_crl(revoked, leaf, issuer, 0).unwrap().signature, CertificateSignature::Matches);
        assert_eq!(inspect_crl(empty, leaf, other, 0).unwrap().signature, CertificateSignature::Differs);
        assert_eq!(inspect_crl(empty, leaf, p384, 0).unwrap().signature, CertificateSignature::Unsupported);
        let mut damaged = empty.to_vec(); let last = damaged.len() - 1; damaged[last] ^= 1;
        assert_eq!(inspect_crl(&damaged, leaf, issuer, 0).unwrap().signature, CertificateSignature::Differs);
        let mut crl = CertificateList::from_der(empty.as_slice()).unwrap();
        crl.signature_algorithm.parameters = Some(Any::null());
        assert_eq!(inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 0).unwrap().signature, CertificateSignature::Unsupported);
        crl.tbs_cert_list.signature.parameters = Some(Any::null());
        assert_eq!(inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 0).unwrap().signature, CertificateSignature::Unsupported);
    }

    #[test]
    fn certificate_path_candidates_are_bounded_and_not_trust_verdicts() {
        let leaf = include_bytes!("../tests/fixtures/signatures/issuer-signed-leaf.der");
        let issuer = include_bytes!("../tests/fixtures/signatures/issuer.der");
        let other = include_bytes!("../tests/fixtures/signatures/other-issuer.der");
        let result = find_candidate_certificate_paths(leaf, &[], &[issuer, other, issuer]).unwrap();
        assert!(!result.truncated);
        assert_eq!(result.paths.len(), 2);
        assert_eq!(result.paths[0].links[0].child_signature, CertificateSignature::Matches);
        assert_eq!(result.paths[1].links[0].child_signature, CertificateSignature::Differs);
        assert_eq!(result.paths[0].fingerprints_sha256.len(), 2);
        let unknown = find_candidate_certificate_paths(leaf, &[issuer], &[]).unwrap();
        assert!(unknown.paths.is_empty());
        assert!(!unknown.truncated);
        let direct = find_candidate_certificate_paths(leaf, &[], &[leaf]).unwrap();
        assert_eq!(direct.paths.len(), 1);
        assert!(direct.paths[0].links.is_empty());
        let too_many = vec![issuer.as_slice(); 64];
        assert!(find_candidate_certificate_paths(leaf, &too_many, &[]).is_err());
        assert!(find_candidate_certificate_paths(leaf, &[b"bad"], &[]).is_err());
        let many: Vec<Vec<u8>> = (0u8..35).map(|i| { let mut cert = issuer.to_vec(); let last = cert.len() - 1; cert[last] = i; cert }).collect();
        let refs: Vec<&[u8]> = many.iter().map(Vec::as_slice).collect();
        let limited = find_candidate_certificate_paths(leaf, &[], &refs).unwrap();
        assert_eq!(limited.paths.len(), 32);
        assert!(limited.truncated);
    }

    #[test]
    fn crl_observations_keep_absence_separate_from_revocation_status() {
        use x509_cert::{crl::CertificateList, der::{Decode, Encode, asn1::{ObjectIdentifier, OctetString}}, ext::Extension};
        let leaf = include_bytes!("../tests/fixtures/signatures/crl-leaf.der");
        let issuer = include_bytes!("../tests/fixtures/signatures/crl-issuer.der");
        let empty = include_bytes!("../tests/fixtures/signatures/empty.crl.der");
        let revoked = include_bytes!("../tests/fixtures/signatures/revoked.crl.der");
        let initial = inspect_crl(empty, leaf, issuer, 0).unwrap();
        assert_eq!(initial.time_status, CrlTime::NotYetIssued);
        let facts = inspect_crl(empty, leaf, issuer, initial.this_update).unwrap();
        assert!(facts.issuer_name_exact && facts.certificate_issuer_name_exact);
        assert_eq!(facts.signature, CertificateSignature::Matches);
        assert_eq!(facts.issuer_crl_sign, Some(true));
        assert_eq!(facts.time_status, CrlTime::WithinWindow);
        assert_eq!(facts.scope, CrlScope::NoScopeExtensionsObserved);
        assert_eq!(facts.listed_revocation_time, None);
        let revoked_facts = inspect_crl(revoked, leaf, issuer, initial.this_update).unwrap();
        assert!(revoked_facts.listed_revocation_time.is_some());
        assert_eq!(revoked_facts.signature, CertificateSignature::Matches);
        assert_eq!(inspect_crl(empty, leaf, issuer, initial.next_update.unwrap()).unwrap().time_status, CrlTime::Expired);
        let other = include_bytes!("../tests/fixtures/signatures/other-issuer.der");
        assert_eq!(inspect_crl(empty, leaf, other, initial.this_update).unwrap().signature, CertificateSignature::Differs);
        let mut modified = empty.to_vec(); let last = modified.len() - 1; modified[last] ^= 1;
        assert_eq!(inspect_crl(&modified, leaf, issuer, initial.this_update).unwrap().signature, CertificateSignature::Differs);
        let mut scoped = CertificateList::from_der(empty.as_slice()).unwrap();
        scoped.tbs_cert_list.crl_extensions.get_or_insert_with(Vec::new).push(Extension { extn_id: ObjectIdentifier::new_unwrap("2.5.29.28"), critical: true, extn_value: OctetString::new(vec![0x30, 0]).unwrap() });
        assert_eq!(inspect_crl(&scoped.to_der().unwrap(), leaf, issuer, initial.this_update).unwrap().scope, CrlScope::UnsupportedExtensions);
    }
}

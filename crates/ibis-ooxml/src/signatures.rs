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
    /// Relationship selection is unauthenticated and applies to this reference only.
    pub relationships: Option<RelationshipReference>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RelationshipReference {
    /// The reference declares the complete relationship part as raw bytes.
    WholePart,
    Filtered(RelationshipSelection),
    Unsupported,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RelationshipSelection {
    pub selected: usize,
    pub unselected: usize,
    pub unmatched_id_selectors: usize,
    pub unmatched_type_selectors: usize,
    /// Up to eight examples each, truncated at 200 characters.
    pub selected_examples: Vec<String>,
    pub unselected_examples: Vec<String>,
    pub examples_truncated: bool,
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

/// Literal current package-entry inventory relative to one manifest. This does not
/// establish coverage: transformations, relationships, other references,
/// signatures and OPC metadata require separate interpretation.
#[derive(Debug, PartialEq, Eq)]
pub struct ManifestInventory {
    pub entries: usize,
    pub unlisted: usize,
    pub ambiguous_entries: usize,
    pub unresolved_references: usize,
    /// At most 16 examples; names longer than 200 characters are shortened.
    pub unlisted_examples: Vec<String>,
    pub examples_truncated: bool,
}

/// Unauthenticated metadata from one certificate's certificatePolicies.
/// No policy tree, qualifiers, path, jurisdiction, or legal status is validated.
#[derive(Debug, PartialEq, Eq)]
pub enum CertificatePolicyObservations {
    Absent,
    InvalidOrDuplicateExtension,
    Present { critical: bool, total: usize, inspected: usize, truncated: bool,
        repeated_oid_entries_inspected: usize, any_policy_inspected: bool,
        qualifier_entries_inspected: usize, empty_qualifier_sequences_inspected: usize,
        examples: Vec<String> },
}

/// Inspect policy extension metadata in an explicitly supplied certificate.
pub fn inspect_certificate_policies(certificate_der: &[u8]) -> Result<CertificatePolicyObservations, Error> {
    use x509_cert::{Certificate, der::Decode, ext::pkix::CertificatePolicies};
    if certificate_der.len() > 1024 * 1024 { return Err(Error::new("Certificate is too large")); }
    let cert = Certificate::from_der(certificate_der).map_err(|_| Error::new("Invalid certificate"))?;
    let (critical, policies) = match cert.tbs_certificate.get::<CertificatePolicies>() {
        Ok(None) => return Ok(CertificatePolicyObservations::Absent),
        Ok(Some((critical, policies))) if !policies.0.is_empty() => (critical, policies),
        _ => return Ok(CertificatePolicyObservations::InvalidOrDuplicateExtension),
    };
    let total = policies.0.len();
    let mut seen = Vec::with_capacity(64);
    let mut examples = Vec::new();
    let mut repeated = 0usize;
    let mut any_policy = false;
    let mut qualifier_entries = 0usize;
    let mut empty_qualifiers = 0usize;
    for policy in policies.0.iter().take(64) {
        if seen.contains(&policy.policy_identifier) { repeated += 1; } else { seen.push(policy.policy_identifier); }
        let oid = policy.policy_identifier.to_string();
        any_policy |= oid == "2.5.29.32.0";
        if examples.len() < 8 { examples.push(oid); }
        if let Some(qualifiers) = &policy.policy_qualifiers {
            qualifier_entries += 1;
            if qualifiers.is_empty() { empty_qualifiers += 1; }
        }
    }
    Ok(CertificatePolicyObservations::Present { critical, total, inspected: total.min(64), truncated: total > 64,
        repeated_oid_entries_inspected: repeated, any_policy_inspected: any_policy,
        qualifier_entries_inspected: qualifier_entries, empty_qualifier_sequences_inspected: empty_qualifiers, examples })
}

/// Encoded policy-related data from one untrusted certificate. No values are
/// applied as path constraints or treated as evidence of policy compliance.
#[derive(Debug, PartialEq, Eq)]
pub struct CertificatePolicyControls {
    pub mappings: CertificateControl<CertificatePolicyMappings>,
    pub constraints: CertificateControl<CertificatePolicyConstraints>,
    pub inhibit_any_policy: CertificateControl<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CertificateControl<T> {
    Absent,
    /// Includes invalid, duplicate, and values outside this parser's limits.
    UnparsedOrDuplicate,
    Present { critical: bool, value: T },
}

#[derive(Debug, PartialEq, Eq)]
pub struct CertificatePolicyMappings {
    pub total: usize,
    pub inspected: usize,
    pub truncated: bool,
    pub any_policy_pairs_inspected: usize,
    pub repeated_pairs_inspected: usize,
    /// At most eight literal issuer/subject OID pairs; no mapping is applied.
    pub examples: Vec<(String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CertificatePolicyConstraints {
    pub require_explicit_policy: Option<u32>,
    pub inhibit_policy_mapping: Option<u32>,
}

/// Observe only policy control extensions on one supplied certificate.
/// No trust, constraint applicability, validity or path policy is decided.
pub fn inspect_certificate_policy_controls(certificate_der: &[u8]) -> Result<CertificatePolicyControls, Error> {
    use x509_cert::{Certificate, der::Decode, ext::pkix::{InhibitAnyPolicy, PolicyConstraints, PolicyMappings}};
    if certificate_der.len() > 1024 * 1024 { return Err(Error::new("Certificate is too large")); }
    let cert = Certificate::from_der(certificate_der).map_err(|_| Error::new("Invalid certificate"))?;
    let mappings = match cert.tbs_certificate.get::<PolicyMappings>() {
        Ok(None) => CertificateControl::Absent,
        Ok(Some((critical, mappings))) if !mappings.0.is_empty() => {
            let total = mappings.0.len();
            let mut seen = Vec::with_capacity(64);
            let mut examples = Vec::new();
            let mut repeated = 0;
            let mut any_policy = 0;
            for mapping in mappings.0.iter().take(64) {
                let pair = (mapping.issuer_domain_policy, mapping.subject_domain_policy);
                if seen.contains(&pair) { repeated += 1; } else { seen.push(pair); }
                let issuer = pair.0.to_string(); let subject = pair.1.to_string();
                if issuer == "2.5.29.32.0" || subject == "2.5.29.32.0" { any_policy += 1; }
                if examples.len() < 8 { examples.push((issuer, subject)); }
            }
            CertificateControl::Present { critical, value: CertificatePolicyMappings {
                total, inspected: total.min(64), truncated: total > 64,
                any_policy_pairs_inspected: any_policy, repeated_pairs_inspected: repeated, examples,
            } }
        }
        _ => CertificateControl::UnparsedOrDuplicate,
    };
    let constraints = match cert.tbs_certificate.get::<PolicyConstraints>() {
        Ok(None) => CertificateControl::Absent,
        Ok(Some((critical, value))) => CertificateControl::Present { critical, value: CertificatePolicyConstraints {
            require_explicit_policy: value.require_explicit_policy, inhibit_policy_mapping: value.inhibit_policy_mapping,
        } },
        Err(_) => CertificateControl::UnparsedOrDuplicate,
    };
    let inhibit_any_policy = match cert.tbs_certificate.get::<InhibitAnyPolicy>() {
        Ok(None) => CertificateControl::Absent,
        Ok(Some((critical, value))) => CertificateControl::Present { critical, value: value.0 },
        Err(_) => CertificateControl::UnparsedOrDuplicate,
    };
    Ok(CertificatePolicyControls { mappings, constraints, inhibit_any_policy })
}

/// Untrusted certificate extension hints. No URI is fetched or validated.
#[derive(Debug, PartialEq, Eq)]
pub struct CertificateLocationHints {
    pub aia: LocationExtension,
    pub crl_distribution_points: LocationExtension,
    pub ocsp: LocationList,
    pub ca_issuers: LocationList,
    pub crl: LocationList,
    pub other_aia_methods: usize,
    pub scoped_crl_points: usize,
    pub alternate_crl_issuer_points: usize,
    pub other_crl_points: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LocationExtension { Absent, Parsed { critical: bool }, Unsupported }

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LocationList {
    pub total_uris: usize,
    /// Up to eight complete URI strings, each at most 2048 bytes.
    pub examples: Vec<String>,
    pub omitted_uris: usize,
    pub non_uri_names: usize,
}

impl LocationList {
    fn record(&mut self, name: &x509_cert::ext::pkix::name::GeneralName) {
        use x509_cert::ext::pkix::name::GeneralName;
        match name {
            GeneralName::UniformResourceIdentifier(uri) if !uri.as_str().is_empty() => {
                self.total_uris += 1;
                if uri.as_str().len() <= 2048 && self.examples.len() < 8 { self.examples.push(uri.as_str().to_string()); }
                else { self.omitted_uris += 1; }
            }
            _ => self.non_uri_names += 1,
        }
    }
}

/// Individual observations, deliberately without an overall valid verdict.
#[derive(Debug, PartialEq, Eq)]
pub struct SignatureInspection {
    pub signature_part: String,
    pub signature_value: SignatureValueCheck,
    /// None means the embedded certificate could not be inspected.
    pub certificate_locations: Option<CertificateLocationHints>,
    /// Embedded certificate policy OIDs are untrusted, not validated policies.
    pub certificate_policies: Option<CertificatePolicyObservations>,
    pub package_object: ObjectDigest,
    /// Additional SignedInfo reference observations, not authenticated results.
    pub additional_signed_info_references: Vec<AdditionalReference>,
    pub manifest: Vec<ManifestReference>,
    pub manifest_inventory: ManifestInventory,
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

pub(crate) fn verify_rsa_sha2(algorithm: &str, key: rsa::RsaPublicKey, message: &[u8], signature: &rsa::pkcs1v15::Signature) -> bool {
    use rsa::{pkcs1v15::VerifyingKey, signature::Verifier};
    match algorithm {
        "sha256" => VerifyingKey::<Sha256>::new(key).verify(message, signature).is_ok(),
        "sha384" => VerifyingKey::<Sha384>::new(key).verify(message, signature).is_ok(),
        "sha512" => VerifyingKey::<Sha512>::new(key).verify(message, signature).is_ok(),
        _ => false,
    }
}

pub(crate) fn rsa_x509_hash(oid: &str) -> Option<&'static str> {
    match oid {
        "1.2.840.113549.1.1.11" => Some("sha256"),
        "1.2.840.113549.1.1.12" => Some("sha384"),
        "1.2.840.113549.1.1.13" => Some("sha512"),
        _ => None,
    }
}

/// Compare X.509 RSA-PSS signatures only for explicit SHA-256/MGF1-SHA256,
/// a 32-byte salt, and an ordinary rsaEncryption issuer key.
pub(crate) fn inspect_rsa_pss_x509_signature(
    algorithm: &x509_cert::spki::AlgorithmIdentifierOwned,
    issuer: &x509_cert::spki::SubjectPublicKeyInfoOwned,
    message: &[u8], signature: &[u8],
) -> Result<CertificateSignature, Error> {
    use rsa::{pkcs1::{DecodeRsaPublicKey, RsaPssParams}, pss, signature::Verifier, traits::PublicKeyParts, RsaPublicKey};
    use x509_cert::der::{Decode, Encode, asn1::AnyRef};
    if algorithm.oid.to_string() != "1.2.840.113549.1.1.10" { return Ok(CertificateSignature::Unsupported); }
    let Some(encoded) = algorithm.parameters.as_ref().and_then(|p| p.to_der().ok()) else { return Ok(CertificateSignature::Unsupported) };
    let Ok(params) = RsaPssParams::from_der(&encoded) else { return Ok(CertificateSignature::Unsupported) };
    let sha256 = |id: &x509_cert::spki::AlgorithmIdentifierRef<'_>| id.oid.to_string() == "2.16.840.1.101.3.4.2.1" && id.parameters.is_none_or(|p| p == AnyRef::NULL);
    if !sha256(&params.hash) || params.mask_gen.oid.to_string() != "1.2.840.113549.1.1.8"
        || params.mask_gen.parameters.as_ref().is_none_or(|hash| !sha256(hash)) || params.salt_len != 32
        || issuer.algorithm.oid.to_string() != "1.2.840.113549.1.1.1"
        || issuer.algorithm.parameters.as_ref().is_some_and(|p| !p.to_der().is_ok_and(|v| v == [0x05, 0x00])) {
        return Ok(CertificateSignature::Unsupported);
    }
    let bytes = issuer.subject_public_key.as_bytes().ok_or_else(|| Error::new("Invalid RSA issuer public key"))?;
    let key = RsaPublicKey::from_pkcs1_der(bytes).map_err(|_| Error::new("Invalid RSA issuer public key"))?;
    if !(2048..=8192).contains(&key.n().bits()) { return Ok(CertificateSignature::Unsupported); }
    Ok(if pss::Signature::try_from(signature).is_ok_and(|sig| pss::VerifyingKey::<Sha256>::new_with_salt_len(key, 32).verify(message, &sig).is_ok()) { CertificateSignature::Matches } else { CertificateSignature::Differs })
}

/// Compare supported X.509 ECDSA signatures, encoded as DER (unlike XMLDSig).
pub(crate) fn inspect_ecdsa_x509_signature(
    algorithm: &x509_cert::spki::AlgorithmIdentifierOwned,
    issuer: &x509_cert::spki::SubjectPublicKeyInfoOwned,
    message: &[u8], signature: &[u8],
) -> Result<CertificateSignature, Error> {
    use p256::ecdsa::signature::Verifier;
    use x509_cert::der::asn1::ObjectIdentifier;
    if algorithm.parameters.is_some() || issuer.algorithm.oid.to_string() != "1.2.840.10045.2.1" { return Ok(CertificateSignature::Unsupported); }
    let curve = issuer.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok()).map(|oid| oid.to_string());
    let variant = match (algorithm.oid.to_string().as_str(), curve.as_deref()) {
        ("1.2.840.10045.4.3.2", Some("1.2.840.10045.3.1.7")) => 256,
        ("1.2.840.10045.4.3.3", Some("1.3.132.0.34")) => 384,
        _ => return Ok(CertificateSignature::Unsupported),
    };
    let bytes = issuer.subject_public_key.as_bytes().ok_or_else(|| Error::new("Invalid EC issuer public key"))?;
    let matches = if variant == 256 {
        let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(bytes).map_err(|_| Error::new("Invalid P-256 issuer public key"))?;
        p256::ecdsa::Signature::from_der(signature).is_ok_and(|sig| key.verify(message, &sig).is_ok())
    } else {
        let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(bytes).map_err(|_| Error::new("Invalid P-384 issuer public key"))?;
        p384::ecdsa::Signature::from_der(signature).is_ok_and(|sig| key.verify(message, &sig).is_ok())
    };
    Ok(if matches { CertificateSignature::Matches } else { CertificateSignature::Differs })
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
    pub scope_extensions: CrlScopeExtensions,
    pub authority_key: CrlAuthorityKeyFacts,
    /// Structural observation only; does not establish CRL applicability or trust.
    pub crl_number: CrlNumberObservation,
    /// A serial number match only. Meaning depends on scope, issuer and time.
    pub listed_revocation_time: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CrlTime { NotYetIssued, WithinWindow, Expired, NoNextUpdate }

#[derive(Debug, PartialEq, Eq)]
pub enum CrlScope { NoScopeExtensionsObserved, UnsupportedExtensions }

/// Untrusted syntax observations, not validation of profile constraints or CRL applicability.
#[derive(Debug, PartialEq, Eq)]
pub enum CrlExtensionObservation<T> {
    Absent,
    Malformed { critical: bool },
    Decoded { critical: bool, value: T },
}

#[derive(Debug, PartialEq, Eq)]
pub struct CrlScopeExtensions {
    /// The base CRL number is not compared with any other CRL.
    pub delta_indicator: CrlExtensionObservation<()>,
    pub issuing_distribution_point: CrlExtensionObservation<IssuingDistributionPointClaims>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct IssuingDistributionPointClaims {
    pub distribution_point_present: bool,
    pub only_user_certificates: bool,
    pub only_ca_certificates: bool,
    pub reasons_present: bool,
    pub indirect_crl: bool,
    pub only_attribute_certificates: bool,
}

/// Presence, criticality, and DER decoding of the CRL number extension.
/// No comparison with another CRL or issuer policy is performed.
#[derive(Debug, PartialEq, Eq)]
pub enum CrlNumberObservation { Missing, NonCritical, Critical, MalformedNonCritical, MalformedCritical }

/// Literal, unauthenticated extension observations; no path or issuer selection is performed.
#[derive(Debug, PartialEq, Eq)]
pub struct CrlAuthorityKeyFacts {
    pub crl_extension: IdentifierExtensionObservation,
    pub issuer_subject_key_extension: IdentifierExtensionObservation,
    pub key_identifiers: KeyIdentifierComparison,
    /// Presence only. These fields are not compared to the supplied certificate.
    pub authority_certificate_issuer_present: Option<bool>,
    pub authority_certificate_serial_present: Option<bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum IdentifierExtensionObservation { Missing, NonCritical, Critical, MalformedNonCritical, MalformedCritical }

#[derive(Debug, PartialEq, Eq)]
pub enum KeyIdentifierComparison { NotComparable, EmptyIdentifier, Matches, Differs }

/// Inspect a supplied CRL at an explicit time without network or trust lookup.
/// Delta, indirect, and scoped CRLs are not interpreted; an absent serial is
/// never presented as a positive certificate-status decision.
pub fn inspect_crl(crl_der: &[u8], cert_der: &[u8], issuer_der: &[u8], reference_unix_seconds: u64) -> Result<CrlFacts, Error> {
    use rsa::{pkcs1::DecodeRsaPublicKey, pkcs1v15, traits::PublicKeyParts, RsaPublicKey};
    use x509_cert::{crl::CertificateList, der::{Decode, Encode}, ext::pkix::{AuthorityKeyIdentifier, BaseCrlNumber, CrlNumber, IssuingDistributionPoint, KeyUsage, SubjectKeyIdentifier}, Certificate};
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
    let mut crl_number = CrlNumberObservation::Missing;
    let mut crl_authority_key = None;
    let mut crl_authority_key_state = IdentifierExtensionObservation::Missing;
    let mut scope_extensions = CrlScopeExtensions { delta_indicator: CrlExtensionObservation::Absent, issuing_distribution_point: CrlExtensionObservation::Absent };
    let mut extension_ids = std::collections::HashSet::new();
    for ext in crl.tbs_cert_list.crl_extensions.as_deref().unwrap_or(&[]) {
        let oid = ext.extn_id.to_string();
        if !extension_ids.insert(oid.clone()) { return Err(Error::new("Duplicate CRL extension")); }
        if oid == "2.5.29.20" {
            crl_number = match (ext.critical, CrlNumber::from_der(ext.extn_value.as_bytes()).is_ok()) {
                (false, true) => CrlNumberObservation::NonCritical,
                (true, true) => CrlNumberObservation::Critical,
                (false, false) => CrlNumberObservation::MalformedNonCritical,
                (true, false) => CrlNumberObservation::MalformedCritical,
            };
        }
        if oid == "2.5.29.35" {
            crl_authority_key = AuthorityKeyIdentifier::from_der(ext.extn_value.as_bytes()).ok();
            crl_authority_key_state = match (ext.critical, crl_authority_key.is_some()) {
                (false, true) => IdentifierExtensionObservation::NonCritical,
                (true, true) => IdentifierExtensionObservation::Critical,
                (false, false) => IdentifierExtensionObservation::MalformedNonCritical,
                (true, false) => IdentifierExtensionObservation::MalformedCritical,
            };
        }
        if oid == "2.5.29.27" {
            scope_extensions.delta_indicator = match BaseCrlNumber::from_der(ext.extn_value.as_bytes()) {
                Ok(_) => CrlExtensionObservation::Decoded { critical: ext.critical, value: () },
                Err(_) => CrlExtensionObservation::Malformed { critical: ext.critical },
            };
        }
        if oid == "2.5.29.28" {
            scope_extensions.issuing_distribution_point = match IssuingDistributionPoint::from_der(ext.extn_value.as_bytes()) {
                Ok(point) => CrlExtensionObservation::Decoded { critical: ext.critical, value: IssuingDistributionPointClaims {
                    distribution_point_present: point.distribution_point.is_some(),
                    only_user_certificates: point.only_contains_user_certs,
                    only_ca_certificates: point.only_contains_ca_certs,
                    reasons_present: point.only_some_reasons.is_some(),
                    indirect_crl: point.indirect_crl,
                    only_attribute_certificates: point.only_contains_attribute_certs,
                } },
                Err(_) => CrlExtensionObservation::Malformed { critical: ext.critical },
            };
        }
        if ext.critical || !matches!(oid.as_str(), "2.5.29.20" | "2.5.29.35") { scope = CrlScope::UnsupportedExtensions; }
    }
    let mut ski = None;
    let mut ski_state = IdentifierExtensionObservation::Missing;
    let mut ski_seen = false;
    for ext in issuer.tbs_certificate.extensions.as_deref().unwrap_or(&[]) {
        if ext.extn_id.to_string() != "2.5.29.14" { continue; }
        if ski_seen { return Err(Error::new("Duplicate issuer SubjectKeyIdentifier extension")); }
        ski_seen = true;
        ski = SubjectKeyIdentifier::from_der(ext.extn_value.as_bytes()).ok();
        ski_state = match (ext.critical, ski.is_some()) {
            (false, true) => IdentifierExtensionObservation::NonCritical,
            (true, true) => IdentifierExtensionObservation::Critical,
            (false, false) => IdentifierExtensionObservation::MalformedNonCritical,
            (true, false) => IdentifierExtensionObservation::MalformedCritical,
        };
    }
    let key_identifiers = match (crl_authority_key.as_ref().and_then(|a| a.key_identifier.as_ref()), ski.as_ref()) {
        (Some(a), Some(b)) if a.as_bytes().is_empty() || b.0.as_bytes().is_empty() => KeyIdentifierComparison::EmptyIdentifier,
        (Some(a), Some(b)) if a.as_bytes() == b.0.as_bytes() => KeyIdentifierComparison::Matches,
        (Some(_), Some(_)) => KeyIdentifierComparison::Differs,
        _ => KeyIdentifierComparison::NotComparable,
    };
    let authority_key = CrlAuthorityKeyFacts {
        crl_extension: crl_authority_key_state,
        issuer_subject_key_extension: ski_state,
        key_identifiers,
        authority_certificate_issuer_present: crl_authority_key.as_ref().map(|a| a.authority_cert_issuer.is_some()),
        authority_certificate_serial_present: crl_authority_key.as_ref().map(|a| a.authority_cert_serial_number.is_some()),
    };
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
    let supported = algorithm == &crl.tbs_cert_list.signature && rsa_x509_hash(&algorithm.oid.to_string()).is_some() && null_or_absent(&algorithm.parameters)
        && spki.algorithm.oid.to_string() == "1.2.840.113549.1.1.1" && null_or_absent(&spki.algorithm.parameters);
    let signature = if algorithm == &crl.tbs_cert_list.signature && algorithm.oid.to_string() == "1.2.840.113549.1.1.10" {
        let tbs = crl.tbs_cert_list.to_der().map_err(|_| Error::new("Invalid CRL body"))?;
        let raw = crl.signature.as_bytes().ok_or_else(|| Error::new("Invalid CRL signature"))?;
        inspect_rsa_pss_x509_signature(algorithm, spki, &tbs, raw)?
    } else if algorithm == &crl.tbs_cert_list.signature && matches!(algorithm.oid.to_string().as_str(), "1.2.840.10045.4.3.2" | "1.2.840.10045.4.3.3") {
        let tbs = crl.tbs_cert_list.to_der().map_err(|_| Error::new("Invalid CRL body"))?;
        let raw = crl.signature.as_bytes().ok_or_else(|| Error::new("Invalid CRL signature"))?;
        inspect_ecdsa_x509_signature(algorithm, spki, &tbs, raw)?
    } else if !supported { CertificateSignature::Unsupported } else {
        let key_bytes = spki.subject_public_key.as_bytes().ok_or_else(|| Error::new("Invalid CRL issuer public key"))?;
        let key = RsaPublicKey::from_pkcs1_der(key_bytes).map_err(|_| Error::new("Invalid CRL issuer RSA key"))?;
        if !(2048..=8192).contains(&key.n().bits()) { CertificateSignature::Unsupported } else {
            let tbs = crl.tbs_cert_list.to_der().map_err(|_| Error::new("Invalid CRL body"))?;
            let raw = crl.signature.as_bytes().ok_or_else(|| Error::new("Invalid CRL signature"))?;
            match pkcs1v15::Signature::try_from(raw) {
                Ok(sig) if verify_rsa_sha2(rsa_x509_hash(&algorithm.oid.to_string()).unwrap(), key, &tbs, &sig) => CertificateSignature::Matches,
                _ => CertificateSignature::Differs,
            }
        }
    };
    Ok(CrlFacts { issuer_name_exact: crl.tbs_cert_list.issuer == issuer.tbs_certificate.subject,
        certificate_issuer_name_exact: cert.tbs_certificate.issuer == crl.tbs_cert_list.issuer,
        issuer_crl_sign: issuer_usage, signature, this_update: start, next_update: end, time_status, scope, scope_extensions, authority_key, crl_number, listed_revocation_time: listed })
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
/// This checks RSA PKCS#1 SHA-256/384/512, a fixed RSA-PSS SHA-256 subset,
/// and ECDSA P-256/SHA-256 or P-384/SHA-384. It does not evaluate
/// time, revocation, policies, name constraints, anchor trust, or a path.
pub fn inspect_certificate_link(child_der: &[u8], issuer_der: &[u8]) -> Result<CertificateLinkFacts, Error> {
    use rsa::{pkcs1::DecodeRsaPublicKey, pkcs1v15, traits::PublicKeyParts, RsaPublicKey};
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
        && rsa_x509_hash(&algorithm.oid.to_string()).is_some() && null_or_absent(&algorithm.parameters)
        && spki.algorithm.oid.to_string() == "1.2.840.113549.1.1.1" && null_or_absent(&spki.algorithm.parameters);
    let child_signature = if algorithm == &child.tbs_certificate.signature && algorithm.oid.to_string() == "1.2.840.113549.1.1.10" {
        let tbs = child.tbs_certificate.to_der().map_err(|_| Error::new("Invalid certificate body"))?;
        let raw = child.signature.as_bytes().ok_or_else(|| Error::new("Invalid certificate signature"))?;
        inspect_rsa_pss_x509_signature(algorithm, spki, &tbs, raw)?
    } else if algorithm == &child.tbs_certificate.signature && matches!(algorithm.oid.to_string().as_str(), "1.2.840.10045.4.3.2" | "1.2.840.10045.4.3.3") {
        let tbs = child.tbs_certificate.to_der().map_err(|_| Error::new("Invalid certificate body"))?;
        let raw = child.signature.as_bytes().ok_or_else(|| Error::new("Invalid certificate signature"))?;
        inspect_ecdsa_x509_signature(algorithm, spki, &tbs, raw)?
    } else if !supported { CertificateSignature::Unsupported } else {
        let key_bytes = spki.subject_public_key.as_bytes().ok_or_else(|| Error::new("Invalid issuer public key"))?;
        let key = RsaPublicKey::from_pkcs1_der(key_bytes).map_err(|_| Error::new("Invalid issuer RSA key"))?;
        if !(2048..=8192).contains(&key.n().bits()) { CertificateSignature::Unsupported } else {
            let tbs = child.tbs_certificate.to_der().map_err(|_| Error::new("Invalid certificate body"))?;
            let raw = child.signature.as_bytes().ok_or_else(|| Error::new("Invalid certificate signature"))?;
            match pkcs1v15::Signature::try_from(raw) {
                Ok(sig) if verify_rsa_sha2(rsa_x509_hash(&algorithm.oid.to_string()).unwrap(), key, &tbs, &sig) => CertificateSignature::Matches,
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

/// Read untrusted AIA and CRL distribution-point hints from a DER certificate.
/// This does not fetch URLs, validate the certificate, or determine revocation.
pub fn inspect_certificate_location_hints(certificate_der: &[u8]) -> Result<CertificateLocationHints, Error> {
    use x509_cert::{Certificate, der::Decode, ext::pkix::{AuthorityInfoAccessSyntax, CrlDistributionPoints, name::DistributionPointName}};
    if certificate_der.len() > 1024 * 1024 { return Err(Error::new("Certificate is too large")); }
    let cert = Certificate::from_der(certificate_der).map_err(|_| Error::new("Invalid certificate"))?;
    let mut hints = CertificateLocationHints { aia: LocationExtension::Absent, crl_distribution_points: LocationExtension::Absent, ocsp: LocationList::default(), ca_issuers: LocationList::default(), crl: LocationList::default(), other_aia_methods: 0, scoped_crl_points: 0, alternate_crl_issuer_points: 0, other_crl_points: 0 };
    match cert.tbs_certificate.get::<AuthorityInfoAccessSyntax>() {
        Ok(Some((critical, access))) if !access.0.is_empty() => {
            hints.aia = LocationExtension::Parsed { critical };
            for item in &access.0 {
                match item.access_method.to_string().as_str() {
                    "1.3.6.1.5.5.7.48.1" => hints.ocsp.record(&item.access_location),
                    "1.3.6.1.5.5.7.48.2" => hints.ca_issuers.record(&item.access_location),
                    _ => hints.other_aia_methods += 1,
                }
            }
        }
        Ok(None) => (),
        _ => hints.aia = LocationExtension::Unsupported,
    }
    match cert.tbs_certificate.get::<CrlDistributionPoints>() {
        Ok(Some((critical, points))) if !points.0.is_empty() => {
            hints.crl_distribution_points = LocationExtension::Parsed { critical };
            for point in &points.0 {
                if point.reasons.is_some() { hints.scoped_crl_points += 1; }
                if point.crl_issuer.is_some() { hints.alternate_crl_issuer_points += 1; }
                match &point.distribution_point {
                    Some(DistributionPointName::FullName(names)) if !names.is_empty() => {
                        for name in names { hints.crl.record(name); }
                    }
                    _ => hints.other_crl_points += 1,
                }
            }
        }
        Ok(None) => (),
        _ => hints.crl_distribution_points = LocationExtension::Unsupported,
    }
    Ok(hints)
}

/// Inspect one signature linked from the package. Unregistered parts are never
/// treated as package signatures. Observations are not a trust decision.
pub fn inspect_signature(package: &Package, signature_part: &str) -> Result<SignatureInspection, Error> {
    if !signature_parts(package)?.iter().any(|part| part == signature_part) {
        return Err(Error::new("Signature part is not registered"));
    }
    inspect_discovered_signature(package, signature_part)
}

/// Inspect the signatures linked from the package. The resulting observations
/// do not constitute signature verification or a trust decision.
pub fn inspect_signatures(package: &Package) -> Result<Vec<SignatureInspection>, Error> {
    signature_parts(package)?.iter().map(|part| inspect_discovered_signature(package, part)).collect()
}

fn inspect_discovered_signature(package: &Package, part: &str) -> Result<SignatureInspection, Error> {
    let binding = inspect_package_object_binding(package, part)?;
    let object_id = match binding {
        PackageObjectBinding::Declared { object_id } => object_id,
        PackageObjectBinding::Unsupported => return Err(Error::new("Unsupported package object binding")),
    };
    let manifest = inspect_manifest_digests(package, part)?;
    let manifest_inventory = inspect_manifest_inventory(package, &manifest);
    Ok(SignatureInspection {
        signature_value: inspect_signature_value(package, part)?,
        certificate_locations: extract_single_embedded_certificate(package, part).ok().and_then(|der| inspect_certificate_location_hints(&der).ok()),
        certificate_policies: extract_single_embedded_certificate(package, part).ok().and_then(|der| inspect_certificate_policies(&der).ok()),
        package_object: inspect_package_object_digest(package, part)?,
        additional_signed_info_references: additional_signed_info_references(package, part, &object_id)?,
        manifest,
        manifest_inventory,
        trust: TrustCheck::NotEvaluated,
        signature_part: part.to_string(),
    })
}

/// Inventory current package entry names against recognizable manifest URIs. An
/// entry listed here may be metadata, may be covered in another way, or may
/// not affect the displayed document; this function makes no coverage verdict.
fn inspect_manifest_inventory(package: &Package, manifest: &[ManifestReference]) -> ManifestInventory {
    use std::collections::HashSet;
    fn simple_path(path: &str) -> bool {
        !path.is_empty() && !path.starts_with('/') && !path.contains(['%', '\\', '#', ':', '?'])
            && !path.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..")
    }
    let mut paths = HashSet::new();
    let mut unresolved_references = 0usize;
    for reference in manifest {
        let parsed = reference.uri.strip_prefix('/').and_then(|uri| uri.split_once("?ContentType="));
        match parsed {
            Some((path, content_type)) if simple_path(path) && !content_type.is_empty() && !content_type.contains(['%', '&', '#', '?']) => { paths.insert(path); }
            _ => unresolved_references += 1,
        }
    }
    let mut inventory = ManifestInventory { entries: 0, unlisted: 0, ambiguous_entries: 0, unresolved_references, unlisted_examples: Vec::new(), examples_truncated: false };
    for name in package.names().filter(|name| !name.ends_with('/')) {
        inventory.entries += 1;
        if !simple_path(name) { inventory.ambiguous_entries += 1; continue; }
        if !paths.contains(name) {
            inventory.unlisted += 1;
            if inventory.unlisted_examples.len() < 16 {
                let mut chars = name.chars();
                let mut example: String = chars.by_ref().take(200).collect();
                if chars.next().is_some() { example.push('…'); inventory.examples_truncated = true; }
                inventory.unlisted_examples.push(example);
            } else { inventory.examples_truncated = true; }
        }
    }
    inventory
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
    /// ECDSA comparison against an untrusted P-384 certificate.
    EcdsaP384 { matches: bool },
    /// RSA-PSS SHA-256 comparison against an untrusted certificate.
    RsaPssSha256 { matches: bool },
    MatchesUntrustedCertificate,
    Differs,
    Unsupported,
}

fn ds_children<'a, 'input>(parent: ua_cryptography_xml_c14n::roxmltree::Node<'a, 'input>, name: &str) -> Vec<ua_cryptography_xml_c14n::roxmltree::Node<'a, 'input>> {
    parent.children().filter(|n| n.is_element() && n.tag_name().namespace() == Some(DS) && n.tag_name().name() == name).collect()
}

/// Accept only the RFC 9231 SHA-256 / MGF1-SHA256 / salt 32 / trailer 1
/// subset, including its XML defaults. Unknown or ambiguous markup is refused.
fn supported_parameterized_xml_pss(method: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>) -> bool {
    const PSS: &str = "http://www.w3.org/2007/05/xmldsig-more#";
    const SHA256_URI: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
    fn whitespace_or_element(node: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>) -> bool {
        node.children().all(|n| n.is_element() || (n.is_text() && n.text().is_none_or(|t| t.trim_matches([' ', '\t', '\n', '\r']).is_empty())))
    }
    fn attrs_only_algorithm(node: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>) -> bool {
        node.attributes().all(|a| a.namespace().is_none() && a.name() == "Algorithm")
    }
    fn digest_is_sha256(node: ua_cryptography_xml_c14n::roxmltree::Node<'_, '_>) -> bool {
        node.tag_name().namespace() == Some(DS) && node.tag_name().name() == "DigestMethod"
            && attrs_only_algorithm(node) && node.attribute("Algorithm") == Some(SHA256_URI)
            && !node.children().any(|n| n.is_element()) && whitespace_or_element(node)
    }
    if !attrs_only_algorithm(method) || !whitespace_or_element(method) { return false; }
    let nodes: Vec<_> = method.children().filter(|n| n.is_element()).collect();
    if nodes.is_empty() { return true; }
    let [params] = nodes.as_slice() else { return false };
    if params.tag_name().namespace() != Some(PSS) || params.tag_name().name() != "RSAPSSParams"
        || params.attributes().len() != 0 || !whitespace_or_element(*params) { return false; }
    let mut previous = 0u8;
    for field in params.children().filter(|n| n.is_element()) {
        let position = match (field.tag_name().namespace(), field.tag_name().name()) {
            (Some(DS), "DigestMethod") => { if !digest_is_sha256(field) { return false; } 1 }
            (Some(PSS), "MaskGenerationFunction") => {
                if !attrs_only_algorithm(field) || field.attribute("Algorithm").is_some_and(|a| a != "http://www.w3.org/2007/05/xmldsig-more#MGF1") || !whitespace_or_element(field) { return false; }
                let children: Vec<_> = field.children().filter(|n| n.is_element()).collect();
                if children.len() > 1 || children.first().is_some_and(|n| !digest_is_sha256(*n)) { return false; }
                2
            }
            (Some(PSS), "SaltLength") | (Some(PSS), "TrailerField") => {
                if field.attributes().len() != 0 || field.children().any(|n| !n.is_text()) { return false; }
                let value: String = field.children().filter_map(|n| n.text()).collect();
                let value = value.trim_matches([' ', '\t', '\n', '\r']);
                if field.tag_name().name() == "SaltLength" { if value != "32" { return false; } 3 }
                else { if value != "1" { return false; } 4 }
            }
            _ => return false,
        };
        if position <= previous { return false; }
        previous = position;
    }
    true
}

/// Compare RSA PKCS#1 SHA-256/384/512, RSA-PSS SHA-256, legacy RSA-SHA1, or ECDSA P-256/SHA-256 and P-384/SHA-384 over SignedInfo against exactly one
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
        || c14n[0].children().any(|n| n.is_element()) {
        return Ok(SignatureValueCheck::Unsupported);
    }
    if method[0].attribute("Algorithm") == Some("http://www.w3.org/2007/05/xmldsig-more#rsa-pss") {
        if !supported_parameterized_xml_pss(method[0]) { return Ok(SignatureValueCheck::Unsupported); }
    } else if method[0].children().any(|n| n.is_element()) {
        return Ok(SignatureValueCheck::Unsupported);
    }
    let (legacy, ecdsa_bits, pss, rsa_hash) = match method[0].attribute("Algorithm") {
        Some("http://www.w3.org/2000/09/xmldsig#rsa-sha1") => (true, 0, false, ""),
        Some("http://www.w3.org/2001/04/xmldsig-more#rsa-sha256") => (false, 0, false, "sha256"),
        Some("http://www.w3.org/2001/04/xmldsig-more#rsa-sha384") => (false, 0, false, "sha384"),
        Some("http://www.w3.org/2001/04/xmldsig-more#rsa-sha512") => (false, 0, false, "sha512"),
        Some("http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256") => (false, 256, false, ""),
        Some("http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha384") => (false, 384, false, ""),
        Some("http://www.w3.org/2007/05/xmldsig-more#sha256-rsa-MGF1") | Some("http://www.w3.org/2007/05/xmldsig-more#rsa-pss") => (false, 0, true, ""),
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
    if ecdsa_bits != 0 {
        use x509_cert::der::asn1::ObjectIdentifier;
        let curve = spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok()).map(|oid| oid.to_string());
        let expected_curve = if ecdsa_bits == 256 { "1.2.840.10045.3.1.7" } else { "1.3.132.0.34" };
        if spki.algorithm.oid.to_string() != "1.2.840.10045.2.1" || curve.as_deref() != Some(expected_curve) { return Ok(SignatureValueCheck::Unsupported); }
        let raw_signature = decode(values[0])?;
        let canonical = canonicalize(infos[0], Algorithm::Inclusive, &[], None);
        if ecdsa_bits == 256 {
            let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(public_key).map_err(|_| Error::new("Invalid P-256 public key"))?;
            let matches = p256::ecdsa::Signature::from_slice(&raw_signature).is_ok_and(|signature| key.verify(&canonical, &signature).is_ok());
            return Ok(SignatureValueCheck::EcdsaP256 { matches });
        }
        let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(public_key).map_err(|_| Error::new("Invalid P-384 public key"))?;
        let matches = p384::ecdsa::Signature::from_slice(&raw_signature).is_ok_and(|signature| key.verify(&canonical, &signature).is_ok());
        return Ok(SignatureValueCheck::EcdsaP384 { matches });
    }
    if spki.algorithm.oid.to_string() != "1.2.840.113549.1.1.1" { return Ok(SignatureValueCheck::Unsupported); }
    let key = RsaPublicKey::from_pkcs1_der(public_key).map_err(|_| Error::new("Invalid RSA public key"))?;
    let key_bits = key.n().bits();
    if !((if legacy { 1024 } else { 2048 })..=8192).contains(&key_bits) { return Ok(SignatureValueCheck::Unsupported); }
    let raw_signature = decode(values[0])?;
    let canonical = canonicalize(infos[0], Algorithm::Inclusive, &[], None);
    if pss {
        let matches = rsa::pss::Signature::try_from(raw_signature.as_slice()).is_ok_and(|signature| rsa::pss::VerifyingKey::<Sha256>::new_with_salt_len(key, 32).verify(&canonical, &signature).is_ok());
        return Ok(SignatureValueCheck::RsaPssSha256 { matches });
    }
    let signature = pkcs1v15::Signature::try_from(raw_signature.as_slice());
    if legacy {
        let matches = signature.is_ok_and(|signature| pkcs1v15::VerifyingKey::<sha1::Sha1>::new(key).verify(&canonical, &signature).is_ok());
        Ok(SignatureValueCheck::LegacySha1 { key_bits, matches })
    } else {
        let matches = signature.is_ok_and(|signature| verify_rsa_sha2(rsa_hash, key, &canonical, &signature));
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
                let valid_shape = match children.as_slice() {
                    [method, value] => *method == methods[0] && *value == values[0],
                    [transform, method, value] => transforms.len() == 1 && *transform == transforms[0] && *method == methods[0] && *value == values[0],
                    _ => false,
                };
                let allowed_attrs = |node, allowed: &[&str]| {
                    let mut seen = std::collections::HashSet::new();
                    doc.attributes(node).iter().all(|a| seen.insert(a.name.as_str()) && (is_namespace_attr(&a.name) || allowed.contains(&a.name.as_str())))
                };
                let valid_fields = valid_shape && allowed_attrs(methods[0], &["Algorithm"]) && allowed_attrs(values[0], &[])
                    && doc.elements(methods[0]).next().is_none() && doc.elements(values[0]).next().is_none();
                let mut relationships = path.filter(|path| path.ends_with(".rels")).map(|_| RelationshipReference::Unsupported);
                let digest = if !valid_fields || transforms.len() > 1 || path.is_none_or(|p| p.is_empty() || p.contains(['%', '\\', '#', ':']) || p.split('/').any(|s| s.is_empty() || s == "." || s == "..")) {
                    ManifestDigest::Unsupported
                } else {
                    let path = path.unwrap();
                    if !package.contains(path) { ManifestDigest::MissingPart } else {
                        let bytes = if transforms.is_empty() {
                            if relationships.is_some() { relationships = Some(RelationshipReference::WholePart); }
                            Some(package.read(path)?)
                        } else {
                            match relationship_transform(package, path, &doc, transforms[0])? {
                                Some(transformed) => {
                                    relationships = Some(RelationshipReference::Filtered(transformed.selection));
                                    Some(transformed.bytes)
                                }
                                None => None,
                            }
                        };
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
                result.push(ManifestReference { uri, digest, content_type, relationships });
            }
        }
    }
    Ok(result)
}

// Implements only the strict, namespace-clean subset of OPC Relationship
// Transform followed by inclusive C14N 1.0. Markup Compatibility and other
// extensions are rejected rather than silently transformed incorrectly.
struct RelationshipTransformResult {
    bytes: Vec<u8>,
    selection: RelationshipSelection,
}

fn relationship_transform(package: &Package, path: &str, signature: &crate::XmlDocument, transforms: crate::NodeId) -> Result<Option<RelationshipTransformResult>, Error> {
    use std::collections::HashSet;
    if !path.ends_with(".rels") { return Ok(None) }
    let attrs_allowed = |doc: &crate::XmlDocument, node: crate::NodeId, allowed: &[&str]| {
        let mut seen = HashSet::new();
        doc.attributes(node).iter().all(|a| seen.insert(a.name.as_str()) && (is_namespace_attr(&a.name) || allowed.contains(&a.name.as_str())))
    };
    let content_allowed = |doc: &crate::XmlDocument, node: crate::NodeId| {
        doc.children(node).iter().all(|&n| match doc.kind(n) {
            crate::xml::NodeKind::Element | crate::xml::NodeKind::Comment => true,
            crate::xml::NodeKind::Text | crate::xml::NodeKind::CData => doc.text(n).trim_matches([' ', '\t', '\n', '\r']).is_empty(),
            _ => false,
        })
    };
    let steps: Vec<_> = signature.elements(transforms).collect();
    if !attrs_allowed(signature, transforms, &[]) || !content_allowed(signature, transforms)
        || steps.len() != 2 || steps.iter().any(|&n| signature.local_name(n) != "Transform" || signature.namespace(n) != Some(DS) || !attrs_allowed(signature, n, &["Algorithm"]) || !content_allowed(signature, n))
        || !matches!(signature.attr(steps[0], "Algorithm"), Some(REL_TRANSFORM | REL_TRANSFORM_2006)) || signature.attr(steps[1], "Algorithm") != Some(C14N)
        || signature.elements(steps[1]).next().is_some() { return Ok(None) }
    let mut ids = HashSet::new();
    let mut types = HashSet::new();
    for selector in signature.elements(steps[0]) {
        if signature.namespace(selector) != Some(OPC_DS) || signature.elements(selector).next().is_some() || !content_allowed(signature, selector) { return Ok(None) }
        match signature.local_name(selector) {
            "RelationshipReference" if attrs_allowed(signature, selector, &["SourceId"]) => match signature.attr(selector, "SourceId") { Some(s) if !s.is_empty() => { ids.insert(s); }, _ => return Ok(None) },
            "RelationshipsGroupReference" if attrs_allowed(signature, selector, &["SourceType"]) => match signature.attr(selector, "SourceType") { Some(s) if !s.is_empty() => { types.insert(s); }, _ => return Ok(None) },
            _ => return Ok(None),
        }
    }
    if ids.is_empty() && types.is_empty() { return Ok(None) }
    let rels = package.xml(path)?;
    let root = rels.root_element().ok_or_else(|| Error::new("Empty relationships part"))?;
    if rels.local_name(root) != "Relationships" || rels.namespace(root) != Some(REL_NS)
        || !attrs_allowed(&rels, root, &[]) || !content_allowed(&rels, root)
        || rels.children(crate::XmlDocument::ROOT).iter().any(|&n| matches!(rels.kind(n), crate::xml::NodeKind::Doctype | crate::xml::NodeKind::ProcessingInstruction)) {
        return Ok(None);
    }
    let mut seen = HashSet::new();
    let mut matched_types = HashSet::new();
    let mut selected = Vec::new();
    let mut selection = RelationshipSelection { selected: 0, unselected: 0, unmatched_id_selectors: 0, unmatched_type_selectors: 0, selected_examples: Vec::new(), unselected_examples: Vec::new(), examples_truncated: false };
    for n in rels.elements(root) {
        if rels.local_name(n) != "Relationship" || rels.namespace(n) != Some(REL_NS)
            || !attrs_allowed(&rels, n, &["Id", "Type", "Target", "TargetMode"]) || rels.elements(n).next().is_some() || !content_allowed(&rels, n) { return Ok(None) }
        let (Some(id), Some(kind), Some(target)) = (rels.attr(n, "Id"), rels.attr(n, "Type"), rels.attr(n, "Target")) else { return Ok(None) };
        if id.is_empty() || kind.is_empty() || target.is_empty() || !seen.insert(id) { return Ok(None) }
        let mode = rels.attr(n, "TargetMode").unwrap_or("Internal");
        if mode != "Internal" && mode != "External" { return Ok(None) }
        if types.contains(kind) { matched_types.insert(kind); }
        let is_selected = ids.contains(id) || types.contains(kind);
        let (count, examples) = if is_selected { (&mut selection.selected, &mut selection.selected_examples) } else { (&mut selection.unselected, &mut selection.unselected_examples) };
        *count += 1;
        if examples.len() < 8 {
            let mut chars = id.chars(); let mut value: String = chars.by_ref().take(200).collect();
            if chars.next().is_some() { value.push('…'); selection.examples_truncated = true; }
            examples.push(value);
        } else { selection.examples_truncated = true; }
        if is_selected { selected.push((id, kind, target, mode)); }
    }
    selection.unmatched_id_selectors = ids.difference(&seen).count();
    selection.unmatched_type_selectors = types.difference(&matched_types).count();
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
    Ok(Some(RelationshipTransformResult { bytes: out.into_bytes(), selection }))
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
        let references = inspect_manifest_digests(&pkg, "sig.xml").unwrap();
        assert_eq!(references[0].digest, ManifestDigest::Matches);
        let Some(RelationshipReference::Filtered(selection)) = &references[0].relationships else { panic!("expected filtered relationships") };
        assert_eq!((selection.selected, selection.unselected), (2, 1));
        assert_eq!(selection.unmatched_id_selectors, 0);
        assert_eq!(selection.unmatched_type_selectors, 0);
        assert_eq!(selection.selected_examples, ["b", "a"]);
        assert_eq!(selection.unselected_examples, ["z"]);
        let untouched = format!(r#"<r:Relationships xmlns:r="{REL_NS}"><r:Relationship Id="z" Type="kind" Target="changed"/><r:Relationship Target="b&amp;c" Type="wanted" Id="b"/><r:Relationship Target="a" Id="a" Type="one" TargetMode="External"/></r:Relationships>"#);
        pkg.set_bytes("word/_rels/document.xml.rels", untouched.into_bytes());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::Matches);
        let extra = xml.replace("<p:RelationshipReference SourceId=\"a\"/>", "<p:RelationshipReference SourceId=\"a\"/><p:RelationshipReference SourceId=\"missing\"/><p:RelationshipsGroupReference SourceType=\"missing-type\"/>");
        pkg.set_bytes("sig.xml", extra.into_bytes());
        let references = inspect_manifest_digests(&pkg, "sig.xml").unwrap();
        assert_eq!(references[0].digest, ManifestDigest::Matches);
        let Some(RelationshipReference::Filtered(selection)) = &references[0].relationships else { panic!("expected filtered relationships") };
        assert_eq!((selection.unmatched_id_selectors, selection.unmatched_type_selectors), (1, 1));
        pkg.set_bytes("sig.xml", xml.replace(REL_TRANSFORM, REL_TRANSFORM_2006).into_bytes());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::Matches);
        let changed = format!(r#"<Relationships xmlns="{REL_NS}"><Relationship Id="b" Type="wanted" Target="changed"/><Relationship Id="a" Type="one" Target="a" TargetMode="External"/></Relationships>"#);
        pkg.set_bytes("word/_rels/document.xml.rels", changed.into_bytes());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].digest, ManifestDigest::Differs);
    }

    #[test]
    fn relationship_selection_rejects_ambiguous_and_unknown_markup() {
        let mut pkg = package(&rels(""), None);
        pkg.set_bytes("r.rels", format!(r#"<Relationships xmlns="{REL_NS}"><Relationship Id="a" Type="one" Target="a"/></Relationships>"#).into_bytes());
        let xml = format!(r#"<Signature xmlns="{DS}" xmlns:p="{OPC_DS}"><Object><Manifest><Reference URI="/r.rels?ContentType=application/xml"><Transforms><Transform Algorithm="{REL_TRANSFORM}"><p:RelationshipReference SourceId="a"/></Transform><Transform Algorithm="{C14N}"/></Transforms><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>YQ==</DigestValue></Reference></Manifest></Object></Signature>"#);
        for bad in [
            xml.replace("SourceId=\"a\"", "SourceId=\"a\" unknown=\"x\""),
            xml.replace("SourceId=\"a\"", "SourceId=\"a\" SourceId=\"a\""),
            xml.replace(&format!("Algorithm=\"{REL_TRANSFORM}\""), &format!("Algorithm=\"{REL_TRANSFORM}\" unknown=\"x\"")),
            xml.replace("<p:RelationshipReference", "unexpected<p:RelationshipReference"),
            xml.replace("<DigestMethod", "<Unknown/><DigestMethod"),
            xml.replace("<DigestValue>YQ==</DigestValue>", "<DigestValue><Unexpected>YQ==</Unexpected></DigestValue>"),
            xml.replace("<DigestValue>YQ==</DigestValue>", "<DigestValue unexpected=\"x\">YQ==</DigestValue>"),
        ] {
            pkg.set_bytes("sig.xml", bad.into_bytes());
            match inspect_manifest_digests(&pkg, "sig.xml") {
                Ok(refs) => { assert_eq!(refs[0].digest, ManifestDigest::Unsupported); assert_eq!(refs[0].relationships, Some(RelationshipReference::Unsupported)); }
                Err(_) => (),
            }
        }
        let raw = xml.replace(&format!("<Transforms><Transform Algorithm=\"{REL_TRANSFORM}\"><p:RelationshipReference SourceId=\"a\"/></Transform><Transform Algorithm=\"{C14N}\"/></Transforms>"), "");
        assert_ne!(raw, xml);
        pkg.set_bytes("sig.xml", raw.into_bytes());
        assert_eq!(inspect_manifest_digests(&pkg, "sig.xml").unwrap()[0].relationships, Some(RelationshipReference::WholePart));
    }

    #[test]
    fn relationship_selection_examples_are_bounded() {
        let mut pkg = package(&rels(""), None);
        let long_id = "a".repeat(250);
        let mut records = format!(r#"<Relationship Id="{long_id}" Type="one" Target="a"/>"#);
        for i in 0..12 { records.push_str(&format!(r#"<Relationship Id="r{i}" Type="other" Target="b"/>"#)); }
        pkg.set_bytes("r.rels", format!(r#"<Relationships xmlns="{REL_NS}">{records}</Relationships>"#).into_bytes());
        let xml = format!(r#"<Signature xmlns="{DS}" xmlns:p="{OPC_DS}"><Object><Manifest><Reference URI="/r.rels?ContentType=application/xml"><Transforms><Transform Algorithm="{REL_TRANSFORM}"><p:RelationshipReference SourceId="{long_id}"/></Transform><Transform Algorithm="{C14N}"/></Transforms><DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><DigestValue>YQ==</DigestValue></Reference></Manifest></Object></Signature>"#);
        pkg.set_bytes("sig.xml", xml.into_bytes());
        let references = inspect_manifest_digests(&pkg, "sig.xml").unwrap();
        let Some(RelationshipReference::Filtered(selection)) = &references[0].relationships else { panic!("expected filtered relationships") };
        assert_eq!((selection.selected, selection.unselected), (1, 12));
        assert_eq!(selection.selected_examples[0].chars().count(), 201);
        assert!(selection.selected_examples[0].ends_with('…'));
        assert_eq!(selection.unselected_examples.len(), 8);
        assert!(selection.examples_truncated);
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
    fn single_signature_inspection_requires_registration_and_is_independent() {
        let data = include_bytes!("../tests/fixtures/signatures/synthetic-signed.docx");
        let mut pkg = Package::open(data.as_slice()).unwrap();
        let part = "_xmlsignatures/sig1.xml";
        let report = inspect_signature(&pkg, part).unwrap();
        assert_eq!(inspect_signatures(&pkg).unwrap(), vec![report]);
        pkg.set_bytes("_xmlsignatures/sig2.xml", b"not a signature".to_vec());
        assert!(inspect_signature(&pkg, "_xmlsignatures/sig2.xml").is_err());
        let rels = "_xmlsignatures/_rels/origin.sigs.rels";
        let text = String::from_utf8(pkg.read(rels).unwrap()).unwrap();
        let extra = format!(r#"<Relationship Id="r2" Type="{SIGNATURE}" Target="sig2.xml"/>"#);
        let changed = text.replace("</Relationships>", &format!("{extra}</Relationships>"));
        assert_ne!(text, changed);
        pkg.set_bytes(rels, changed.into_bytes());
        assert!(inspect_signature(&pkg, part).is_ok());
        assert!(inspect_signature(&pkg, "_xmlsignatures/sig2.xml").is_err());
        assert!(inspect_signatures(&pkg).is_err());
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
    fn rsa_pss_requires_sha256_mgf1_and_fixed_salt_length() {
        let bytes = include_bytes!("../tests/fixtures/signatures/rsa-pss-sha256-signed.docx");
        let mut pkg = Package::open(bytes.to_vec()).unwrap();
        let part = "_xmlsignatures/sig1.xml";
        let xml = String::from_utf8(pkg.read(part).unwrap()).unwrap();
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::RsaPssSha256 { matches: true });
        let doc = ua_cryptography_xml_c14n::DocumentText::decode(xml.as_bytes()).unwrap();
        let parsed = doc.parse().unwrap();
        let value = ds_children(parsed.root_element(), "SignatureValue")[0].text().unwrap();
        for signature in [include_bytes!("../tests/fixtures/signatures/rsa-pss-salt16.sig").as_slice(), include_bytes!("../tests/fixtures/signatures/rsa-pss-mgf384.sig").as_slice()] {
            pkg.set_bytes(part, xml.replace(value, &crate::base64::encode(signature)).into_bytes());
            assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::RsaPssSha256 { matches: false });
        }
        pkg.set_bytes(part, xml.replace("#package", "#changed").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::RsaPssSha256 { matches: false });
        let cert = extract_single_embedded_certificate(&pkg, part).unwrap();
        let ec_cert = include_bytes!("../tests/fixtures/signatures/ecdsa-p384-cert.der");
        pkg.set_bytes(part, xml.replace(&crate::base64::encode(&cert), &crate::base64::encode(ec_cert)).into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::Unsupported);
        pkg.set_bytes(part, xml.replace("sha256-rsa-MGF1", "rsa-pss").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::RsaPssSha256 { matches: false });
        let with_params = xml.replace("sha256-rsa-MGF1\"/>", "sha256-rsa-MGF1\"><p:RSAPSSParams xmlns:p=\"http://www.w3.org/2007/05/xmldsig-more#\"/></ds:SignatureMethod>");
        assert_ne!(xml, with_params);
        pkg.set_bytes(part, with_params.into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::Unsupported);
    }

    #[test]
    fn parameterized_xml_pss_accepts_only_unambiguous_supported_parameters() {
        use ua_cryptography_xml_c14n::DocumentText;
        let check = |body: &str| {
            let xml = format!(r#"<ds:SignatureMethod xmlns:ds="{DS}" xmlns:p="http://www.w3.org/2007/05/xmldsig-more#" Algorithm="http://www.w3.org/2007/05/xmldsig-more#rsa-pss">{body}</ds:SignatureMethod>"#);
            let source = DocumentText::decode(xml.as_bytes()).unwrap();
            let doc = source.parse().unwrap();
            supported_parameterized_xml_pss(doc.root_element())
        };
        let digest = r#"<ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>"#;
        assert!(check(""));
        assert!(check("<p:RSAPSSParams/>"));
        assert!(check("<p:RSAPSSParams><p:MaskGenerationFunction/></p:RSAPSSParams>"));
        assert!(check(&format!(r#"<p:RSAPSSParams>{digest}<p:MaskGenerationFunction Algorithm="http://www.w3.org/2007/05/xmldsig-more#MGF1">{digest}</p:MaskGenerationFunction><p:SaltLength> 32 </p:SaltLength><p:TrailerField>1</p:TrailerField></p:RSAPSSParams>"#)));
        for body in [
            "<p:RSAPSSParams/><p:RSAPSSParams/>".to_string(),
            "<p:RSAPSSParams unexpected=\"x\"/>".to_string(),
            "<p:RSAPSSParams><p:SaltLength>16</p:SaltLength></p:RSAPSSParams>".to_string(),
            "<p:RSAPSSParams><p:TrailerField>2</p:TrailerField></p:RSAPSSParams>".to_string(),
            "<p:RSAPSSParams><p:SaltLength>32</p:SaltLength><p:SaltLength>32</p:SaltLength></p:RSAPSSParams>".to_string(),
            format!("<p:RSAPSSParams><p:SaltLength>32</p:SaltLength>{digest}</p:RSAPSSParams>"),
            "<p:RSAPSSParams><p:MaskGenerationFunction Algorithm=\"urn:unknown\"/></p:RSAPSSParams>".to_string(),
            "<p:RSAPSSParams><p:MaskGenerationFunction><ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha512\"/></p:MaskGenerationFunction></p:RSAPSSParams>".to_string(),
            "<p:RSAPSSParams><p:Unknown/></p:RSAPSSParams>".to_string(),
            "<p:RSAPSSParams><p:SaltLength>32<evil/></p:SaltLength></p:RSAPSSParams>".to_string(),
        ] { assert!(!check(&body), "{body}"); }
        for document in [include_bytes!("../tests/fixtures/signatures/rsa-pss-param-default.docx").as_slice(), include_bytes!("../tests/fixtures/signatures/rsa-pss-param-empty.docx").as_slice(), include_bytes!("../tests/fixtures/signatures/rsa-pss-param-explicit.docx").as_slice()] {
            let mut pkg = Package::open(document.to_vec()).unwrap(); let part = "_xmlsignatures/sig1.xml";
            let xml = String::from_utf8(pkg.read(part).unwrap()).unwrap();
            assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::RsaPssSha256 { matches: true });
            pkg.set_bytes(part, xml.replace("#package", "#changed").into_bytes());
            assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::RsaPssSha256 { matches: false });
        }
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
    fn ecdsa_p384_checks_document_certificate_and_crl_signatures() {
        use x509_cert::{Certificate, crl::CertificateList, der::{Decode, Encode, asn1::Any}};
        let mut pkg = Package::open(include_bytes!("../tests/fixtures/signatures/p384-signed-synthetic.docx").to_vec()).unwrap();
        let part = "_xmlsignatures/sig1.xml";
        let xml = String::from_utf8(pkg.read(part).unwrap()).unwrap();
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::EcdsaP384 { matches: true });
        pkg.set_bytes(part, xml.replace("#package", "#changed").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::EcdsaP384 { matches: false });
        pkg.set_bytes(part, xml.replace("ecdsa-sha384", "ecdsa-sha256").into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::Unsupported);
        let old_cert = extract_single_embedded_certificate(&pkg, part).unwrap();
        let p256 = include_bytes!("../tests/fixtures/signatures/ec-ca.der");
        pkg.set_bytes(part, xml.replace(&crate::base64::encode(&old_cert), &crate::base64::encode(p256)).into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::Unsupported);
        let doc = ua_cryptography_xml_c14n::DocumentText::decode(xml.as_bytes()).unwrap();
        let parsed = doc.parse().unwrap();
        let value = ds_children(parsed.root_element(), "SignatureValue")[0].text().unwrap();
        let der = include_bytes!("../tests/fixtures/signatures/p384-signature.der");
        pkg.set_bytes(part, xml.replace(value, &crate::base64::encode(der)).into_bytes());
        assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::EcdsaP384 { matches: false });
        let ca = include_bytes!("../tests/fixtures/signatures/p384-ca.der");
        let other = include_bytes!("../tests/fixtures/signatures/p384-other.der");
        let leaf = include_bytes!("../tests/fixtures/signatures/p384-normal-leaf.der");
        let crl = include_bytes!("../tests/fixtures/signatures/p384-normal.crl.der");
        let facts = inspect_certificate_link(leaf, ca).unwrap();
        assert!(facts.issuer_name_exact);
        assert_eq!(facts.child_signature, CertificateSignature::Matches);
        assert_eq!(inspect_certificate_link(leaf, other).unwrap().child_signature, CertificateSignature::Differs);
        assert_eq!(inspect_certificate_link(leaf, p256).unwrap().child_signature, CertificateSignature::Unsupported);
        assert_eq!(inspect_certificate_link(include_bytes!("../tests/fixtures/signatures/p384-sha256-leaf.der"), ca).unwrap().child_signature, CertificateSignature::Unsupported);
        let paths = find_candidate_certificate_paths(leaf, &[], &[ca, other]).unwrap();
        assert_eq!(paths.paths.len(), 2);
        assert_eq!(paths.paths[0].links[0].child_signature, CertificateSignature::Matches);
        assert_eq!(paths.paths[1].links[0].child_signature, CertificateSignature::Differs);
        let mut cert = Certificate::from_der(leaf.as_slice()).unwrap();
        cert.signature_algorithm.parameters = Some(Any::null());
        cert.tbs_certificate.signature.parameters = Some(Any::null());
        assert_eq!(inspect_certificate_link(&cert.to_der().unwrap(), ca).unwrap().child_signature, CertificateSignature::Unsupported);
        assert_eq!(inspect_crl(crl, leaf, ca, 0).unwrap().signature, CertificateSignature::Matches);
        assert_eq!(inspect_crl(crl, leaf, other, 0).unwrap().signature, CertificateSignature::Differs);
        assert_eq!(inspect_crl(include_bytes!("../tests/fixtures/signatures/p384-sha256.crl.der"), leaf, ca, 0).unwrap().signature, CertificateSignature::Unsupported);
        let mut damaged = crl.to_vec(); let last = damaged.len() - 1; damaged[last] ^= 1;
        assert_eq!(inspect_crl(&damaged, leaf, ca, 0).unwrap().signature, CertificateSignature::Differs);
        let mut invalid_crl = CertificateList::from_der(crl.as_slice()).unwrap();
        invalid_crl.signature_algorithm.parameters = Some(Any::null());
        invalid_crl.tbs_cert_list.signature.parameters = Some(Any::null());
        assert_eq!(inspect_crl(&invalid_crl.to_der().unwrap(), leaf, ca, 0).unwrap().signature, CertificateSignature::Unsupported);
    }

    #[test]
    fn manifest_inventory_is_literal_bounded_and_not_a_coverage_verdict() {
        let bytes = include_bytes!("../tests/fixtures/signatures/synthetic-signed.docx");
        let mut pkg = Package::open(bytes.to_vec()).unwrap();
        let report = inspect_signatures(&pkg).unwrap();
        assert_eq!(report.len(), 1);
        let original = &report[0].manifest_inventory;
        assert_eq!(original.entries, 6);
        assert_eq!(original.unlisted, 5);
        assert_eq!(original.ambiguous_entries, 0);
        assert_eq!(original.unresolved_references, 0);
        assert!(!original.unlisted_examples.iter().any(|name| name == "word/document.xml"));
        pkg.set_bytes("word/added.xml", b"changed".to_vec());
        pkg.set_bytes("word/ambiguous%20name.xml", b"data".to_vec());
        pkg.set_bytes("folder/", b"".to_vec());
        let report = inspect_signatures(&pkg).unwrap();
        assert_eq!(report[0].signature_value, SignatureValueCheck::MatchesUntrustedCertificate);
        assert_eq!(report[0].manifest_inventory.entries, 8);
        assert_eq!(report[0].manifest_inventory.unlisted, 6);
        assert_eq!(report[0].manifest_inventory.ambiguous_entries, 1);
        assert!(report[0].manifest_inventory.unlisted_examples.iter().any(|name| name == "word/added.xml"));
        let unsupported = [ManifestReference { uri: "/word/added.xml?ContentType=application/xml".into(), digest: ManifestDigest::Unsupported, content_type: ContentTypeCheck::Unsupported, relationships: None }, ManifestReference { uri: "/word/ambiguous%20name.xml?ContentType=application/xml".into(), digest: ManifestDigest::Unsupported, content_type: ContentTypeCheck::Unsupported, relationships: None }];
        let inventory = inspect_manifest_inventory(&pkg, &unsupported);
        assert_eq!(inventory.unresolved_references, 1);
        assert!(!inventory.unlisted_examples.iter().any(|name| name == "word/added.xml"));
        assert!(inventory.unlisted_examples.iter().any(|name| name == "word/document.xml"));
        pkg.set_bytes(&format!("extra/{}", "a".repeat(250)), b"x".to_vec());
        let inventory = inspect_manifest_inventory(&pkg, &[]);
        assert!(inventory.examples_truncated);
        assert!(inventory.unlisted_examples.iter().any(|name| name.ends_with('…') && name.chars().count() == 201));
        for i in 0..25 { pkg.set_bytes(&format!("extra/{i}.xml"), b"x".to_vec()); }
        let inventory = inspect_manifest_inventory(&pkg, &[]);
        assert_eq!(inventory.unlisted_examples.len(), 16);
        assert!(inventory.examples_truncated);
        assert!(inventory.unlisted > inventory.unlisted_examples.len());
    }

    #[test]
    fn policy_control_observations_do_not_apply_constraints() {
        use x509_cert::{Certificate, der::{Decode, Encode, asn1::{ObjectIdentifier, OctetString}}, ext::pkix::{PolicyMappings, PolicyMapping}};
        let der = include_bytes!("../tests/fixtures/signatures/policy-controls.der");
        let facts = inspect_certificate_policy_controls(der).unwrap();
        assert_eq!(facts.mappings, CertificateControl::Present { critical: true, value: CertificatePolicyMappings {
            total: 1, inspected: 1, truncated: false, any_policy_pairs_inspected: 0, repeated_pairs_inspected: 0,
            examples: vec![("1.2.3.4".into(), "1.2.3.5".into())],
        }});
        assert_eq!(facts.constraints, CertificateControl::Present { critical: true, value: CertificatePolicyConstraints { require_explicit_policy: Some(2), inhibit_policy_mapping: Some(3) } });
        assert_eq!(facts.inhibit_any_policy, CertificateControl::Present { critical: true, value: 4 });
        let absent = inspect_certificate_policy_controls(include_bytes!("../tests/fixtures/signatures/ocsp-leaf.der")).unwrap();
        assert_eq!(absent.mappings, CertificateControl::Absent);
        assert_eq!(absent.constraints, CertificateControl::Absent);
        assert_eq!(absent.inhibit_any_policy, CertificateControl::Absent);
        assert!(inspect_certificate_policy_controls(b"bad").is_err());
        assert!(inspect_certificate_policy_controls(&vec![0; 1024 * 1024 + 1]).is_err());
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        let extensions = cert.tbs_certificate.extensions.as_mut().unwrap();
        let constraints = extensions.iter().find(|ext| ext.extn_id.to_string() == "2.5.29.36").unwrap().clone();
        extensions.push(constraints);
        let facts = inspect_certificate_policy_controls(&cert.to_der().unwrap()).unwrap();
        assert_eq!(facts.constraints, CertificateControl::UnparsedOrDuplicate);
        assert!(matches!(facts.mappings, CertificateControl::Present { .. }));
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        let extensions = cert.tbs_certificate.extensions.as_mut().unwrap();
        extensions.iter_mut().find(|ext| ext.extn_id.to_string() == "2.5.29.54").unwrap().extn_value = OctetString::new(vec![0x02, 0x01, 0xff]).unwrap();
        let facts = inspect_certificate_policy_controls(&cert.to_der().unwrap()).unwrap();
        assert_eq!(facts.inhibit_any_policy, CertificateControl::UnparsedOrDuplicate);
        assert!(matches!(facts.constraints, CertificateControl::Present { .. }));
        let ext = cert.tbs_certificate.extensions.as_mut().unwrap().iter_mut().find(|ext| ext.extn_id.to_string() == "2.5.29.54").unwrap();
        ext.extn_value = OctetString::new(vec![0x02, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(inspect_certificate_policy_controls(&cert.to_der().unwrap()).unwrap().inhibit_any_policy, CertificateControl::UnparsedOrDuplicate);
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        let mut mappings = cert.tbs_certificate.get::<PolicyMappings>().unwrap().unwrap().1;
        let pair = PolicyMapping { issuer_domain_policy: ObjectIdentifier::new_unwrap("2.5.29.32.0"), subject_domain_policy: ObjectIdentifier::new_unwrap("1.2.3.4") };
        for _ in 0..65 { mappings.0.push(pair.clone()); }
        let ext = cert.tbs_certificate.extensions.as_mut().unwrap().iter_mut().find(|ext| ext.extn_id.to_string() == "2.5.29.33").unwrap();
        ext.extn_value = OctetString::new(mappings.to_der().unwrap()).unwrap();
        let facts = inspect_certificate_policy_controls(&cert.to_der().unwrap()).unwrap();
        assert!(matches!(facts.mappings, CertificateControl::Present { value: CertificatePolicyMappings { total: 66, inspected: 64, truncated: true, any_policy_pairs_inspected: 63, repeated_pairs_inspected: 62, .. }, .. }));
        if let CertificateControl::Present { value, .. } = facts.mappings { assert_eq!(value.examples.len(), 8); }
        mappings.0.clear();
        let ext = cert.tbs_certificate.extensions.as_mut().unwrap().iter_mut().find(|ext| ext.extn_id.to_string() == "2.5.29.33").unwrap();
        ext.extn_value = OctetString::new(mappings.to_der().unwrap()).unwrap();
        assert_eq!(inspect_certificate_policy_controls(&cert.to_der().unwrap()).unwrap().mappings, CertificateControl::UnparsedOrDuplicate);
    }

    #[test]
    fn certificate_policy_observations_are_bounded_and_untrusted() {
        use x509_cert::{Certificate, der::{Decode, Encode, asn1::OctetString}, ext::pkix::CertificatePolicies};
        let der = include_bytes!("../tests/fixtures/signatures/certificate-policies.der");
        let facts = inspect_certificate_policies(der).unwrap();
        assert_eq!(facts, CertificatePolicyObservations::Present { critical: true, total: 2, inspected: 2, truncated: false,
            repeated_oid_entries_inspected: 0, any_policy_inspected: true, qualifier_entries_inspected: 1,
            empty_qualifier_sequences_inspected: 0, examples: vec!["1.2.3.4.5".into(), "2.5.29.32.0".into()] });
        assert_eq!(inspect_certificate_policies(include_bytes!("../tests/fixtures/signatures/ocsp-leaf.der")).unwrap(), CertificatePolicyObservations::Absent);
        assert!(inspect_certificate_policies(b"bad").is_err());
        assert!(inspect_certificate_policies(&vec![0; 1024 * 1024 + 1]).is_err());
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        let extensions = cert.tbs_certificate.extensions.as_mut().unwrap();
        extensions.push(extensions[0].clone());
        assert_eq!(inspect_certificate_policies(&cert.to_der().unwrap()).unwrap(), CertificatePolicyObservations::InvalidOrDuplicateExtension);
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        cert.tbs_certificate.extensions.as_mut().unwrap()[0].extn_value = OctetString::new(vec![0xff]).unwrap();
        assert_eq!(inspect_certificate_policies(&cert.to_der().unwrap()).unwrap(), CertificatePolicyObservations::InvalidOrDuplicateExtension);
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        let mut policies = cert.tbs_certificate.get::<CertificatePolicies>().unwrap().unwrap().1;
        let repeated = policies.0[0].clone();
        for _ in 0..64 { policies.0.push(repeated.clone()); }
        cert.tbs_certificate.extensions.as_mut().unwrap()[0].extn_value = OctetString::new(policies.to_der().unwrap()).unwrap();
        let overflow = inspect_certificate_policies(&cert.to_der().unwrap()).unwrap();
        assert!(matches!(overflow, CertificatePolicyObservations::Present { total: 66, inspected: 64, truncated: true, repeated_oid_entries_inspected: 62, qualifier_entries_inspected: 63, .. }));
        if let CertificatePolicyObservations::Present { examples, .. } = overflow { assert_eq!(examples.len(), 8); }
        policies.0.clear();
        cert.tbs_certificate.extensions.as_mut().unwrap()[0].extn_value = OctetString::new(policies.to_der().unwrap()).unwrap();
        assert_eq!(inspect_certificate_policies(&cert.to_der().unwrap()).unwrap(), CertificatePolicyObservations::InvalidOrDuplicateExtension);
    }

    #[test]
    fn certificate_location_hints_are_bounded_and_not_revocation_decisions() {
        use x509_cert::{Certificate, der::{Decode, Encode, asn1::OctetString}};
        let der = include_bytes!("../tests/fixtures/signatures/location-hints.der");
        let hints = inspect_certificate_location_hints(der).unwrap();
        assert_eq!(hints.aia, LocationExtension::Parsed { critical: false });
        assert_eq!(hints.crl_distribution_points, LocationExtension::Parsed { critical: false });
        assert_eq!(hints.ocsp.total_uris, 1);
        assert_eq!(hints.ocsp.examples, ["http://ocsp.example.invalid/status"]);
        assert_eq!(hints.ocsp.non_uri_names, 1);
        assert_eq!(hints.ca_issuers.examples, ["https://ca.example.invalid/issuer.der"]);
        assert_eq!(hints.crl.total_uris, 2);
        assert_eq!(hints.crl.non_uri_names, 1);
        assert_eq!(hints.other_aia_methods, 1);
        assert_eq!((hints.scoped_crl_points, hints.alternate_crl_issuer_points, hints.other_crl_points), (1, 1, 1));
        let overflow = inspect_certificate_location_hints(include_bytes!("../tests/fixtures/signatures/location-hints-overflow.der")).unwrap();
        assert_eq!(overflow.ocsp.total_uris, 11);
        assert_eq!(overflow.ocsp.examples.len(), 8);
        assert_eq!(overflow.ocsp.omitted_uris, 3);
        assert!(overflow.ocsp.examples.iter().all(|uri| uri.len() <= 2048));
        assert_eq!(overflow.crl_distribution_points, LocationExtension::Absent);
        let absent = inspect_certificate_location_hints(include_bytes!("../tests/fixtures/signatures/ec-ca.der")).unwrap();
        assert_eq!(absent.aia, LocationExtension::Absent);
        assert_eq!(absent.crl_distribution_points, LocationExtension::Absent);
        assert!(inspect_certificate_location_hints(b"bad").is_err());
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        let extensions = cert.tbs_certificate.extensions.as_mut().unwrap();
        let aia = extensions.iter().find(|e| e.extn_id.to_string() == "1.3.6.1.5.5.7.1.1").unwrap().clone();
        extensions.push(aia);
        let duplicate = inspect_certificate_location_hints(&cert.to_der().unwrap()).unwrap();
        assert_eq!(duplicate.aia, LocationExtension::Unsupported);
        assert_eq!(duplicate.ocsp.total_uris, 0);
        assert_eq!(duplicate.crl_distribution_points, LocationExtension::Parsed { critical: false });
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        let extensions = cert.tbs_certificate.extensions.as_mut().unwrap();
        extensions.iter_mut().find(|e| e.extn_id.to_string() == "2.5.29.31").unwrap().extn_value = OctetString::new(vec![0xff]).unwrap();
        let malformed = inspect_certificate_location_hints(&cert.to_der().unwrap()).unwrap();
        assert_eq!(malformed.crl_distribution_points, LocationExtension::Unsupported);
        assert_eq!(malformed.aia, LocationExtension::Parsed { critical: false });
        let mut cert = Certificate::from_der(der.as_slice()).unwrap();
        cert.tbs_certificate.extensions.as_mut().unwrap().iter_mut().find(|e| e.extn_id.to_string() == "1.3.6.1.5.5.7.1.1").unwrap().critical = true;
        assert_eq!(inspect_certificate_location_hints(&cert.to_der().unwrap()).unwrap().aia, LocationExtension::Parsed { critical: true });
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
        assert_eq!(inspect_signature_value(&pkg, "sig.xml").unwrap(), SignatureValueCheck::Differs);
        pkg.set_bytes("sig.xml", xml.replace("rsa-sha256", "rsa-sha3").into_bytes());
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
    fn rsa_sha384_and_sha512_cover_document_certificate_and_crl() {
        use x509_cert::{Certificate, crl::CertificateList, der::{Decode, Encode, asn1::Any}};
        let ca = include_bytes!("../tests/fixtures/signatures/rsa-sha2-ca.der");
        let other = include_bytes!("../tests/fixtures/signatures/issuer.der");
        for (document, leaf, crl, algorithm) in [
            (include_bytes!("../tests/fixtures/signatures/rsa-sha384-signed.docx").as_slice(), include_bytes!("../tests/fixtures/signatures/rsa-sha384-leaf.der").as_slice(), include_bytes!("../tests/fixtures/signatures/rsa-sha384.crl.der").as_slice(), "rsa-sha384"),
            (include_bytes!("../tests/fixtures/signatures/rsa-sha512-signed.docx").as_slice(), include_bytes!("../tests/fixtures/signatures/rsa-sha512-leaf.der").as_slice(), include_bytes!("../tests/fixtures/signatures/rsa-sha512.crl.der").as_slice(), "rsa-sha512"),
        ] {
            let mut pkg = Package::open(document.to_vec()).unwrap();
            let part = "_xmlsignatures/sig1.xml";
            let xml = String::from_utf8(pkg.read(part).unwrap()).unwrap();
            assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::MatchesUntrustedCertificate);
            pkg.set_bytes(part, xml.replace("#package", "#changed").into_bytes());
            assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::Differs);
            let other_algorithm = if algorithm == "rsa-sha384" { "rsa-sha512" } else { "rsa-sha384" };
            pkg.set_bytes(part, xml.replace(algorithm, other_algorithm).into_bytes());
            assert_eq!(inspect_signature_value(&pkg, part).unwrap(), SignatureValueCheck::Differs);
            let facts = inspect_certificate_link(leaf, ca).unwrap();
            assert!(facts.issuer_name_exact);
            assert_eq!(facts.child_signature, CertificateSignature::Matches);
            assert_eq!(inspect_certificate_link(leaf, other).unwrap().child_signature, CertificateSignature::Differs);
            let mut bad_cert = Certificate::from_der(leaf).unwrap();
            bad_cert.signature_algorithm.parameters = Some(Any::new(x509_cert::der::Tag::Integer, vec![1]).unwrap());
            bad_cert.tbs_certificate.signature.parameters = bad_cert.signature_algorithm.parameters.clone();
            assert_eq!(inspect_certificate_link(&bad_cert.to_der().unwrap(), ca).unwrap().child_signature, CertificateSignature::Unsupported);
            assert_eq!(inspect_crl(crl, leaf, ca, 0).unwrap().signature, CertificateSignature::Matches);
            assert_eq!(inspect_crl(crl, leaf, other, 0).unwrap().signature, CertificateSignature::Differs);
            let mut damaged = crl.to_vec(); let last = damaged.len() - 1; damaged[last] ^= 1;
            assert_eq!(inspect_crl(&damaged, leaf, ca, 0).unwrap().signature, CertificateSignature::Differs);
            let mut bad_crl = CertificateList::from_der(crl).unwrap();
            bad_crl.signature_algorithm.parameters = Some(Any::new(x509_cert::der::Tag::Integer, vec![1]).unwrap());
            bad_crl.tbs_cert_list.signature.parameters = bad_crl.signature_algorithm.parameters.clone();
            assert_eq!(inspect_crl(&bad_crl.to_der().unwrap(), leaf, ca, 0).unwrap().signature, CertificateSignature::Unsupported);
        }
    }

    #[test]
    fn rsa_pss_certificate_and_crl_require_supported_parameters() {
        use x509_cert::{Certificate, crl::CertificateList, der::{Decode, Encode, asn1::{Any, ObjectIdentifier}}};
        let ca = include_bytes!("../tests/fixtures/signatures/pss-ca.der");
        let other = include_bytes!("../tests/fixtures/signatures/pss-other.der");
        let leaf = include_bytes!("../tests/fixtures/signatures/pss-normal-leaf.der");
        let crl = include_bytes!("../tests/fixtures/signatures/pss-normal.crl.der");
        let facts = inspect_certificate_link(leaf, ca).unwrap();
        assert!(facts.issuer_name_exact);
        assert_eq!(facts.child_signature, CertificateSignature::Matches);
        let parsed_child = Certificate::from_der(leaf.as_slice()).unwrap();
        let parsed_ca = Certificate::from_der(ca.as_slice()).unwrap();
        let mut alternative = parsed_child.signature_algorithm.clone();
        let encoded = alternative.parameters.as_ref().unwrap().to_der().unwrap();
        let mut parameters = rsa::pkcs1::RsaPssParams::from_der(&encoded).unwrap();
        parameters.hash.parameters = None;
        parameters.mask_gen.parameters.as_mut().unwrap().parameters = None;
        alternative.parameters = Some(Any::encode_from(&parameters).unwrap());
        let body = parsed_child.tbs_certificate.to_der().unwrap();
        let signature = parsed_child.signature.as_bytes().unwrap();
        assert_eq!(inspect_rsa_pss_x509_signature(&alternative, &parsed_ca.tbs_certificate.subject_public_key_info, &body, signature).unwrap(), CertificateSignature::Matches);
        let mut explicit_trailer = encoded.clone();
        assert!(explicit_trailer[1] < 0x80);
        explicit_trailer[1] += 5;
        explicit_trailer.extend_from_slice(&[0xa3, 0x03, 0x02, 0x01, 0x01]);
        alternative.parameters = Some(Any::from_der(&explicit_trailer).unwrap());
        assert_eq!(inspect_rsa_pss_x509_signature(&alternative, &parsed_ca.tbs_certificate.subject_public_key_info, &body, signature).unwrap(), CertificateSignature::Matches);
        assert!(inspect_certificate_link(leaf, other).unwrap().issuer_name_exact);
        assert_eq!(inspect_certificate_link(leaf, other).unwrap().child_signature, CertificateSignature::Differs);
        for unsupported in [include_bytes!("../tests/fixtures/signatures/pss-salt20-leaf.der").as_slice(), include_bytes!("../tests/fixtures/signatures/pss-mgf384-leaf.der").as_slice()] {
            assert_eq!(inspect_certificate_link(unsupported, ca).unwrap().child_signature, CertificateSignature::Unsupported);
        }
        let paths = find_candidate_certificate_paths(leaf, &[], &[ca, other]).unwrap();
        assert_eq!(paths.paths.len(), 2);
        assert_eq!(paths.paths[0].links[0].child_signature, CertificateSignature::Matches);
        assert_eq!(paths.paths[1].links[0].child_signature, CertificateSignature::Differs);
        let mut cert = Certificate::from_der(leaf.as_slice()).unwrap();
        cert.signature_algorithm.parameters = None;
        assert_eq!(inspect_certificate_link(&cert.to_der().unwrap(), ca).unwrap().child_signature, CertificateSignature::Unsupported);
        cert.tbs_certificate.signature.parameters = None;
        assert_eq!(inspect_certificate_link(&cert.to_der().unwrap(), ca).unwrap().child_signature, CertificateSignature::Unsupported);
        let defaults = Any::encode_from(&rsa::pkcs1::RsaPssParams::default()).unwrap();
        cert.signature_algorithm.parameters = Some(defaults.clone());
        cert.tbs_certificate.signature.parameters = Some(defaults);
        assert_eq!(inspect_certificate_link(&cert.to_der().unwrap(), ca).unwrap().child_signature, CertificateSignature::Unsupported);
        let mut restricted = Certificate::from_der(ca.as_slice()).unwrap();
        restricted.tbs_certificate.subject_public_key_info.algorithm.oid = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.10");
        assert_eq!(inspect_certificate_link(leaf, &restricted.to_der().unwrap()).unwrap().child_signature, CertificateSignature::Unsupported);
        assert_eq!(inspect_crl(crl, leaf, ca, 0).unwrap().signature, CertificateSignature::Matches);
        assert_eq!(inspect_crl(crl, leaf, other, 0).unwrap().signature, CertificateSignature::Differs);
        for unsupported in [include_bytes!("../tests/fixtures/signatures/pss-salt20.crl.der").as_slice(), include_bytes!("../tests/fixtures/signatures/pss-mgf384.crl.der").as_slice()] {
            assert_eq!(inspect_crl(unsupported, leaf, ca, 0).unwrap().signature, CertificateSignature::Unsupported);
        }
        let mut damaged = crl.to_vec(); let last = damaged.len() - 1; damaged[last] ^= 1;
        assert_eq!(inspect_crl(&damaged, leaf, ca, 0).unwrap().signature, CertificateSignature::Differs);
        let mut changed = CertificateList::from_der(crl.as_slice()).unwrap();
        changed.signature_algorithm.parameters = None;
        changed.tbs_cert_list.signature.parameters = None;
        assert_eq!(inspect_crl(&changed.to_der().unwrap(), leaf, ca, 0).unwrap().signature, CertificateSignature::Unsupported);
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

    #[test]
    fn crl_number_observations_do_not_decide_revocation() {
        use x509_cert::{crl::CertificateList, der::{Decode, Encode, asn1::OctetString}};
        let root = include_bytes!("../../../experiments/certval-crl-probe/data/root.der");
        let leaf = include_bytes!("../../../experiments/certval-crl-probe/data/leaf.der");
        let time = 1796083200;
        for (bytes, expected) in [
            (include_bytes!("../../../experiments/certval-crl-probe/data/empty.der").as_slice(), CrlNumberObservation::NonCritical),
            (include_bytes!("../../../experiments/certval-crl-probe/data/missing-number.der").as_slice(), CrlNumberObservation::Missing),
            (include_bytes!("../../../experiments/certval-crl-probe/data/critical-number.der").as_slice(), CrlNumberObservation::Critical),
        ] {
            let facts = inspect_crl(bytes, leaf, root, time).unwrap();
            assert_eq!(facts.crl_number, expected);
            assert_eq!(facts.signature, CertificateSignature::Matches);
            assert_eq!(facts.listed_revocation_time, None);
        }
        let critical = inspect_crl(include_bytes!("../../../experiments/certval-crl-probe/data/critical-number.der"), leaf, root, time).unwrap();
        assert_eq!(critical.scope, CrlScope::UnsupportedExtensions);
        assert!(inspect_crl(include_bytes!("../../../experiments/certval-crl-probe/data/duplicate-unrelated.der"), leaf, root, time).is_err());

        let mut crl = CertificateList::from_der(include_bytes!("../../../experiments/certval-crl-probe/data/empty.der").as_slice()).unwrap();
        let number = &mut crl.tbs_cert_list.crl_extensions.as_mut().unwrap()[0];
        number.extn_value = OctetString::new([0x02, 0x01, 0xff]).unwrap(); // negative INTEGER
        let changed = crl.to_der().unwrap();
        let facts = inspect_crl(&changed, leaf, root, time).unwrap();
        assert_eq!(facts.crl_number, CrlNumberObservation::MalformedNonCritical);
        assert_eq!(facts.signature, CertificateSignature::Differs);
        crl.tbs_cert_list.crl_extensions.as_mut().unwrap()[0].critical = true;
        assert_eq!(inspect_crl(&crl.to_der().unwrap(), leaf, root, time).unwrap().crl_number, CrlNumberObservation::MalformedCritical);
        let duplicate = crl.tbs_cert_list.crl_extensions.as_ref().unwrap()[0].clone();
        crl.tbs_cert_list.crl_extensions.as_mut().unwrap().push(duplicate);
        assert!(inspect_crl(&crl.to_der().unwrap(), leaf, root, time).is_err());
    }

    #[test]
    fn crl_authority_key_identifiers_are_only_literal_observations() {
        use x509_cert::{Certificate, crl::CertificateList, der::{Decode, Encode, asn1::OctetString}};
        let issuer = include_bytes!("../tests/fixtures/signatures/aki-issuer.der");
        let leaf = include_bytes!("../tests/fixtures/signatures/crl-leaf.der");
        let matching = include_bytes!("../tests/fixtures/signatures/aki-match.crl.der");
        let different = include_bytes!("../tests/fixtures/signatures/aki-mismatch.crl.der");
        for (crl, comparison) in [(matching.as_slice(), KeyIdentifierComparison::Matches), (different.as_slice(), KeyIdentifierComparison::Differs)] {
            let facts = inspect_crl(crl, leaf, issuer, 1796083200).unwrap();
            assert_eq!(facts.signature, CertificateSignature::Matches);
            assert_eq!(facts.authority_key.crl_extension, IdentifierExtensionObservation::NonCritical);
            assert_eq!(facts.authority_key.issuer_subject_key_extension, IdentifierExtensionObservation::NonCritical);
            assert_eq!(facts.authority_key.key_identifiers, comparison);
            assert_eq!(facts.authority_key.authority_certificate_issuer_present, Some(false));
            assert_eq!(facts.authority_key.authority_certificate_serial_present, Some(false));
        }
        let mut crl = CertificateList::from_der(matching.as_slice()).unwrap();
        let extensions = crl.tbs_cert_list.crl_extensions.as_mut().unwrap();
        let index = extensions.iter().position(|e| e.extn_id.to_string() == "2.5.29.35").unwrap();
        extensions[index].critical = true;
        let facts = inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 1796083200).unwrap();
        assert_eq!(facts.authority_key.crl_extension, IdentifierExtensionObservation::Critical);
        assert_eq!(facts.authority_key.key_identifiers, KeyIdentifierComparison::Matches);
        assert_eq!(facts.scope, CrlScope::UnsupportedExtensions);
        crl.tbs_cert_list.crl_extensions.as_mut().unwrap()[index].critical = false;
        crl.tbs_cert_list.crl_extensions.as_mut().unwrap()[index].extn_value = OctetString::new([0x30, 0x00]).unwrap();
        assert_eq!(inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 1796083200).unwrap().authority_key.key_identifiers, KeyIdentifierComparison::NotComparable);
        let extensions = crl.tbs_cert_list.crl_extensions.as_mut().unwrap();
        extensions[index].extn_value = OctetString::new([0x30, 0x02, 0x80, 0x00]).unwrap();
        assert_eq!(inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 1796083200).unwrap().authority_key.key_identifiers, KeyIdentifierComparison::EmptyIdentifier);
        crl.tbs_cert_list.crl_extensions.as_mut().unwrap()[index].extn_value = OctetString::new([0xff]).unwrap();
        let facts = inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 1796083200).unwrap();
        assert_eq!(facts.authority_key.crl_extension, IdentifierExtensionObservation::MalformedNonCritical);
        assert_eq!(facts.authority_key.key_identifiers, KeyIdentifierComparison::NotComparable);
        crl.tbs_cert_list.crl_extensions.as_mut().unwrap()[index].critical = true;
        assert_eq!(inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 1796083200).unwrap().authority_key.crl_extension, IdentifierExtensionObservation::MalformedCritical);
        crl.tbs_cert_list.crl_extensions.as_mut().unwrap().remove(index);
        assert_eq!(inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 1796083200).unwrap().authority_key.crl_extension, IdentifierExtensionObservation::Missing);

        let mut cert = Certificate::from_der(issuer.as_slice()).unwrap();
        let exts = cert.tbs_certificate.extensions.as_mut().unwrap();
        let index = exts.iter().position(|e| e.extn_id.to_string() == "2.5.29.14").unwrap();
        exts[index].extn_value = OctetString::new([0xff]).unwrap();
        let facts = inspect_crl(matching, leaf, &cert.to_der().unwrap(), 1796083200).unwrap();
        assert_eq!(facts.authority_key.issuer_subject_key_extension, IdentifierExtensionObservation::MalformedNonCritical);
        assert_eq!(facts.authority_key.key_identifiers, KeyIdentifierComparison::NotComparable);
        cert.tbs_certificate.extensions.as_mut().unwrap()[index].critical = true;
        assert_eq!(inspect_crl(matching, leaf, &cert.to_der().unwrap(), 1796083200).unwrap().authority_key.issuer_subject_key_extension, IdentifierExtensionObservation::MalformedCritical);
        let duplicate = cert.tbs_certificate.extensions.as_ref().unwrap()[index].clone();
        cert.tbs_certificate.extensions.as_mut().unwrap().push(duplicate);
        assert!(inspect_crl(matching, leaf, &cert.to_der().unwrap(), 1796083200).is_err());
        cert.tbs_certificate.extensions.as_mut().unwrap().pop();
        cert.tbs_certificate.extensions.as_mut().unwrap().remove(index);
        assert_eq!(inspect_crl(matching, leaf, &cert.to_der().unwrap(), 1796083200).unwrap().authority_key.issuer_subject_key_extension, IdentifierExtensionObservation::Missing);
    }

    #[test]
    fn crl_scope_claims_remain_unsupported() {
        use x509_cert::{crl::CertificateList, der::{Decode, Encode, asn1::OctetString}};
        let issuer = include_bytes!("../tests/fixtures/signatures/scope-issuer.der");
        let leaf = include_bytes!("../tests/fixtures/signatures/crl-leaf.der");
        let encoded = include_bytes!("../tests/fixtures/signatures/scope-delta.crl.der");
        let facts = inspect_crl(encoded, leaf, issuer, 1796083200).unwrap();
        assert_eq!(facts.signature, CertificateSignature::Matches);
        assert_eq!(facts.scope, CrlScope::UnsupportedExtensions);
        assert_eq!(facts.scope_extensions.delta_indicator, CrlExtensionObservation::Decoded { critical: true, value: () });
        assert_eq!(facts.scope_extensions.issuing_distribution_point, CrlExtensionObservation::Decoded { critical: true, value: IssuingDistributionPointClaims {
            distribution_point_present: true, only_user_certificates: true, only_ca_certificates: false,
            reasons_present: true, indirect_crl: true, only_attribute_certificates: false,
        } });
        assert_eq!(facts.listed_revocation_time, None);

        let mut crl = CertificateList::from_der(encoded.as_slice()).unwrap();
        let exts = crl.tbs_cert_list.crl_extensions.as_mut().unwrap();
        let delta = exts.iter().position(|e| e.extn_id.to_string() == "2.5.29.27").unwrap();
        let idp = exts.iter().position(|e| e.extn_id.to_string() == "2.5.29.28").unwrap();
        exts[delta].critical = false;
        exts[idp].critical = false;
        let modified = crl.to_der().unwrap();
        let facts = inspect_crl(&modified, leaf, issuer, 1796083200).unwrap();
        assert_eq!(facts.scope, CrlScope::UnsupportedExtensions);
        assert_eq!(facts.scope_extensions.delta_indicator, CrlExtensionObservation::Decoded { critical: false, value: () });
        assert!(matches!(facts.scope_extensions.issuing_distribution_point, CrlExtensionObservation::Decoded { critical: false, .. }));
        let exts = crl.tbs_cert_list.crl_extensions.as_mut().unwrap();
        exts[delta].extn_value = OctetString::new([0x02, 0x01, 0xff]).unwrap();
        exts[idp].extn_value = OctetString::new([0xff]).unwrap();
        let facts = inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 1796083200).unwrap();
        assert_eq!(facts.scope_extensions.delta_indicator, CrlExtensionObservation::Malformed { critical: false });
        assert_eq!(facts.scope_extensions.issuing_distribution_point, CrlExtensionObservation::Malformed { critical: false });
        assert_eq!(facts.scope, CrlScope::UnsupportedExtensions);
        let exts = crl.tbs_cert_list.crl_extensions.as_mut().unwrap();
        let duplicate = exts[idp].clone();
        exts.push(duplicate);
        assert!(inspect_crl(&crl.to_der().unwrap(), leaf, issuer, 1796083200).is_err());
        let empty = include_bytes!("../tests/fixtures/signatures/empty.crl.der");
        let issuer = include_bytes!("../tests/fixtures/signatures/crl-issuer.der");
        let facts = inspect_crl(empty, leaf, issuer, 1796083200).unwrap();
        assert_eq!(facts.scope_extensions.delta_indicator, CrlExtensionObservation::Absent);
        assert_eq!(facts.scope_extensions.issuing_distribution_point, CrlExtensionObservation::Absent);
    }
}

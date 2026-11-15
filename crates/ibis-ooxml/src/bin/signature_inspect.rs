// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Inspect signature-related observations, without making a trust decision.

use std::process::ExitCode;
use ibis_ooxml::{Package, signatures::{self, ContentTypeCheck, ManifestDigest, ObjectDigest, SignatureValueCheck, XadesIssuerSerial, RelationshipReference}};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("ibis-signature-inspect: {error}"); ExitCode::from(2) }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: ibis-signature-inspect <docx|xlsx|pptx> [--at <unix-seconds>] [--anchor <der>] [--intermediate <der>] [--crl <der> --crl-issuer <der>] [--ocsp <der> --ocsp-issuer <der> --ocsp-responder <der> [--ocsp-nonce <raw-file> | --ocsp-request <der>]]")?;
    let mut reference_time: Option<u64> = None;
    let mut anchors = Vec::new();
    let mut intermediates = Vec::new();
    let mut crls = Vec::new();
    let mut crl_issuer = None;
    let mut ocsps = Vec::new();
    let mut ocsp_issuer = None;
    let mut ocsp_responder = None;
    let mut ocsp_nonce = None;
    let mut ocsp_request = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--at") if reference_time.is_none() => {
                let value = args.next().ok_or("--at requires Unix seconds")?;
                reference_time = Some(value.to_str().ok_or("invalid --at")?.parse().map_err(|_| "invalid --at")?);
            }
            Some("--anchor") | Some("--intermediate") => {
                let cert_path = args.next().ok_or("certificate option requires a DER file")?;
                let cert = read_limited(&cert_path, 1024 * 1024)?;
                if arg == "--anchor" { anchors.push(cert); } else { intermediates.push(cert); }
                if anchors.len() + intermediates.len() > 63 { return Err("too many certificate files".into()); }
            }
            Some("--crl") => {
                let crl_path = args.next().ok_or("--crl requires a DER file")?;
                crls.push(read_limited(&crl_path, 8 * 1024 * 1024)?);
                if crls.len() > 8 { return Err("too many CRL files".into()); }
            }
            Some("--crl-issuer") if crl_issuer.is_none() => {
                let issuer_path = args.next().ok_or("--crl-issuer requires a DER file")?;
                crl_issuer = Some(read_limited(&issuer_path, 1024 * 1024)?);
            }
            Some("--ocsp") => {
                let response_path = args.next().ok_or("--ocsp requires a DER file")?;
                ocsps.push(read_limited(&response_path, 8 * 1024 * 1024)?);
                if ocsps.len() > 8 { return Err("too many OCSP files".into()); }
            }
            Some("--ocsp-issuer") if ocsp_issuer.is_none() => {
                let issuer_path = args.next().ok_or("--ocsp-issuer requires a DER file")?;
                ocsp_issuer = Some(read_limited(&issuer_path, 1024 * 1024)?);
            }
            Some("--ocsp-request") if ocsp_request.is_none() => {
                let request_path = args.next().ok_or("--ocsp-request requires a DER file")?;
                ocsp_request = Some(read_limited(&request_path, 1024 * 1024)?);
            }
            Some("--ocsp-nonce") if ocsp_nonce.is_none() => {
                let nonce_path = args.next().ok_or("--ocsp-nonce requires a raw file")?;
                let nonce = read_limited(&nonce_path, 32)?;
                if nonce.is_empty() { return Err("OCSP nonce file must contain 1 to 32 bytes".into()); }
                ocsp_nonce = Some(nonce);
            }
            Some("--ocsp-responder") if ocsp_responder.is_none() => {
                let responder_path = args.next().ok_or("--ocsp-responder requires a DER file")?;
                ocsp_responder = Some(read_limited(&responder_path, 1024 * 1024)?);
            }
            _ => return Err("unknown or repeated option".into()),
        }
    }
    if !intermediates.is_empty() && anchors.is_empty() { return Err("--intermediate requires --anchor".into()); }
    if (!crls.is_empty() || crl_issuer.is_some()) && (crls.is_empty() || crl_issuer.is_none() || reference_time.is_none()) {
        return Err("CRL inspection requires --crl, --crl-issuer, and --at".into());
    }
    if (!ocsps.is_empty() || ocsp_issuer.is_some() || ocsp_responder.is_some() || ocsp_nonce.is_some() || ocsp_request.is_some()) && (ocsps.is_empty() || ocsp_issuer.is_none() || ocsp_responder.is_none() || reference_time.is_none()) {
        return Err("OCSP inspection requires --ocsp, --ocsp-issuer, --ocsp-responder, and --at".into());
    }
    if ocsp_request.is_some() && ocsp_nonce.is_some() { return Err("--ocsp-request and --ocsp-nonce are mutually exclusive".into()); }
    if ocsps.iter().map(Vec::len).sum::<usize>() > 16 * 1024 * 1024 { return Err("OCSP inputs exceed 16 MiB".into()); }
    if crls.iter().map(Vec::len).sum::<usize>() > 16 * 1024 * 1024 { return Err("CRL inputs exceed 16 MiB".into()); }
    if anchors.iter().chain(intermediates.iter()).map(Vec::len).sum::<usize>() > 4 * 1024 * 1024 { return Err("certificate inputs exceed 4 MiB".into()); }
    let bytes = read_limited(&path, 128 * 1024 * 1024)?;
    let anchor_refs: Vec<&[u8]> = anchors.iter().map(Vec::as_slice).collect();
    let intermediate_refs: Vec<&[u8]> = intermediates.iter().map(Vec::as_slice).collect();
    let package = Package::open(bytes).map_err(|e| e.to_string())?;
    let reports = signatures::inspect_signatures(&package).map_err(|e| e.to_string())?;
    println!("Inspection only: signature trust, certificate paths, revocation, timestamps, and complete coverage are NOT evaluated.");
    println!("Exit status 0 means inspection completed, not that a signature is valid.");
    if reports.is_empty() { println!("No package signature relationships found."); }
    for report in reports {
        println!("\nSignature part: {:?}", report.signature_part);
        if let Some(hints) = &report.certificate_locations {
            println!("  Certificate location hints (UNTRUSTED; NOT FETCHED): AIA {:?}; CRL distribution points {:?}", hints.aia, hints.crl_distribution_points);
            for (label, list) in [("OCSP", &hints.ocsp), ("CA issuer", &hints.ca_issuers), ("CRL", &hints.crl)] {
                println!("    {label}: {} URI hints, {} omitted, {} non-URI names", list.total_uris, list.omitted_uris, list.non_uri_names);
                for uri in &list.examples { println!("      {uri:?}"); }
            }
            println!("    Other AIA methods {}; CRL points with reason restrictions {}; alternate issuers {}; other point forms {}. Applicability NOT EVALUATED.", hints.other_aia_methods, hints.scoped_crl_points, hints.alternate_crl_issuer_points, hints.other_crl_points);
        } else { println!("  Certificate location hints: NOT CHECKED; NOT FETCHED."); }
        if let Some(time) = reference_time {
            let facts = signatures::inspect_signer_certificate(&package, &report.signature_part, time, &anchor_refs).map_err(|e| e.to_string())?;
            println!("  Certificate time at {time}: {:?} (notBefore {}, notAfter {})", facts.time_status, facts.not_before, facts.not_after);
            println!("  Certificate key usage: {:?}; identical to supplied anchor: {}", facts.key_usage, facts.identical_to_supplied_anchor);
        }
        if !anchors.is_empty() {
            let cert = signatures::extract_single_embedded_certificate(&package, &report.signature_part).map_err(|e| e.to_string())?;
            let candidates = signatures::find_candidate_certificate_paths(&cert, &intermediate_refs, &anchor_refs).map_err(|e| e.to_string())?;
            println!("  Candidate paths: {} (truncated: {}); NOT validated", candidates.paths.len(), candidates.truncated);
            for (index, candidate) in candidates.paths.iter().enumerate() {
                println!("    Candidate {}: {} certificates", index + 1, candidate.fingerprints_sha256.len());
                for (position, fingerprint) in candidate.fingerprints_sha256.iter().enumerate() {
                    let hex: String = fingerprint.iter().map(|b| format!("{b:02x}")).collect();
                    println!("      Certificate {} SHA-256: {hex}", position + 1);
                }
                for (edge, link) in candidate.links.iter().enumerate() {
                    println!("      Link {}: signature {:?}, CA {:?}, keyCertSign {:?}", edge + 1, link.child_signature, link.issuer_ca, link.issuer_key_cert_sign);
                }
            }
            println!("  Candidate search uses exact names only; zero candidates does not prove no path exists.");
        }
        if let (Some(time), Some(issuer)) = (reference_time, crl_issuer.as_deref()) {
            let cert = signatures::extract_single_embedded_certificate(&package, &report.signature_part).map_err(|e| e.to_string())?;
            for (index, crl) in crls.iter().enumerate() {
                let facts = signatures::inspect_crl(crl, &cert, issuer, time).map_err(|e| e.to_string())?;
                println!("  CRL {} observations (NOT a revocation verdict):", index + 1);
                println!("    Signature {:?}; issuer name exact {}; certificate issuer name exact {}; cRLSign {:?}", facts.signature, facts.issuer_name_exact, facts.certificate_issuer_name_exact, facts.issuer_crl_sign);
                println!("    Time {:?}; thisUpdate {}; nextUpdate {:?}", facts.time_status, facts.this_update, facts.next_update);
                println!("    Scope {:?}; listed revocation time {:?}", facts.scope, facts.listed_revocation_time);
                println!("    CRL applicability and issuer trust: NOT EVALUATED");
            }
        }
        if let (Some(time), Some(issuer), Some(responder)) = (reference_time, ocsp_issuer.as_deref(), ocsp_responder.as_deref()) {
            let cert = signatures::extract_single_embedded_certificate(&package, &report.signature_part).map_err(|e| e.to_string())?;
            for (index, response) in ocsps.iter().enumerate() {
                let observation = ibis_ooxml::ocsp::inspect_ocsp_with_nonce(response, &cert, issuer, responder, time, ocsp_nonce.as_deref()).map_err(|e| e.to_string())?;
                println!("  OCSP {} observations (NOT a revocation verdict): protocol {:?}; response type {:?}", index + 1, observation.response_status, observation.response_type);
                if let Some(basic) = observation.basic {
                    println!("    Signature {:?}; responder ID {:?}; target issuer name exact {}", basic.signature, basic.responder_id, basic.issuer_name_exact);
                    println!("    Responder certificate observations (NOT authorization): {:?}", basic.responder_certificate);
                    println!("    Nonce observation (UNAUTHENTICATED; NOT freshness proof): {:?}", basic.nonce);
                    println!("    Response extension inventory (UNTRUSTED; NOT interpreted): {:?}", basic.response_extension_inventory);
                    println!("    Extended-revoke response extension (UNAUTHENTICATED): {:?}", basic.extended_revoke);
                    println!("    Produced at {}; after reference time {}; response extensions present {}; embedded certificates {}; responses {} (truncated: {})", basic.produced_at, basic.produced_after_reference_time, basic.response_extensions_present, basic.embedded_certificates, basic.total_responses, basic.responses_truncated);
                    for (position, single) in basic.responses.iter().enumerate() {
                        println!("    Single-response extension inventory (UNTRUSTED; NOT interpreted): {:?}; extended-revoke at single-response scope {:?}", single.extension_inventory, single.extended_revoke);
                        println!("    Response {}: CertID {:?}; UNAUTHENTICATED claim {:?}; thisUpdate {}; nextUpdate {:?}; window {:?}; extensions present {}", position + 1, single.cert_id, single.claimed_status, single.this_update, single.next_update, single.time_window, single.extensions_present);
                    }
                } else { println!("    Basic response: NOT INSPECTED."); }
                if let Some(request) = ocsp_request.as_deref() {
                    let comparison = ibis_ooxml::ocsp::inspect_ocsp_request(request, response).map_err(|e| e.to_string())?;
                    println!("    Request/response observations (UNTRUSTED; NOT acceptance): {:?}", comparison);
                }
                println!("    Responder authorization, issuer trust, nonce policy, other extension semantics and revocation: NOT EVALUATED");
            }
        }
        let value = match report.signature_value {
            SignatureValueCheck::LegacySha1 { key_bits, matches } => format!("legacy weak RSA-SHA1 ({key_bits} bits): {}; certificate is untrusted", if matches { "matches" } else { "differs" }),
            SignatureValueCheck::RsaPssSha256 { matches } => format!("RSA-PSS SHA-256: {}; certificate is untrusted", if matches { "matches" } else { "differs" }),
            SignatureValueCheck::EcdsaP384 { matches } => format!("ECDSA P-384 SHA-384: {}; certificate is untrusted", if matches { "matches" } else { "differs" }),
            SignatureValueCheck::EcdsaP256 { matches } => format!("ECDSA P-256 SHA-256: {}; certificate is untrusted", if matches { "matches" } else { "differs" }),
            SignatureValueCheck::MatchesUntrustedCertificate => "matches embedded untrusted certificate".to_string(),
            SignatureValueCheck::Differs => "does not match".to_string(), SignatureValueCheck::Unsupported => "not checked (unsupported)".to_string(),
        };
        println!("  Signature value: {value}");
        println!("  Package object digest: {}", match report.package_object {
            ObjectDigest::LegacySha1 { matches: true } => "matches (weak legacy SHA-1)", ObjectDigest::LegacySha1 { matches: false } => "differs (weak legacy SHA-1)",
            ObjectDigest::Matches => "matches", ObjectDigest::Differs => "differs", ObjectDigest::Unsupported => "not checked (unsupported)",
        });
        for item in &report.additional_signed_info_references {
            let status = match item.digest {
                ObjectDigest::Matches => "digest matches",
                ObjectDigest::Differs => "digest differs",
                ObjectDigest::LegacySha1 { matches: true } => "digest matches (weak legacy SHA-1)",
                ObjectDigest::LegacySha1 { matches: false } => "digest differs (weak legacy SHA-1)",
                ObjectDigest::Unsupported => "NOT CHECKED",
            };
            match &item.uri {
                Some(uri) => println!("  Additional SignedInfo reference {uri:?}: {status}"),
                None => println!("  Additional SignedInfo reference without URI: {status}"),
            }
            if let Some(cert) = &item.xades_certificate_digest {
                let status = match cert {
                    ObjectDigest::Matches => "matches embedded certificate",
                    ObjectDigest::Differs => "differs from embedded certificate",
                    ObjectDigest::LegacySha1 { matches: true } => "matches embedded certificate (weak legacy SHA-1)",
                    ObjectDigest::LegacySha1 { matches: false } => "differs from embedded certificate (weak legacy SHA-1)",
                    ObjectDigest::Unsupported => "NOT CHECKED",
                };
                println!("    XAdES SigningCertificateV2 digest: {status}; NOT AUTHENTICATED");
            }
            if let Some(issuer) = &item.xades_issuer_serial {
                match issuer {
                    XadesIssuerSerial::Absent => println!("    XAdES IssuerSerialV2: absent; NOT AUTHENTICATED"),
                    XadesIssuerSerial::Unsupported => println!("    XAdES IssuerSerialV2: NOT CHECKED; NOT AUTHENTICATED"),
                    XadesIssuerSerial::Compared { issuer_der_equal, serial_equal } => println!("    XAdES IssuerSerialV2: issuer DER identical {issuer_der_equal}; serial equal {serial_equal}; NOT AUTHENTICATED; semantic name matching NOT CHECKED"),
                }
            }
        }
        let inventory = &report.manifest_inventory;
        println!("  Manifest inventory (per signature, unauthenticated): {} package entries; {} not literally listed; {} ambiguous entry names; {} unresolved reference URIs.", inventory.entries, inventory.unlisted, inventory.ambiguous_entries, inventory.unresolved_references);
        println!("  Inventory includes metadata and relationship parts; listing or absence does NOT establish coverage or unsigned status.");
        for name in &inventory.unlisted_examples { println!("    Not literally listed: {name:?}"); }
        if inventory.examples_truncated { println!("    Some names or entries omitted from inventory examples."); }
        if report.manifest.is_empty() { println!("  No manifest references found; coverage is not established."); }
        for item in report.manifest {
            let digest = match item.digest { ManifestDigest::LegacySha1 { matches: true } => "matches (weak legacy SHA-1)", ManifestDigest::LegacySha1 { matches: false } => "differs (weak legacy SHA-1)", ManifestDigest::Matches => "matches", ManifestDigest::Differs => "differs", ManifestDigest::Unsupported => "not checked (unsupported)", ManifestDigest::MissingPart => "part missing" };
            let content = match item.content_type { ContentTypeCheck::Matches => "matches", ContentTypeCheck::Differs => "differs", ContentTypeCheck::Missing => "missing", ContentTypeCheck::Unsupported => "not checked (unsupported)" };
            println!("  Reference {:?}: digest {digest}; content type {content}", item.uri);
            match item.relationships {
                Some(RelationshipReference::WholePart) => println!("    Relationship reference declares raw part bytes; NOT AUTHENTICATED."),
                Some(RelationshipReference::Unsupported) => println!("    Relationship selection: NOT CHECKED; NOT AUTHENTICATED."),
                Some(RelationshipReference::Filtered(selection)) => {
                    println!("    Current relationship selection: {} selected, {} not selected; {} distinct ID selectors and {} distinct type selectors without current matches; NOT AUTHENTICATED.", selection.selected, selection.unselected, selection.unmatched_id_selectors, selection.unmatched_type_selectors);
                    for id in &selection.selected_examples { println!("      Selected ID: {id:?}"); }
                    for id in &selection.unselected_examples { println!("      Not selected ID: {id:?}"); }
                    if selection.examples_truncated { println!("      Some IDs or examples omitted."); }
                }
                None => (),
            }
        }
        println!("  Certificate trust: NOT EVALUATED");
    }
    Ok(())
}

fn read_limited(path: &std::ffi::OsStr, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit { return Err(format!("input exceeds {limit} byte limit")); }
    Ok(bytes)
}

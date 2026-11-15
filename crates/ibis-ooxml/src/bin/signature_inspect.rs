// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Inspect signature-related observations, without making a trust decision.

use std::process::ExitCode;
use ibis_ooxml::{Package, signatures::{self, ContentTypeCheck, ManifestDigest, ObjectDigest, SignatureValueCheck}};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("ibis-signature-inspect: {error}"); ExitCode::from(2) }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: ibis-signature-inspect <docx|xlsx|pptx> [--at <unix-seconds>] [--anchor <der>] [--intermediate <der>]")?;
    let mut reference_time: Option<u64> = None;
    let mut anchors = Vec::new();
    let mut intermediates = Vec::new();
    let mut crls = Vec::new();
    let mut crl_issuer = None;
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
            _ => return Err("unknown or repeated option".into()),
        }
    }
    if !intermediates.is_empty() && anchors.is_empty() { return Err("--intermediate requires --anchor".into()); }
    if (!crls.is_empty() || crl_issuer.is_some()) && (crls.is_empty() || crl_issuer.is_none() || reference_time.is_none()) {
        return Err("CRL inspection requires --crl, --crl-issuer, and --at".into());
    }
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
        let value = match report.signature_value {
            SignatureValueCheck::LegacySha1 { key_bits, matches } => format!("legacy weak RSA-SHA1 ({key_bits} bits): {}; certificate is untrusted", if matches { "matches" } else { "differs" }),
            SignatureValueCheck::MatchesUntrustedCertificate => "matches embedded untrusted certificate".to_string(),
            SignatureValueCheck::Differs => "does not match".to_string(), SignatureValueCheck::Unsupported => "not checked (unsupported)".to_string(),
        };
        println!("  Signature value: {value}");
        println!("  Package object digest: {}", match report.package_object {
            ObjectDigest::LegacySha1 { matches: true } => "matches (weak legacy SHA-1)", ObjectDigest::LegacySha1 { matches: false } => "differs (weak legacy SHA-1)",
            ObjectDigest::Matches => "matches", ObjectDigest::Differs => "differs", ObjectDigest::Unsupported => "not checked (unsupported)",
        });
        if report.manifest.is_empty() { println!("  No manifest references found; coverage is not established."); }
        for item in report.manifest {
            let digest = match item.digest { ManifestDigest::LegacySha1 { matches: true } => "matches (weak legacy SHA-1)", ManifestDigest::LegacySha1 { matches: false } => "differs (weak legacy SHA-1)", ManifestDigest::Matches => "matches", ManifestDigest::Differs => "differs", ManifestDigest::Unsupported => "not checked (unsupported)", ManifestDigest::MissingPart => "part missing" };
            let content = match item.content_type { ContentTypeCheck::Matches => "matches", ContentTypeCheck::Differs => "differs", ContentTypeCheck::Missing => "missing", ContentTypeCheck::Unsupported => "not checked (unsupported)" };
            println!("  Reference {:?}: digest {digest}; content type {content}", item.uri);
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

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Opt-in offline experiment; never a document or trust verdict.

use super::{
    CertificationPathSettings, DocumentCandidateResult, TimeOfInterest,
    inspect_document_candidate_paths,
};
use std::{ffi::OsString, fs::File, io::Read, process::ExitCode};

const USAGE: &str = "usage: ibis-certval-path-probe <document> --signature <part> --at <nonzero-unix-seconds> --anchor <der> [--anchor <der>...] [--intermediate <der>...] (--policy-any | --policy <oid>...) [--inhibit-any] [--inhibit-mapping]";

pub(super) fn main() -> ExitCode {
    match run(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ibis-certval-path-probe: {error}");
            ExitCode::from(2)
        }
    }
}

fn read_limited(path: &OsString, limit: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| format!("cannot open input: {e}"))?;
    let mut data = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|e| format!("cannot read input: {e}"))?;
    if data.len() as u64 > limit {
        return Err("input exceeds size limit".into());
    }
    Ok(data)
}

fn text_arg(value: OsString) -> Result<String, String> {
    value
        .into_string()
        .map_err(|_| "option value must be UTF-8".to_string())
}

fn shortened(value: &str) -> String {
    let mut chars = value.chars();
    let mut prefix: String = chars.by_ref().take(200).collect();
    if chars.next().is_some() {
        prefix.push('…');
    }
    prefix
}

fn run(mut args: impl Iterator<Item = OsString>) -> Result<(), String> {
    let document = args.next().ok_or(USAGE)?;
    let mut part = None;
    let mut time = None;
    let mut anchors = Vec::new();
    let mut intermediates = Vec::new();
    let mut any_policy = false;
    let mut policies = Vec::new();
    let mut inhibit_any = false;
    let mut inhibit_mapping = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--signature") if part.is_none() => {
                let value = text_arg(args.next().ok_or("--signature requires a part name")?)?;
                if value.is_empty() || value.len() > 4096 {
                    return Err("invalid signature part name".into());
                }
                part = Some(value);
            }
            Some("--at") if time.is_none() => {
                let value = text_arg(args.next().ok_or("--at requires Unix seconds")?)?;
                let seconds = value.parse::<u64>().map_err(|_| "invalid --at")?;
                if TimeOfInterest::from_unix_secs(seconds).map_or(true, |t| t.is_disabled()) {
                    return Err("--at must be a supported nonzero time; zero disables the underlying time check".into());
                }
                time = Some(seconds);
            }
            Some("--anchor") | Some("--intermediate") => {
                let path = args
                    .next()
                    .ok_or("certificate option requires a DER file")?;
                let cert = read_limited(&path, 1024 * 1024)?;
                if arg == "--anchor" {
                    anchors.push(cert);
                } else {
                    intermediates.push(cert);
                }
                if anchors.len() > 8 || intermediates.len() > 16 {
                    return Err("too many certificate inputs".into());
                }
            }
            Some("--policy-any") if !any_policy => {
                any_policy = true;
            }
            Some("--policy") => {
                let raw = text_arg(args.next().ok_or("--policy requires an OID")?)?;
                if raw.len() > 128 {
                    return Err("policy OID is too long".into());
                }
                let oid = der::asn1::ObjectIdentifier::new(&raw)
                    .map_err(|_| "invalid policy OID")?
                    .to_string();
                if oid == "2.5.29.32.0" {
                    return Err("use --policy-any for anyPolicy".into());
                }
                if policies.contains(&oid) || policies.len() >= 16 {
                    return Err("duplicate or too many policy OIDs".into());
                }
                policies.push(oid);
            }
            Some("--inhibit-any") if !inhibit_any => {
                inhibit_any = true;
            }
            Some("--inhibit-mapping") if !inhibit_mapping => {
                inhibit_mapping = true;
            }
            _ => return Err(format!("unknown or repeated option; {USAGE}")),
        }
    }
    if anchors.is_empty() {
        return Err("at least one explicitly supplied --anchor is required".into());
    }
    if any_policy == !policies.is_empty() {
        return Err("select either --policy-any or one or more --policy OIDs".into());
    }
    let part = part.ok_or("--signature is required")?;
    let time = time.ok_or("--at is required")?;
    if anchors
        .iter()
        .chain(&intermediates)
        .map(Vec::len)
        .sum::<usize>()
        > 4 * 1024 * 1024
    {
        return Err("certificate inputs exceed 4 MiB".into());
    }
    let bytes: std::sync::Arc<[u8]> = read_limited(&document, 128 * 1024 * 1024)?.into();
    let archive = ibis_ooxml::zip::Archive::new(bytes.clone()).map_err(|e| e.to_string())?;
    if archive.entries().len() > 4096 {
        return Err("document has too many ZIP entries".into());
    }
    let mut expanded = 0usize;
    for entry in archive.entries() {
        if entry.size > 16 * 1024 * 1024 {
            return Err("document ZIP entry exceeds 16 MiB expanded size".into());
        }
        expanded = expanded
            .checked_add(entry.size)
            .ok_or("document expanded size is too large")?;
    }
    if expanded > 128 * 1024 * 1024 {
        return Err("document ZIP expanded size exceeds 128 MiB".into());
    }
    let package = ibis_ooxml::Package::open(bytes).map_err(|e| e.to_string())?;
    let anchor_refs: Vec<&[u8]> = anchors.iter().map(Vec::as_slice).collect();
    let intermediate_refs: Vec<&[u8]> = intermediates.iter().map(Vec::as_slice).collect();
    let mut settings = CertificationPathSettings::new();
    if !any_policy {
        settings.set_initial_policy_set(policies.clone());
        settings.set_initial_explicit_policy_indicator(true);
    }
    settings.set_initial_inhibit_any_policy_indicator(inhibit_any);
    settings.set_initial_policy_mapping_inhibit_indicator(inhibit_mapping);
    let report = inspect_document_candidate_paths(
        &package,
        &part,
        &anchor_refs,
        &intermediate_refs,
        time,
        settings,
    )
    .map_err(|e| format!("inspection could not complete: {e:?}"))?;
    println!(
        "EXPERIMENT ONLY: no overall document validity, revocation, trust, timestamp or jurisdiction decision."
    );
    println!(
        "Exit status 0 means observation completed, NOT that the document or any certificate is valid."
    );
    println!(
        "Offline only: no network, system trust store, OCSP, CRL or trusted timestamp validation."
    );
    println!(
        "Inputs: time {time}; {} caller-supplied anchor(s); {} intermediate(s); policy {:?}; explicit policy required {}; inhibit anyPolicy {}; inhibit mapping {}.",
        anchors.len(),
        intermediates.len(),
        if any_policy {
            vec!["anyPolicy".to_string()]
        } else {
            policies
        },
        !any_policy,
        inhibit_any,
        inhibit_mapping
    );
    println!("Signature part: {:?}", shortened(&part));
    println!(
        "XML signature against untrusted embedded certificate: {:?}",
        report.xml_signature
    );
    println!(
        "Signed package object digest: {:?}; NOT a coverage verdict",
        report.package_object_digest
    );
    println!(
        "Manifest references: {}; examples at most 16; NOT a complete coverage verdict",
        report.manifest_references.len()
    );
    for (index, item) in report.manifest_references.iter().take(16).enumerate() {
        println!(
            "  Reference {}: {:?}; digest {:?}; content type {:?}; relationship transform {:?}",
            index + 1,
            shortened(&item.uri),
            item.digest,
            item.content_type,
            item.relationships
        );
    }
    if report.manifest_references.len() > 16 {
        println!("  Additional references omitted.");
    }
    let fingerprint: String = report
        .signer_fingerprint
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!("Signer SHA-256: {fingerprint}; identity and trust NOT established.");
    println!(
        "Candidate paths: {} (search truncated: {}); exact-name search can omit paths.",
        report.candidates.len(),
        report.candidate_search_truncated
    );
    for (index, candidate) in report.candidates.iter().enumerate() {
        println!(
            "  Candidate {} (signer toward caller-supplied anchor):",
            index + 1
        );
        for hash in &candidate.fingerprints {
            println!(
                "    {}",
                hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
            );
        }
        match &candidate.result {
            DocumentCandidateResult::AnchorIsSignerNotEvaluated => {
                println!("    Anchor equals signer: NOT EVALUATED")
            }
            DocumentCandidateResult::Evaluated(path) => {
                match &path.engine_result {
                    Ok(()) => println!(
                        "    Path engine: PASS under supplied inputs; revocation and trust NOT established"
                    ),
                    Err(error) => println!(
                        "    Path engine: DID NOT PASS OR INDETERMINATE: {:?}",
                        shortened(error)
                    ),
                }
                println!(
                    "    Edge observations (anchor to signer; NOT path or trust verdict): {:?}",
                    path.edge_observations
                );
            }
        }
    }
    println!(
        "No candidate, a failed candidate or a truncated search does not prove that no valid path exists."
    );
    Ok(())
}

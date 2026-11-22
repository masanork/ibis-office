// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::io::{Read, Seek, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use wait_timeout::ChildExt;

/// Validate and print the existing CLI JSON report.
pub fn run(input: &Path, profile: &str, validator: &Path) -> Result<bool, String> {
    let (compliant, report) = validate(input, profile, validator)?;
    writeln!(std::io::stdout().lock(), "{report:#}").map_err(|error| error.to_string())?;
    Ok(compliant)
}

/// Run veraPDF against the requested profile and return a parsed result.
pub fn validate(input: &Path, profile: &str, validator: &Path) -> Result<(bool, Value), String> {
    let input = std::path::absolute(input).map_err(|error| error.to_string())?;
    if !input.is_file() {
        return Err(format!("{}: expected a PDF file", input.display()));
    }
    let mut header = Vec::new();
    std::fs::File::open(&input)
        .and_then(|file| file.take(1024).read_to_end(&mut header))
        .map_err(|error| format!("{}: {error}", input.display()))?;
    if !header.windows(5).any(|bytes| bytes == b"%PDF-") {
        return Err(format!("{}: PDF header missing", input.display()));
    }

    let mut output = tempfile::tempfile().map_err(|error| error.to_string())?;
    let stdout = output.try_clone().map_err(|error| error.to_string())?;
    let mut child = Command::new(validator)
        .args([
            "--format",
            "xml",
            "--flavour",
            profile,
            "--maxfailuresdisplayed",
            "1",
        ])
        .arg(input)
        .stdin(Stdio::null())
        .stdout(stdout)
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", validator.display()))?;
    let status = match child.wait_timeout(Duration::from_secs(90)) {
        Ok(Some(status)) => status,
        result => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(match result {
                Ok(None) => "veraPDF timed out after 90 seconds".into(),
                Err(error) => format!("cannot wait for veraPDF: {error}"),
                Ok(Some(_)) => unreachable!(),
            });
        }
    };
    // veraPDF 1.30 exits with 1 for a completed non-compliant validation.
    if !status.success() && status.code() != Some(1) {
        return Err(format!("veraPDF exited with {status}"));
    }

    const REPORT_LIMIT: u64 = 8 * 1024 * 1024;
    output.rewind().map_err(|error| error.to_string())?;
    let mut xml = String::new();
    output
        .take(REPORT_LIMIT + 1)
        .read_to_string(&mut xml)
        .map_err(|error| format!("cannot read veraPDF report: {error}"))?;
    if xml.len() as u64 > REPORT_LIMIT {
        return Err("veraPDF report exceeds 8 MiB".into());
    }

    let (compliant, report) = parse_report(&xml, profile)?;
    if compliant && !status.success() {
        return Err(format!(
            "veraPDF exited with {status} despite reporting compliance"
        ));
    }
    Ok((compliant, report))
}

fn parse_report(xml: &str, profile: &str) -> Result<(bool, Value), String> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| format!("invalid veraPDF report: {error}"))?;
    let root = document.root_element();
    if !root.has_tag_name("report") {
        return Err("unexpected veraPDF report format".into());
    }
    let mut jobs = root
        .children()
        .filter(|node| node.has_tag_name("jobs"))
        .flat_map(|node| node.children().filter(|child| child.has_tag_name("job")));
    let job = jobs
        .next()
        .ok_or("veraPDF did not report a validation job")?;
    if jobs.next().is_some() {
        return Err("veraPDF reported multiple jobs for one input file".into());
    }
    let mut reports = job
        .children()
        .filter(|node| node.has_tag_name("validationReport"));
    let report = reports
        .next()
        .ok_or("veraPDF could not validate the input PDF")?;
    if reports.next().is_some() {
        return Err("veraPDF reported multiple validation results for one input file".into());
    }
    if report
        .attribute("jobEndStatus")
        .is_some_and(|status| status != "normal")
    {
        return Err("veraPDF did not complete validation".into());
    }
    let profile_name = format!("PDF/A-{profile}");
    let expected = format!("{profile_name} validation profile");
    if report.attribute("profileName") != Some(expected.as_str()) {
        return Err(format!("veraPDF did not validate against {profile_name}"));
    }
    let compliant = match report.attribute("isCompliant") {
        Some("true") => true,
        Some("false") => false,
        _ => return Err("veraPDF did not report a conformance result".into()),
    };
    let details = report
        .children()
        .find(|node| node.has_tag_name("details"))
        .ok_or("veraPDF did not report validation details")?;
    let failed_rules: Vec<_> = details.children()
        .filter(|node| node.has_tag_name("rule") && node.attribute("status") == Some("failed"))
        .map(|rule| {
            let description = rule.children().find(|child| child.has_tag_name("description"))
                .and_then(|node| node.text());
            let context = rule.children().find(|child| child.has_tag_name("check"))
                .and_then(|check| check.children().find(|node| node.has_tag_name("context")))
                .and_then(|node| node.text());
            json!({
                "specification": rule.attribute("specification"),
                "clause": rule.attribute("clause"),
                "test_number": rule.attribute("testNumber"),
                "description": description,
                "first_context": context,
                "failed_checks": rule.attribute("failedChecks").and_then(|count| count.parse::<u64>().ok()),
            })
        })
        .collect();
    if details
        .attribute("failedRules")
        .and_then(|count| count.parse::<usize>().ok())
        != Some(failed_rules.len())
        || compliant != failed_rules.is_empty()
    {
        return Err("veraPDF reported inconsistent failed rules".into());
    }
    Ok((
        compliant,
        json!({
            "validator": "veraPDF",
            "profile": profile_name,
            "validation": if compliant { "compliant" } else { "non_compliant" },
            "failed_rules": failed_rules,
        }),
    ))
}

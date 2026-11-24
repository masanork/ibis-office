// SPDX-License-Identifier: AGPL-3.0-only
//! Offline comparison of synthetic employment disclosures. The keys are public fixtures.

use ibis_doc_model::SlideId;
use ibis_doc_model::sd_jwt::{Es256KeyPair, SdJwtOptions, issue_sd_jwt};
use ibis_doc_model::sd_jwt_binding::{BindingSource, BoundClaim, ClaimValue, SheetClaimBindings};
use ibis_doc_model::sd_jwt_import::{parse_sd_jwt, parse_single_mdoc_text};
use ibis_doc_model::sd_jwt_mdoc::{MdocOptions, issue_mdoc};
use serde_json::{Value, json};

const AUDIENCE: &str = "https://verifier.example";
const NONCE: &str = "childcare-application-1";

fn binding(path: &str, value: &Value, disclose: bool) -> BoundClaim {
    BoundClaim {
        path: path.split('.').map(str::to_owned).collect(),
        path_str: path.to_owned(),
        value: ClaimValue::parse_json(&value.to_string()).expect("synthetic input is JSON"),
        selective: true,
        disclose,
        source: BindingSource::Cell { row: 0, column: 0 },
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input: Value = serde_json::from_reader(std::io::stdin().lock())?;
    let record = &input["record"];
    let issuer = Es256KeyPair::demo_issuer_key();
    let holder = Es256KeyPair::demo_holder_key();
    let issuer_jwk = issuer.to_public_jwk().to_json();

    let mut results = Vec::new();
    for (format, total_path, days_path) in [
        ("sd-jwt", "work_schedule.total", "work_schedule.days"),
        ("mdoc", "work_schedule_total", "work_schedule_days"),
    ] {
        let query = &input[format]["credentials"][0];
        let expected = ["certification_date", total_path];
        let bindings = SheetClaimBindings {
            slide: SlideId(1),
            claims: vec![
                binding(expected[0], &record["certification_date"], true),
                binding(expected[1], &record["work_schedule"]["total"], true),
                binding("employment_type", &record["employment_type"], false),
                binding(days_path, &record["work_schedule"]["days"], false),
            ],
            disclosures: Default::default(),
        };
        assert_eq!(query["require_cryptographic_holder_binding"], true);
        let mut incomplete = bindings.clone();
        incomplete.claims[1].disclose = false;
        let (token, incomplete_token, full_token, namespace) = if format == "sd-jwt" {
            assert_eq!(query["format"], "dc+sd-jwt");
            assert_eq!(
                query["claims"],
                json!([
                    {"path": ["certification_date"]},
                    {"path": ["work_schedule", "total"]},
                ])
            );
            let options = SdJwtOptions {
                default_vct: Some(
                    query["meta"]["vct_values"][0]
                        .as_str()
                        .ok_or("missing vct")?
                        .into(),
                ),
                holder_public_jwk: Some(holder.to_public_jwk()),
                ..Default::default()
            };
            let output = issue_sd_jwt(&bindings, &options, &issuer);
            (
                output
                    .attach_key_binding(AUDIENCE, NONCE, 1_700_000_000, &holder)
                    .vp_token,
                issue_sd_jwt(&incomplete, &options, &issuer)
                    .attach_key_binding(AUDIENCE, NONCE, 1_700_000_000, &holder)
                    .vp_token,
                output.sd_jwt_all,
                None,
            )
        } else {
            assert_eq!(query["format"], "mso_mdoc");
            let namespace = query["claims"][0]["path"][0]
                .as_str()
                .ok_or("missing namespace")?;
            assert_eq!(
                query["claims"],
                json!([
                    {"path": [namespace, "certification_date"], "intent_to_retain": false},
                    {"path": [namespace, total_path], "intent_to_retain": false},
                ])
            );
            let options = MdocOptions {
                doc_type: query["meta"]["doctype_value"]
                    .as_str()
                    .ok_or("missing doctype")?
                    .into(),
                name_space: namespace.into(),
            };
            let output = issue_mdoc(&bindings, &options, &issuer);
            (
                output.base64url_presentation,
                issue_mdoc(&incomplete, &options, &issuer).base64url_presentation,
                output.base64url_all,
                Some(namespace),
            )
        };

        let parse = |text| {
            if format == "sd-jwt" {
                parse_sd_jwt(text, Some(&issuer_jwk))
            } else {
                parse_single_mdoc_text(text, Some(&issuer_jwk))
            }
        };
        let full = parse(&full_token)?;
        assert!(
            full.claims
                .iter()
                .any(|claim| claim.path == "employment_type")
        );
        assert!(full.claims.iter().any(|claim| claim.path == days_path));

        let parsed = parse(&token)?;
        assert_eq!(parsed.signature_valid, Some(true));
        let mut actual = serde_json::Map::new();
        for claim in parsed
            .claims
            .iter()
            .filter(|claim| !matches!(claim.path.as_str(), "iss" | "vct"))
        {
            if let Some(namespace) = namespace {
                assert_eq!(
                    claim
                        .mdoc_element
                        .as_ref()
                        .map(|(ns, element)| (ns.as_str(), element.as_str())),
                    Some((namespace, claim.path.as_str()))
                );
            }
            assert!(
                actual
                    .insert(
                        claim.path.clone(),
                        serde_json::from_str(&claim.value.to_json())?
                    )
                    .is_none()
            );
        }
        assert_eq!(actual.len(), expected.len());
        assert_eq!(actual[expected[0]], record["certification_date"]);
        assert_eq!(actual[expected[1]], record["work_schedule"]["total"]);

        let id = query["id"].as_str().ok_or("missing DCQL credential ID")?;
        let report = |token: &str, dcql: &Value, audience, nonce| {
            ibis_doc_service::compare_presentation(&json!({
                "format": format, "issuer_jwk": serde_json::from_str::<Value>(&issuer_jwk).unwrap(),
                "vp_token": {(id): [token]}, "dcql_query": dcql, "audience": audience, "nonce": nonce,
            }))
        };
        let dcql = &input[format];
        let matched = report(&token, dcql, AUDIENCE, NONCE)?;
        assert_eq!(matched["dcql"]["credential_type_matches"], true);
        assert_eq!(matched["vp_token_credential_id"], id);
        let missing = report(&incomplete_token, dcql, AUDIENCE, NONCE)?;
        let extra = report(&full_token, dcql, AUDIENCE, NONCE)?;
        let rendered_path = |element| match namespace {
            Some(namespace) => format!("/{namespace}/{element}"),
            None => element,
        };
        assert_eq!(
            missing["missing_paths"],
            json!([rendered_path(total_path.to_owned())])
        );
        assert_eq!(missing["unexpected_paths"], json!([]));
        assert_eq!(extra["missing_paths"], json!([]));
        assert_eq!(
            extra["unexpected_paths"],
            json!([
                rendered_path("employment_type".to_owned()),
                rendered_path(days_path.to_owned())
            ])
        );
        if format == "sd-jwt" {
            let mut literal_dot = dcql.clone();
            literal_dot["credentials"][0]["claims"][1]["path"] = json!(["work_schedule.total"]);
            let result = report(&token, &literal_dot, AUDIENCE, NONCE)?;
            assert_eq!(result["missing_paths"], json!([r"work_schedule\.total"]));
            assert_eq!(result["unexpected_paths"], json!(["work_schedule.total"]));
        }
        assert_eq!(matched["issuer_signature_valid"], true);
        assert_eq!(matched["supplied_disclosure_digests_match"], true);
        assert_eq!(matched["missing_paths"], json!([]));
        assert_eq!(matched["unexpected_paths"], json!([]));
        let other_recipient = report(&token, dcql, "https://other-verifier.example", NONCE)?;
        let other_application = report(&token, dcql, AUDIENCE, "childcare-application-2")?;
        if format == "sd-jwt" {
            for key in [
                "holder_signature_valid",
                "holder_sd_hash_valid",
                "audience_matches",
                "nonce_matches",
            ] {
                assert_eq!(matched[key], true, "{key}");
            }
            assert_eq!(other_recipient["audience_matches"], false);
            assert_eq!(other_recipient["nonce_matches"], true);
            assert_eq!(other_application["nonce_matches"], false);
            assert_eq!(other_application["audience_matches"], true);
        } else {
            assert_eq!(matched["device_authentication"], "not_checked");
            assert_eq!(other_recipient["audience_matches"], Value::Null);
            assert_eq!(other_application["nonce_matches"], Value::Null);
        }
        results.push(json!({
            "format": format, "certification_date": actual[expected[0]], "total": actual[expected[1]],
            "issuer_signature_valid": matched["issuer_signature_valid"],
            "disclosure_digests_verified": matched["disclosure_digests_verified"],
            "missing_when_omitted": missing["missing_paths"],
            "unexpected_when_fully_disclosed": extra["unexpected_paths"],
            "other_recipient_matches": other_recipient["audience_matches"],
            "other_application_matches": other_application["nonce_matches"],
            "device_authentication": matched["device_authentication"],
        }));
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}

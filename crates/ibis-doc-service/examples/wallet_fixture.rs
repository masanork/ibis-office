// SPDX-License-Identifier: AGPL-3.0-only
//! Offline synthetic Wallet for the presentation diagnostics regression.

use ibis_doc_model::SlideId;
use ibis_doc_model::sd_jwt::{Es256KeyPair, SdJwtOptions, issue_sd_jwt};
use ibis_doc_model::sd_jwt_binding::{BindingSource, BoundClaim, ClaimValue, SheetClaimBindings};
use ibis_doc_model::sd_jwt_mdoc::{MdocOptions, issue_mdoc};
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (format, scenario, wrap) = match args.as_slice() {
        [format, scenario] => (format, scenario, false),
        [format, scenario, flag] if flag == "--vp-token" => (format, scenario, true),
        _ => {
            return Err(
                "usage: wallet_fixture <sd-jwt|mdoc> <normal|extra|wrong-nonce> [--vp-token]"
                    .into(),
            );
        }
    };
    let extra = match scenario.as_str() {
        "normal" | "wrong-nonce" => false,
        "extra" => true,
        _ => return Err("unknown scenario".into()),
    };
    let claims = [
        ("age_over_18", ClaimValue::Bool(true), true),
        ("birth_date", ClaimValue::String("2000-01-01".into()), extra),
    ]
    .into_iter()
    .map(|(name, value, disclose)| BoundClaim {
        path: vec![name.into()],
        path_str: name.into(),
        selective: true,
        value,
        disclose,
        source: BindingSource::Cell { row: 0, column: 0 },
    })
    .collect();
    let bindings = SheetClaimBindings {
        slide: SlideId(1),
        claims,
        disclosures: Default::default(),
    };
    let issuer = Es256KeyPair::demo_issuer_key();
    let holder = Es256KeyPair::demo_holder_key();
    let audience = "https://verifier.example";
    let token = match format.as_str() {
        "sd-jwt" => {
            let options = SdJwtOptions {
                holder_public_jwk: Some(holder.to_public_jwk()),
                ..Default::default()
            };
            let nonce = if scenario == "wrong-nonce" {
                "another-session"
            } else {
                "age-check"
            };
            issue_sd_jwt(&bindings, &options, &issuer)
                .attach_key_binding(audience, nonce, 1_700_000_000, &holder)
                .vp_token
        }
        "mdoc" if scenario != "wrong-nonce" => {
            issue_mdoc(&bindings, &MdocOptions::default(), &issuer).base64url_presentation
        }
        _ => return Err("unsupported format or scenario".into()),
    };
    let public_key: Value = serde_json::from_str(&issuer.to_public_jwk().to_json())?;
    let mut input = json!({
        "format": format, "issuer_jwk": public_key, "nonce": "age-check", "audience": audience,
    });
    if wrap {
        let (credential_format, meta, path) = if format == "mdoc" {
            let options = MdocOptions::default();
            (
                "mso_mdoc",
                json!({"doctype_value": options.doc_type}),
                json!([options.name_space, "age_over_18"]),
            )
        } else {
            (
                "dc+sd-jwt",
                json!({"vct_values": [SdJwtOptions::default().default_vct]}),
                json!(["age_over_18"]),
            )
        };
        input["vp_token"] = json!({"age_check": [token]});
        input["dcql_query"] = json!({"credentials": [{
            "id": "age_check", "format": credential_format, "meta": meta,
            "claims": [{"path": path, "values": [true]}]
        }]});
    } else {
        input["token"] = json!(token);
        input["expected_paths"] = json!(["age_over_18"]);
    }
    println!("{input}");
    Ok(())
}

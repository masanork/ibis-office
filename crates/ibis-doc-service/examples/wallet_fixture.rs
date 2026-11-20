// SPDX-License-Identifier: AGPL-3.0-only
//! Offline synthetic Wallet for the presentation diagnostics regression.

use ibis_doc_model::SlideId;
use ibis_doc_model::sd_jwt::{Es256KeyPair, SdJwtOptions, issue_sd_jwt};
use ibis_doc_model::sd_jwt_binding::{BindingSource, BoundClaim, ClaimValue, SheetClaimBindings};
use ibis_doc_model::sd_jwt_mdoc::{MdocOptions, issue_mdoc};
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [format, scenario] = args.as_slice() else {
        return Err("usage: wallet_fixture <sd-jwt|mdoc> <normal|extra|wrong-nonce>".into());
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
    println!(
        "{}",
        json!({
            "format": format, "token": token, "expected_paths": ["age_over_18"],
            "issuer_jwk": public_key, "nonce": "age-check", "audience": audience,
        })
    );
    Ok(())
}

// SPDX-License-Identifier: AGPL-3.0-only
//! Generate fictitious training records to compare in the structured preview.

use ibis_doc_model::SlideId;
use ibis_doc_model::sd_jwt::{Es256KeyPair, SdJwtOptions, issue_sd_jwt};
use ibis_doc_model::sd_jwt_binding::{BindingSource, BoundClaim, ClaimValue, SheetClaimBindings};
use ibis_doc_model::sd_jwt_mdoc::{MdocOptions, issue_mdoc};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("provide an output directory")?;
    std::fs::create_dir_all(&folder)?;
    let pairs = [
        ("研修名", "安全研修（サンプル）", true),
        ("修了日", "2026-04-01", true),
        ("管理番号", "00123", false),
    ];
    let record = serde_json::Value::Object(
        pairs
            .iter()
            .map(|(name, value, _)| {
                (
                    name.to_string(),
                    serde_json::Value::String(value.to_string()),
                )
            })
            .collect(),
    );
    let xml = format!(
        "<修了記録>\n{}</修了記録>\n",
        pairs
            .iter()
            .map(|(name, value, _)| format!("  <{name}>{value}</{name}>\n"))
            .collect::<String>()
    );
    let bindings = SheetClaimBindings {
        slide: SlideId(1),
        claims: pairs
            .iter()
            .map(|(name, value, disclose)| BoundClaim {
                path: vec![name.to_string()],
                path_str: name.to_string(),
                selective: true,
                value: ClaimValue::String(value.to_string()),
                disclose: *disclose,
                source: BindingSource::Cell { row: 0, column: 0 },
            })
            .collect(),
        disclosures: Default::default(),
    };
    let issuer = Es256KeyPair::demo_issuer_key();
    let sd_jwt = issue_sd_jwt(
        &bindings,
        &SdJwtOptions {
            default_vct: Some("urn:example:training-completion".into()),
            ..Default::default()
        },
        &issuer,
    );
    let mdoc = issue_mdoc(
        &bindings,
        &MdocOptions {
            doc_type: "org.example.training".into(),
            name_space: "org.example.training".into(),
        },
        &issuer,
    );
    std::fs::write(
        folder.join("training.json"),
        serde_json::to_string_pretty(&record)? + "\n",
    )?;
    std::fs::write(folder.join("training.xml"), xml)?;
    std::fs::write(
        folder.join("training.sd-jwt"),
        sd_jwt.sd_jwt_presentation + "\n",
    )?;
    std::fs::write(
        folder.join("training.mdoc"),
        mdoc.base64url_presentation + "\n",
    )?;
    Ok(())
}

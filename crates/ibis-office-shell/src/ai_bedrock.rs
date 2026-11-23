// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::percent_encode_component;

pub(super) fn messages_url(region: &str) -> Result<String, String> {
    if region.is_empty()
        || !region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || region.starts_with('-')
        || region.ends_with('-')
    {
        return Err("AWS リージョンの形式が無効です。".into());
    }
    let suffix = if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else {
        "amazonaws.com"
    };
    Ok(format!(
        "https://bedrock-runtime.{region}.{suffix}/anthropic/v1/messages"
    ))
}

fn aws(args: &[&str]) -> Result<Output, String> {
    Command::new("aws")
        .args([
            "--no-cli-pager",
            "--no-cli-auto-prompt",
            "--cli-connect-timeout",
            "10",
            "--cli-read-timeout",
            "20",
        ])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "Bedrock の AWS プロファイル認証には AWS CLI v2 が必要です。".into()
            } else {
                format!("AWS CLI を起動できませんでした ({})。", error.kind())
            }
        })
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Credentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

pub(super) fn profile_connection(region: Option<String>) -> Result<(String, String), String> {
    let region = match region {
        Some(region) => region,
        None => {
            let output = aws(&["configure", "get", "region"])?;
            if !output.status.success() {
                return Err(
                    "AWS_REGION または AWS プロファイルの region を設定してください。".into(),
                );
            }
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };
    let url = messages_url(&region)?;
    let output = aws(&[
        "configure",
        "export-credentials",
        "--format",
        "process",
        "--region",
        &region,
    ])?;
    if !output.status.success() {
        return Err("AWS プロファイルの認証情報を取得できませんでした。SSO の場合は aws sso login --profile <プロファイル名> を実行してください。".into());
    }
    let credentials: Credentials = serde_json::from_slice(&output.stdout)
        .map_err(|_| "AWS CLI の認証情報を読み取れませんでした。".to_string())?;
    let timestamp = time::OffsetDateTime::now_utc()
        .format(time::macros::format_description!(
            "[year][month][day]T[hour][minute][second]Z"
        ))
        .map_err(|_| "AWS 署名用の現在時刻を取得できませんでした。".to_string())?;
    Ok((url, bearer_token(&credentials, &region, &timestamp)))
}

/// Sign the CallWithBearerToken request used by AWS's Bedrock token generator.
fn bearer_token(credentials: &Credentials, region: &str, timestamp: &str) -> String {
    fn mac(key: &[u8], value: &str) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
        mac.update(value.as_bytes());
        mac.finalize().into_bytes().into()
    }

    let date = &timestamp[..8];
    let scope = format!("{date}/{region}/bedrock/aws4_request");
    let credential = percent_encode_component(&format!("{}/{scope}", credentials.access_key_id));
    let mut query = format!(
        "Action=CallWithBearerToken&X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={credential}&X-Amz-Date={timestamp}&X-Amz-Expires=300",
    );
    if let Some(token) = &credentials.session_token {
        query.push_str("&X-Amz-Security-Token=");
        query.push_str(&percent_encode_component(token));
    }
    query.push_str("&X-Amz-SignedHeaders=host");
    let host = "bedrock.amazonaws.com";
    let payload_hash = Sha256::digest([]);
    let request = format!("POST\n/\n{query}\nhost:{host}\n\nhost\n{payload_hash:x}");
    let request_hash = Sha256::digest(request.as_bytes());
    let message = format!("AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{request_hash:x}");
    let date_key = mac(
        format!("AWS4{}", credentials.secret_access_key).as_bytes(),
        date,
    );
    let region_key = mac(&date_key, region);
    let service_key = mac(&region_key, "bedrock");
    let signing_key = mac(&service_key, "aws4_request");
    let signature = mac(&signing_key, &message)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let signed = format!("{host}/?{query}&X-Amz-Signature={signature}&Version=1");
    format!(
        "bedrock-api-key-{}",
        base64::engine::general_purpose::STANDARD.encode(signed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bedrock_signature_matches_botocore_with_session_credentials() {
        let credentials = Credentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "example-secret".into(),
            session_token: Some("sample+/=".into()),
        };
        let token = bearer_token(&credentials, "ap-northeast-1", "20261123T000000Z");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(token.strip_prefix("bedrock-api-key-").unwrap())
            .unwrap();
        let signed = std::str::from_utf8(&decoded).unwrap();
        assert!(signed.contains("X-Amz-Security-Token=sample%2B%2F%3D"));
        assert!(signed.ends_with(concat!(
            "&X-Amz-Signature=03e9988d1a9f3942817b52b61df3edecc1bf858239c76463e2aaf7fb81f50407",
            "&Version=1"
        )));
    }
}

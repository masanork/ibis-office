// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use serde::Deserialize;

use super::percent_encode_component;

pub(super) fn messages_url(region: &str) -> Result<String, String> {
    Ok(format!("{}/anthropic/v1/messages", runtime_url(region)?))
}

fn runtime_url(region: &str) -> Result<String, String> {
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
    Ok(format!("https://bedrock-runtime.{region}.{suffix}"))
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

fn profile_credentials(region: Option<String>) -> Result<(String, Credentials), String> {
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
    runtime_url(&region)?;
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
    Ok((region, credentials))
}

pub(super) fn check_profile(region: Option<String>) -> Result<String, String> {
    profile_credentials(region).map(|(region, _)| region)
}

pub(super) fn stream_profile(
    region: Option<String>,
    model: &str,
    payload: &[u8],
    mut emit: impl FnMut(String) -> bool,
) -> Result<(), String> {
    let (region, credentials) = profile_credentials(region)?;
    let url = format!(
        "{}/model/{}/invoke-with-response-stream",
        runtime_url(&region)?,
        percent_encode_component(model)
    );
    let mut request: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|error| format!("Bedrock のリクエストを読み取れませんでした: {error}"))?;
    let fields = request
        .as_object_mut()
        .expect("Messages request is an object");
    fields.remove("model");
    fields.remove("stream");
    fields.insert("anthropic_version".into(), "bedrock-2023-05-31".into());

    let quote = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
    let mut config = format!(
        "user = \"{}:{}\"\ndata = \"{}\"\n",
        quote(&credentials.access_key_id),
        quote(&credentials.secret_access_key),
        quote(&request.to_string()),
    );
    if let Some(token) = &credentials.session_token {
        config.push_str(&format!(
            "header = \"X-Amz-Security-Token: {}\"\n",
            quote(token)
        ));
    }
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--fail",
            "--no-buffer",
            "--proto",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "180",
            "--aws-sigv4",
            &format!("aws:amz:{region}:bedrock"),
            "--header",
            "Content-Type: application/json",
            "--header",
            "Accept: application/vnd.amazon.eventstream",
            "--header",
            "X-Amzn-Bedrock-Accept: application/json",
            "--config",
            "-",
            "--globoff",
            "--url",
            &url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("curl を起動できませんでした: {error}"))?;
    let result: Result<bool, String> = (|| {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(config.as_bytes())
            .map_err(|error| format!("Bedrock のリクエストを送信できませんでした: {error}"))?;
        let mut reader = std::io::BufReader::new(child.stdout.take().expect("piped stdout"));
        while let Some(event) = read_event(&mut reader)? {
            if !emit(format!("data: {event}")) {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    if !matches!(result, Ok(true)) {
        let _ = child.kill();
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if result? && !output.status.success() {
        return Err(format!(
            "Bedrock の SigV4 リクエストに失敗しました: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn read_event(reader: &mut impl Read) -> Result<Option<String>, String> {
    let mut prelude = [0; 12];
    let read_error = |error| format!("Bedrock のイベントストリームを読み取れませんでした: {error}");
    if reader.read(&mut prelude[..1]).map_err(read_error)? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut prelude[1..]).map_err(read_error)?;
    let word = |bytes: &[u8]| u32::from_be_bytes(bytes.try_into().expect("four bytes"));
    let length = word(&prelude[..4]) as usize;
    let headers = word(&prelude[4..8]) as usize;
    if !(16..=16 * 1024 * 1024).contains(&length)
        || headers > length - 16
        || crc32fast::hash(&prelude[..8]) != word(&prelude[8..])
    {
        return Err("Bedrock のイベントヘッダーが無効です。".into());
    }
    let mut frame = vec![0; length - 12];
    reader.read_exact(&mut frame).map_err(read_error)?;
    let crc_start = frame.len() - 4;
    let mut crc = crc32fast::Hasher::new();
    crc.update(&prelude);
    crc.update(&frame[..crc_start]);
    if crc.finalize() != word(&frame[crc_start..]) {
        return Err("Bedrock のイベントのチェックサムが一致しません。".into());
    }
    let wrapper: serde_json::Value = serde_json::from_slice(&frame[headers..crc_start])
        .map_err(|_| "Bedrock のイベントが無効です。".to_string())?;
    let encoded = wrapper["bytes"].as_str().ok_or_else(|| {
        wrapper["message"]
            .as_str()
            .unwrap_or("Bedrock から推論エラーが返りました。")
            .to_string()
    })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "Bedrock のイベントを復号できませんでした。".to_string())?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "Bedrock のイベントが有効な UTF-8 ではありません。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_bedrock_event_frames_and_rejects_corruption() {
        let event = r#"{"type":"message_stop"}"#;
        let body = serde_json::json!({
            "bytes": base64::engine::general_purpose::STANDARD.encode(event)
        })
        .to_string();
        let mut frame = Vec::new();
        frame.extend((16 + body.len() as u32).to_be_bytes());
        frame.extend(0u32.to_be_bytes());
        frame.extend(crc32fast::hash(&frame).to_be_bytes());
        frame.extend(body.as_bytes());
        frame.extend(crc32fast::hash(&frame).to_be_bytes());
        assert_eq!(
            read_event(&mut frame.as_slice()).unwrap().as_deref(),
            Some(event)
        );
        frame[12] ^= 1;
        assert!(read_event(&mut frame.as_slice()).is_err());
    }
}

// SPDX-License-Identifier: AGPL-3.0-only

use serde_json::Value;

use crate::ToolError;

pub(crate) struct DirectPost {
    pub vp_token: Value,
    pub state: Option<String>,
}

fn decode_component(input: &str) -> Result<String, ToolError> {
    let mut bytes = input.bytes();
    let mut decoded = Vec::with_capacity(input.len());
    while let Some(byte) = bytes.next() {
        decoded.push(match byte {
            b'+' => b' ',
            b'%' => {
                let mut digit = || {
                    bytes
                        .next()
                        .and_then(|byte| (byte as char).to_digit(16))
                        .map(|digit| digit as u8)
                        .ok_or_else(|| {
                            ToolError::invalid("invalid percent escape in direct_post body")
                        })
                };
                (digit()? << 4) | digit()?
            }
            byte => byte,
        });
    }
    String::from_utf8(decoded)
        .map_err(|_| ToolError::invalid("direct_post body must decode to UTF-8"))
}

pub(crate) fn parse(body: &str) -> Result<DirectPost, ToolError> {
    // The decoded presentation has a separate, lower size limit.
    if body.len() > 4 * 1024 * 1024 {
        return Err(ToolError::invalid("direct_post body exceeds 4 MiB"));
    }
    let mut vp_token = None;
    let mut state = None;
    for parameter in body.split('&') {
        let (name, value) = parameter
            .split_once('=')
            .ok_or_else(|| ToolError::invalid("malformed direct_post parameter"))?;
        let name = decode_component(name)?;
        match name.as_str() {
            "vp_token" if vp_token.is_none() => vp_token = Some(decode_component(value)?),
            "state" if state.is_none() => state = Some(decode_component(value)?),
            "vp_token" | "state" => {
                return Err(ToolError::invalid("duplicate direct_post parameter"));
            }
            "response" => return Err(ToolError::invalid("encrypted response is unsupported")),
            "error" => return Err(ToolError::invalid("Wallet returned an authorization error")),
            _ => return Err(ToolError::invalid("unsupported direct_post parameter")),
        }
    }
    let token = vp_token.ok_or_else(|| ToolError::invalid("direct_post requires vp_token"))?;
    let vp_token = serde_json::from_str(&token)
        .map_err(|_| ToolError::invalid("direct_post vp_token must be JSON"))?;
    Ok(DirectPost { vp_token, state })
}

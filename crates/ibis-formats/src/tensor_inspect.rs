// SPDX-License-Identifier: AGPL-3.0-only
//! Pure-Rust metadata and shape inspector for NumPy (`.npy`/`.npz`) and SafeTensors (`.safetensors`).

use crate::Error;
use serde_json::{Value, json};

const MAX_NPZ_ENTRIES: usize = 512;
const MAX_NPY_HEADER_BYTES: usize = 1024 * 1024;
const MAX_SAFETENSORS_HEADER_BYTES: usize = 16 * 1024 * 1024;

fn parse_npy_header_dict(header: &str) -> (Option<String>, Option<bool>, Vec<usize>) {
    let descr = extract_quoted_field(header, "'descr'")
        .or_else(|| extract_quoted_field(header, "\"descr\""));
    let fortran_order = if header.contains("'fortran_order': True")
        || header.contains("\"fortran_order\": True")
    {
        Some(true)
    } else if header.contains("'fortran_order': False")
        || header.contains("\"fortran_order\": False")
    {
        Some(false)
    } else {
        None
    };

    let mut shape = Vec::new();
    if let Some(idx) = header.find("'shape'").or_else(|| header.find("\"shape\""))
        && let Some(open) = header[idx..].find('(')
        && let Some(close) = header[idx + open..].find(')')
    {
        let inner = &header[idx + open + 1..idx + open + close];
        for part in inner.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty()
                && let Ok(dim) = trimmed.parse::<usize>()
            {
                shape.push(dim);
            }
        }
    }

    (descr, fortran_order, shape)
}

fn extract_quoted_field(haystack: &str, key: &str) -> Option<String> {
    let idx = haystack.find(key)?;
    let rest = &haystack[idx + key.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let quote = after.chars().next().filter(|&c| c == '\'' || c == '"')?;
    let end = after[1..].find(quote)?;
    Some(after[1..1 + end].to_owned())
}

fn inspect_npy_bytes(bytes: &[u8]) -> Result<Value, Error> {
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err(Error::new("Invalid NumPy (.npy) magic header"));
    }
    let major = bytes[6];
    let minor = bytes[7];
    let (header_len, header_start) = match major {
        1 => (usize::from(u16::from_le_bytes([bytes[8], bytes[9]])), 10usize),
        2 | 3 => {
            if bytes.len() < 12 {
                return Err(Error::new("Truncated NumPy v2/v3 header"));
            }
            (
                u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize,
                12usize,
            )
        }
        other => {
            return Err(Error::new(format!(
                "Unsupported NumPy .npy format version {other}.{minor}"
            )));
        }
    };
    if header_len > MAX_NPY_HEADER_BYTES || header_start + header_len > bytes.len() {
        return Err(Error::new("Invalid NumPy header length"));
    }
    let header_str = std::str::from_utf8(&bytes[header_start..header_start + header_len])
        .map_err(|_| Error::new("NumPy header is not valid UTF-8"))?;
    let (descr, fortran_order, shape) = parse_npy_header_dict(header_str);
    let element_count: usize = if shape.is_empty() {
        1
    } else {
        shape.iter().copied().fold(1usize, usize::saturating_mul)
    };
    Ok(json!({
        "format": "npy",
        "version": format!("{major}.{minor}"),
        "dtype": descr,
        "fortran_order": fortran_order,
        "shape": shape,
        "element_count": element_count,
        "data_bytes": bytes.len().saturating_sub(header_start + header_len),
    }))
}

fn inspect_npz_bytes(bytes: &[u8]) -> Result<Value, Error> {
    let archive = crate::zip::Archive::new(bytes)?;
    let mut names = archive.names();
    names.sort();
    if names.len() > MAX_NPZ_ENTRIES {
        return Err(Error::new(format!(
            "NPZ archive exceeds {MAX_NPZ_ENTRIES} entries"
        )));
    }
    let mut arrays = Vec::new();
    for name in names {
        if !name.ends_with(".npy") {
            continue;
        }
        let entry_bytes = archive
            .read_bounded(&name, 64 * 1024 * 1024)?
            .ok_or_else(|| Error::new(format!("Missing NPZ entry: {name}")))?;
        let mut info = inspect_npy_bytes(&entry_bytes)?;
        info["name"] = json!(name.trim_end_matches(".npy"));
        arrays.push(info);
    }
    Ok(json!({
        "format": "npz",
        "array_count": arrays.len(),
        "arrays": arrays,
    }))
}

fn inspect_safetensors_bytes(bytes: &[u8]) -> Result<Value, Error> {
    if bytes.len() < 8 {
        return Err(Error::new("File is too small to be SafeTensors"));
    }
    let header_len = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    if header_len == 0
        || header_len > MAX_SAFETENSORS_HEADER_BYTES
        || 8 + header_len > bytes.len()
    {
        return Err(Error::new("Invalid SafeTensors header length"));
    }
    let header_val: Value = serde_json::from_slice(&bytes[8..8 + header_len])
        .map_err(|e| Error::new(format!("Invalid SafeTensors JSON header: {e}")))?;
    let obj = header_val
        .as_object()
        .ok_or_else(|| Error::new("SafeTensors header must be a JSON object"))?;

    let metadata = obj.get("__metadata__").cloned().unwrap_or(Value::Null);
    let mut tensors = Vec::new();
    let mut total_params = 0u64;
    for (name, val) in obj {
        if name == "__metadata__" {
            continue;
        }
        let shape: Vec<u64> = val
            .get("shape")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        let params: u64 = if shape.is_empty() {
            1
        } else {
            shape.iter().copied().fold(1u64, u64::saturating_mul)
        };
        total_params = total_params.saturating_add(params);
        tensors.push(json!({
            "name": name,
            "dtype": val.get("dtype").cloned().unwrap_or(Value::Null),
            "shape": shape,
            "data_offsets": val.get("data_offsets").cloned().unwrap_or(Value::Null),
            "parameter_count": params,
        }));
    }

    Ok(json!({
        "format": "safetensors",
        "tensor_count": tensors.len(),
        "total_parameters": total_params,
        "metadata": metadata,
        "tensors": tensors,
    }))
}

/// Inspect a `.npy`, `.npz`, or `.safetensors` tensor/array file.
pub fn inspect_tensor_file(extension: Option<&str>, bytes: &[u8]) -> Result<Value, Error> {
    match extension.map(str::to_ascii_lowercase).as_deref() {
        Some("npy") => inspect_npy_bytes(bytes),
        Some("npz") => inspect_npz_bytes(bytes),
        Some("safetensors") => inspect_safetensors_bytes(bytes),
        _ => {
            if bytes.starts_with(b"\x93NUMPY") {
                inspect_npy_bytes(bytes)
            } else if bytes.starts_with(b"PK\x03\x04") {
                inspect_npz_bytes(bytes)
            } else {
                inspect_safetensors_bytes(bytes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_npy(descr: &str, shape: &str, payload: &[u8]) -> Vec<u8> {
        let mut dict = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
        while (10 + dict.len() + 1) % 64 != 0 {
            dict.push(' ');
        }
        dict.push('\n');
        let mut out = b"\x93NUMPY\x01\x00".to_vec();
        out.extend_from_slice(&(dict.len() as u16).to_le_bytes());
        out.extend_from_slice(dict.as_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn inspects_npy_and_safetensors_headers() {
        let npy = make_npy("<f4", "(2, 3)", &[0u8; 24]);
        let npy_info = inspect_tensor_file(Some("npy"), &npy).unwrap();
        assert_eq!(npy_info["format"], "npy");
        assert_eq!(npy_info["dtype"], "<f4");
        assert_eq!(npy_info["shape"], json!([2, 3]));
        assert_eq!(npy_info["element_count"], 6);

        let header = br#"{"__metadata__":{"framework":"pt"},"weight":{"dtype":"BF16","shape":[4,8],"data_offsets":[0,64]}}"#;
        let mut st = (header.len() as u64).to_le_bytes().to_vec();
        st.extend_from_slice(header);
        st.extend_from_slice(&[0u8; 64]);
        let st_info = inspect_tensor_file(Some("safetensors"), &st).unwrap();
        assert_eq!(st_info["format"], "safetensors");
        assert_eq!(st_info["tensor_count"], 1);
        assert_eq!(st_info["total_parameters"], 32);
        assert_eq!(st_info["metadata"]["framework"], "pt");
    }
}

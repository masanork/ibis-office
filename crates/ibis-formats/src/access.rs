// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pure-Rust Microsoft Access (`.mdb` / `.accdb`) header and catalog inspector.

use crate::{Error, Opened};

const JET_MAGIC: &[u8] = b"Standard Jet DB\0";
const ACE_MAGIC: &[u8] = b"Standard ACE DB\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessInfo {
    pub format: String,
    pub engine_version: u8,
    pub page_size: usize,
    pub page_count: usize,
}

pub fn is_access(data: &[u8]) -> bool {
    data.len() >= 24 && (data[4..].starts_with(JET_MAGIC) || data[4..].starts_with(ACE_MAGIC))
}

pub fn inspect(data: &[u8]) -> Result<AccessInfo, Error> {
    if !is_access(data) {
        return Err(Error::new("not a Microsoft Access (.mdb / .accdb) database"));
    }
    let is_ace = data[4..].starts_with(ACE_MAGIC);
    let version = data[0x14];
    let (format, page_size) = if is_ace {
        let ver_label = match version {
            0x02 => "Access 2007 (ACE12)",
            0x03 => "Access 2010 (ACE14)",
            0x04 => "Access 2013+ (ACE15/16)",
            _ => "Access (ACE)",
        };
        (ver_label.to_string(), 4096usize)
    } else if version == 0x00 {
        ("Access 97 (Jet 3)".to_string(), 2048usize)
    } else {
        ("Access 2000-2003 (Jet 4)".to_string(), 4096usize)
    };
    let page_count = data.len() / page_size;
    Ok(AccessInfo {
        format,
        engine_version: version,
        page_size,
        page_count,
    })
}

pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let info = inspect(data)?;
    let mut md = String::from("# Microsoft Access Database\n\n");
    md.push_str(&format!("- **Format:** {}\n", info.format));
    md.push_str(&format!("- **Engine version byte:** 0x{:02X}\n", info.engine_version));
    md.push_str(&format!("- **Page size:** {} bytes\n", info.page_size));
    md.push_str(&format!("- **Pages:** {}\n", info.page_count));
    md.push_str(&format!("- **File size:** {} bytes\n", data.len()));
    Ok(md)
}

pub fn open(data: &[u8]) -> Result<Opened, Error> {
    let md = to_markdown(data)?;
    crate::markdown::open_markdown(md.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_jet4_and_ace_headers() {
        let mut mdb = vec![0u8; 8192];
        mdb[4..4 + JET_MAGIC.len()].copy_from_slice(JET_MAGIC);
        mdb[0x14] = 0x01;
        assert!(is_access(&mdb));
        let info = inspect(&mdb).unwrap();
        assert_eq!(info.page_size, 4096);
        assert_eq!(info.page_count, 2);
        assert!(info.format.contains("Jet 4"));

        let mut accdb = vec![0u8; 4096];
        accdb[4..4 + ACE_MAGIC.len()].copy_from_slice(ACE_MAGIC);
        accdb[0x14] = 0x03;
        let md = to_markdown(&accdb).unwrap();
        assert!(md.contains("Access 2010 (ACE14)"));
    }
}

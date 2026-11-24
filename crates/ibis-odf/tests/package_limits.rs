// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_formats::odf::XML_PART_LIMIT;
use ibis_ooxml::zip::{EntryMeta, Writer, crc32};

#[test]
fn open_rejects_oversized_odf_content_before_inflating() {
    for kind in ["text", "spreadsheet", "presentation"] {
        let mut writer = Writer::new();
        let mimetype = format!("application/vnd.oasis.opendocument.{kind}");
        writer
            .add_raw(
                "mimetype",
                EntryMeta::new(),
                0,
                crc32(mimetype.as_bytes()),
                mimetype.len(),
                mimetype.as_bytes(),
            )
            .unwrap();
        writer
            .add_raw(
                "content.xml",
                EntryMeta::new(),
                8,
                0,
                XML_PART_LIMIT + 1,
                &[],
            )
            .unwrap();

        let result = ibis_odf::open(writer.finish().unwrap());
        assert!(
            matches!(result, Err(err) if err.to_string().contains("content.xml が大きすぎます")),
            "{kind}"
        );
    }
}

// SPDX-License-Identifier: AGPL-3.0-only
//! An explicitly unplaced image list for JTD files with no readable body.
use ibis_doc_model::{
    Block, EmuSize, FlowParagraph, FlowRun, FlowRunKind, FlowRunStyle, MediaFormat, PictureLook,
    Presentation,
};

pub(crate) fn read(data: &[u8]) -> Result<(Presentation, bool), String> {
    match ibis_pptx::jtd::read_preview(data).map_err(|e| e.to_string())? {
        ibis_pptx::jtd::Preview::Document(document) => Ok((document, false)),
        ibis_pptx::jtd::Preview::UnplacedImages(images) => {
            gallery(images, "jtd.gallery-unavailable").map(|document| (document, true))
        }
    }
}

/// Inspect unplaced images separately even when the document has body text.
pub(crate) fn images(data: &[u8]) -> Result<Presentation, String> {
    let images = ibis_pptx::jtd::inspect_embedded_metafiles(data).map_err(|e| e.to_string())?;
    gallery(images, "jtd.no-embedded-images")
}

fn gallery(mut images: ibis_pptx::jtd::EmbeddedMetafileInspection, empty_key: &str) -> Result<Presentation, String> {
    let bytes = ibis_pptx::blank_file(ibis_pptx::Kind::Document).map_err(|e| e.to_string())?;
    let mut document = ibis_pptx::read(&bytes).map_err(|e| e.to_string())?;
    images.metafiles.sort_by_key(|entry| entry.embedding_index);
    let mut blocks = vec![paragraph(
        1,
        FlowRunKind::Text(crate::shell::i18n::text("jtd.gallery-heading").to_string()),
    )];
    let mut inspected_bytes = 0usize;
    for image in images.metafiles.into_iter().take(32) {
        inspected_bytes = inspected_bytes.saturating_add(image.wmf.len());
        if inspected_bytes > 8 * 1024 * 1024 {
            break;
        }
        let Some((width, height)) =
            crate::pptx::editing::validated_picture_size(MediaFormat::Wmf, &image.wmf)
        else {
            continue;
        };
        // An inspection scale, not a source-document dimension. Preserve aspect ratio.
        let size = inspection_size(width, height);
        let media = document.add_media(MediaFormat::Wmf, image.wmf);
        let id = blocks.len() as u32 + 1;
        blocks.push(paragraph(
            id,
            FlowRunKind::Picture {
                media,
                size,
                look: PictureLook::default(),
            },
        ));
    }
    if blocks.len() == 1 {
        return Err(crate::shell::i18n::text(empty_key).to_string());
    }
    let flow = document.flow.as_mut().ok_or("cannot create image list")?;
    std::sync::Arc::make_mut(flow).body = blocks;
    Ok(document)
}

fn paragraph(id: u32, kind: FlowRunKind) -> Block {
    Block::Paragraph(FlowParagraph {
        id,
        runs: vec![FlowRun::new(kind, FlowRunStyle::default())],
        ..FlowParagraph::default()
    })
}

pub(crate) fn inspection_size(width: u32, height: u32) -> EmuSize {
    let (w, h) = (u64::from(width) * 9_525, u64::from(height) * 9_525);
    let limit = 5 * 914_400u64;
    let longest = w.max(h).max(1);
    let fit = |n: u64| {
        if longest > limit {
            ((u128::from(n) * u128::from(limit)) / u128::from(longest)) as i64
        } else {
            n as i64
        }
    };
    EmuSize::new(fit(w).max(1), fit(h).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_dimensions_are_bounded_even_for_hostile_sizes() {
        let size = inspection_size(u32::MAX, 1);
        assert_eq!(size.width, 5 * 914_400);
        assert_eq!(size.height, 1);
        let size = inspection_size(100, 200);
        assert_eq!(size.width * 2, size.height);
    }
}

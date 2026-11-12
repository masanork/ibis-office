// SPDX-License-Identifier: AGPL-3.0-only
//! Compare raw frame candidates with WMF mapping records, without placing images.
use ibis_pptx::jtd::{self, EmbeddedMetafile, FrameLink};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
enum Observation {
    MapMode(u16),
    WindowExtent { x: i16, y: i16 },
    ViewportExtent { x: i16, y: i16 },
}

// Preserve records in source order; this is not a GDI playback-state machine.
fn mapping_records(data: &[u8]) -> Result<Vec<Observation>, &'static str> {
    let bad = || "invalid or unsupported WMF mapping record";
    let u16_at = |offset: usize| -> Option<u16> {
        Some(u16::from_le_bytes(
            data.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
        ))
    };
    let u32_at = |offset: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            data.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    if !matches!(u16_at(0), Some(1 | 2))
        || u16_at(2) != Some(9)
        || u32_at(6)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| n.checked_mul(2))
            != Some(data.len())
    {
        return Err(bad());
    }
    let mut offset = 18usize;
    let mut observations = Vec::new();
    while offset.checked_add(6).ok_or_else(bad)? <= data.len() {
        let len = usize::try_from(u32_at(offset).ok_or_else(bad)?)
            .ok()
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(bad)?;
        let end = offset.checked_add(len).ok_or_else(bad)?;
        if len < 6 || end > data.len() {
            return Err(bad());
        }
        let function = u16_at(offset + 4).ok_or_else(bad)?;
        match function {
            0 => {
                if len != 6 || end != data.len() {
                    return Err(bad());
                }
                return Ok(observations);
            }
            0x0103 => {
                if len != 8 {
                    return Err(bad());
                }
                observations.push(Observation::MapMode(u16_at(offset + 6).ok_or_else(bad)?));
            }
            0x020c | 0x020e => {
                if len != 10 {
                    return Err(bad());
                }
                let y = u16_at(offset + 6).ok_or_else(bad)? as i16;
                let x = u16_at(offset + 8).ok_or_else(bad)? as i16;
                observations.push(if function == 0x020c {
                    Observation::WindowExtent { x, y }
                } else {
                    Observation::ViewportExtent { x, y }
                });
            }
            _ => {}
        }
        if observations.len() > 4096 {
            return Err(bad());
        }
        offset = end;
    }
    Err(bad())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: jtd_wmf_frame_probe INPUT")?;
    if args.next().is_some() {
        return Err("usage: jtd_wmf_frame_probe INPUT".into());
    }
    let data = std::fs::read(path)?;
    let links = jtd::inspect_frame_links(&data)?;
    let images = jtd::inspect_embedded_metafiles(&data)?;
    let rows = comparison_rows(links, &images.metafiles)?;
    println!(
        "embedding\tframe_ref\tinfo_width_candidate\tinfo_height_candidate\tframe_matches\twmf_matches\tsize_equal\trecord\tx_or_mode\ty\tplacement_proven"
    );
    for row in rows {
        println!("{row}");
    }
    eprintln!(
        "rejected_embedded_press={} (mapping records are observations, not playback state or placement)",
        images.rejected
    );
    Ok(())
}

fn comparison_rows(
    links: Vec<FrameLink>,
    images: &[EmbeddedMetafile],
) -> Result<Vec<String>, &'static str> {
    let mut by_index: BTreeMap<u32, Vec<&EmbeddedMetafile>> = BTreeMap::new();
    for image in images {
        by_index
            .entry(image.embedding_index)
            .or_default()
            .push(image);
    }
    // Compute all output before printing so malformed input doesn't look complete.
    let mut rows = Vec::new();
    for link in links {
        let candidates = by_index
            .get(&link.embedding_index)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if candidates.len() != 1 || link.matches.len() != 1 {
            rows.push(format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t-\t-\t-\t-\tfalse",
                link.embedding_index,
                link.frame_ref,
                link.width,
                link.height,
                link.matches.len(),
                candidates.len()
            ));
            continue;
        }
        let frame = &link.matches[0];
        let size_equal =
            link.width == u32::from(frame.width) && link.height == u32::from(frame.height);
        let observations = mapping_records(&candidates[0].wmf)?;
        if rows
            .len()
            .checked_add(observations.len().max(1))
            .is_none_or(|n| n > 16384)
        {
            return Err("too many mapping observations".into());
        }
        if observations.is_empty() {
            rows.push(format!(
                "{}\t{}\t{}\t{}\t1\t1\t{}\t-\t-\t-\tfalse",
                link.embedding_index, link.frame_ref, link.width, link.height, size_equal
            ));
        }
        for observation in observations {
            let (kind, x, y) = match observation {
                Observation::MapMode(mode) => ("map_mode", mode.to_string(), "-".to_owned()),
                Observation::WindowExtent { x, y } => {
                    ("window_extent", x.to_string(), y.to_string())
                }
                Observation::ViewportExtent { x, y } => {
                    ("viewport_extent", x.to_string(), y.to_string())
                }
            };
            rows.push(format!(
                "{}\t{}\t{}\t{}\t1\t1\t{}\t{}\t{}\t{}\tfalse",
                link.embedding_index,
                link.frame_ref,
                link.width,
                link.height,
                size_equal,
                kind,
                x,
                y
            ));
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wmf() -> Vec<u8> {
        let mut b = vec![1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0];
        b.extend_from_slice(&[4, 0, 0, 0, 3, 1, 8, 0]);
        b.extend_from_slice(&[5, 0, 0, 0, 12, 2, 0xed, 0, 0xe5, 1]);
        b.extend_from_slice(&[3, 0, 0, 0, 0, 0]);
        let size = (b.len() / 2) as u32;
        b[6..10].copy_from_slice(&size.to_le_bytes());
        b
    }
    #[test]
    fn preserves_record_order_signed_extents_and_mode() {
        let observed = mapping_records(&wmf()).unwrap();
        assert_eq!(
            observed,
            vec![
                Observation::MapMode(8),
                Observation::WindowExtent { x: 485, y: 237 }
            ]
        );
        let mut b = wmf();
        b[34..36].copy_from_slice(&(-10i16).to_le_bytes());
        assert_eq!(
            mapping_records(&b).unwrap()[1],
            Observation::WindowExtent { x: -10, y: 237 }
        );
    }
    #[test]
    fn rejects_truncation_invalid_lengths_and_missing_terminator() {
        let b = wmf();
        for n in 0..b.len() {
            assert!(mapping_records(&b[..n]).is_err());
        }
        let mut wrong = b.clone();
        wrong[26] = 4;
        assert!(mapping_records(&wrong).is_err());
        let mut wrong = b;
        let end = wrong.len();
        wrong[end - 2] = 1;
        assert!(mapping_records(&wrong).is_err());
    }
    #[test]
    fn ambiguous_or_missing_matches_are_not_selected() {
        use ibis_pptx::jtd::FrameCandidate;
        let frame = || FrameCandidate {
            row: 0,
            id: 2,
            x: 0,
            y: 0,
            width: 12,
            height: 34,
        };
        let link = |matches| FrameLink {
            embedding_index: 7,
            frame_ref: 2,
            width: 12,
            height: 34,
            embedded_press_present: true,
            matches,
        };
        let image = || EmbeddedMetafile {
            embedding_index: 7,
            wmf: wmf(),
        };
        let multiple_frames =
            comparison_rows(vec![link(vec![frame(), frame()])], &[image()]).unwrap();
        assert_eq!(multiple_frames, ["7\t2\t12\t34\t2\t1\t-\t-\t-\t-\tfalse"]);
        let multiple_images =
            comparison_rows(vec![link(vec![frame()])], &[image(), image()]).unwrap();
        assert_eq!(multiple_images, ["7\t2\t12\t34\t1\t2\t-\t-\t-\t-\tfalse"]);
        let absent = comparison_rows(vec![link(vec![])], &[]).unwrap();
        assert_eq!(absent, ["7\t2\t12\t34\t0\t0\t-\t-\t-\t-\tfalse"]);
        let mut different = frame();
        different.width = 13;
        let mismatch = comparison_rows(vec![link(vec![different])], &[image()]).unwrap();
        assert!(mismatch.iter().all(|row| row.contains("\t1\t1\tfalse\t")));
    }
}

// SPDX-License-Identifier: AGPL-3.0-only
//! Diagnostic only: no recovered page association, placement, or paint order.
use ibis_pptx::jtd;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: jtd_frame_links INPUT")?;
    if args.next().is_some() {
        return Err("usage: jtd_frame_links INPUT".into());
    }
    let data = std::fs::read(path)?;
    let links = jtd::inspect_frame_links(&data)?;
    println!(
        "embedding\tframe_ref\tinfo_width\tinfo_height\tpress_present\tid_matches\tframe_row\tframe_x\tframe_y\tframe_width\tframe_height\tsize_equal\tplacement_proven"
    );
    for link in links {
        let count = link.matches.len();
        if count == 0 {
            println!(
                "{}\t{}\t{}\t{}\t{}\t0\t-\t-\t-\t-\t-\t-\tfalse",
                link.embedding_index,
                link.frame_ref,
                link.width,
                link.height,
                link.embedded_press_present
            );
        }
        for frame in link.matches {
            let equal =
                link.width == u32::from(frame.width) && link.height == u32::from(frame.height);
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\tfalse",
                link.embedding_index,
                link.frame_ref,
                link.width,
                link.height,
                link.embedded_press_present,
                count,
                frame.row,
                frame.x,
                frame.y,
                frame.width,
                frame.height,
                equal
            );
        }
    }
    Ok(())
}

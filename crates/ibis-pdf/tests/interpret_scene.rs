// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#![cfg(feature = "interpret")]

use ibis_pdf::PdfContainer;
use ibis_pdf::interpret::interpret_page_scene;
use ibis_pdf::scene::DisplayCmd;

fn build_vector_and_text_pdf() -> Vec<u8> {
    use std::io::Write as _;
    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n");

    let mut offsets = Vec::new();
    offsets.push((1, out.len()));
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    offsets.push((2, out.len()));
    out.extend_from_slice(
        b"2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] /MediaBox [0 0 595 842] >>\nendobj\n",
    );
    offsets.push((3, out.len()));
    out.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n",
    );

    let stream = b"0.2 0.4 0.8 rg 50 700 200 40 re f 0 0 0 rg BT /F1 18 Tf 72 720 Td (Hello ibisOffice) Tj ET";
    offsets.push((4, out.len()));
    let _ = writeln!(out, "4 0 obj\n<< /Length {} >>\nstream", stream.len());
    out.extend_from_slice(stream);
    out.extend_from_slice(b"\nendstream\nendobj\n");

    offsets.push((5, out.len()));
    out.extend_from_slice(
        b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    );

    let xref_offset = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for &(_, off) in &offsets {
        let _ = writeln!(out, "{off:010} 00000 n ");
    }
    let _ = writeln!(
        out,
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF"
    );
    out
}

#[test]
fn interprets_paths_and_extracts_text_coordinates() {
    let pdf = build_vector_and_text_pdf();
    let mut container = PdfContainer::from_bytes(pdf).unwrap();
    let scene = interpret_page_scene(&mut container, 0).expect("interpret page 0");

    assert_eq!(scene.text_index.plain_text(), "Hello ibisOffice");

    let has_rect_path = scene.display_list.iter().any(|cmd| {
        matches!(
            cmd,
            DisplayCmd::DrawPath {
                fill_rgba: Some([51, 102, 204, 255]),
                ..
            }
        )
    });
    assert!(has_rect_path, "expected filled blue rectangle in display_list");

    let glyph_count = scene
        .display_list
        .iter()
        .filter(|cmd| matches!(cmd, DisplayCmd::DrawGlyphRun { .. }))
        .count();
    assert_eq!(glyph_count, "Hello ibisOffice".len());
}

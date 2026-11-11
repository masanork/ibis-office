// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Metafiles played into SVG. The fixtures are a fictitious drawing (shapes.fodg: a rectangle, an ellipse, a triangle,
//! a rounded rectangle and a text) exported to EMF and WMF.

use ibis_emf::{emf_to_svg, wmf_to_svg};

fn record(kind: u32, body: &[u32]) -> Vec<u8> {
    let mut v = kind.to_le_bytes().to_vec();
    v.extend(((body.len() as u32 + 2) * 4).to_le_bytes());
    for x in body {
        v.extend(x.to_le_bytes());
    }
    v
}

/// A header and the records, with 100 by 100 device units.
fn emf(records: &[Vec<u8>]) -> Vec<u8> {
    let mut h = vec![0u8; 88];
    h[0..4].copy_from_slice(&1u32.to_le_bytes());
    h[4..8].copy_from_slice(&88u32.to_le_bytes());
    for (i, v) in [0i32, 0, 99, 99].iter().enumerate() {
        h[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    h[40..44].copy_from_slice(b" EMF");
    for (at, v) in [(72, 100i32), (76, 100), (80, 50), (84, 50)] {
        h[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    let mut out = h;
    for r in records {
        out.extend(r);
    }
    out.extend(record(14, &[0, 0, 0]));
    out
}

#[test]
fn draws_a_filled_rectangle_with_a_pen() {
    let data = emf(&[
        record(0x27, &[1, 0, 0x0000_00FF, 0]),  // brush: red
        record(0x26, &[2, 0, 3, 0, 0x00FF_0000]), // pen: blue, 3 wide
        record(0x25, &[1]),
        record(0x25, &[2]),
        record(0x2B, &[10, 20, 60, 70]),
    ]);
    let svg = emf_to_svg(&data).expect("drawn");
    assert!(svg.contains("fill=\"#FF0000\""), "{svg}");
    assert!(svg.contains("stroke=\"#0000FF\""), "{svg}");
    assert!(svg.contains("M10 20"), "{svg}");
}

#[test]
fn applies_the_world_transform() {
    // Move by (30, 40) before drawing a polygon at the origin.
    let data = emf(&[
        record(0x24, &[0x3F80_0000, 0, 0, 0x3F80_0000, 30.0f32.to_bits(), 40.0f32.to_bits(), 4]),
        record(0x03, &[0, 0, 0, 0, 3, 0, 0, 10, 0, 0, 10]),
    ]);
    let svg = emf_to_svg(&data).expect("drawn");
    assert!(svg.contains("M30 40"), "{svg}");
    assert!(svg.contains("L40 40"), "{svg}");
}

#[test]
fn draws_text_with_the_selected_colour() {
    let mut rec = vec![0u32; 17];
    rec[7] = 5;
    rec[8] = 30;
    rec[9] = 2;
    rec[10] = 76;
    rec.push(u32::from(b'H') | (u32::from(b'i') << 16));
    let data = emf(&[record(0x18, &[0x0000_8800]), record(0x54, &rec)]);
    let svg = emf_to_svg(&data).expect("drawn");
    assert!(svg.contains(">Hi<"), "{svg}");
    assert!(svg.contains("fill=\"#008800\""), "{svg}");
}

#[test]
fn rejects_what_is_not_a_metafile() {
    assert!(emf_to_svg(b"nothing here").is_none());
    assert!(wmf_to_svg(b"nothing here").is_none());
    // A metafile that draws nothing.
    assert!(emf_to_svg(&emf(&[])).is_none());
}

fn has(svg: &str, needle: &str) -> bool {
    svg.contains(needle)
}

#[test]
fn plays_the_exported_emf() {
    let svg = emf_to_svg(include_bytes!("fixtures/shapes.emf")).expect("drawn");
    assert!(has(&svg, "fill=\"#3366CC\"") && has(&svg, "fill=\"#FFCC00\""), "fills");
    assert!(has(&svg, "stroke-dasharray") || has(&svg, "fill=\"#CC3300\""), "dashes");
    assert!(has(&svg, ">Hello"), "text");
}

#[test]
fn plays_the_exported_wmf() {
    let svg = wmf_to_svg(include_bytes!("fixtures/shapes.wmf")).expect("drawn");
    assert!(has(&svg, "fill=\"#3366CC\"") && has(&svg, "fill=\"#FFCC00\""), "fills");
    assert!(has(&svg, ">Hello"), "text");
}

fn wmf_record(func: u16, params: &[u8]) -> Vec<u8> {
    let mut p = params.to_vec();
    if p.len() % 2 == 1 {
        p.push(0);
    }
    let mut v = (((p.len() + 6) / 2) as u32).to_le_bytes().to_vec();
    v.extend(func.to_le_bytes());
    v.extend(p);
    v
}

fn wmf(records: &[Vec<u8>]) -> Vec<u8> {
    // The 18-byte header: type 1 (memory), size 9 words, version 0x300.
    let mut v = vec![1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for r in records {
        v.extend(r);
    }
    v.extend(wmf_record(0, &[]));
    v
}

#[test]
fn rejects_wmf_records_whose_counts_exceed_the_record() {
    let drawing = wmf_record(0x041B, &[0, 0, 0, 0, 10, 0, 10, 0]);
    // One polygon claims 65,535 points but contains none.
    let polygon = wmf_record(0x0538, &[1, 0, 0xff, 0xff]);
    assert!(wmf_to_svg(&wmf(&[polygon, drawing.clone()])).is_none());
    let polyline = wmf_record(0x0325, &[0xff, 0xff]);
    assert!(wmf_to_svg(&wmf(&[polyline, drawing.clone()])).is_none());
    let text = wmf_record(0x0521, &[0xff, 0xff]);
    assert!(wmf_to_svg(&wmf(&[text, drawing])).is_none());
}

#[test]
fn deleting_an_unused_wmf_handle_does_not_duplicate_the_free_slot() {
    let brush = |color: u32| {
        let mut p = 0u16.to_le_bytes().to_vec();
        p.extend(color.to_le_bytes());
        p.extend(0u16.to_le_bytes());
        wmf_record(0x02FC, &p)
    };
    let data = wmf(&[
        wmf_record(0x020C, &[100, 0, 100, 0]),
        brush(0x0000FF00),
        wmf_record(0x01F0, &[0, 0]),
        wmf_record(0x01F0, &[0, 0]),
        brush(0x00FF0000),
        brush(0x000000FF),
        wmf_record(0x012D, &[1, 0]),
        wmf_record(0x041B, &[0, 0, 0, 0, 10, 0, 10, 0]),
    ]);
    assert!(wmf_to_svg(&data).unwrap().contains("#FF0000"));
}

// 日本語 and the face "ＭＳ ゴシック" in Shift_JIS.
const NIHONGO_SJIS: [u8; 6] = [0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA];
const FACE_SJIS: [u8; 13] = [0x82, 0x6C, 0x82, 0x72, 0x20, 0x83, 0x53, 0x83, 0x56, 0x83, 0x62, 0x83, 0x4E];

#[test]
fn wmf_text_is_read_by_the_character_set_of_the_font() {
    let mut font = Vec::new();
    for v in [-20i16, 0, 0, 0, 400] {
        font.extend(v.to_le_bytes());
    }
    // Italic, underline, strike-out, charset (128: Shift_JIS), then precisions and the face.
    font.extend([0, 0, 0, 128, 0, 0, 0, 0]);
    font.extend(FACE_SJIS);
    font.push(0);
    let mut text = (NIHONGO_SJIS.len() as u16).to_le_bytes().to_vec();
    text.extend(NIHONGO_SJIS);
    text.extend(10i16.to_le_bytes()); // y
    text.extend(5i16.to_le_bytes()); // x
    let data = wmf(&[
        wmf_record(0x020B, &[0, 0, 0, 0]),
        wmf_record(0x020C, &[100, 0, 100, 0]),
        wmf_record(0x02FB, &font),
        wmf_record(0x012D, &[0, 0]),
        wmf_record(0x0521, &text),
    ]);
    let svg = wmf_to_svg(&data).expect("drawn");
    assert!(svg.contains(">日本語<"), "{svg}");
    assert!(svg.contains("ＭＳ ゴシック"), "{svg}");
}

#[test]
fn emf_ansi_text_is_read_by_the_character_set_of_the_font() {
    // EMR_EXTCREATEFONTINDIRECTW (handle 1, charset 128), SELECTOBJECT, then EMR_EXTTEXTOUTA.
    let mut font = vec![0u32; 2 + 1 + 23];
    font[2] = 1; // handle
    let mut bytes: Vec<u8> = font.iter().flat_map(|v| v.to_le_bytes()).collect();
    bytes[12 + 23] = 128;
    let mut rec = 0x52u32.to_le_bytes().to_vec();
    rec.extend(((bytes.len() + 4) as u32).to_le_bytes());
    rec.extend(&bytes[8..]);
    // The record above is rebuilt properly: type, size, handle, LogFontW (92 bytes).
    let mut font_rec = 0x52u32.to_le_bytes().to_vec();
    font_rec.extend(104u32.to_le_bytes());
    font_rec.extend(1u32.to_le_bytes());
    let mut log = vec![0u8; 92];
    log[0..4].copy_from_slice(&(-20i32).to_le_bytes());
    log[23] = 128;
    font_rec.extend(log);
    let select = record(0x25, &[1]);
    let mut text = vec![0u32; 17];
    text[7] = 5;
    text[8] = 30;
    text[9] = NIHONGO_SJIS.len() as u32;
    text[10] = 76;
    let mut text_rec = 0x53u32.to_le_bytes().to_vec();
    let mut body: Vec<u8> = text.iter().flat_map(|v| v.to_le_bytes()).collect();
    body.truncate(68);
    body.extend(NIHONGO_SJIS);
    body.extend([0, 0]);
    text_rec.extend(((body.len() + 8) as u32).to_le_bytes());
    text_rec.extend(body);
    let mut data = emf(&[]);
    // (Insert the records before the end-of-file record.)
    let eof = data.split_off(data.len() - 20);
    data.extend(font_rec);
    data.extend(select);
    data.extend(text_rec);
    data.extend(eof);
    let svg = emf_to_svg(&data).expect("drawn");
    assert!(svg.contains(">日本語<"), "{svg}");
}

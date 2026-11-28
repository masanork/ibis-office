use ibis_formats::{Kind, open};

#[test]
fn opens_procurement_formats_via_ibis_formats_open() {
    // 1. EML -> Document
    let eml = b"From: a@example.jp\r\nSubject: Procurement Notice\r\n\r\nBody text\r\n";
    let opened = open(eml.to_vec()).expect("eml opens");
    assert_eq!(Kind::of(&opened.presentation), Kind::Document);

    // 2. CALS/EC XML -> Document
    let cals = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><report><基礎情報><工事名>橋梁点検</工事名></基礎情報></report>";
    let opened = open(cals.as_bytes().to_vec()).expect("cals xml opens");
    assert_eq!(Kind::of(&opened.presentation), Kind::Document);

    // 3. DXF -> Document
    let dxf = b"0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nWALL\n10\n0\n20\n0\n11\n10\n21\n10\n0\nENDSEC\n0\nEOF\n";
    let opened = open(dxf.to_vec()).expect("dxf opens");
    assert_eq!(Kind::of(&opened.presentation), Kind::Document);

    // 4. GeoJSON -> Document
    let gj = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"庁舎"},"geometry":{"type":"Point","coordinates":[139.75,35.68]}}]}"#;
    let opened = open(gj.as_bytes().to_vec()).expect("geojson opens");
    assert_eq!(Kind::of(&opened.presentation), Kind::Document);

    // 5. Synthetic Sanshiro (.jsd) -> Workbook
    let mut sheet_rec = vec![0u8; 10];
    sheet_rec[0..2].copy_from_slice(&0x000Au16.to_le_bytes());
    for u in "予算".encode_utf16() {
        sheet_rec.extend_from_slice(&u.to_le_bytes());
    }
    sheet_rec.extend_from_slice(&[0, 0]);
    let slen = sheet_rec.len() as u16;
    sheet_rec[2..4].copy_from_slice(&slen.to_le_bytes());
    let mut book = vec![0xff, 0xff];
    book.extend_from_slice(&sheet_rec);

    let mut sub = vec![0u8; 6];
    sub[0..2].copy_from_slice(&0x0001u16.to_le_bytes());
    sub[4..6].copy_from_slice(&0x0027u16.to_le_bytes());
    for u in "品目A".encode_utf16() {
        sub.extend_from_slice(&u.to_le_bytes());
    }
    sub.extend_from_slice(&[0, 0]);
    let sub_len = sub.len() as u16;
    sub[2..4].copy_from_slice(&sub_len.to_le_bytes());

    let mut ws = vec![0u8; 12];
    ws[0..2].copy_from_slice(&0x1000u16.to_le_bytes());
    ws[4..8].copy_from_slice(&5u32.to_le_bytes());
    ws.extend_from_slice(&sub);
    let ws_len = ws.len() as u16;
    ws[2..4].copy_from_slice(&ws_len.to_le_bytes());

    let jsd = ibis_ooxml::cfb::write(&[
        ibis_ooxml::cfb::Item::stream("Book", book),
        ibis_ooxml::cfb::Item::stream("WorkSheet1", ws),
        ibis_ooxml::cfb::Item::stream("WorkSheet2", vec![0u8; 8]),
    ]);
    let opened = open(jsd).expect("sanshiro opens as workbook");
    assert_eq!(Kind::of(&opened.presentation), Kind::Workbook);
    let sheet_count = opened
        .presentation
        .workbook
        .as_ref()
        .map(|w| w.sheets.len())
        .unwrap_or(opened.presentation.slides.len());
    assert_eq!(sheet_count, 2);

    // 6. Synthetic Hanako (.jhd) -> Presentation with shapes
    let mut vec_rec = vec![0u8; 32];
    vec_rec[0..4].copy_from_slice(&[0x01, 0x00, 0x01, 0x60]);
    vec_rec[4..6].copy_from_slice(&32u16.to_be_bytes());
    vec_rec[16..20].copy_from_slice(&100i32.to_be_bytes());
    vec_rec[20..24].copy_from_slice(&200i32.to_be_bytes());
    vec_rec[24..28].copy_from_slice(&500i32.to_be_bytes());
    vec_rec[28..32].copy_from_slice(&400i32.to_be_bytes());
    let jhd = ibis_ooxml::cfb::write(&[ibis_ooxml::cfb::Item::stream("FDMVector", vec_rec)]);
    let opened = open(jhd).expect("hanako opens as presentation");
    assert_eq!(Kind::of(&opened.presentation), Kind::Presentation);
    assert!(!opened.presentation.slides[0].shapes.is_empty());

    // 7. Extension registration checks
    for ext in ["dxf", "sfc", "jww", "geojson", "shp", "eml", "lzh", "7z", "mdb"] {
        assert_eq!(Kind::from_extension(ext), Some(Kind::Document), "ext {ext}");
        assert!(Kind::OPENABLE_EXTENSIONS.contains(&ext), "openable {ext}");
    }
    for ext in ["jhd", "jhb"] {
        assert_eq!(Kind::from_extension(ext), Some(Kind::Presentation), "ext {ext}");
        assert!(Kind::OPENABLE_EXTENSIONS.contains(&ext), "openable {ext}");
    }
    for ext in ["jsd", "jst", "dbf"] {
        assert_eq!(Kind::from_extension(ext), Some(Kind::Workbook), "ext {ext}");
        assert!(Kind::OPENABLE_EXTENSIONS.contains(&ext), "openable {ext}");
    }
}

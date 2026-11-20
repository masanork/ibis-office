#[test]
fn changes_order_between_resolution_ranges() {
    // OpenJPEG lossless RGB, four tiles and two quality layers.
    // Each tile's first resolution uses RLCP and its remaining resolutions use CPRL.
    let source = include_bytes!("fixtures/mixed-tile-poc.j2k");
    let actual = j2k_rs::decode(source).unwrap();
    assert_eq!((actual.width, actual.height, actual.channels), (64, 64, 3));

    assert_eq!(actual.data, expected_pixels());
}

#[test]
fn main_header_orders_span_tile_parts() {
    // The same synthetic packets, with both progression entries in the main
    // header and either one or two parts per tile.
    for source in [
        include_bytes!("fixtures/mixed-main-poc.j2k").as_slice(),
        include_bytes!("fixtures/mixed-main-multipart-poc.j2k").as_slice(),
    ] {
        let actual = j2k_rs::decode(source).unwrap();
        assert_eq!((actual.width, actual.height, actual.channels), (64, 64, 3));
        assert_eq!(actual.data, expected_pixels());
    }
}

#[test]
fn rejects_non_increasing_main_header_ranges() {
    let mut source = include_bytes!("fixtures/mixed-main-poc.j2k").to_vec();
    let marker = source.windows(2).position(|m| m == [0xff, 0x5f]).unwrap();
    source[marker + 15] = source[marker + 8];
    assert!(j2k_rs::decode(&source).is_err());
}

fn expected_pixels() -> Vec<u8> {
    let mut expected = Vec::with_capacity(64 * 64 * 3);
    for y in 0..64 {
        for x in 0..64 {
            expected.extend([(x * 7 + y * 2) as u8, (y * 11) as u8, (x * 3 ^ y * 5) as u8]);
        }
    }
    expected
}

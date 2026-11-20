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
fn accepts_explicit_starts_and_rejects_gaps_in_main_header_ranges() {
    let mut source = include_bytes!("fixtures/mixed-main-poc.j2k").to_vec();
    let marker = source.windows(2).position(|m| m == [0xff, 0x5f]).unwrap();
    let second_start_resolution = marker + 11;
    source[second_start_resolution] = 1;
    assert_eq!(j2k_rs::decode(&source).unwrap().data, expected_pixels());
    source[second_start_resolution] = 2;
    assert!(j2k_rs::decode(&source).is_err());
}

#[test]
fn accepts_explicit_starts_in_tile_part_ranges() {
    let mut source = include_bytes!("fixtures/mixed-tile-poc.j2k").to_vec();
    let mut offset = source.windows(2).position(|m| m == [0xff, 0x90]).unwrap();
    let mut second_parts = 0;
    while source[offset..].starts_with(&[0xff, 0x90]) {
        let length = u32::from_be_bytes(source[offset + 6..offset + 10].try_into().unwrap());
        if source[offset + 10] == 1 {
            assert_eq!(&source[offset + 12..offset + 14], &[0xff, 0x5f]);
            source[offset + 16] = 1;
            second_parts += 1;
        }
        offset += length as usize;
    }
    assert_eq!(second_parts, 4);
    assert_eq!(j2k_rs::decode(&source).unwrap().data, expected_pixels());
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

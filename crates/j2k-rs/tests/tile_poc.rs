#[test]
fn changes_order_between_resolution_ranges() {
    // OpenJPEG lossless RGB, four tiles and two quality layers.
    // Each tile's first resolution uses RLCP and its remaining resolutions use CPRL.
    let source = include_bytes!("fixtures/mixed-tile-poc.j2k");
    let actual = j2k_rs::decode(source).unwrap();
    assert_eq!((actual.width, actual.height, actual.channels), (64, 64, 3));

    let mut expected = Vec::with_capacity(64 * 64 * 3);
    for y in 0..64 {
        for x in 0..64 {
            expected.extend([(x * 7 + y * 2) as u8, (y * 11) as u8, (x * 3 ^ y * 5) as u8]);
        }
    }
    assert_eq!(actual.data, expected);
}

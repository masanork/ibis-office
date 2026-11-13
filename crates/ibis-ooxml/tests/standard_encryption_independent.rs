// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
use ibis_ooxml::{cfb, encryption::{decrypt, DecryptError}, Package};
use sha2::{Digest, Sha256};

#[test]
fn independent_standard_fixture() {
    let file = include_bytes!("fixtures/standard-password.docx");
    assert_eq!(decrypt(file, "wrong"), Err(DecryptError::WrongPassword));
    let plain = decrypt(file, "Password1234_").unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&plain)), "ca1c0ebb465553361b9034e696d4081df0a2d41918f820060325b3ca634eb69b");
    assert!(Package::open(plain).unwrap().contains("word/document.xml"));
    assert!(matches!(decrypt(&file[..file.len() / 2], "Password1234_"), Err(DecryptError::Damaged(_))));
}

#[test]
fn malformed_and_unsupported_standard_headers() {
    let file = include_bytes!("fixtures/standard-password.docx");
    let info = cfb::stream(file, "EncryptionInfo").unwrap();
    let package = cfb::stream(file, "EncryptedPackage").unwrap();
    let wrap = |info: Vec<u8>, package: Vec<u8>| cfb::write(&[
        cfb::Item::stream("EncryptionInfo", info),
        cfb::Item::stream("EncryptedPackage", package),
    ]);
    let mut unknown = info.clone();
    unknown[20..24].copy_from_slice(&0x6611u32.to_le_bytes());
    assert!(matches!(decrypt(&wrap(unknown, package.clone()), "Password1234_"), Err(DecryptError::Unsupported(_))));
    let mut truncated = info.clone();
    truncated.truncate(40);
    assert!(matches!(decrypt(&wrap(truncated, package.clone()), "Password1234_"), Err(DecryptError::Damaged(_))));
    let mut bad_size = package;
    bad_size[..8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(decrypt(&wrap(info, bad_size), "Password1234_"), Err(DecryptError::Damaged(_))));
}

/// Independently produced with msoffcrypto-tool and Python cryptography, using
/// a UTF-16 password with a supplementary character and all three AES sizes.
#[test]
fn standard_key_sizes_and_unicode_password() {
    fn bytes(s: &str) -> Vec<u8> {
        s.as_bytes().chunks_exact(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
    }
    for (bits, algorithm, encrypted_verifier, encrypted_hash, encrypted_data) in [
        (128u32, 0x660eu32, "9d544e3af57fbe64cae260c09ba8efd1", "ad2742639dd96ea073da81926342a8f285a4002c1c4dc29de7b68e4af08ef938", "a68d85df89735ca37b04277385d6ffafdc07ad863e4c3537733013d6f03f9a64"),
        (192, 0x660f, "5d06b737785b66d32037b90fd9976ed7", "e3775965d4a3c607992878cc1f0de86f8a390549ffff90ce36f805323f7d61c0", "cb191c7ddf628aee962ab9e58b940b6f0d981a97b5998f41211d9f3e7018c046"),
        (256, 0x6610, "178e4ae5bc38b931d377dfae70e3932c", "9a5ec3cc17f773f9ecc798cbf03840d902fcc2341e41026cbd42efc4e215b7ee", "81003ce7ff0ce26d5397541d9d34a9db6dbb036fe40b2262671d4f706b615fbe"),
    ] {
        let mut info = vec![3, 0, 2, 0];
        for n in [0x24u32, 32, 0x24, 0, algorithm, 0x8004, bits, 0x18, 0, 0, 16] { info.extend(n.to_le_bytes()); }
        info.extend(0u8..16);
        info.extend(bytes(encrypted_verifier));
        info.extend(20u32.to_le_bytes());
        info.extend(bytes(encrypted_hash));
        let mut package = 26u64.to_le_bytes().to_vec();
        package.extend(bytes(encrypted_data));
        let file = cfb::write(&[cfb::Item::stream("EncryptionInfo", info), cfb::Item::stream("EncryptedPackage", package)]);
        assert_eq!(decrypt(&file, "日本語🔑").unwrap(), b"PK\x03\x04 standard test payload", "AES-{bits}");
        assert_eq!(decrypt(&file, "日本語"), Err(DecryptError::WrongPassword));
    }
}

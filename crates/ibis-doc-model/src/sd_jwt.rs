// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! RFC 9901 (SD-JWT) and SD-JWT VC generation engine with pure-Rust SHA-256, Base64url,
//! NIST P-256 (secp256r1) JWK handling, and RFC 6979 deterministic ES256 (ECDSA P-256 / SHA-256)
//! signing and verification.

use std::collections::BTreeMap;

use crate::model::{Presentation, SlideId};
use crate::sd_jwt_binding::{BoundClaim, ClaimValue, SheetClaimBindings, extract_sheet_claims};

// ---------------------------------------------------------------------------
// SHA-256 & HMAC-SHA256 (FIPS 180-4 / RFC 2104)
// ---------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Computes the 32-byte SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((data.len() + 9 + 63) & !63);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 64];
    for chunk in padded.chunks_exact(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Computes the 32-byte HMAC-SHA256 of `data` under `key`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k_block[i];
        opad[i] ^= k_block[i];
    }
    let mut inner = Vec::with_capacity(64 + data.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(data);
    let inner_hash = sha256(&inner);

    let mut outer = [0u8; 96];
    outer[..64].copy_from_slice(&opad);
    outer[64..].copy_from_slice(&inner_hash);
    sha256(&outer)
}

// ---------------------------------------------------------------------------
// Base64url (RFC 4648 §5, unpadded)
// ---------------------------------------------------------------------------

const B64URL_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encodes `data` as unpadded Base64url (RFC 4648 §5).
pub fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(B64URL_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(B64URL_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(B64URL_ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(B64URL_ALPHABET[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        out.push(B64URL_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(B64URL_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(B64URL_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(B64URL_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(B64URL_ALPHABET[((n >> 6) & 0x3F) as usize] as char);
    }
    out
}

/// Decodes an unpadded (or `=`-padded) Base64url string.
pub fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let trimmed = input.trim().trim_end_matches('=');
    if trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    let bytes = trimmed.as_bytes();
    let val = |b: u8| -> Option<u32> {
        match b {
            b'A'..=b'Z' => Some((b - b'A') as u32),
            b'a'..=b'z' => Some((b - b'a' + 26) as u32),
            b'0'..=b'9' => Some((b - b'0' + 52) as u32),
            b'-' | b'+' => Some(62),
            b'_' | b'/' => Some(63),
            _ => None,
        }
    };
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let n = (val(bytes[i])? << 18)
            | (val(bytes[i + 1])? << 12)
            | (val(bytes[i + 2])? << 6)
            | val(bytes[i + 3])?;
        out.push(((n >> 16) & 0xFF) as u8);
        out.push(((n >> 8) & 0xFF) as u8);
        out.push((n & 0xFF) as u8);
        i += 4;
    }
    let rem = bytes.len() - i;
    if rem == 2 {
        let n = (val(bytes[i])? << 18) | (val(bytes[i + 1])? << 12);
        out.push(((n >> 16) & 0xFF) as u8);
    } else if rem == 3 {
        let n = (val(bytes[i])? << 18) | (val(bytes[i + 1])? << 12) | (val(bytes[i + 2])? << 6);
        out.push(((n >> 16) & 0xFF) as u8);
        out.push(((n >> 8) & 0xFF) as u8);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// 256-bit Modular Arithmetic & NIST P-256 (secp256r1) for ES256 (RFC 7515)
// ---------------------------------------------------------------------------

/// Little-endian 4-limb 256-bit unsigned integer (`limbs[0]` is least significant).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct U256([u64; 4]);

const P256_P: U256 = U256([
    0xFFFFFFFFFFFFFFFF,
    0x00000000FFFFFFFF,
    0x0000000000000000,
    0xFFFFFFFF00000001,
]);

const P256_N: U256 = U256([
    0xF3B9CAC2FC632551,
    0xBCE6FAADA7179E84,
    0xFFFFFFFFFFFFFFFF,
    0xFFFFFFFF00000000,
]);

const P256_B: U256 = U256([
    0x3BCE3C3E27D2604B,
    0x651D06B0CC53B0F6,
    0xB3EBBD55769886BC,
    0x5AC635D8AA3A93E7,
]);

const P256_GX: U256 = U256([
    0xF4A13945D898C296,
    0x77037D812DEB33A0,
    0xF8BCE6E563A440F2,
    0x6B17D1F2E12C4247,
]);

const P256_GY: U256 = U256([
    0xCBB6406837BF51F5,
    0x2BCE33576B315ECE,
    0x8EE7EB4A7C0F9E16,
    0x4FE342E2FE1A7F9B,
]);

impl U256 {
    const ZERO: Self = Self([0, 0, 0, 0]);
    const ONE: Self = Self([1, 0, 0, 0]);

    fn is_zero(&self) -> bool {
        self.0 == [0, 0, 0, 0]
    }

    fn from_be_bytes(b: &[u8; 32]) -> Self {
        Self([
            u64::from_be_bytes(b[24..32].try_into().unwrap()),
            u64::from_be_bytes(b[16..24].try_into().unwrap()),
            u64::from_be_bytes(b[8..16].try_into().unwrap()),
            u64::from_be_bytes(b[0..8].try_into().unwrap()),
        ])
    }

    fn to_be_bytes(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[0..8].copy_from_slice(&self.0[3].to_be_bytes());
        out[8..16].copy_from_slice(&self.0[2].to_be_bytes());
        out[16..24].copy_from_slice(&self.0[1].to_be_bytes());
        out[24..32].copy_from_slice(&self.0[0].to_be_bytes());
        out
    }

    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        for i in (0..4).rev() {
            if self.0[i] != other.0[i] {
                return self.0[i].cmp(&other.0[i]);
            }
        }
        std::cmp::Ordering::Equal
    }

    fn bit(&self, idx: usize) -> bool {
        let limb = idx / 64;
        let shift = idx % 64;
        if limb >= 4 {
            false
        } else {
            ((self.0[limb] >> shift) & 1) != 0
        }
    }

    fn add_raw(&self, rhs: &Self) -> (Self, u64) {
        let mut out = [0u64; 4];
        let mut carry = 0u128;
        for i in 0..4 {
            let sum = (self.0[i] as u128) + (rhs.0[i] as u128) + carry;
            out[i] = sum as u64;
            carry = sum >> 64;
        }
        (Self(out), carry as u64)
    }

    fn sub_raw(&self, rhs: &Self) -> (Self, u64) {
        let mut out = [0u64; 4];
        let mut borrow = 0u128;
        for i in 0..4 {
            let a = self.0[i] as u128;
            let b = (rhs.0[i] as u128) + borrow;
            if a >= b {
                out[i] = (a - b) as u64;
                borrow = 0;
            } else {
                out[i] = ((1u128 << 64) + a - b) as u64;
                borrow = 1;
            }
        }
        (Self(out), borrow as u64)
    }

    fn add_mod(&self, rhs: &Self, m: &Self) -> Self {
        let (sum, carry) = self.add_raw(rhs);
        if carry != 0 || sum.cmp(m) != std::cmp::Ordering::Less {
            sum.sub_raw(m).0
        } else {
            sum
        }
    }

    fn sub_mod(&self, rhs: &Self, m: &Self) -> Self {
        if self.cmp(rhs) != std::cmp::Ordering::Less {
            self.sub_raw(rhs).0
        } else {
            let diff = rhs.sub_raw(self).0;
            m.sub_raw(&diff).0
        }
    }

    fn mul_mod(&self, rhs: &Self, m: &Self) -> Self {
        // Double-and-add from MSB (255) down to 0: safe, constant-structure, no 512-bit division needed.
        let mut acc = Self::ZERO;
        let a = if self.cmp(m) != std::cmp::Ordering::Less {
            self.sub_raw(m).0
        } else {
            *self
        };
        for i in (0..256).rev() {
            acc = acc.add_mod(&acc, m);
            if rhs.bit(i) {
                acc = acc.add_mod(&a, m);
            }
        }
        acc
    }

    fn pow_mod(&self, exp: &Self, m: &Self) -> Self {
        let mut result = Self::ONE;
        let mut base = *self;
        for i in 0..256 {
            if exp.bit(i) {
                result = result.mul_mod(&base, m);
            }
            base = base.mul_mod(&base, m);
        }
        result
    }

    fn inv_mod(&self, prime: &Self) -> Option<Self> {
        if self.is_zero() {
            return None;
        }
        let two = Self([2, 0, 0, 0]);
        let exp = prime.sub_raw(&two).0;
        Some(self.pow_mod(&exp, prime))
    }
}

#[derive(Clone, Copy, Debug)]
struct JacobianPoint {
    x: U256,
    y: U256,
    z: U256,
}

impl JacobianPoint {
    const INFINITY: Self = Self {
        x: U256::ONE,
        y: U256::ONE,
        z: U256::ZERO,
    };

    fn from_affine(x: U256, y: U256) -> Self {
        Self { x, y, z: U256::ONE }
    }

    fn is_infinity(&self) -> bool {
        self.z.is_zero()
    }

    fn to_affine(&self) -> Option<(U256, U256)> {
        if self.is_infinity() {
            return None;
        }
        let z_inv = self.z.inv_mod(&P256_P)?;
        let z_inv2 = z_inv.mul_mod(&z_inv, &P256_P);
        let z_inv3 = z_inv2.mul_mod(&z_inv, &P256_P);
        let x = self.x.mul_mod(&z_inv2, &P256_P);
        let y = self.y.mul_mod(&z_inv3, &P256_P);
        Some((x, y))
    }

    fn double(&self) -> Self {
        if self.is_infinity() || self.y.is_zero() {
            return Self::INFINITY;
        }
        let p = &P256_P;
        let z2 = self.z.mul_mod(&self.z, p);
        // On P-256, a = -3, so M = 3 * (X - Z^2) * (X + Z^2)
        let x_minus_z2 = self.x.sub_mod(&z2, p);
        let x_plus_z2 = self.x.add_mod(&z2, p);
        let prod = x_minus_z2.mul_mod(&x_plus_z2, p);
        let m = prod.add_mod(&prod, p).add_mod(&prod, p);

        let y2 = self.y.mul_mod(&self.y, p);
        let xy2 = self.x.mul_mod(&y2, p);
        let xy2_2 = xy2.add_mod(&xy2, p);
        let s = xy2_2.add_mod(&xy2_2, p); // 4 * X * Y^2

        let m2 = m.mul_mod(&m, p);
        let two_s = s.add_mod(&s, p);
        let x3 = m2.sub_mod(&two_s, p);

        let y4 = y2.mul_mod(&y2, p);
        let y4_2 = y4.add_mod(&y4, p);
        let y4_4 = y4_2.add_mod(&y4_2, p);
        let y4_8 = y4_4.add_mod(&y4_4, p); // 8 * Y^4

        let s_minus_x3 = s.sub_mod(&x3, p);
        let y3 = m.mul_mod(&s_minus_x3, p).sub_mod(&y4_8, p);

        let yz = self.y.mul_mod(&self.z, p);
        let z3 = yz.add_mod(&yz, p);

        Self { x: x3, y: y3, z: z3 }
    }

    fn add(&self, other: &Self) -> Self {
        if self.is_infinity() {
            return *other;
        }
        if other.is_infinity() {
            return *self;
        }
        let p = &P256_P;
        let z1z1 = self.z.mul_mod(&self.z, p);
        let z2z2 = other.z.mul_mod(&other.z, p);
        let u1 = self.x.mul_mod(&z2z2, p);
        let u2 = other.x.mul_mod(&z1z1, p);
        let z2_cubed = z2z2.mul_mod(&other.z, p);
        let z1_cubed = z1z1.mul_mod(&self.z, p);
        let s1 = self.y.mul_mod(&z2_cubed, p);
        let s2 = other.y.mul_mod(&z1_cubed, p);

        if u1 == u2 {
            if s1 == s2 {
                return self.double();
            } else {
                return Self::INFINITY;
            }
        }

        let h = u2.sub_mod(&u1, p);
        let r = s2.sub_mod(&s1, p);
        let hh = h.mul_mod(&h, p);
        let hhh = hh.mul_mod(&h, p);
        let v = u1.mul_mod(&hh, p);

        let r2 = r.mul_mod(&r, p);
        let two_v = v.add_mod(&v, p);
        let x3 = r2.sub_mod(&hhh, p).sub_mod(&two_v, p);

        let v_minus_x3 = v.sub_mod(&x3, p);
        let s1_hhh = s1.mul_mod(&hhh, p);
        let y3 = r.mul_mod(&v_minus_x3, p).sub_mod(&s1_hhh, p);

        let z1z2 = self.z.mul_mod(&other.z, p);
        let z3 = z1z2.mul_mod(&h, p);

        Self { x: x3, y: y3, z: z3 }
    }

    fn scalar_mul(&self, k: &U256) -> Self {
        let mut res = Self::INFINITY;
        for i in (0..256).rev() {
            res = res.double();
            if k.bit(i) {
                res = res.add(self);
            }
        }
        res
    }
}

fn is_on_p256_curve(x: &U256, y: &U256) -> bool {
    if x.cmp(&P256_P) != std::cmp::Ordering::Less || y.cmp(&P256_P) != std::cmp::Ordering::Less {
        return false;
    }
    let p = &P256_P;
    let lhs = y.mul_mod(y, p);
    let x2 = x.mul_mod(x, p);
    let x3 = x2.mul_mod(x, p);
    let three_x = x.add_mod(x, p).add_mod(x, p);
    let rhs = x3.sub_mod(&three_x, p).add_mod(&P256_B, p);
    lhs == rhs
}

/// RFC 6979 deterministic nonce `k` generation for ECDSA P-256 / SHA-256.
fn rfc6979_nonce(secret_d: &U256, msg_hash: &[u8; 32]) -> U256 {
    let x_bytes = secret_d.to_be_bytes();
    let h1_int = U256::from_be_bytes(msg_hash);
    let h1_mod = if h1_int.cmp(&P256_N) != std::cmp::Ordering::Less {
        h1_int.sub_raw(&P256_N).0
    } else {
        h1_int
    };
    let h1_bytes = h1_mod.to_be_bytes();

    let mut v = [0x01u8; 32];
    let mut k = [0x00u8; 32];

    let mut buf = Vec::with_capacity(32 + 1 + 32 + 32);
    buf.extend_from_slice(&v);
    buf.push(0x00);
    buf.extend_from_slice(&x_bytes);
    buf.extend_from_slice(&h1_bytes);
    k = hmac_sha256(&k, &buf);
    v = hmac_sha256(&k, &v);

    buf.clear();
    buf.extend_from_slice(&v);
    buf.push(0x01);
    buf.extend_from_slice(&x_bytes);
    buf.extend_from_slice(&h1_bytes);
    k = hmac_sha256(&k, &buf);
    v = hmac_sha256(&k, &v);

    loop {
        v = hmac_sha256(&k, &v);
        let candidate = U256::from_be_bytes(&v);
        if !candidate.is_zero() && candidate.cmp(&P256_N) == std::cmp::Ordering::Less {
            return candidate;
        }
        let mut retry = [0u8; 33];
        retry[..32].copy_from_slice(&v);
        retry[32] = 0x00;
        k = hmac_sha256(&k, &retry);
        v = hmac_sha256(&k, &v);
    }
}

/// An ES256 (ECDSA P-256 / SHA-256) keypair with JWK import/export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Es256KeyPair {
    pub d: [u8; 32],
    pub x: [u8; 32],
    pub y: [u8; 32],
}

impl Es256KeyPair {
    /// Creates an `Es256KeyPair` from a 32-byte big-endian private scalar `d`, computing `(x, y) = d * G`.
    pub fn from_secret_bytes(d: [u8; 32]) -> Option<Self> {
        let scalar = U256::from_be_bytes(&d);
        if scalar.is_zero() || scalar.cmp(&P256_N) != std::cmp::Ordering::Less {
            return None;
        }
        let g = JacobianPoint::from_affine(P256_GX, P256_GY);
        let (px, py) = g.scalar_mul(&scalar).to_affine()?;
        Some(Self {
            d,
            x: px.to_be_bytes(),
            y: py.to_be_bytes(),
        })
    }

    /// Generates an ephemeral P-256 keypair (or a deterministic test keypair if `ibis_calc::random` is in stand-in mode).
    pub fn generate_ephemeral(domain: &str) -> Self {
        let mut seed_material = Vec::from(domain.as_bytes());
        for i in 0..4u32 {
            let r = ibis_calc::random::next().unwrap_or(0.123456789 * (i as f64 + 1.0));
            seed_material.extend_from_slice(&r.to_bits().to_le_bytes());
        }
        let mut counter = 0u32;
        loop {
            let mut input = seed_material.clone();
            input.extend_from_slice(&counter.to_be_bytes());
            let mut candidate = sha256(&input);
            candidate[0] &= 0x7F;
            if let Some(kp) = Self::from_secret_bytes(candidate) {
                return kp;
            }
            counter = counter.wrapping_add(1);
        }
    }

    /// Deterministic demo Issuer keypair for immediate testing / preview.
    pub fn demo_issuer_key() -> Self {
        let d = sha256(b"ibis-office-sd-jwt-demo-issuer-es256-v1");
        Self::from_secret_bytes(d).expect("valid demo issuer scalar")
    }

    /// Deterministic demo Holder keypair for `cnf.jwk` and OID4VP Key Binding JWT (`kb+jwt`).
    pub fn demo_holder_key() -> Self {
        let d = sha256(b"ibis-office-sd-jwt-demo-holder-es256-v1");
        Self::from_secret_bytes(d).expect("valid demo holder scalar")
    }

    /// Parses an EC P-256 private JWK JSON string (extracting `"d"` and verifying/deriving `"x"`, `"y"`).
    pub fn from_jwk_json(jwk_json: &str) -> Option<Self> {
        let d_b64 = extract_json_string_field(jwk_json, "d")?;
        let d_vec = base64url_decode(&d_b64)?;
        let d_arr: [u8; 32] = d_vec.try_into().ok()?;
        let kp = Self::from_secret_bytes(d_arr)?;
        if let (Some(x_b64), Some(y_b64)) = (
            extract_json_string_field(jwk_json, "x"),
            extract_json_string_field(jwk_json, "y"),
        ) {
            let x_vec = base64url_decode(&x_b64)?;
            let y_vec = base64url_decode(&y_b64)?;
            if x_vec.as_slice() != kp.x || y_vec.as_slice() != kp.y {
                return None;
            }
        }
        Some(kp)
    }

    /// Exports the public key as a JWK `ClaimValue::Object` (`{"crv":"P-256","kty":"EC","x":"...","y":"..."}`).
    pub fn to_public_jwk(&self) -> ClaimValue {
        let mut map = BTreeMap::new();
        map.insert("crv".into(), ClaimValue::String("P-256".into()));
        map.insert("kty".into(), ClaimValue::String("EC".into()));
        map.insert("x".into(), ClaimValue::String(base64url_encode(&self.x)));
        map.insert("y".into(), ClaimValue::String(base64url_encode(&self.y)));
        ClaimValue::Object(map)
    }

    /// Exports the private key as a JWK `ClaimValue::Object` (`{"crv":"P-256","d":"...","kty":"EC","x":"...","y":"..."}`).
    pub fn to_private_jwk(&self) -> ClaimValue {
        let mut map = BTreeMap::new();
        map.insert("crv".into(), ClaimValue::String("P-256".into()));
        map.insert("d".into(), ClaimValue::String(base64url_encode(&self.d)));
        map.insert("kty".into(), ClaimValue::String("EC".into()));
        map.insert("x".into(), ClaimValue::String(base64url_encode(&self.x)));
        map.insert("y".into(), ClaimValue::String(base64url_encode(&self.y)));
        ClaimValue::Object(map)
    }

    /// Signs `message` using ES256 (ECDSA P-256 with SHA-256, RFC 6979 deterministic `k`),
    /// returning the 64-byte JWS signature `r || s`.
    pub fn sign_es256(&self, message: &[u8]) -> [u8; 64] {
        let z_bytes = sha256(message);
        let z_raw = U256::from_be_bytes(&z_bytes);
        let z = if z_raw.cmp(&P256_N) != std::cmp::Ordering::Less {
            z_raw.sub_raw(&P256_N).0
        } else {
            z_raw
        };
        let d = U256::from_be_bytes(&self.d);
        let k = rfc6979_nonce(&d, &z_bytes);
        let g = JacobianPoint::from_affine(P256_GX, P256_GY);
        let (x1, _y1) = g.scalar_mul(&k).to_affine().expect("non-zero k * G");
        let r = if x1.cmp(&P256_N) != std::cmp::Ordering::Less {
            x1.sub_raw(&P256_N).0
        } else {
            x1
        };
        let k_inv = k.inv_mod(&P256_N).expect("k invertible mod n");
        let rd = r.mul_mod(&d, &P256_N);
        let z_plus_rd = z.add_mod(&rd, &P256_N);
        let s = k_inv.mul_mod(&z_plus_rd, &P256_N);

        let mut sig = [0u8; 64];
        sig[..32].copy_from_slice(&r.to_be_bytes());
        sig[32..].copy_from_slice(&s.to_be_bytes());
        sig
    }

    /// Signs a JWS compact serialization `<b64url(header)>.<b64url(payload)>.<b64url(sig)>`.
    pub fn sign_jws(&self, header_json: &str, payload_json: &str) -> String {
        let h_b64 = base64url_encode(header_json.as_bytes());
        let p_b64 = base64url_encode(payload_json.as_bytes());
        let signing_input = format!("{h_b64}.{p_b64}");
        let sig = self.sign_es256(signing_input.as_bytes());
        let sig_b64 = base64url_encode(&sig);
        format!("{signing_input}.{sig_b64}")
    }
}

/// Verifies a 64-byte ES256 signature `r || s` over `message` against public key `(pub_x, pub_y)`.
pub fn verify_es256(pub_x: &[u8; 32], pub_y: &[u8; 32], message: &[u8], sig: &[u8; 64]) -> bool {
    let qx = U256::from_be_bytes(pub_x);
    let qy = U256::from_be_bytes(pub_y);
    if !is_on_p256_curve(&qx, &qy) {
        return false;
    }
    let r = U256::from_be_bytes(sig[..32].try_into().unwrap());
    let s = U256::from_be_bytes(sig[32..64].try_into().unwrap());
    if r.is_zero()
        || r.cmp(&P256_N) != std::cmp::Ordering::Less
        || s.is_zero()
        || s.cmp(&P256_N) != std::cmp::Ordering::Less
    {
        return false;
    }
    let z_bytes = sha256(message);
    let z_raw = U256::from_be_bytes(&z_bytes);
    let z = if z_raw.cmp(&P256_N) != std::cmp::Ordering::Less {
        z_raw.sub_raw(&P256_N).0
    } else {
        z_raw
    };
    let Some(w) = s.inv_mod(&P256_N) else {
        return false;
    };
    let u1 = z.mul_mod(&w, &P256_N);
    let u2 = r.mul_mod(&w, &P256_N);

    let g = JacobianPoint::from_affine(P256_GX, P256_GY);
    let q = JacobianPoint::from_affine(qx, qy);
    let p1 = g.scalar_mul(&u1);
    let p2 = q.scalar_mul(&u2);
    let Some((x1, _)) = p1.add(&p2).to_affine() else {
        return false;
    };
    let v = if x1.cmp(&P256_N) != std::cmp::Ordering::Less {
        x1.sub_raw(&P256_N).0
    } else {
        x1
    };
    v == r
}

fn extract_json_string_field(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let mut search_from = 0;
    while let Some(pos) = json[search_from..].find(&pattern) {
        let after_key = search_from + pos + pattern.len();
        let rest = json[after_key..].trim_start();
        if let Some(after_colon) = rest.strip_prefix(':') {
            let val_rest = after_colon.trim_start();
            if let Some(inside) = val_rest.strip_prefix('"') {
                let mut out = String::new();
                let mut chars = inside.chars();
                while let Some(ch) = chars.next() {
                    if ch == '"' {
                        return Some(out);
                    } else if ch == '\\' {
                        if let Some(esc) = chars.next() {
                            out.push(esc);
                        }
                    } else {
                        out.push(ch);
                    }
                }
            }
        }
        search_from = after_key;
    }
    None
}

// ---------------------------------------------------------------------------
// RFC 9901 SD-JWT Disclosure & Issuance Engine
// ---------------------------------------------------------------------------

/// A single RFC 9901 object-property Disclosure (`["<salt>", "<claim_name>", <value>]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdJwtDisclosure {
    /// Full dot-split claim path (e.g. `["address", "region"]`).
    pub path: Vec<String>,
    /// Canonical dot-joined path (e.g. `"address.region"`).
    pub path_str: String,
    /// 128-bit Base64url salt string.
    pub salt: String,
    /// Leaf property name in its parent object (e.g. `"region"`).
    pub claim_name: String,
    /// Claim value.
    pub value: ClaimValue,
    /// Compact JSON array `["<salt>","<claim_name>",<value>]`.
    pub json: String,
    /// Base64url-encoded Disclosure string.
    pub encoded: String,
    /// Base64url-encoded SHA-256 digest over the US-ASCII bytes of `encoded` (placed in `_sd`).
    pub digest: String,
    /// Whether this Disclosure is currently selected by `disclose.<path>` checkboxes.
    pub disclose: bool,
}

impl SdJwtDisclosure {
    pub fn new(
        path: Vec<String>,
        path_str: String,
        salt: String,
        claim_name: String,
        value: ClaimValue,
        disclose: bool,
    ) -> Self {
        let arr = ClaimValue::Array(vec![
            ClaimValue::String(salt.clone()),
            ClaimValue::String(claim_name.clone()),
            value.clone(),
        ]);
        let json = arr.to_json();
        let encoded = base64url_encode(json.as_bytes());
        let digest = base64url_encode(&sha256(encoded.as_bytes()));
        Self {
            path,
            path_str,
            salt,
            claim_name,
            value,
            json,
            encoded,
            digest,
            disclose,
        }
    }
}

/// Configuration for issuing an SD-JWT from worksheet bindings.
#[derive(Clone, Debug)]
pub struct SdJwtOptions {
    /// JWT header `typ` (defaults to `"dc+sd-jwt"` for SD-JWT VC).
    pub typ: String,
    /// Fallback `iss` claim if `claim.iss` is not bound on the worksheet.
    pub default_iss: Option<String>,
    /// Fallback `vct` claim if `claim.vct` is not bound on the worksheet.
    pub default_vct: Option<String>,
    /// Fallback `iat` NumericDate if `claim.iat` is not bound on the worksheet.
    pub default_iat: Option<i64>,
    /// Fallback `exp` NumericDate if `claim.exp` is not bound on the worksheet.
    pub default_exp: Option<i64>,
    /// Optional Holder public JWK to embed as `cnf: { jwk: ... }` for Key Binding.
    pub holder_public_jwk: Option<ClaimValue>,
    /// Optional deterministic salt overrides by `path_str` (for reproducible tests).
    pub salt_overrides: BTreeMap<String, String>,
}

impl Default for SdJwtOptions {
    fn default() -> Self {
        Self {
            typ: "dc+sd-jwt".to_string(),
            default_iss: Some("https://issuer.example.com".to_string()),
            default_vct: Some("https://credentials.example.com/identity_credential".to_string()),
            default_iat: Some(1_700_000_000),
            default_exp: None,
            holder_public_jwk: None,
            salt_overrides: BTreeMap::new(),
        }
    }
}

/// Complete SD-JWT issuance and selective-presentation artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdJwtOutput {
    /// JWS protected header JSON (`{"alg":"ES256","typ":"dc+sd-jwt"}`).
    pub header_json: String,
    /// Issued JWT payload containing plain claims, `_sd` digest arrays, and `_sd_alg: "sha-256"`.
    pub payload: ClaimValue,
    /// Pretty-printed JWT payload JSON.
    pub payload_json: String,
    /// Compact JWS `<header>.<payload>.<signature>`.
    pub issuer_jwt: String,
    /// All generated Disclosures (for every `sd.<path>` claim).
    pub disclosures: Vec<SdJwtDisclosure>,
    /// Full issued SD-JWT containing all Disclosures (`<Issuer-JWT>~<D1>~...~<Dn>~`).
    pub sd_jwt_all: String,
    /// Selective presentation SD-JWT containing only Disclosures where `disclose == true` (`<Issuer-JWT>~<D_sel>~`).
    pub sd_jwt_presentation: String,
    /// Plain JSON view of the claims disclosed in `sd_jwt_presentation`.
    pub disclosed_claims: ClaimValue,
}

fn generate_salt_for_path(path_str: &str, index: usize) -> String {
    let mut seed = Vec::from(path_str.as_bytes());
    seed.extend_from_slice(&(index as u64).to_be_bytes());
    for i in 0..2u32 {
        let r = ibis_calc::random::next().unwrap_or(0.987654321 * (i as f64 + 1.0));
        seed.extend_from_slice(&r.to_bits().to_le_bytes());
    }
    let digest = sha256(&seed);
    // 128-bit (16-byte) salt per RFC 9901 §4.2.1 recommendation
    base64url_encode(&digest[..16])
}

fn insert_plain_at(root: &mut BTreeMap<String, ClaimValue>, path: &[String], value: ClaimValue) {
    let Some((first, rest)) = path.split_first() else { return };
    if rest.is_empty() {
        root.insert(first.clone(), value);
        return;
    }
    let entry = root
        .entry(first.clone())
        .or_insert_with(|| ClaimValue::Object(BTreeMap::new()));
    if !matches!(entry, ClaimValue::Object(_)) {
        *entry = ClaimValue::Object(BTreeMap::new());
    }
    if let ClaimValue::Object(child) = entry {
        insert_plain_at(child, rest, value);
    }
}

fn insert_sd_digest_at(root: &mut BTreeMap<String, ClaimValue>, parent_path: &[String], digest: String) {
    let Some((first, rest)) = parent_path.split_first() else {
        let sd_entry = root
            .entry("_sd".to_string())
            .or_insert_with(|| ClaimValue::Array(Vec::new()));
        if let ClaimValue::Array(arr) = sd_entry {
            arr.push(ClaimValue::String(digest));
            arr.sort_by(|a, b| a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")));
        }
        return;
    };
    let entry = root
        .entry(first.clone())
        .or_insert_with(|| ClaimValue::Object(BTreeMap::new()));
    if !matches!(entry, ClaimValue::Object(_)) {
        *entry = ClaimValue::Object(BTreeMap::new());
    }
    if let ClaimValue::Object(child) = entry {
        insert_sd_digest_at(child, rest, digest);
    }
}

/// Issues an RFC 9901 SD-JWT from extracted worksheet `bindings`, signing with `issuer_key`.
pub fn issue_sd_jwt(
    bindings: &SheetClaimBindings,
    options: &SdJwtOptions,
    issuer_key: &Es256KeyPair,
) -> SdJwtOutput {
    let mut payload_map: BTreeMap<String, ClaimValue> = BTreeMap::new();
    let mut disclosures: Vec<SdJwtDisclosure> = Vec::new();

    // 1. Populate plain claims (`claim.*`) and build Disclosures for selective claims (`sd.*`).
    for (idx, BoundClaim { path, path_str, selective, value, disclose, .. }) in bindings.claims.iter().enumerate() {
        if !*selective {
            insert_plain_at(&mut payload_map, path, value.clone());
        } else if let Some((leaf, parent)) = path.split_last() {
            let salt = options
                .salt_overrides
                .get(path_str)
                .cloned()
                .unwrap_or_else(|| generate_salt_for_path(path_str, idx));
            let d = SdJwtDisclosure::new(
                path.clone(),
                path_str.clone(),
                salt,
                leaf.clone(),
                value.clone(),
                *disclose,
            );
            insert_sd_digest_at(&mut payload_map, parent, d.digest.clone());
            disclosures.push(d);
        }
    }

    // 2. Apply metadata defaults if not already bound on the worksheet.
    if !payload_map.contains_key("iss")
        && let Some(iss) = options.default_iss.as_ref().filter(|s| !s.trim().is_empty())
    {
        payload_map.insert("iss".into(), ClaimValue::String(iss.trim().to_string()));
    }
    if !payload_map.contains_key("vct")
        && let Some(vct) = options.default_vct.as_ref().filter(|s| !s.trim().is_empty())
    {
        payload_map.insert("vct".into(), ClaimValue::String(vct.trim().to_string()));
    }
    if !payload_map.contains_key("iat")
        && let Some(iat) = options.default_iat
    {
        payload_map.insert("iat".into(), ClaimValue::Int(iat));
    }
    if !payload_map.contains_key("exp")
        && let Some(exp) = options.default_exp
    {
        payload_map.insert("exp".into(), ClaimValue::Int(exp));
    }
    if !payload_map.contains_key("cnf")
        && let Some(jwk) = options.holder_public_jwk.clone()
    {
        let mut cnf = BTreeMap::new();
        cnf.insert("jwk".into(), jwk);
        payload_map.insert("cnf".into(), ClaimValue::Object(cnf));
    }
    if !disclosures.is_empty() {
        payload_map.insert("_sd_alg".into(), ClaimValue::String("sha-256".into()));
    }

    let payload = ClaimValue::Object(payload_map);
    let payload_compact = payload.to_json();
    let payload_json = payload.to_json_pretty();

    let mut header_map = BTreeMap::new();
    header_map.insert("alg".into(), ClaimValue::String("ES256".into()));
    header_map.insert(
        "typ".into(),
        ClaimValue::String(if options.typ.trim().is_empty() {
            "dc+sd-jwt".to_string()
        } else {
            options.typ.trim().to_string()
        }),
    );
    let header_json = ClaimValue::Object(header_map).to_json();

    let issuer_jwt = issuer_key.sign_jws(&header_json, &payload_compact);

    let mut sd_jwt_all = format!("{issuer_jwt}~");
    let mut sd_jwt_presentation = format!("{issuer_jwt}~");
    for d in &disclosures {
        sd_jwt_all.push_str(&d.encoded);
        sd_jwt_all.push('~');
        if d.disclose {
            sd_jwt_presentation.push_str(&d.encoded);
            sd_jwt_presentation.push('~');
        }
    }

    let disclosed_claims = bindings.to_disclosed_claims_json();

    SdJwtOutput {
        header_json,
        payload,
        payload_json,
        issuer_jwt,
        disclosures,
        sd_jwt_all,
        sd_jwt_presentation,
        disclosed_claims,
    }
}

/// Convenience helper: extracts worksheet claims for `slide_id` and issues an SD-JWT in one call.
pub fn issue_sheet_sd_jwt(
    presentation: &Presentation,
    slide_id: SlideId,
    options: &SdJwtOptions,
    issuer_key: &Es256KeyPair,
) -> Option<(SheetClaimBindings, SdJwtOutput)> {
    let bindings = extract_sheet_claims(presentation, slide_id)?;
    let output = issue_sd_jwt(&bindings, options, issuer_key);
    Some((bindings, output))
}

// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork

use super::field_match::fold_ascii_width;
use crate::eval::Value;

/// Common ASCII mailbox syntax. Preserve the local part verbatim;
/// any aliases are a property of the receiving service.
pub(super) fn email_key(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let (local, domain) = text.split_once('@')?;
    if text.len() > 254
        || local.len() > 64
        || !local
            .split('.')
            .all(|atom| !atom.is_empty() && atom.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_{}|~\x60".contains(&b)))
        || !domain.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    Some((local.to_owned(), domain.to_ascii_lowercase()))
}

fn fixed_width_digits(value: &Value, width: usize) -> Option<String> {
    let digits = match value {
        Value::Text(text) => text.trim().chars().map(fold_ascii_width).filter(|c| !matches!(c, ' ' | '　' | '-')).collect(),
        Value::Number(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => format!("{n:.0}"),
        _ => return None,
    };
    (digits.len() == width && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// Japanese corporate number. All 13 digits fit exactly in an f64 cell.
pub(super) fn corporate_number(value: &Value) -> Option<String> {
    let digits = fixed_width_digits(value, 13)?;

    // National Tax Agency: start with weight 1 at the rightmost digit,
    // alternating 1 and 2 across the 12-digit body.
    let sum: u32 = digits.as_bytes()[1..]
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &digit)| u32::from(digit - b'0') * (1 + (i % 2) as u32))
        .sum();
    if u32::from(digits.as_bytes()[0] - b'0') != 9 - sum % 9 {
        return None;
    }
    Some(digits)
}

/// Japanese individual number. Preserve leading zeros from text cells.
pub(super) fn mynumber(value: &Value) -> Option<String> {
    let digits = fixed_width_digits(value, 12)?;
    let (body, check_digit) = digits.as_bytes().split_at(11);
    // From the right of the body, weights 2 through 7 repeat.
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &digit)| u32::from(digit - b'0') * (2 + (i % 6) as u32))
        .sum();
    let remainder = sum % 11;
    let expected = if remainder <= 1 { 0 } else { 11 - remainder };
    (u32::from(check_digit[0] - b'0') == expected).then_some(digits)
}

fn modulus_11(body: &[u8]) -> u32 {
    // From the right of the body, weights 2 through 7 repeat.
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &digit)| u32::from(digit - b'0') * (2 + (i % 6) as u32))
        .sum();
    sum % 11
}

fn drivers_license(value: &Value) -> Option<String> {
    let digits = fixed_width_digits(value, 12)?;
    let bytes = digits.as_bytes();
    // The NPA places the check digit at position 11 and the reissue
    // digit at position 12. Only the first ten digits enter the sum.
    // Take the units digit of the complement, including when it is 11.
    let expected = (11 - modulus_11(&bytes[..10])) % 10;
    (u32::from(bytes[10] - b'0') == expected).then_some(digits)
}

pub(super) fn compare_drivers_licenses(left: &Value, right: &Value) -> Option<u8> {
    let left = drivers_license(left)?;
    let right = drivers_license(right)?;
    Some(if left == right {
        2
    } else if left[..11] == right[..11] {
        1
    } else {
        0
    })
}

/// The printed number or two-line TD3 MRZ of a Japanese passport.
/// Issuer selection is explicit in the formula's mode.
pub(super) fn japanese_passport_number(value: &Value) -> Option<String> {
    let Value::Text(text) = value else { return None };
    let text = text.trim();
    if text.contains('\n') {
        return td3_passport_number(text).map(str::to_owned);
    }
    let number: String = text
        .chars()
        .map(fold_ascii_width)
        .filter(|c| !matches!(c, ' ' | '　' | '-'))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    is_japanese_passport_number(number.as_bytes()).then_some(number)
}

fn is_japanese_passport_number(bytes: &[u8]) -> bool {
    bytes.len() == 9 && bytes[..2].iter().all(u8::is_ascii_uppercase) && bytes[2..].iter().all(u8::is_ascii_digit)
}

fn td3_passport_number(text: &str) -> Option<&str> {
    let (upper, lower) = text.split_once('\n')?;
    let upper = upper.strip_suffix('\r').unwrap_or(upper);
    let upper_bytes = upper.as_bytes();
    let lower_bytes = lower.as_bytes();
    // ICAO Doc 9303 part 4: the issuer is on line 1. The nationality
    // on line 2 cannot stand in for the issuer.
    if upper_bytes.len() != 44
        || lower_bytes.len() != 44
        || upper_bytes[0] != b'P'
        || !matches!(upper_bytes[1], b'A'..=b'Z' | b'<')
        || &upper_bytes[2..5] != b"JPN"
        || !upper_bytes[5..].iter().all(|c| matches!(c, b'A'..=b'Z' | b'<'))
        || !lower_bytes.iter().all(|c| matches!(c, b'A'..=b'Z' | b'0'..=b'9' | b'<'))
        || !is_japanese_passport_number(&lower_bytes[..9])
    {
        return None;
    }
    // ICAO Doc 9303 part 3: modulus 10, cyclic weights 7, 3, 1.
    let sum: u32 = lower_bytes[..9]
        .iter()
        .zip([7, 3, 1].into_iter().cycle())
        .map(|(&c, weight)| {
            let value = if c.is_ascii_digit() { c - b'0' } else { c - b'A' + 10 };
            u32::from(value) * weight
        })
        .sum();
    (lower_bytes[9] == b'0' + (sum % 10) as u8).then_some(&lower[..9])
}

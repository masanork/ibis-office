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

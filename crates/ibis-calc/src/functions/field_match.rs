// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Pairwise comparison of explicitly typed fields.

use std::sync::LazyLock;

use regex::Regex;

use super::{err, number};
use crate::eval::convert::{date_from_serial, serial_from_date};
use crate::eval::{Ctx, Value};
use crate::formula::Expr;
use crate::japanese_date::{DateRange, parse_date};
use crate::value::CellError;

pub(super) fn field_match(ctx: &mut Ctx, args: &[Expr]) -> Value {
    ctx.map_args(args, |ctx, values| {
        if let Some(error) = values.iter().find_map(|value| match value {
            Value::Error(error) => Some(*error),
            _ => None,
        }) {
            return err(error);
        }
        let Value::Text(mode) = &values[2] else {
            return err(CellError::Value);
        };
        let (left, right) = (&values[0], &values[1]);
        let result = if mode.eq_ignore_ascii_case("date") {
            compare_dates(left, right, ctx.date1904())
        } else if mode.eq_ignore_ascii_case("address") {
            compare_keys(left, right, address_key)
        } else if mode.eq_ignore_ascii_case("gender") {
            compare_keys(left, right, gender_key)
        } else {
            return err(CellError::Value);
        };
        result.map_or_else(|| err(CellError::NA), |rank| number(f64::from(rank)))
    })
}

fn compare_keys<T: Eq>(left: &Value, right: &Value, key: impl Fn(&str) -> Option<T>) -> Option<u8> {
    let (Value::Text(left), Value::Text(right)) = (left, right) else {
        return None;
    };
    Some(if key(left)? == key(right)? { 2 } else { 0 })
}

fn compare_dates(left: &Value, right: &Value, date1904: bool) -> Option<u8> {
    let left = date_range(left, date1904)?;
    let right = date_range(right, date1904)?;
    Some(if left.last < right.first || right.last < left.first {
        0
    } else if left.complete && right.complete && !left.inferred && !right.inferred {
        2
    } else {
        1
    })
}

fn date_range(value: &Value, date1904: bool) -> Option<DateRange> {
    match value {
        Value::Text(text) => parse_date(text, true).ok(),
        Value::Number(serial) => {
            let last_serial = serial_from_date(9999, 12, 31, date1904);
            if !(0.0..=last_serial as f64).contains(serial) || serial.fract() != 0.0 {
                return None;
            }
            if !date1904 && matches!(*serial as i64, 0 | 60) {
                return None;
            }
            let day = date_from_serial(*serial as i64, date1904);
            Some(DateRange {
                first: day,
                last: day,
                inferred: false,
                complete: true,
            })
        }
        _ => None,
    }
}

fn fold_ascii_width(c: char) -> char {
    match c {
        '！'..='～' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
        _ => c,
    }
}

fn gender_key(text: &str) -> Option<bool> {
    let text: String = text.trim().chars().map(|c| fold_ascii_width(c).to_ascii_lowercase()).collect();
    match text.as_str() {
        "男" | "男性" | "m" | "male" => Some(true),
        "女" | "女性" | "f" | "female" => Some(false),
        _ => None,
    }
}

fn address_key(text: &str) -> Option<String> {
    // Anchor the pattern to the whole address. Only an explicit, complete
    // 丁目/番/号 sequence is interchangeable with the three-number form.
    // The prefix must end outside the numeral run and contain a Han or kana
    // character, so bare numeric identifiers do not take this path.
    static LABELED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\A(.*?[^0-9〇零一二三四五六七八九十百千\-‐‑−\s])\s*([0-9〇零一二三四五六七八九十百千]+)丁目\s*([0-9〇零一二三四五六七八九十百千]+)番\s*([0-9〇零一二三四五六七八九十百千]+)号\z")
            .expect("valid labeled address pattern")
    });
    static HYPHENATED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\A(.*?[^0-9〇零一二三四五六七八九十百千\-‐‑−\s])\s*([0-9]+)\s*[-‐‑−]\s*([0-9]+)\s*[-‐‑−]\s*([0-9]+)\z")
            .expect("valid hyphenated address pattern")
    });
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let folded: String = text
        .chars()
        .map(|c| match c {
            '０'..='９' | '－' => fold_ascii_width(c),
            _ => c,
        })
        .collect();
    if let Some(fields) = LABELED.captures(&folded).or_else(|| HYPHENATED.captures(&folded)) {
        let prefix = fields[1].trim_end();
        if prefix.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}'))
            && let (Some(chome), Some(block), Some(lot)) = (address_number(&fields[2]), address_number(&fields[3]), address_number(&fields[4]))
        {
            return Some(format!("{prefix}{chome}-{block}-{lot}"));
        }
    }
    Some(folded)
}

fn address_number(text: &str) -> Option<String> {
    if text.bytes().all(|b| b.is_ascii_digit()) {
        return Some(text.to_owned());
    }
    // Japanese positional units, in strictly descending order. Limit the
    // conversion to the small numerals used in these explicit address fields.
    let mut total = 0;
    let mut pending = None;
    let mut previous_unit = 10_000;
    for c in text.chars() {
        let c = if c == '零' { '〇' } else { c };
        if let Some(digit) = "〇一二三四五六七八九".chars().position(|d| d == c) {
            if pending.replace(digit).is_some() {
                return None;
            }
        } else {
            let unit = match c {
                '十' => 10,
                '百' => 100,
                '千' => 1000,
                _ => return None,
            };
            if unit >= previous_unit || pending == Some(0) {
                return None;
            }
            total += pending.take().unwrap_or(1) * unit;
            previous_unit = unit;
        }
    }
    Some((total + pending.unwrap_or(0)).to_string())
}

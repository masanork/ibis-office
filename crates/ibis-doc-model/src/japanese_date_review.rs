// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) masanork
//! Conservative date candidates for sheet review. Input is never changed here.
//! Only complete dates in explicitly supported forms are accepted.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateReview {
    Candidate(String),
    /// The century was inferred using the documented 30-year pivot.
    Inferred(String),
    Unrecognized,
    Invalid,
}

/// Recognize complete Gregorian or Japanese-era dates. Incomplete dates are
/// not guessed. Two-digit years use
/// 00–29 = 2000–2029 and 30–99 = 1930–1999 and are marked as inferred.
pub fn review_date(input: &str) -> DateReview {
    let input = input.trim();
    let normalized: String = input
        .chars()
        .map(|c| match c {
            '０'..='９' => char::from_u32(c as u32 - '０' as u32 + '0' as u32).unwrap_or(c),
            '／' => '/',
            '－' => '-',
            '．' => '.',
            _ => c,
        })
        .collect();
    let eras = [
        ("令和", 2018, (2019, 5, 1), None),
        ("R", 2018, (2019, 5, 1), None),
        ("r", 2018, (2019, 5, 1), None),
        ("平成", 1988, (1989, 1, 8), Some((2019, 4, 30))),
        ("H", 1988, (1989, 1, 8), Some((2019, 4, 30))),
        ("h", 1988, (1989, 1, 8), Some((2019, 4, 30))),
        ("昭和", 1925, (1926, 12, 25), Some((1989, 1, 7))),
        ("S", 1925, (1926, 12, 25), Some((1989, 1, 7))),
        ("s", 1925, (1926, 12, 25), Some((1989, 1, 7))),
        ("大正", 1911, (1912, 7, 30), Some((1926, 12, 24))),
        ("T", 1911, (1912, 7, 30), Some((1926, 12, 24))),
        ("t", 1911, (1912, 7, 30), Some((1926, 12, 24))),
        ("明治", 1867, (1873, 1, 1), Some((1912, 7, 29))),
        ("M", 1867, (1873, 1, 1), Some((1912, 7, 29))),
        ("m", 1867, (1873, 1, 1), Some((1912, 7, 29))),
    ];
    let era = eras.iter().find_map(|(name, base, start, end)| {
        normalized
            .strip_prefix(name)
            .map(|rest| (rest, *base, *start, *end))
    });
    let text = era.map_or(normalized.as_str(), |v| v.0);
    let text = if era.is_some() && text.starts_with('元') {
        format!("1{}", &text['元'.len_utf8()..])
    } else {
        text.to_owned()
    };
    let parts: Vec<&str> = if text.ends_with('日') {
        match text
            .strip_suffix('日')
            .and_then(|s| s.split_once('年'))
            .and_then(|(y, md)| md.split_once('月').map(|(m, d)| vec![y, m, d]))
        {
            Some(parts) => parts,
            None => return DateReview::Unrecognized,
        }
    } else {
        let delimiter = if text.contains('/') {
            '/'
        } else if text.contains('-') {
            '-'
        } else if text.contains('.') {
            '.'
        } else {
            return DateReview::Unrecognized;
        };
        text.split(delimiter).collect()
    };
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return DateReview::Unrecognized;
    }
    if era.is_none() && !matches!(parts[0].len(), 2 | 4) {
        return DateReview::Unrecognized;
    }
    let inferred = era.is_none() && parts[0].len() == 2;
    let (Ok(y), Ok(m), Ok(d)) = (
        parts[0].parse::<i32>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) else {
        return DateReview::Invalid;
    };
    if y <= 0 && !inferred {
        return DateReview::Invalid;
    }
    let year = match era {
        Some((_, base, _, _)) => match y.checked_add(base) {
            Some(year) => year,
            None => return DateReview::Invalid,
        },
        None if inferred && y < 30 => 2000 + y,
        None if inferred => 1900 + y,
        None => y,
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return DateReview::Invalid,
    };
    if year > 9999 || d == 0 || d > days {
        return DateReview::Invalid;
    }
    if era.is_some() && year < 1873 {
        return DateReview::Unrecognized;
    }
    if let Some((_, _, start, end)) = era {
        let date = (year, m, d);
        if date < start || end.is_some_and(|end| date > end) {
            return DateReview::Invalid;
        }
    }
    let result = format!("{year:04}-{m:02}-{d:02}");
    if inferred {
        DateReview::Inferred(result)
    } else {
        DateReview::Candidate(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_dates_and_era_boundaries() {
        assert_eq!(
            review_date("令和元年5月1日"),
            DateReview::Candidate("2019-05-01".into())
        );
        assert_eq!(
            review_date("平成３１年４月３０日"),
            DateReview::Candidate("2019-04-30".into())
        );
        assert_eq!(
            review_date("S60.4.1"),
            DateReview::Candidate("1985-04-01".into())
        );
        assert_eq!(review_date("平成31年5月1日"), DateReview::Invalid);
        assert_eq!(review_date("令和元年4月30日"), DateReview::Invalid);
        assert_eq!(
            review_date("2024/2/29"),
            DateReview::Candidate("2024-02-29".into())
        );
        assert_eq!(review_date("明治5年1月1日"), DateReview::Unrecognized);
        assert_eq!(review_date("1900/2/29"), DateReview::Invalid);
        assert_eq!(
            review_date("24/2/29"),
            DateReview::Inferred("2024-02-29".into())
        );
        assert_eq!(
            review_date("00/1/1"),
            DateReview::Inferred("2000-01-01".into())
        );
        assert_eq!(
            review_date("29/12/31"),
            DateReview::Inferred("2029-12-31".into())
        );
        assert_eq!(
            review_date("30/1/1"),
            DateReview::Inferred("1930-01-01".into())
        );
        assert_eq!(
            review_date("99/1/1"),
            DateReview::Inferred("1999-01-01".into())
        );
        assert_eq!(review_date("令和5年2月"), DateReview::Unrecognized);
        assert_eq!(review_date("2024/1-2"), DateReview::Unrecognized);
    }
}

// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read},
    path::Path,
};

pub(super) struct Difference {
    pub field: &'static str,
    pub declared: String,
    pub actual: String,
}

pub(super) enum Check {
    Missing,
    Unreadable,
    Invalid,
    Compared(Vec<Difference>),
}

impl Check {
    pub fn read(path: &Path, bytes: usize, count: usize) -> Self {
        let companion = path.with_extension("hed");
        match companion.symlink_metadata() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::Missing,
            Ok(meta) if meta.is_file() => {}
            _ => return Self::Unreadable,
        }
        let mut data = Vec::new();
        if File::open(companion)
            .and_then(|file| file.take(4097).read_to_end(&mut data))
            .is_err()
        {
            return Self::Unreadable;
        }
        if data.len() > 4096 {
            return Self::Invalid;
        }
        Self::compare(&data, path.file_name().unwrap_or_default(), bytes, count)
    }

    fn compare(data: &[u8], filename: &OsStr, bytes: usize, count: usize) -> Self {
        let Ok(text) = std::str::from_utf8(data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data))
        else {
            return Self::Invalid;
        };
        let mut lines = text.lines();
        let (Some(name), Some(size), Some(rows), None) =
            (lines.next(), lines.next(), lines.next(), lines.next())
        else {
            return Self::Invalid;
        };
        let number = |value: &str| {
            if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
                value.parse::<u64>().ok()
            } else {
                None
            }
        };
        let (Some(size), Some(rows)) = (number(size), number(rows)) else {
            return Self::Invalid;
        };
        if name.is_empty() {
            return Self::Invalid;
        }
        let mut differences = Vec::new();
        for (matches, field, declared, actual) in [
            (
                OsStr::new(name) == filename,
                "structured.hed-filename",
                name.to_owned(),
                filename.to_string_lossy().into_owned(),
            ),
            (
                size == bytes as u64,
                "structured.hed-bytes",
                size.to_string(),
                bytes.to_string(),
            ),
            (
                rows == count as u64,
                "structured.hed-records",
                rows.to_string(),
                count.to_string(),
            ),
        ] {
            if !matches {
                differences.push(Difference {
                    field,
                    declared,
                    actual,
                });
            }
        }
        Self::Compared(differences)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Missing => "structured.hed-missing",
            Self::Unreadable => "structured.hed-unreadable",
            Self::Invalid => "structured.hed-invalid",
            Self::Compared(differences) if differences.is_empty() => "structured.hed-matched",
            Self::Compared(_) => "structured.hed-mismatch",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Check, OsStr};

    #[test]
    fn compares_bom_and_crlf_notification_and_reports_each_mismatch() {
        let name = OsStr::new("test.csv");
        let good = b"\xef\xbb\xbftest.csv\r\n720\r\n0\r\n";
        assert!(
            matches!(Check::compare(good, name, 720, 0), Check::Compared(rows) if rows.is_empty())
        );
        let Check::Compared(rows) = Check::compare(b"other.csv\n719\n2\n", name, 720, 1) else {
            panic!("expected comparison");
        };
        assert_eq!(
            rows.iter().map(|row| row.field).collect::<Vec<_>>(),
            [
                "structured.hed-filename",
                "structured.hed-bytes",
                "structured.hed-records"
            ]
        );
        assert!(matches!(
            Check::compare(b"test.csv\n1\n+0\n", name, 1, 0),
            Check::Invalid
        ));
    }
}

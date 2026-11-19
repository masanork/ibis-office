// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::path::{Path, PathBuf};

const MAX_DIRECTORY_ENTRIES: usize = 10_000;

pub(crate) struct Frame {
    pub(crate) path: PathBuf,
    number: u64,
    pub(crate) missing: u64,
}

pub(crate) struct ImageSequence {
    pub(crate) number: u64,
    pub(crate) padding: usize,
    pub(crate) previous: Option<Frame>,
    pub(crate) next: Option<Frame>,
}

fn frame_name(path: &Path) -> Option<(&str, &str, u64)> {
    let stem = path.file_stem()?.to_str()?;
    let prefix = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &stem[prefix.len()..];
    Some((prefix, digits, digits.parse().ok()?))
}

impl ImageSequence {
    /// Discover the adjacent files with the same prefix, digit width and extension.
    pub(crate) fn discover(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        let (prefix, digits, number) = frame_name(path)?;
        let parent = path.parent()?;
        let directory = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let mut sequence = Self {
            number,
            padding: digits.len(),
            previous: None,
            next: None,
        };
        for (index, entry) in std::fs::read_dir(directory).ok()?.enumerate() {
            // A partial scan could report a false gap or a wrong neighbor.
            if index == MAX_DIRECTORY_ENTRIES {
                return None;
            }
            let entry = entry.ok()?;
            let name = entry.file_name();
            let candidate = Path::new(&name);
            if !candidate
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
            {
                continue;
            }
            let Some((other_prefix, other_digits, candidate_number)) = frame_name(candidate) else {
                continue;
            };
            if prefix != other_prefix
                || digits.len() != other_digits.len()
                || candidate_number == number
            {
                continue;
            }
            if entry.file_type().ok()?.is_file() {
                sequence.consider(parent.join(name), candidate_number);
            }
        }
        (sequence.previous.is_some() || sequence.next.is_some()).then_some(sequence)
    }

    fn consider(&mut self, path: PathBuf, number: u64) {
        let missing = self.number.abs_diff(number) - 1;
        let neighbor = if number < self.number {
            &mut self.previous
        } else {
            &mut self.next
        };
        if neighbor.as_ref().is_none_or(|frame| {
            missing < frame.missing || (number == frame.number && path < frame.path)
        }) {
            *neighbor = Some(Frame {
                path,
                number,
                missing,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_nearest_frame_and_counts_missing_numbers() {
        assert_eq!(
            frame_name(Path::new("shot-v2.0010.exr")),
            Some(("shot-v2.", "0010", 10))
        );
        assert!(frame_name(Path::new("plate.exr")).is_none());
        let mut sequence = ImageSequence {
            number: 10,
            padding: 4,
            previous: None,
            next: None,
        };
        for (name, number) in [
            ("0002.exr", 2),
            ("0017.exr", 17),
            ("0013.exr", 13),
            ("0009.exr", 9),
        ] {
            sequence.consider(name.into(), number);
        }
        let previous = sequence.previous.unwrap();
        let next = sequence.next.unwrap();
        assert_eq!((previous.path, previous.missing), ("0009.exr".into(), 0));
        assert_eq!((next.path, next.missing), ("0013.exr".into(), 2));
    }
}

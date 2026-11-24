// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Trust {
    Unverified,
    MachineConfirmed,
    HumanReviewed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    Unspecified,
    Fresh,
    Stale,
    Malformed,
}

#[derive(Clone, Debug)]
pub struct Review {
    pub trust: Trust,
    pub verifier: Option<(String, String)>,
    pub generated_at: Option<String>,
    pub malformed_generation: bool,
    pub stale_after: Option<Value>,
    deadline: Option<DateTime<Utc>>,
}

pub(super) fn present(value: &Value) -> bool {
    !value.is_null() && value.as_str() != Some("") && !value.as_array().is_some_and(Vec::is_empty)
}

pub(super) fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?;
    if text.len() == 10 {
        return Some(
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)?
                .and_utc(),
        );
    }
    // RFC 3339 date-times have a T (case-insensitive) as the separator.
    if !matches!(text.as_bytes().get(10), Some(b'T' | b't')) {
        return None;
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|value| value.to_utc())
}

impl Review {
    pub fn from_metadata(metadata: &Value) -> Self {
        let generated = &metadata["generated"];
        let generation = timestamp(&generated["at"]);
        let malformed_generation = present(generated) && generation.is_none();
        let verified = &metadata["verified"];
        let events = verified
            .as_array()
            .map_or_else(|| std::slice::from_ref(verified), Vec::as_slice);
        let evidence = events
            .iter()
            .filter_map(|event| {
                let actor = event["by"]
                    .as_str()
                    .filter(|actor| !actor.trim().is_empty())?;
                let at = timestamp(&event["at"])?;
                if malformed_generation || generation.is_some_and(|generation| at < generation) {
                    return None;
                }
                let trust = if actor
                    .strip_prefix("human:")
                    .is_some_and(|id| !id.trim().is_empty())
                {
                    Trust::HumanReviewed
                } else {
                    Trust::MachineConfirmed
                };
                Some((trust, at, actor, event["at"].as_str()?))
            })
            .max_by_key(|(trust, at, _, _)| (*trust, *at));
        let stale_after = &metadata["stale_after"];
        Self {
            trust: evidence.map_or(Trust::Unverified, |(trust, ..)| trust),
            verifier: evidence.map(|(_, _, by, at)| (by.to_owned(), at.to_owned())),
            generated_at: generated["at"].as_str().map(str::to_owned),
            malformed_generation,
            stale_after: present(stale_after).then(|| stale_after.clone()),
            deadline: timestamp(stale_after),
        }
    }

    pub fn freshness_at(&self, now: DateTime<Utc>) -> Freshness {
        match self.deadline {
            Some(deadline) if now >= deadline => Freshness::Stale,
            Some(_) => Freshness::Fresh,
            None if self.stale_after.is_some() => Freshness::Malformed,
            None => Freshness::Unspecified,
        }
    }

    pub fn freshness(&self) -> Freshness {
        self.freshness_at(Utc::now())
    }

    pub fn report(&self) -> Value {
        json!({
            "trust": match self.trust {
                Trust::Unverified => "unverified",
                Trust::MachineConfirmed => "machine-confirmed",
                Trust::HumanReviewed => "human-reviewed",
            },
            "freshness": match self.freshness() {
                Freshness::Unspecified => "unspecified",
                Freshness::Fresh => "fresh",
                Freshness::Stale => "stale",
                Freshness::Malformed => "malformed",
            },
            "verified": self.verifier.as_ref().map(|(by, at)| json!({"by": by, "at": at})),
            "generated_at": self.generated_at,
            "malformed_generation": self.malformed_generation,
            "stale_after": self.stale_after,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_verification_is_compared_as_an_instant_and_ranked_by_tier() {
        let mut metadata = json!({
            "generated": {"at": "2026-11-24T09:00:00+09:00"},
            "verified": [
                {"by": "human:old", "at": "2026-11-23T23:59:59Z"},
                {"by": "human:missing-date"},
                {"by": "team:ops", "at": "2026-11-25"}
            ]
        });
        assert_eq!(
            Review::from_metadata(&metadata).trust,
            Trust::MachineConfirmed
        );
        metadata["verified"]
            .as_array_mut()
            .unwrap()
            .push(json!({"by": "human:reviewer", "at": "2026-11-24"}));
        let review = Review::from_metadata(&metadata);
        assert_eq!(review.trust, Trust::HumanReviewed);
        assert_eq!(review.verifier.unwrap().0, "human:reviewer");
        metadata["generated"]["at"] = json!("invalid");
        let review = Review::from_metadata(&metadata);
        assert_eq!(review.trust, Trust::Unverified);
        assert!(review.malformed_generation);
    }

    #[test]
    fn freshness_handles_midnight_and_absent_or_malformed_values() {
        let raw = "---\ntype: Metric\nverified: {by: 'human:reviewer', at: 2026-11-23}\nstale_after: 2026-11-24\n---\n";
        let metadata = crate::okf::concept("a.md", raw).unwrap().unwrap();
        let review = Review::from_metadata(&metadata);
        assert_eq!(review.trust, Trust::HumanReviewed);
        let day = timestamp(&json!("2026-11-24")).unwrap();
        assert_eq!(
            review.freshness_at(day - chrono::TimeDelta::seconds(1)),
            Freshness::Fresh
        );
        assert_eq!(review.freshness_at(day), Freshness::Stale);
        for absent in [Value::Null, json!(""), json!([])] {
            assert_eq!(
                Review::from_metadata(&json!({"stale_after": absent})).freshness_at(day),
                Freshness::Unspecified
            );
        }
        for invalid in [
            json!("tomorrow"),
            json!(20261124),
            json!("2026-11-24 09:00:00Z"),
        ] {
            assert_eq!(
                Review::from_metadata(&json!({"stale_after": invalid})).freshness_at(day),
                Freshness::Malformed
            );
        }
    }
}

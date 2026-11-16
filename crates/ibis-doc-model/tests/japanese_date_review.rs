// SPDX-License-Identifier: AGPL-3.0-only
use ibis_doc_model::japanese_date_review::{DateReview, review_date};

#[test]
fn century_pivot_is_explicit_and_era_dates_are_validated() {
    for (input, expected) in [
        ("00/1/1", "2000-01-01"),
        ("29/12/31", "2029-12-31"),
        ("30/1/1", "1930-01-01"),
        ("99/12/31", "1999-12-31"),
    ] {
        assert_eq!(review_date(input), DateReview::Inferred(expected.into()));
    }
    for (input, expected) in [
        ("令和元年5月1日", "2019-05-01"),
        ("平成31年4月30日", "2019-04-30"),
        ("S60.4.1", "1985-04-01"),
        ("2024/02/29", "2024-02-29"),
    ] {
        assert_eq!(review_date(input), DateReview::Candidate(expected.into()));
    }
    for input in [
        "令和元年4月30日",
        "平成31年5月1日",
        "1900/02/29",
        "2024/13/1",
        "令和0年1月1日",
    ] {
        assert_eq!(review_date(input), DateReview::Invalid);
    }
    for input in ["明治5年1月1日", "令和5年2月", "2024/1-2", "4/2/1"] {
        assert_eq!(review_date(input), DateReview::Unrecognized);
    }
}

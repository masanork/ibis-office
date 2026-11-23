# e-Gov fixture

Source: [e-Gov 法令検索（デジタル庁）法令 API](https://laws.e-gov.go.jp/api/1/lawdata/321CONSTITUTION), retrieved 2026-11-18. The fixture contains the original Law element extracted from the API response, without the response wrapper. [Source terms](https://laws.e-gov.go.jp/terms).

SHA-256: `c1085ee027eccaa6ac956c5877ad3a169288775d82a3f473e67b2503f8a920f6`.

CI uses this snapshot without contacting the API. Lawtext is generated from the XML by ibisOffice and parsed back in the test. This verifies conversion with a real source document. It does not independently validate compatibility with other Lawtext implementations.

`administrative-procedure-act.lawtext` is the Administrative Procedure Act excerpt from the [upstream Lawtext README](https://github.com/yamachig/Lawtext/blob/60567c6ca7de6b14c8c1d895338c4614b49969d1/README.md#lawtextの例--example-of-lawtext), with its original whitespace and a final newline. The text is a public Japanese statute.

`supplementary-amendment.xml` is a synthetic statute. Its corpus test builds a second revision with an inserted supplementary block, checks the XML–Lawtext round trip, and runs the five-piece CLI with and without an explicit amendment law number. It exercises the same supplementary-article numbers, extract ranges, and verification paths as the public vaccination-law example.

Two public food-labeling snapshots were retrieved from the e-Gov Law API on 2026-11-23. Only the law XML is used, and linked form assets are excluded.

- `food-labeling-table.xml` is an excerpt of [食品表示基準](https://laws.e-gov.go.jp/api/1/lawdata/427M60000002010), retaining the title, enactment statement, first article, and 別表第十八. The table includes a row span.
- `food-inspection-form.xml` is the extracted `Law` element from the [inspection ordinance](https://laws.e-gov.go.jp/api/1/lawdata/427M60000042001). It includes three supplementary blocks and a prescribed form linked by a `Fig` element.

The corpus test checks supported text and records two current conversion losses, the merged cell geometry and the prescribed form.


`food-labeling-2024-before.xml` and `food-labeling-2024-after.xml` are excerpts from the e-Gov revisions of 食品表示基準 for [2024-04-01](https://laws.e-gov.go.jp/law/427M60000002010/20240401_506M60000002050) and [2024-09-01](https://laws.e-gov.go.jp/law/427M60000002010/20240901_506M60000002071). They retain the title, first article, and existing 別表第二十五 plus the two newly added tables in the later revision. This isolates table insertion from the other changes in the published amendment.

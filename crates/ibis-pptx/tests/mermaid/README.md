# Mermaid compatibility corpus

`tests/mermaid_corpus.rs` renders every example of the official Mermaid
documentation with `ibis_pptx::mermaid::render_mermaid` and reports, per
diagram document:

- **rendered**: the example produced an SVG or a native chart that passes the
  structural checks (well-formed SVG, plausible size, something drawn, no
  non-finite coordinates, chart series present).
- **clean**: rendered, and the example uses no syntax listed as `gap` in
  `features.txt`.

`thresholds.json` holds a floor for both numbers per document. CI fails when a
number drops below its floor, when a renderer panics or emits a broken SVG, or
when a diagram type without a floor starts rendering (add its floor). Raise the
floors when support improves.

The feature list doubles as the backlog: the report ranks `gap` features by the
number of examples that use them.

Frontmatter `config:` is tracked per option: a `config` line matches a dotted
key below `config` (`gitGraph.showBranches`, `themeVariables.git0`; a trailing
`$` asks for the whole key), and the `config | *` line catches every option no
other line names, so a new or unsupported option shows up as a gap instead of
passing silently. `wontfix` marks options left out on purpose: `look`
(hand-drawn), `layout` (ELK; the renderer has its own layout), `themeCSS`
(arbitrary CSS is not applied to documents) and edge animation (documents are
static).

```sh
cargo test -p ibis-pptx --test mermaid_corpus -- --nocapture
# write the report as Markdown, and every rendered SVG for inspection
MERMAID_CORPUS_REPORT=/tmp/report.md MERMAID_CORPUS_DUMP=/tmp/mermaid-svg \
  cargo test -p ibis-pptx --test mermaid_corpus
```

## Markdown demo

```sh
MERMAID_CORPUS_DEMO=/tmp/ibis-mermaid-demo cargo test -p ibis-pptx --test mermaid_corpus
```

Open the generated `index.md` for links to the per-diagram Markdown pages,
the pinned source, and the MIT license. The index lists every tracked feature,
including those with no matching official example. Each live Mermaid fence
includes the renderer result and any known limitations matched by the test.
CI publishes the same pages as `mermaid-markdown-demo`. The checked-in
[diagram gallery](../../../../docs/mermaid-demo-ja.md) has a shorter example from each
corpus document and supplemental cases.

## Corpus

`corpus/*.mmd` are the ```` ```mermaid ```` / ```` ```mermaid-example ```` blocks
of `docs/syntax/*.md` in mermaid-js/mermaid at a pinned commit (see
`corpus/SOURCE.json`), de-duplicated per document. Each case starts with a
`%%%% case <n> (<doc>.md:<line>)` line. Refresh with
`scripts/mermaid-corpus/fetch.py [--ref <sha> --tag <tag>]`, then re-run the
test and adjust `thresholds.json`.

The examples are MIT licensed (Copyright (c) 2014 - 2022 Knut Sveidqvist); see
`corpus/LICENSE`. They are test fixtures and are not shipped.

## mermaid.js reference comparison

`.github/workflows/mermaid-compat.yml` also runs a non-blocking job that
renders the same corpus with mermaid@12.3.0 (the version pinned in
`corpus/SOURCE.json`) in headless Chrome and compares structural metrics with
the ibisOffice SVGs: node and edge counts, and whether each mermaid.js label
is present in our output. Layout is not expected to match mermaid.js pixel for
pixel. The report is written to the job summary and uploaded as the
`mermaid-js-compare` artifact.

```sh
MERMAID_CORPUS_DUMP=/tmp/mermaid-svg cargo test -p ibis-pptx --test mermaid_corpus
(cd scripts/mermaid-corpus && npm ci)
node scripts/mermaid-corpus/compare-ref.mjs   --corpus crates/ibis-pptx/tests/mermaid/corpus   --ours /tmp/mermaid-svg   --out /tmp/mermaid-js-compare
```

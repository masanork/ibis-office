# Machine-readability reference fixtures

Synthetic input and selected outcomes from Harunobu 0.2.2 at commit
`88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f`, standard mode, AI disabled.
Source: https://github.com/digital-go-jp/machine_readability_rule

`structure.xlsx` contains a data merge, hidden row and column, and a formula.
`header-merge.xlsx` contains a header merge, which Harunobu reports as
information while passing L1-12. `data.csv` is a simple CSV; Excel-only rules
are absent from that reference result. The JSON records only L1-12, L1-13 and
L2-06; it is not a complete Harunobu report or a conformance result.

Regenerate using a clean checkout of the pinned commit and `uv sync --frozen`,
then run `python3 scripts/machine-readability-reference.py /path/to/harunobu`.
The Rust scanner detects structural candidates on the live sheet. It does not
infer tables or headers, so its findings are not Harunobu pass/fail results.

The Harunobu sample implementation is distributed under MIT; its rule data
and documentation use PDL 1.0. This fixture and recorded result were generated
for ibisOffice and are not an official Digital Agency certification.
See https://github.com/digital-go-jp/machine_readability_rule#ライセンスについて

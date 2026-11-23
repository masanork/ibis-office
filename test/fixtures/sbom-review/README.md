# Vulnerability review demo

These CycloneDX and OSV-Scanner files describe a fictional npm application.
Package names, versions and advisory IDs are synthetic.

1. In the desktop app, open `old.cdx.json` and inspect the **Components** sheet.
2. Open `old.osv.json` and select **Findings**. On the row for `fixed`, enter
   an owner, `2000-01-01` in **Due date**, `Deferred` in **Decision**, and a
   short rationale with an example evidence link. Save as `review.xlsx` and reopen it.
3. Choose **File → Refresh vulnerability review…**, and select `new.osv.json`.
   The `fixed` row retains its annotations and is missing from the latest scan.
   The `persistent` row changes version and `new` is newly detected.
4. Choose **File → Show findings requiring action…** in the refreshed workbook.
   The expired exception for `fixed` appears with the original decision and
   rationale. The two scan changes also require review. Save the separate
   workbook as `action-required.xlsx` to share the subset. Record decisions
   in the review workbook.

You can also compare the two releases in a terminal from this directory.

```sh
ibisOffice --export sbom-review \
  --old-sbom old.cdx.json --new-sbom new.cdx.json \
  --old-scan old.osv.json --new-scan new.osv.json \
  --out-file remediation.xlsx
```

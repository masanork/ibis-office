# Browser HTML export

The browser can download locally generated HTML from Word, Excel and
PowerPoint documents, including current edits. The output is a structured
approximation, not a pixel-perfect reproduction of the source.

- Word preserves basic text, formatting, links and simple tables; pictures and
  page layout are not reproduced.
- Excel includes visible worksheets and displayed values, basic formatting and
  merged cells. Hidden sheets, rows and columns are omitted. Formulas become
  displayed values; charts and images are not reproduced. Extremely large
  used grids are rejected to bound resource use.
- PowerPoint includes visible slides, positioned text, basic tables, solid
  shapes and supported raster images. Hidden slides, notes, animations and
  unsupported drawing features are omitted.

After building the Web app, serve the repository root on port 8800. Install
`playwright-core` separately for these optional browser tests, then run:

```sh
node scripts/test-html-browser.mjs
node scripts/test-office-html-browser.mjs
```

They edit synthetic DOCX/XLSX/PPTX fixtures, inspect the downloaded HTML,
and render it at a mobile viewport. `IBIS_CHROME_PATH` and the URL environment
variables in the scripts allow a different local browser or server. These
checks do not establish fidelity with every Office document or physical device.

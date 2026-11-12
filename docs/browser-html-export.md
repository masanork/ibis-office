# Browser HTML export

Open a Word, Excel, or PowerPoint document in the browser editor and choose
**File > Export > Web page (.html)**. The browser downloads a standalone UTF-8
HTML file generated locally from the current document, including edits.

- Word: basic text, formatting, links and simple tables. Embedded pictures and
  Word page layout are not reproduced.
- Excel: displayed cell values and basic formatting from all visible worksheets,
  with merged cells. Hidden or filtered rows, hidden columns and hidden sheets
  are omitted. Formulas are exported as displayed values; charts, images and
  other worksheet objects are not included. A workbook whose used grid exceeds
  250,000 cells returns an error instead of generating an unbounded file.
- PowerPoint: visible slides with positioned text, basic tables, solid shapes,
  and PNG, JPEG, GIF or WebP images embedded in the file. Hidden slides and
  shapes and speaker notes are omitted. Complex geometry, charts, animations,
  image cropping/effects and some formats are not reproduced.

This is a structured approximation rather than a pixel-accurate Office
rendering. Links in Word export use relative, HTTP, HTTPS, mailto or tel targets;
unsupported protocols remain plain text.

## Browser smoke tests

Build the Web app and run the Word test against a server for `www/`:

```sh
npm ci
RUSTUP_TOOLCHAIN=nightly-2026-10-28 scripts/build-web.sh
python3 -m http.server 8794 --bind 127.0.0.1 --directory www
# In another terminal:
IBIS_CHROME_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npm run test:html-browser
```

For the Excel and PowerPoint fixtures, serve the repository root on port 8795:

```sh
python3 -m http.server 8795 --bind 127.0.0.1 --directory .
# In another terminal:
IBIS_CHROME_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npm run test:office-html-browser
```

`IBIS_HTML_TEST_URL` and `IBIS_HTML_TEST_ROOT` override the respective default
URLs. The headless Chrome tests edit the fixtures, use the export menu, inspect
the downloaded HTML and render it at phone width. This is not a physical-device
or full Office visual-fidelity test. The `html-browser` job in Web font tests
builds the Web app and runs both smoke tests in CI.

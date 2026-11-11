# Browser HTML export

In the browser editor, open a Word document and choose **File > Export > Web
page (.html)**. The browser downloads a standalone UTF-8 HTML file generated
from the current document, including edits. Conversion happens locally.

The current exporter preserves basic text, formatting, links and simple tables.
It does not reproduce Word's page layout and does not export embedded pictures,
all nested table structures, or every document feature. Links use relative,
HTTP, HTTPS, mailto or tel targets; unsupported protocols remain plain text. Excel and PowerPoint
HTML export are not included.

For the browser smoke test, build the Web app, serve `www/`, and use a
Chrome-compatible browser with WebGPU:

```sh
npm ci
RUSTUP_TOOLCHAIN=nightly-2026-10-28 scripts/build-web.sh
python3 -m http.server 8794 --bind 127.0.0.1 --directory www
# In another terminal:
IBIS_CHROME_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npm run test:html-browser
```

`IBIS_HTML_TEST_URL` can override the default sample URL. The smoke test uses
Chrome's headless software GPU and checks the downloaded file's name, MIME type,
HTML, current edit and table. It expects the bundled sample at 1024×768.

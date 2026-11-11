# Browser touch smoke test

This optional smoke test opens the bundled sample in a local browser and sends
synthetic touch input through Chrome's DevTools protocol. It checks a tap,
two-finger zoom in and out, one-finger scrolling, phone page fitting, the
phone zoom button and menu scrolling, a text edit followed by a DOCX download,
and browser-side panics. It
requires a Chrome-compatible browser with WebGPU and does not substitute for
testing on physical iOS and Android devices.

```sh
npm ci
RUSTUP_TOOLCHAIN=nightly-2026-10-28 scripts/build-web.sh
python3 -m http.server 8793 --bind 127.0.0.1 --directory www
# In another terminal:
IBIS_CHROME_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npm run test:touch-browser
```

`IBIS_CHROME_PATH` selects the browser executable. `IBIS_TOUCH_URL` can override
the default local sample URL; the test expects the sample document's workspace
and zoom control at 1024×768 and 390×844. The script uses Chrome's software GPU
flags so it can run headlessly where available.

The input check uses Chrome's synthetic text insertion rather than a real
mobile keyboard or IME. It checks downloaded OOXML content both for the initial
URL document and after opening the sample through the file picker.

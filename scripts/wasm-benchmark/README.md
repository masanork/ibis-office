# Wasm delivery comparison fixture

This is a measurement fixture, not a production Worker. Prepare it only from a distributable build: its Worker URL and bundled assets are public. Use a new, isolated Worker name and account, with no production routes, resource bindings, credentials or data. Do not reuse a production Worker configuration.

Requirements: Python 3, `brotli` CLI, this repository's `playwright-core` and local Wrangler, and Chrome. `prepare.py` checks the original manifest, every part and the concatenated SHA-256, then writes four precompressed parts and a single gzip/Brotli asset into a **new** directory. The fixture expects exactly four parts.

```sh
python3 scripts/wasm-benchmark/prepare.py /path/to/www/pkg/ibis_office_app_bg.manifest.json /tmp/wasm-bench-assets
```

Put `worker.mjs` and a separate Wrangler configuration next to that asset directory. Configure an `ASSETS` binding with `run_worker_first: true`, `workers_dev: true`, and no custom routes or other bindings. Check the Worker name does not already exist and run `wrangler deploy --dry-run` before deploying the fixture. The Worker streams precompressed files with `encodeBody: 'manual'`. Its unconditional encoding selection and permissive CORS are suitable only for the controlled benchmark; a production implementation must negotiate encodings and validate clients, headers, cache policy, version skew, and memory behavior.

Run the actual app startup comparison with the candidate Worker URL and the same local manifest. It verifies the production manifest's digest before testing, uses fresh Chrome contexts with the browser cache disabled, emulates 40 ms latency and 5,000,000 B/s downlink, and alternates the order. It measures navigation until the app's `#status` element is removed; this is **not** proof of the first document being painted. Both loader variants are supplied by the test browser from code fetched before timing, so loading `wasm-loader.js` over the network is excluded equally for both. The candidate loader is injected only into the benchmark browser. In the streaming candidate, compilation runs alongside downloading, and the digest is checked before the compiled module is handed to the application.

```sh
BENCH_APP_URL=https://example.invalid/editor.html \
BENCH_CDN_ORIGIN=https://example.invalid \
BENCH_MANIFEST=/path/to/www/pkg/ibis_office_app_bg.manifest.json \
BENCH_CHROME='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
node scripts/wasm-benchmark/app-start.mjs
```

The default is three rounds; set `BENCH_REPEATS` to change it and `BENCH_VERSION` to set the font-setup version marker (default `0.8.7`). It skips the font-setup dialog for version 0.8.7, so its setup cost is excluded. It uses a mobile-size viewport in desktop Chrome, not a physical device. Do not interpret it as a complete font, document, privacy, browser compatibility, or peak-memory test. Collect per-resource transfer sizes, browser cache state, timing, and errors separately. When the measurement is complete, remove only the dedicated Worker and temporary files created for it.

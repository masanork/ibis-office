# Optional split WebAssembly assets

By default `scripts/build-web.sh` produces the existing single `.wasm` file.
For static hosts with a per-file limit, set `IBIS_SPLIT_WASM=1` when building:

```sh
IBIS_SPLIT_WASM=1 scripts/build-web.sh --release
```

The generated manifest lists content-addressed chunks of at most 20 MiB.
The browser checks each chunk's length and SHA-256, reconstructs it in order,
and checks the complete module's SHA-256 before starting. Keep the manifest
and every chunk together when publishing; do not cache the manifest longer
than the application entry point. Content-addressed chunks can be cached
immutably. A regular build removes the split manifest to avoid loading stale
chunks. Split delivery does not reduce total download bytes; reconstruction
also needs memory for both chunks and the combined module. It requires a secure
context that provides Web Crypto (HTTPS or localhost).

After a build, run `node --test test/web/wasm-loader.test.mjs` to check
reconstruction and rejection of missing, corrupted or reordered chunks.

For an optional Chrome smoke test, install `playwright-core` without changing
the project lockfile, serve the repository root on port 8799, and pass the
expected layout (`split` or `standard`) to `node scripts/test-wasm-browser.mjs`.
Set `IBIS_WEB_TEST_URL` or `IBIS_CHROME_PATH` for another server or browser.

# Self-hosted browser fonts

The editor loads TTF/OTF into GPUI before opening documents. GPUI shapes and
rasterizes them itself and uses their metrics for layout. CSS `@font-face`
is not used. The subset engine and serving code live in this repo; no GJS
checkout, generated binary or hosted GJS API is required.

## Build

Install Node dependencies and the wasm-bindgen CLI matching `Cargo.lock`:

```sh
npm ci
cargo install wasm-bindgen-cli --version 0.2.128 --locked
python3 scripts/fetch-web-fonts.py
IBIS_WEB_FONTS_CONFIG=scripts/web-fonts.sorane.json \
  RUSTUP_TOOLCHAIN=nightly-2026-10-28 scripts/build-web.sh --release
```

The script installs its pinned Rust toolchain if missing. `fonts/sources.json`
pins source URLs, extracted font SHA-256, object keys and attribution. The fetcher
checks hashes before writing ignored `data/fonts/`. Original license texts are
in `fonts/licenses/`. Fonts retain their own licenses; the engine is
AGPL-3.0-only. Its provenance is in `crates/ibis-font-subsetter/ORIGIN.md`.

The build compiles the engine, then creates `www/fonts.json` and
content-addressed font/license assets. Alternatively run `npm run build:font-engine`
and `npm run build:fonts -- CONFIG.json WWW_DIR`. When using your own source
fonts, downloading the catalog fonts is unnecessary.

## Font configuration

Paths are relative to the configuration file:

```json
{
  "defaultFont": "mincho",
  "uiFont": "gothic",
  "browserFallback": false,
  "dynamic": { "endpoint": "/api/font/subset", "font": "ipamjm", "strategy": "faces" },
  "fonts": [
    { "id": "mincho", "path": "./Mincho.ttf", "text": "ABCm", "aliases": ["游明朝"], "licenseFile": "./LICENSE.txt" },
    { "id": "gothic", "path": "./Gothic.otf", "aliases": ["游ゴシック"] }
  ]
}
```

- `id` uniquely names an entry. `defaultFont` selects the unresolved document
  face; `uiFont` independently selects menus and toolbars. Omitted `uiFont`
  uses the document default. Aliases affect display without rewriting stored
  typeface names. Conflicting aliases fail the build.
- `text` and/or UTF-8 `textFile` select the subset repertoire. Variation
  selectors are retained. With neither field the source is copied intact.
  Subsets receive separate `Ibis Subset <id>` families. Optional `family` can
  group regular/bold/italic entries. Attribution/license records are retained.
- `licenseFile` is copied and linked from the manifest. Use redistributable
  fonts and include their required notices.
- `browserFallback` defaults to false with a manifest. Set true to allow local
  browser CJK/emoji fallback. Without a manifest, legacy behavior remains.
- `dynamic.endpoint` accepts a same-origin path or HTTP(S) URL. Dynamic loading
  requires strict (`false`) browser fallback. Without `strategy: "faces"`, the
  legacy missing-glyph fallback provider is used.

Full fonts above 20 MiB are split into verified assets, then reassembled without
alteration. Hashes are checked for parts and complete fonts. Invalid assets or
families stop startup visibly. Rebuild and reload after configuration changes;
remove `fonts.json` to remove a deployed font set.

## Own font API and deployment

`worker/index.ts` serves `/api/font/subset` and forwards other paths to static
assets. It streams fonts from this project's `FONT_SOURCES` R2 binding into
its WASM engine. One active allocation bounds concurrent memory use. The
allowlist has IPAmj Mincho (`ipamjm`), Noto Sans CJK JP (`NotoSansCJKJP`) and
the additional administrative Mincho (`acgjm`).

```sh
# Once per account, when the bucket does not exist:
npx wrangler r2 bucket create ibis-office-fonts
# Local runtime qualification:
python3 scripts/upload-web-fonts.py
npx wrangler dev --compatibility-date 2026-10-30
# Production source provisioning and deployment:
python3 scripts/upload-web-fonts.py --remote
npx wrangler deploy --dry-run
npx wrangler deploy
```

The pinned local runtime supports dates through 2026-10-30, so local dev uses
that CLI override; production keeps its existing 2026-11-10 date.

GET parameters are `font`, `text` (1–128 Unicode code points), and `format`
(`otf` or `woff2`, default `otf`). Responses include `X-Font-Id`,
`X-Font-License`, `X-Original-Font`, ETag and `X-Cache`. Cache keys include exact
text, font ID, source ETag, engine hash and format, with a one-hour lifetime.
Unsupported glyphs return 204 without silently changing the face.
Busy/rate-limited requests return 429 with Retry-After.

The config preserves `ibis.sorane.dev`, enables tracing and error events,
and disables automatic invocation logs. Serving code omits requested characters
and request URLs from logs. Missing characters are still sent in the GET URL,
so the operator and network intermediaries can see them. Choose a service
appropriate for document confidentiality.

## ibis.sorane.dev behavior

`scripts/web-fonts.sorane.json` bootstraps ASCII from IPAmj and Noto. Noto is
used for UI; document Mincho and Gothic keep separate on-demand fallback chains.
Administrative Plane 16 characters request `acgjm` without Latin padding.
Current reporting does not detect every absent variation sequence when its
base glyph is available. This migration does not expand the three source fonts'
coverage or import GJS's other fonts/fallbacks. `/font-notices.html` has font
terms and attribution.

## Verification

```sh
cargo test --locked -p ibis-font-subsetter
npm run test:web
npm run check:worker
npx wrangler deploy --dry-run
```

Tests build the actual WASM and exercise IVS, CFF, composite glyphs, Plane 16,
cache isolation, stream failures and resource release. Original tiny synthetic
fixtures make CI independent of external repositories, font downloads and APIs.

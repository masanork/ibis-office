# Self-hosted fonts in the browser

The web editor can load supplied TTF/OTF files into GPUI before opening a
document. GPUI shapes and rasterizes these fonts itself. This also makes the
font's real metrics available to document layout. CSS `@font-face` is not used.

Generate `www/fonts.json` and its content-addressed font assets with:

```sh
npm run build:fonts -- scripts/web-fonts.example.json ../gjs
RUSTUP_TOOLCHAIN=nightly scripts/build-web.sh --release
```

The example uses a full Noto Sans CJK JP from the sibling gjs checkout and an
ASCII subset of the bundled Carlito mapped to Calibri. Download/populate gjs's
source fonts first, or change `path` to your own font files. Generated fonts and
the manifest are ignored by Git; deploy them with the rest of `www/`.

For a single build command, set `IBIS_WEB_FONTS_CONFIG` to the configuration
file and optionally `GJS_DIR` to the gjs checkout:

```sh
IBIS_WEB_FONTS_CONFIG=scripts/web-fonts.example.json \
GJS_DIR=../gjs RUSTUP_TOOLCHAIN=nightly scripts/build-web.sh --release
```

Configuration paths are relative to the configuration file. `GJS_DIR` is
relative to the command's working directory. Only subsetting requires gjs;
full-font delivery works without it. The build tool loads gjs's existing
`crates/font-subsetter/pkg/font_subsetter_bg.js` and `.wasm`. It uses
`subset_font_raw`, with IVS support, to produce uncompressed SFNT fonts that
GPUI can read. WOFF, WOFF2 and TTC files are not accepted as source files.

```json
{
  "defaultFont": "mincho",
  "browserFallback": false,
  "fonts": [
    {
      "id": "mincho",
      "path": "./MyFont.ttf",
      "textFile": "./repertoire.txt",
      "aliases": ["游明朝", "Yu Mincho"],
      "licenseFile": "./LICENSE.txt"
    }
  ]
}
```

- `id` uniquely identifies each face. `defaultFont` selects one of these IDs;
  if omitted, the first face is the default for the UI and unresolved document
  typefaces. Aliases affect display, leaving the document's stored typeface names
  unchanged.
- `text` and/or UTF-8 `textFile` select the subset repertoire. Neither is
  normalized or deduplicated, preserving base characters and their variation
  selectors. Without either field the original font is copied intact.
- gjs names all subsets `GJS Subset`. The build tool assigns each subset a unique
  `Ibis Subset <id>` family so independent fonts cannot collide. For regular,
  bold and italic faces of the same family, set the same optional `family` in
  their subset entries. Original copyright and license name records are retained.
- `aliases` maps Office typeface names to the supplied family. The original
  family name is also mapped automatically. Conflicting mappings fail the build.
- `licenseFile`, when present, is copied alongside the font and linked from its
  manifest entry. Use fonts you can redistribute and include their required
  notices.
- With a manifest, `browserFallback` defaults to `false` in the build tool.
  All text, including emoji, then depends on supplied and bundled fonts. Set
  it to `true` to allow the existing local-browser CJK/emoji fallback for missing
  glyphs. Without `fonts.json` (HTTP 404), the previous browser behavior remains.

Subsets are prepared at build time. Include all characters required by the UI,
documents and subsequent edits, or supply a full font. New characters are not
fetched automatically. No document text or font bytes are sent to the gjs API.
This mechanism is for the browser build; native font discovery is unchanged.

The loader verifies each file's SHA-256 and resolves font URLs relative to the
manifest, including deployments under a subpath. Failed font downloads,
invalid manifests, and unregistered families stop startup with a visible error.
After changing the font configuration, rebuild the assets and reload the page.
To remove a configured font set, remove the deployed `fonts.json` too.

Validation:

```sh
npm run test:fonts
GJS_DIR=../gjs npm run test:fonts  # also exercise the actual gjs subsetter
```

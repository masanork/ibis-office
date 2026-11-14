# ibisOffice font subsetter

An IVS-aware TTF/CFF OpenType subsetter, built and maintained in this workspace.
The code is AGPL-3.0-only; see [ORIGIN.md](ORIGIN.md) for its provenance and
retained original notice. Font and dependency licenses remain independent.

`subset_font_raw` produces SFNT, `subset_font` produces WOFF2, and
`font_has_glyphs` checks coverage. `FontSubsetter` accepts a known-size source
in chunks, owns it in WASM memory, and exposes `free()` to release it.
Incomplete inputs and sources above 50 MiB are rejected. Sources must be single
TTF/OTF fonts, not WOFF, WOFF2 or collections. Selected variation sequences, composite dependencies, OpenType layout features
(including GSUB/GPOS/GDEF and vertical substitutions), vertical metrics, and
attribution/license names are retained. Unsupported legacy tables such as AAT
layout and legacy `kern` are not guaranteed to survive subsetting.
Derivative fonts are renamed `IBS Subset`.

```sh
cargo test --locked -p ibis-font-subsetter
```

The build uses locked dependencies and the matching wasm-bindgen CLI.
Generated `pkg/` files are ignored. Original synthetic TTF/CFF fixtures in
`tests/fonts/` need no downloads. Their optional generator needs fontTools.

#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (c) ibis inc.
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.
# Build the browser version into www/.
#
#   scripts/build-web.sh            debug build
#   scripts/build-web.sh --release  release build (plus wasm-opt when installed)
#   IBIS_WEB_BOM_DIR=path scripts/build-web.sh --release  also save the built pkg/ and its BOMs
#
# Needs: rustup, wasm-bindgen-cli whose version matches the wasm-bindgen crate
# in Cargo.lock, and `npm ci` for tsc. The Rust toolchain is chosen here: the
# wasm_thread crate (threads in Web Workers) uses #![feature], so the browser
# version builds with a pinned nightly (installed with the wasm32 target when
# missing); RUSTUP_TOOLCHAIN set by the caller wins.
set -euo pipefail
cd "$(dirname "$0")/.."
# The distribution keeps the release defaults (no incremental, default LTO); Cargo.toml's release profile is for development.
source scripts/dist-profile.sh

WEB_TOOLCHAIN=nightly-2026-09-17
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-$WEB_TOOLCHAIN}"
if ! rustup run "$RUSTUP_TOOLCHAIN" rustc --version >/dev/null 2>&1; then
  rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal --target wasm32-unknown-unknown
elif ! rustup target list --installed --toolchain "$RUSTUP_TOOLCHAIN" | grep -qx wasm32-unknown-unknown; then
  rustup target add wasm32-unknown-unknown --toolchain "$RUSTUP_TOOLCHAIN"
fi

# The browser version runs in a host's page under a strict Content-Security-Policy and shares its origin: no coupling to a host, no HTML built
# from a string, no eval (scripts/check-web-sources.py; a line with a reason carries `host-lint-allow: <reason>`).
python3 scripts/check-web-sources.py --self-test
python3 scripts/check-web-sources.py

PROFILE=debug
FLAGS=()
if [[ "${1:-}" == "--release" ]]; then PROFILE=release; FLAGS=(--release); fi

# Capture the source before generated notices and assets are written to www/.
if [[ -n "${IBIS_WEB_BOM_DIR:-}" ]]; then
  mkdir -- "$IBIS_WEB_BOM_DIR"
  web_source_sha=$(git rev-parse HEAD)
  EXPECTED_SOURCE_SHA="$web_source_sha" python3 scripts/desktop-ci-info.py source "$IBIS_WEB_BOM_DIR" wasm
fi

# Generate the public demo from the same pinned corpus used in compatibility CI.
mkdir -p www/samples/mermaid
MERMAID_CORPUS_DEMO="$PWD/www/samples/mermaid" cargo test --locked -p ibis-formats --test mermaid_corpus

# A deep UI tree needs more than wasm-ld's default 1 MiB stack in debug builds
# (gpui-component's own web gallery does the same).
# The paths of this machine are not to go into the wasm (see bundle-macos.sh):
# rewritten, in a target directory of its own.
REMAP="--remap-path-prefix=$HOME/.cargo/registry/src=crates.io --remap-path-prefix=$HOME/.cargo/git/checkouts=git --remap-path-prefix=$HOME/.rustup=rustup --remap-path-prefix=$(pwd)=."
export IBIS_THIRD_PARTY_NOTICES="$PWD/target/third-party-web.md"
python3 scripts/gen-web-licenses.py web
RUSTFLAGS="$REMAP" CARGO_TARGET_DIR=target/dist cargo rustc --locked -p ibis-office-app --lib --crate-type cdylib --target wasm32-unknown-unknown ${FLAGS[@]+"${FLAGS[@]}"} -- \
  -C link-arg=-zstack-size=8388608

cp "$IBIS_THIRD_PARTY_NOTICES" www/license.md

WASM="target/dist/wasm32-unknown-unknown/$PROFILE/ibis_office_app.wasm"
wasm-bindgen "$WASM" --out-dir www/pkg --target web
RUSTFLAGS="$REMAP" CARGO_TARGET_DIR=target/dist cargo build --locked -p ibis-heic-web --lib \
  --target wasm32-unknown-unknown ${FLAGS[@]+"${FLAGS[@]}"}
wasm-bindgen "target/dist/wasm32-unknown-unknown/$PROFILE/ibis_heic_web.wasm" \
  --out-dir www/pkg/heic --target web

if [[ "$PROFILE" == release ]]; then
  if command -v wasm-opt >/dev/null; then
    wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
      www/pkg/ibis_office_app_bg.wasm -o www/pkg/ibis_office_app_bg.wasm
  else
    echo "wasm-opt is not installed; the release module skips wasm-opt -Oz" >&2
  fi
fi

# Split the optimized module for static hosts with per-file limits.
if [[ "$PROFILE" == release ]]; then
  python3 scripts/split-web-wasm.py www/pkg/ibis_office_app_bg.wasm
else
  python3 scripts/split-web-wasm.py www/pkg/ibis_office_app_bg.wasm --chunks-only
fi

# main.ts imports the typings wasm-bindgen just wrote.
if [[ ! -x node_modules/.bin/tsc ]]; then
  echo "TypeScript is not installed; run: npm ci" >&2
  exit 1
fi
# The version of this build, for the loader to check a loader-manifest.json against (the parts of the wasm are of one build: scripts/web-loader-setup.mjs).
printf "export const APP_VERSION = '%s';\n" "$(awk -F'\t' '$1 == "wasm" { sub(/\r$/, "", $2); print $2 }' version.txt)" > www/build-info.ts
node_modules/.bin/tsc -p www
# tsc keeps the copyright header of ibisoffice-host.ts in the .js but drops it from the .d.ts: the same header (the first three lines of the
# source) goes on the .d.ts, as it ships in the Web archive.
{ head -n 3 www/ibisoffice-host.ts; echo; grep -v '^// \(SPDX-License-Identifier\|Copyright (c)\|Additional terms\)' www/ibisoffice-host.d.ts; } > www/ibisoffice-host.d.ts.new
mv www/ibisoffice-host.d.ts.new www/ibisoffice-host.d.ts
# The loader and the host bridge's own tests (node; the same TypeScript).
node_modules/.bin/tsc -p www/tsconfig.test.json
node --test target/www-test/test/*.test.js www/test/copy_text.test.mjs

# The same checks on what was built (wasm-bindgen's JavaScript shows what the Rust calls: a `new Function` there would need `unsafe-eval`).
python3 scripts/check-web-sources.py --dist

# Icons are fetched on demand from /assets/icons/*.svg.
ASSETS_DIR=$(cargo metadata --locked --filter-platform wasm32-unknown-unknown --format-version 1 | python3 -c '
import json, os, sys
meta = json.load(sys.stdin)
pkg = next(p for p in meta["packages"] if p["name"] == "gpui-kit-assets")
print(os.path.join(os.path.dirname(pkg["manifest_path"]), "assets"))')
mkdir -p www/assets
rm -rf www/assets/icons
cp -R "$ASSETS_DIR/icons" www/assets/icons

# The art page borders' pictures, ECMA-376's PNG files taken out of the zip
# unmodified (fetched one by one when a document uses them), with the notice.
rm -rf www/assets/artborders
mkdir -p www/assets/artborders
unzip -q -o crates/ibis-formats/data/artborders/OfficeOpenXML-WordprocessingMLArtBorders.zip -d www/assets/artborders
cp crates/ibis-formats/data/artborders/ECMA-NOTICE.txt www/assets/artborders/

# Optional static SIL OFL font chunks (`tools/ibis-font-subsetter` + `loader-manifest.json`).
# Set IBIS_WEB_FONT_SUBSETS=1 to download missing fonts listed in docs/font-list.txt;
# otherwise cached fonts in data/fonts/ are built automatically when present.
if [[ "${IBIS_WEB_FONT_SUBSETS:-}" == "1" ]]; then
  node scripts/build-web-font-subsets.mjs --out www
elif [[ "${IBIS_WEB_FONT_SUBSETS:-}" != "0" ]]; then
  node scripts/build-web-font-subsets.mjs --out www --if-cached
fi

# Versioned service worker: hashed wasm/fonts plus the app shell, one cache per build.
node scripts/build-pwa-sw.mjs

python3 scripts/check-web-assets.py www

if [[ -n "${IBIS_WEB_BOM_DIR:-}" ]]; then
  test "$(git rev-parse HEAD)" = "$web_source_sha"
  python3 -m zipfile -c "$IBIS_WEB_BOM_DIR/wasm-ci.zip" www/pkg
  python3 scripts/build-bom.py wasm "$IBIS_WEB_BOM_DIR/wasm-ci.zip" "$IBIS_WEB_BOM_DIR"
  python3 scripts/desktop-ci-info.py artifacts "$IBIS_WEB_BOM_DIR"
fi

echo "Built www/ ($PROFILE). Serve it, e.g.: python3 -m http.server -d www 8080"

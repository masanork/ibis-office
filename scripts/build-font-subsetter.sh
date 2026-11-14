#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail
cd "$(dirname "$0")/.."
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly-2026-10-28}"
if ! rustup run "$RUSTUP_TOOLCHAIN" rustc --version >/dev/null 2>&1; then
  rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal --target wasm32-unknown-unknown
elif ! rustup target list --installed --toolchain "$RUSTUP_TOOLCHAIN" | grep -qx wasm32-unknown-unknown; then
  rustup target add wasm32-unknown-unknown --toolchain "$RUSTUP_TOOLCHAIN"
fi
BINDGEN_VERSION=$(python3 - <<'PY'
import re
print(re.search(r'name = "wasm-bindgen"\s+version = "([^"]+)"', open('Cargo.lock').read()).group(1))
PY
)
if [[ "$(wasm-bindgen --version 2>/dev/null || true)" != "wasm-bindgen $BINDGEN_VERSION" ]]; then
  echo "Install the matching CLI: cargo install wasm-bindgen-cli --version $BINDGEN_VERSION --locked" >&2
  exit 1
fi
CARGO_TARGET_DIR=target/font-subsetter CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
  CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
  cargo build --locked -p ibis-font-subsetter --lib --release --target wasm32-unknown-unknown
wasm-bindgen target/font-subsetter/wasm32-unknown-unknown/release/ibis_font_subsetter.wasm \
  --target bundler --out-dir crates/ibis-font-subsetter/pkg
python3 - <<'PY'
from pathlib import Path
import hashlib,json
p=Path('crates/ibis-font-subsetter/pkg')
data=(p/'ibis_font_subsetter_bg.wasm').read_bytes()
(p/'build.json').write_text(json.dumps({'sha256':hashlib.sha256(data).hexdigest(),'size':len(data)})+'\n')
(p/'ibis_font_subsetter_bg.d.ts').write_text('export * from \"./ibis_font_subsetter\";\nexport function __wbg_set_wasm(exports: WebAssembly.Exports): void;\n')
print('Built ibisOffice font subsetter:',len(data),'bytes')
PY

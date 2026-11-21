#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ $# != 1 ]]; then
    echo "Usage: law-viewer/build.sh /path/to/houritsu" >&2
    exit 2
fi
target_dir="${CARGO_TARGET_DIR:-$PWD/target}"
cargo build --locked --release --target wasm32-unknown-unknown -p ibis-law-viewer
mkdir -p law-viewer/dist
cp -R law-viewer/public/. law-viewer/dist/
wasm-bindgen --target web --out-dir law-viewer/dist --out-name engine \
  "$target_dir/wasm32-unknown-unknown/release/ibis_law_viewer.wasm"
python3 law-viewer/index-houritsu.py "$1" law-viewer/dist/houritsu-index.json

#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

# Allow Workers to propagate before comparing the published files.
verify_asset() {
  local origin=$1 path=$2 expected actual
  expected=$(sha256sum "www/$path")
  for attempt in {1..6}; do
    if actual=$(curl --fail --silent --show-error --compressed --max-time 25 \
      "$origin/$path" | sha256sum) && [[ ${actual%% *} == "${expected%% *}" ]]; then
      return 0
    fi
    if [[ "$attempt" != 6 ]]; then sleep 5; fi
  done
  echo "::error::Published asset differs at $origin/$path"
  return 1
}

manifest=pkg/ibis_office_app_bg.manifest.json
for origin in https://docs.sorane.dev https://ibis.sorane.dev; do
  verify_asset "$origin" manifest.webmanifest
  verify_asset "$origin" "$manifest"
done

chunks=$(jq -er '.chunks | if length > 0 then .[].file else error("No WASM chunks") end' "www/$manifest")
while IFS= read -r chunk; do
  [[ "$chunk" =~ ^ibis_office_app_bg\.[a-f0-9]{64}\.chunk$ ]]
  verify_asset https://docs.sorane.dev "pkg/$chunk"
done <<< "$chunks"

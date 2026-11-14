#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Fetch the official CPU runtime built for older glibc-based distributions.
set -euo pipefail
arch=${1:?architecture required}
destination=${2:?destination required}
case "$arch" in
  x86_64) asset_arch=x64; expected=8344d55f93d5bc5021ce342db50f62079daf39aaafb5d311a451846228be49b3 ;;
  aarch64|arm64) asset_arch=aarch64; expected=bb76395092d150b52c7092dc6b8f2fe4d80f0f3bf0416d2f269193e347e24702 ;;
  *) echo "Unsupported ONNX Runtime architecture: $arch" >&2; exit 1 ;;
esac
mkdir -p "$destination"
archive=$(mktemp "${TMPDIR:-/tmp}/ibis-onnxruntime.XXXXXXXX")
trap 'rm -f "$archive"' EXIT
if [[ -n ${IBIS_ONNX_RUNTIME_ARCHIVE:-} ]]; then
  cp "$IBIS_ONNX_RUNTIME_ARCHIVE" "$archive"
else
  curl --fail --location --retry 3 --silent --show-error \
    "https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-linux-${asset_arch}-1.22.0.tgz" -o "$archive"
fi
actual=$(sha256sum "$archive" | awk '{print $1}')
[[ "$actual" == "$expected" ]] || { echo 'ONNX Runtime archive checksum mismatch' >&2; exit 1; }
tar -xzf "$archive" --strip-components=1 -C "$destination"
test -f "$destination/lib/libonnxruntime.so.1.22.0"
printf '%s\n' "$(cd "$destination" && pwd)"

#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PATCH_DIR="$ROOT_DIR/patches/gpui-0.8.10"

apply_submodule() {
  local name="$1"
  local upstream_sha="$2"
  local patch_file="$PATCH_DIR/$name.patch"
  local dir="$ROOT_DIR/forks/$name"

  echo "==> $name (upstream $upstream_sha)"
  git -C "$dir" fetch "https://github.com/ibisjp/$name.git" "$upstream_sha"
  git -C "$dir" checkout -B sync/ibis-office-0.8.10 "$upstream_sha"
  if [[ -f "$patch_file" ]]; then
    git -C "$dir" apply --index "$patch_file"
    git -C "$dir" commit -m "Apply sorane fork changes on top of ibisOffice 0.8.10 ($upstream_sha)"
  fi
}

apply_submodule "gpui-base" "8b62758"
apply_submodule "gpui-pre" "63dd0ee"
apply_submodule "gpui-pre-macos" "48e1e34"
apply_submodule "gpui-pre-web" "6dbc49f"
apply_submodule "gpui-pre-wgpu" "2d25714"

echo "Done. Review and push each forks/gpui-* submodule to origin, then update submodule pointers in sorane."

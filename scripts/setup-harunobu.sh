#!/usr/bin/env bash
# Install the fixed Harunobu reference runtime locally; no document is uploaded.
set -euo pipefail
commit=88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f
case "$(uname -s)" in
  Darwin) root="$HOME/Library/Application Support/Sorane Docs/harunobu" ;;
  Linux) root="${XDG_DATA_HOME:-$HOME/.local/share}/sorane-docs/harunobu" ;;
  *) echo "Unsupported platform" >&2; exit 1 ;;
esac
command -v uv >/dev/null || { echo "uv is required" >&2; exit 1; }
if [[ -e "$root" ]]; then
  [[ -d "$root/.git" && "$(git -C "$root" rev-parse HEAD)" == "$commit" && -z "$(git -C "$root" status --porcelain --untracked-files=all | grep -vE '^\?\? \.ibis-harunobu-commit$' || true)" ]] || { echo "Existing runtime is different or modified: $root" >&2; exit 1; }
else
  mkdir -p "$(dirname "$root")"
  git clone --no-checkout https://github.com/digital-go-jp/machine_readability_rule.git "$root"
  git -C "$root" checkout --detach "$commit"
fi
uv sync --frozen --no-dev --directory "$root"
printf '%s\n' "$commit" > "$root/.ibis-harunobu-commit"
echo "Harunobu $commit installed at $root"

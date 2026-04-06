#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo "warning: scripts/refresh_clash_verge.sh is deprecated; use 'mihomo-cli refresh-clash-verge' instead." >&2

nix develop -c cargo run -p mihomo-cli -- refresh-clash-verge "$@"

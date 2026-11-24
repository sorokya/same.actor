#!/usr/bin/env bash
# Run a command inside the pinned Node toolchain container, with pnpm on PATH.
#
#   scripts/node.sh pnpm install
#   scripts/node.sh pnpm run build
#
# The host has no node, no npm and no pnpm: everything runs here. The pnpm version is not chosen
# by this script — it reads `packageManager` from package.json, which is also what the Dockerfile
# and CI read, so the three cannot drift apart.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${NODE_IMAGE:-docker.io/library/node:26-alpine}"

# Caches on the host: without them every run re-downloads pnpm and every package.
mkdir -p "$ROOT/.containers/npm" "$ROOT/.containers/pnpm-store" "$ROOT/.containers/pnpm-global"

exec podman run --rm \
  --userns=keep-id \
  -v "$ROOT":/w:Z \
  -v "$ROOT/.containers/npm":/home/node/.npm:Z \
  -v "$ROOT/.containers/pnpm-store":/home/node/.local/share/pnpm/store:Z \
  -v "$ROOT/.containers/pnpm-global":/home/node/.local/pnpm:Z \
  -w /w \
  -e npm_config_cache=/home/node/.npm \
  -e CI=1 \
  "$IMAGE" sh -c '
    set -e
    # Node 25+ dropped corepack, so pnpm is installed from the pin in package.json — but only
    # when the copy on the mounted volume is missing or the wrong version, which keeps every run
    # after the first instant and offline.
    want="$(node -p "require(\"./package.json\").packageManager.split(\"@\")[1]")"
    have="$(/home/node/.local/pnpm/bin/pnpm --version 2>/dev/null || true)"
    if [ "$have" != "$want" ]; then
      npm install -g --prefix /home/node/.local/pnpm "pnpm@$want" >/dev/null 2>&1
    fi
    export PATH=/home/node/.local/pnpm/bin:$PATH
    exec "$@"
  ' sh "$@"

#!/usr/bin/env bash
# Run a command inside the pinned Node toolchain container, with pnpm on PATH.
#
#   scripts/node.sh pnpm install
#   scripts/node.sh pnpm run build
#
# The host has no node, no npm and no pnpm: everything runs here. The pnpm version is not
# chosen by this script — corepack reads `packageManager` from package.json, so the toolchain
# is pinned in the repository and CI, Docker and this wrapper all agree on it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${NODE_IMAGE:-docker.io/library/node:22-alpine}"

# Caches on the host: without them every run re-downloads the pnpm binary and every package.
mkdir -p "$ROOT/.containers/npm" "$ROOT/.containers/pnpm-store" "$ROOT/.containers/corepack"

exec podman run --rm \
  --userns=keep-id \
  -v "$ROOT":/w:Z \
  -v "$ROOT/.containers/npm":/home/node/.npm:Z \
  -v "$ROOT/.containers/pnpm-store":/home/node/.local/share/pnpm/store:Z \
  -v "$ROOT/.containers/corepack":/home/node/.cache/node/corepack:Z \
  -w /w \
  -e npm_config_cache=/home/node/.npm \
  -e COREPACK_HOME=/home/node/.cache/node/corepack \
  -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
  -e CI=1 \
  "$IMAGE" sh -c '
    set -e
    # `corepack enable` cannot write to /usr/local/bin as a non-root user, and it does not need
    # to: a shim on PATH resolves through the same internals.
    mkdir -p /tmp/shim
    corepack enable --install-directory /tmp/shim pnpm
    export PATH=/tmp/shim:$PATH
    exec "$@"
  ' sh "$@"

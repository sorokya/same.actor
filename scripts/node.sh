#!/usr/bin/env bash
# Run a command inside the pinned Node toolchain container.
#
#   scripts/node.sh npm ci
#   scripts/node.sh npm run build
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${NODE_IMAGE:-docker.io/library/node:22-alpine}"

mkdir -p "$ROOT/.containers/npm"

exec podman run --rm \
  --userns=keep-id \
  -v "$ROOT":/w:Z \
  -v "$ROOT/.containers/npm":/home/node/.npm:Z \
  -w /w \
  -e npm_config_cache=/home/node/.npm \
  -e CI=1 \
  "$IMAGE" "$@"

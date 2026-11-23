#!/usr/bin/env bash
# Run a command inside the pinned Rust toolchain container.
#
# Nothing is installed on the host. The image tag is defined here and only here.
#   scripts/cargo.sh build --release
#   scripts/cargo.sh test
#   scripts/cargo.sh clippy -- -D warnings
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Built from docker/toolchain.Dockerfile: the upstream rust image has no clippy or rustfmt.
IMAGE="${RUST_IMAGE:-localhost/same-actor-toolchain:1}"

if ! podman image exists "$IMAGE"; then
  echo "building $IMAGE from docker/toolchain.Dockerfile" >&2
  podman build -t "$IMAGE" -f "$ROOT/docker/toolchain.Dockerfile" "$ROOT/docker" >&2
fi

mkdir -p "$ROOT/.containers/target" "$ROOT/.containers/registry"

exec podman run --rm \
  --userns=keep-id \
  -v "$ROOT":/w:Z \
  -v "$ROOT/.containers/target":/target:Z \
  -v "$ROOT/.containers/registry":/usr/local/cargo/registry:Z \
  -w /w/api-rs \
  -e CARGO_TARGET_DIR=/target \
  -e CARGO_HOME=/usr/local/cargo \
  -e CARGO_TERM_COLOR=always \
  "$IMAGE" "$@"

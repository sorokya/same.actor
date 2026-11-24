#!/usr/bin/env bash
# Run every gate CI runs, and report each one independently.
#
# Chaining these with `&&` is a trap: a failure in the first command silently skips the
# rest, so "fmt failed, then I fixed fmt" leaves clippy never having run against the final
# code. Everything here is captured and reported, pass or fail.
#
#   scripts/check.sh          # all gates in containers
#   scripts/check.sh backend  # just fmt + clippy + test
#   scripts/check.sh frontend # just lint + typecheck + build
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

target="${1:-all}"
failures=0
passes=0

report() {
  local name="$1" code="$2" log="$3"
  if [ "$code" -eq 0 ]; then
    printf 'ok    %s\n' "$name"
    passes=$((passes + 1))
  else
    printf 'FAIL  %s (exit %s)\n' "$name" "$code"
    # Show the tail of what the gate said, so the failure is actionable in one read.
    sed 's/^/      /' "$log" | tail -20
    failures=$((failures + 1))
  fi
}

gate() {
  local name="$1" runner="$2"
  shift 2
  local log
  log="$(mktemp)"
  "$runner" "$@" >"$log" 2>&1
  report "$name" "$?" "$log"
  rm -f "$log"
}

if [ "$target" = "all" ] || [ "$target" = "backend" ]; then
  gate "rustfmt --check" ./scripts/cargo.sh sh -c 'cargo fmt --check'
  gate "clippy -D warnings" ./scripts/cargo.sh sh -c 'cargo clippy --all-targets -- -D warnings'
  gate "cargo test" ./scripts/cargo.sh sh -c 'cargo test'
fi

if [ "$target" = "all" ] || [ "$target" = "frontend" ]; then
  gate "biome check" ./scripts/node.sh sh -c 'npx biome check src'
  gate "tsc --noEmit" ./scripts/node.sh sh -c 'npx tsc --noEmit'
  gate "vite build" ./scripts/node.sh sh -c 'npx vite build'
  # Tailwind fails silently when it cannot see a source, so the built stylesheet is checked
  # for utilities that only appear in templates.
  gate "css covers templates" ./scripts/check-css.sh
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "all $passes gates passed"
  exit 0
fi

echo "$failures gate(s) failed, $passes passed"
exit 1

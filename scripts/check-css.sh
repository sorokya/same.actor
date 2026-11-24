#!/usr/bin/env bash
# Verify the built stylesheet actually contains the utilities the templates use.
#
# Tailwind scans sources listed with `@source`, and it does that silently: a directory it
# cannot see produces a stylesheet with no error and no warning, and the pages come out
# half-styled. That shipped once — the Dockerfile built the frontend without copying
# `api-rs/templates`, so every class that only appears in a template was missing from the
# image while the local build was fine.
#
#   scripts/check-css.sh
#
# Reads whatever `vite build` last wrote to `dist/`.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

css="$(ls dist/assets/*.css 2>/dev/null | head -1)"
if [ -z "$css" ]; then
  echo "no stylesheet in dist/ — run a build first" >&2
  exit 1
fi

# One class from each template that the island does not use. If the island ever starts using
# one of them the check gets weaker, not wrong — pick a different template-only class.
classes=(
  'max-w-3xl'      # header and page containers
  'sm:flex-row'    # the header's desktop layout
  'grid-cols-3'    # the poster shelf
  'line-clamp-2'   # titles in the results
  'h-11'           # artwork in the results on a phone
)

missing=0
for class in "${classes[@]}"; do
  # Tailwind escapes the colon in a variant class (`sm\:flex-row`), so match the escaped form
  # as well as the plain one.
  escaped="${class//:/\\:}"
  if ! grep -qF -- "$escaped" "$css" && ! grep -qF -- "$class" "$css"; then
    echo "missing from $css: $class" >&2
    missing=$((missing + 1))
  fi
done

if [ "$missing" -ne 0 ]; then
  echo "the stylesheet does not cover the templates — is @source pointing at a directory that exists?" >&2
  exit 1
fi

echo "stylesheet covers the templates ($css)"

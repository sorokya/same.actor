# syntax=docker/dockerfile:1
#
# One image, one process: every page, the API and the share cards on one origin at one version.
#
# Three stages, each arranged so the thing most likely to change is copied last:
#   web     — Tailwind and Vite produce `dist/`: a stylesheet and two small scripts
#   builder — cargo-chef compiles the dependency graph from a recipe, then the crate itself
#   runtime — alpine, a musl binary, and nothing else
#
# Base images carry the version but no digest: a rebuild picks up patches, which is the point.
# Pinning a digest freezes that on purpose, and nothing here wants to.

# --- frontend -----------------------------------------------------------------------------
FROM docker.io/library/node:26-alpine AS web

WORKDIR /web

# pnpm, not npm. The version comes from `packageManager` in package.json, so this stage, CI and
# scripts/node.sh all resolve the same one.
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable

COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

COPY tsconfig.json vite.config.ts postcss.config.mjs ./
COPY src ./src
COPY public ./public
# Tailwind scans these for class names (`@source` in src/index.css). Without them the build
# succeeds and produces a stylesheet with none of the template's utilities — every page
# half-styled, no error anywhere.
COPY api-rs/templates ./api-rs/templates

RUN pnpm run build

# --- backend ------------------------------------------------------------------------------
# musl throughout: `rust:alpine` targets x86_64-unknown-linux-musl, so the binary does not care
# which libc the runtime image carries. `make` is for crates whose build scripts shell out —
# rusqlite's bundled SQLite goes through `cc`, which the image already has.
FROM docker.io/library/rust:alpine3.23 AS chef

RUN apk add --no-cache musl-dev make \
    && cargo install --locked cargo-chef

WORKDIR /app

# The recipe is the dependency graph flattened into one file. Cooking it before the source
# arrives is the whole point: an edit to `src/` then costs one crate instead of two hundred.
FROM chef AS planner
COPY api-rs/ ./
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# `api-rs/` becomes the crate root here, so the binary lands in /app/target/release — and the
# `include_bytes!` assets and askama templates come with it.
COPY api-rs/ ./
RUN cargo build --release

# --- runtime ------------------------------------------------------------------------------
FROM docker.io/library/alpine:3.23

# ca-certificates is the only thing the binary needs at runtime: TMDB and the geo database are
# both HTTPS. The healthcheck uses busybox's wget rather than pulling in curl for one request.
#
# busybox adduser: -D no password, -H no home directory. A high uid with nologin is what a
# runtime user actually wants, and alpine has no `--system` distinction to get wrong.
RUN apk add --no-cache ca-certificates \
    && addgroup -g 10001 sameactor \
    && adduser -D -H -u 10001 -G sameactor -h /nonexistent -s /sbin/nologin sameactor

COPY --from=builder /app/target/release/same-actor /usr/local/bin/same-actor
COPY --from=web /web/dist /srv/dist

ENV STATIC_DIR=/srv/dist \
    BIND=0.0.0.0:3001 \
    DB_PATH=/data/same-actor.db \
    GEO_DB_PATH=/data/dbip-country-lite.mmdb \
    RUST_LOG=info

# The request log lives on a volume. Without this, the database sits in the container's
# writable layer and disappears on every deploy — the failure would look like "analytics
# reset", not like a missing mount.
RUN mkdir -p /data && chown 10001:10001 /data
VOLUME ["/data"]

USER 10001
EXPOSE 3001

# Build with `--format docker` if you want the HEALTHCHECK below preserved: OCI images have
# no healthcheck field, so podman/buildah silently drops it when building the default OCI
# format. Docker and docker/build-push-action use the Docker format and keep it.
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:3001/healthz || exit 1

CMD ["/usr/local/bin/same-actor"]

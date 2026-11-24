# One image, one process: every page, the API and the share cards on one origin at one version.
#
# The frontend stage builds only assets now — a stylesheet and the picker island. Pages are
# rendered by the binary, so the templates are part of the Rust build, not of `dist`.

# --- frontend -----------------------------------------------------------------------------
FROM docker.io/library/node:22-alpine AS spa

WORKDIR /spa
COPY package.json package-lock.json ./
RUN npm ci
COPY tsconfig.json vite.config.ts postcss.config.mjs ./
COPY src ./src
COPY public ./public
RUN npm run build

# --- backend ------------------------------------------------------------------------------
FROM docker.io/library/rust:1-slim-trixie AS api

WORKDIR /build
COPY api-rs/Cargo.toml api-rs/Cargo.lock ./
# Compile the dependency graph on its own so a source edit does not rebuild all of it.
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src
COPY api-rs/src ./src
# Embedded with include_bytes! for the Open Graph renderer.
COPY api-rs/assets ./assets
# Compiled in by askama: a template error fails this build, which is the point.
COPY api-rs/templates ./templates
COPY api-rs/tests ./tests
RUN touch src/main.rs && cargo build --release

# --- runtime ------------------------------------------------------------------------------
FROM docker.io/library/debian:trixie-slim

# `useradd` without `--system`: a system account must be below SYS_UID_MAX (999), and
# combining --system with uid 10001 makes useradd warn on every build while writing a
# system-style shadow entry for a UID that isn't one. An explicit high uid, no home and
# nologin is what a runtime user actually wants.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --no-create-home --shell /usr/sbin/nologin sameactor

COPY --from=api /build/target/release/same-actor /usr/local/bin/same-actor
COPY --from=spa /spa/dist /srv/dist

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
    CMD curl -fsS http://127.0.0.1:3001/healthz || exit 1

CMD ["/usr/local/bin/same-actor"]

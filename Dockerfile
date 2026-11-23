# One image, one process: the SPA and the API on the same origin at the same version.
#
# This replaces the previous split where the frontend was rsynced to a static host and the
# API was published to a registry independently — two artefacts that could disagree about
# the JSON contract.

# --- frontend -----------------------------------------------------------------------------
FROM docker.io/library/node:22-alpine AS spa

WORKDIR /spa
COPY package.json package-lock.json ./
RUN npm ci
COPY tsconfig.json vite.config.ts postcss.config.mjs index.html ./
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
COPY api-rs/tests ./tests
RUN touch src/main.rs && cargo build --release

# --- runtime ------------------------------------------------------------------------------
FROM docker.io/library/debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home sameactor

COPY --from=api /build/target/release/same-actor /usr/local/bin/same-actor
COPY --from=spa /spa/dist /srv/dist

ENV STATIC_DIR=/srv/dist \
    BIND=0.0.0.0:3001 \
    RUST_LOG=info

USER 10001
EXPOSE 3001

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3001/healthz || exit 1

CMD ["/usr/local/bin/same-actor"]

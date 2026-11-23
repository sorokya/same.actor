# Same Actor

Is that the same actor from another movie you remember? Pick two to five films, shows, or
people and see exactly what they share.

One Rust binary serves the API and the single-page app on one origin at one version.

## How it works

`POST /api/findout` fetches each selection from TMDB — metadata and credits in one call via
`append_to_response` — and intersects the casts. Someone credited in at least two of the
selected titles is a match; a title crediting at least two of the selected people is a match.
No database, no accounts, no state beyond a short-lived cache.

`GET /api/search` proxies TMDB's search, filtered to titles or people depending on `mode`.

## Running it

### Docker (the real thing)

```sh
docker build -t same-actor .
docker run --rm -p 3001:3001 -e TMDB_TOKEN=... same-actor
```

### Local, with containers

Nothing is installed on the host: `podman` is the only requirement.

```sh
# frontend: typecheck, lint, build
./scripts/node.sh npm ci
./scripts/node.sh npm run build

# backend: build, lint, test
./scripts/cargo.sh cargo build
./scripts/cargo.sh cargo test

# run it against a stub of the TMDB API, so no key is needed
python3 scripts/demo-stub.py --port 4100 &
TMDB_TOKEN=dev TMDB_BASE=http://127.0.0.1:4100/3 STATIC_DIR=dist \
  ./.containers/target/debug/same-actor
```

Open <http://127.0.0.1:3001>. The stub serves captured catalogue data from
`scripts/fixtures/tmdb.json`, so search, both comparison modes and the whole UI work without
network access or a key.

The first `cargo` or `npm` run builds the toolchain image, which is defined in
`docker/toolchain.Dockerfile` and referenced only by `scripts/`.

### Local, with a real key

Get a TMDB credential. A **v4 read access token** is sent as a bearer token (the default); a
legacy **v3 API key** must be declared explicitly:

```sh
TMDB_TOKEN=... TMDB_AUTH=bearer   # v4 read access token (default)
TMDB_TOKEN=... TMDB_AUTH=query    # v3 api key, sent as ?api_key=
```

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TMDB_TOKEN` | — | Required. `TMDB_KEY` is accepted as an alias. |
| `TMDB_AUTH` | `bearer` | `bearer` (v4) or `query` (v3). |
| `BIND` | `0.0.0.0:3001` | Listen address. |
| `TMDB_BASE` | `https://api.themoviedb.org/3` | Override for the stub or a proxy. |
| `IMAGE_BASE` | `https://image.tmdb.org/t/p/w342` | Poster/portrait prefix. |
| `STATIC_DIR` | `dist` | Directory holding the built SPA. API-only if absent. |
| `CACHE_TTL_SECS` | `600` | Upstream response cache lifetime. |
| `CACHE_CAPACITY` | `10000` | Cache entry ceiling. |
| `RATE_LIMIT` | `10` | Requests per window per client IP. |
| `RATE_WINDOW_SECS` | `1` | Window length. |
| `TRUST_FORWARDED_FOR` | `false` | Honour `X-Forwarded-For`. **Only enable behind a proxy you control** — otherwise any client picks its own rate-limit bucket. |
| `RUST_LOG` | `info` | Log filter. |

## The API

Both endpoints return JSON on success and `{"error":{"code","message"}}` on failure, always
with a real status code. Codes: `validation_error` (422), `not_found` (404), `rate_limited`
(429, with `Retry-After`), `upstream_unavailable` (502), `internal_error` (500).

```
GET /api/search?mode=title|person&term=…
→ { "results": [ { id, kind, name, year|null, overview, image|null } ] }

POST /api/findout  { "mode": "title"|"person",
                     "selection": [ { "id": 603, "kind": "movie"|"tv"|"person" }, … ] }
→ { "mode": "title",
    "selections": [ { id, kind, name, year|null, image|null } ],
    "matches":    [ { id, name, image|null, credits: [ { selection, character: [ … ] } ] } ] }
```

`mode` is what the user is comparing; `kind` is what one selection *is*. Two selections
minimum, five maximum. `findout` returns the resolved selections as well as the matches, so a
shared URL renders its own column headers.

The comparison lives in the URL — `?mode=title&pick=movie:603,tv:1396` — so it survives a
refresh and can be sent to someone.

## Layout

```
api-rs/
  src/
    domain.rs      wire types; Mode and Kind are closed enums, ids are NonZeroU32
    intersect.rs   the comparison itself, pure and unit-tested
    tmdb/          upstream client: URL construction, auth, cache
    routes/        search, findout, health
    error.rs       one error type, one place that maps it to a response
    ratelimit.rs   per-IP fixed window
  tests/api.rs     HTTP-level tests against a stub TMDB
src/
  app.tsx          the screen
  lib/api.ts       typed client; failures are values, not nulls
  lib/types.ts     mirrors api-rs/src/domain.rs
  components/      search panel, chips, result matrix, poster, ui
docs/
  REVIEW.md        what was wrong with the previous implementation
  REWRITE-PLAN.md  how and why it was rebuilt
scripts/
  cargo.sh         run a command in the pinned Rust container
  node.sh          run a command in the pinned Node container
  demo-stub.py     TMDB stand-in for local development
```

## Tests

```sh
./scripts/cargo.sh cargo test     # unit + HTTP-level tests against a stub TMDB
./scripts/cargo.sh cargo clippy --all-targets -- -D warnings
./scripts/node.sh npm run lint    # biome
./scripts/node.sh npm run typecheck
```

The HTTP tests cover the failure modes that broke the previous version: a selection kind that
is not in the enum never reaches a URL, a single selection is a 422 rather than a crash, and
an upstream 5xx is a 502 for that one request while the process keeps serving.

## Credits

This product uses the TMDB API but is not endorsed or certified by TMDB.

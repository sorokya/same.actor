# Same Actor

Is that the same actor from another movie you remember? Pick two to five films, shows, or
people and see exactly what they share.

One Rust binary renders every page, serves the API and draws the share cards, on one origin at
one version. Pages are HTML the server produced: a shared link shows its answer before a byte of
JavaScript runs, and the only script the site loads is the picker — a ~24 kB island on the
comparison pages. The browse pages and the dashboard load no script at all.

## How it works

`POST /api/findout` fetches each selection from TMDB — metadata and credits in one call via
`append_to_response` — and intersects the casts. Someone credited in at least two of the
selected titles is a match; a title crediting at least two of the selected people is a match.
No database, no accounts, no state beyond a short-lived cache.

`GET /api/search` proxies TMDB's search, filtered to titles or people depending on `mode`.

Comparisons are addresses: `/title/movie-603-the-matrix,movie-604-the-matrix-reloaded`. Open
one and the server renders `<title>`, `og:` and `twitter:` tags for it and points `og:image`
at a 1200×630 PNG it rasterises on demand, so a link to a comparison unfurls into a card
instead of a bare URL.

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
| `STATIC_DIR` | `dist` | Directory holding the built CSS and the island. Pages render unstyled if absent. |
| `PUBLIC_ORIGIN` | `http://localhost:3001` | Absolute origin for `og:url` and `og:image`. **Set this in production** — Open Graph requires absolute URLs and it cannot be inferred behind a proxy. |
| `DB_PATH` | `data/same-actor.db` | SQLite file. Relative by default so a local run needs no configuration; the container sets `/data/same-actor.db` on a volume. |
| `LOG_RETENTION_DAYS` | `90` | Days of raw request rows kept before rollup. The aggregates keep no IP and no session id, so this is the privacy cutover as well as the size one. |
| `LOG_ASSETS` | `false` | Log static assets. Off because one page load would otherwise be five rows. |
| `GEO_ENABLED` | `true` | Country lookup via DB-IP Lite. Off means no download and a NULL country. |
| `GEO_DB_PATH` | `data/dbip-country-lite.mmdb` | Where the database is cached. Put it on the same volume as the SQLite file. |
| `TMDB_REGION` | `US` | Region for release dates on "in cinemas" and "coming soon". |
| `ADMIN_TOKEN` | unset | Admin API token. **Unset means the whole `/api/admin` subtree answers 404** — a forgotten env var must not mean an open dashboard. |
| `AUTOBAN_REQUESTS` | `0` (off) | Ban an address that trips the rate limit this many times in the window. |
| `AUTOBAN_ERRORS` | `0` (off) | Ban an address that triggers this many other client errors in the window. |
| `AUTOBAN_WINDOW_SECS` | `60` | The window those counters use. |
| `AUTOBAN_BAN_SECS` | `3600` | How long an automatic ban lasts. Automatic bans always expire. |
| `GEO_URL` | DB-IP Lite, current month | `{month}` is substituted. Override only to use a mirror. |
| `TMDB_LOGO_URL` | TMDB's approved `blue_short` SVG | Override only if TMDB rotates the asset. The card renders without the logo if the fetch fails. |
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

The comparison lives in the URL, so it survives a refresh and can be sent to someone:

```
/title/movie-603-the-matrix,tv-1396-breaking-bad
/person/6384-keanu-reeves,530-carrie-anne-moss
```

The id is authoritative and the slug is decoration, so `/title/movie-603,movie-604` is the
same page — the client rewrites the address to the slug form once it knows the names. The
kind is only written in `title` mode, where a selection can be either a film or a show.

## Rendering

| Page | Rendered by | Script |
| --- | --- | --- |
| `/`, `/person`, `/title/{picks}`, `/person/{picks}` | Rust | the island |
| `/discover/{section}` | Rust | none |
| `/admin` and its pages | Rust | none |
| unknown paths | Rust (404 page) | none |

The comparison pages ask the same route for `X-Fragment: results` after a selection changes, so
the results markup exists in one template and the island never renders a table. Without
JavaScript everything except picking still works, including a shared comparison link.

Templates are compiled into the binary, so a typo is a build error and not a 500. Classes live in
the templates; `src/index.css` names `../api-rs/templates` with `@source` so Tailwind emits the
utilities that only appear there. Forgetting that is silent in development and visible in
production — the fix is to rebuild the stylesheet after adding a template.

## Layout

```
api-rs/
  src/
    domain.rs      wire types; Mode and Kind are closed enums, ids are NonZeroU32
    intersect.rs   the comparison itself, pure and unit-tested
    compare.rs     fetch + intersect, shared by the API and the share cards
    paths.rs       parsing for /title/… and /person/…, mirroring src/lib/url.ts
    og.rs          Open Graph card: SVG in, PNG out (resvg, vendored fonts)
    tmdb/          upstream client: URL construction, auth, cache
    routes/        search, findout, health, og (card image + meta tags)
    error.rs       one error type, one place that maps it to a response
    ratelimit.rs   per-IP fixed window
  tests/api.rs     HTTP-level tests against a stub TMDB
src/
  app.tsx          the screen
  lib/api.ts       typed client; failures are values, not nulls
  lib/types.ts     mirrors api-rs/src/domain.rs
  lib/url.ts       URL parsing and formatting (mirrors api-rs/src/paths.rs)
  lib/palette.ts   one hue per selection, followed from chip to table column
  components/      search panel, chips, result matrix, poster, icons, ui
docs/
  REVIEW.md        what was wrong with the previous implementation
  REWRITE-PLAN.md  how and why it was rebuilt
scripts/
  cargo.sh         run a command in the pinned Rust container
  node.sh          run a command in the pinned Node container
  demo-stub.py     TMDB stand-in for local development
api-rs/assets/fonts/
  DejaVu*.ttf      embedded in the binary so share cards render identically everywhere
```

## Frontend notes

Rust renders the pages (`api-rs/templates`, askama, compiled in). The browser runs one island:
the search box, the chips and Share. It is Preact — not React — with no `preact/compat`, so
nothing underneath it imports React. Three behaviours differ from React and cost an afternoon
if you meet them cold:

- **`onChange` is the DOM `change` event**, not React's input alias. Typing does nothing
  until blur. Use `onInput`.
- **Numeric style values get no unit.** `style={{ width: 22 }}` assigns `"22"` to
  `CSSStyleDeclaration`, which is not a valid width, and the declaration is silently dropped.
  Write `${22}px`.
- **No `JSX.TargetedKeyboardEvent` / `JSX.ButtonHTMLAttributes`.** Event and prop types come
  straight from `preact` (`TargetedKeyboardEvent`, `ComponentProps<'button'>`), and attributes
  are lowercase (`spellcheck`).

## Share cards

`GET /og/{mode}/{picks}.png` renders 1200×630 with resvg from an SVG built in `og.rs`: the
selections with their **posters and portraits** (fetched at `w185`, clipped to a rectangle or
a circle), the shared-credit count, and TMDB's logo. Text is escaped, long titles are
truncated to a width the layout can guarantee, and fonts are `include_bytes!`'d so the output
does not depend on what the host has installed.

Artwork never costs you the card: a failed image fetch leaves an empty well, and a failed logo
fetch leaves a log line. `./scripts/cargo.sh cargo run --example og-preview -- card.png`
writes a sample without starting the server.

The unit tests assert on **pixels**, not file sizes — that a poster's colour reaches the
poster well, and that a round portrait is clipped at the corners. A size comparison passes
whether or not the artwork rendered, which is how the first version of that test lied.

## Storage

One SQLite file, one writer thread, a small pool of read connections under WAL. SQLite
permits exactly one writer, so instead of configuring a pool and hoping `busy_timeout` covers
the contention, a single thread owns the write connection and applies batched transactions
(up to 256 rows or 250 ms, whichever comes first). Readers run concurrently — that is what WAL
is for.

Writes are *offered*, not performed: `Store::record` drops a record when the queue is full and
counts it in `stats().dropped`. Analytics must never be load-bearing, so a slow disk cannot
slow a page. The dropped count is the honest signal, and it is surfaced rather than hidden.

`Store::flush()` is a barrier: everything offered before it is committed when it returns, which
is what makes the tests deterministic without a single sleep. On `SIGTERM` the writer commits
its last batch, checkpoints the WAL (so a backup sees one consistent file), and stops.

`PRAGMA user_version` steps the migrations, each in its own transaction with the version bump
inside it. Opening a database written by a *newer* build is refused outright: running old code
against a newer schema is not something to discover later.

The schema (nine tables) lands in one migration rather than growing one table per phase — an
empty table costs nothing, and one reviewed schema in one place beats eight migrations that
each need their own reasoning.

### The database needs a volume the container can write

The image runs as uid 10001 and `DB_PATH` is `/data/same-actor.db`. Under rootless podman a
bind-mounted host directory belongs to a different uid, so the default mount is refused at
startup:

```
$ podman run -v ~/same-data:/data same-actor
Error: Store("/data exists but this process cannot write to it (Permission denied ...).
        If it is a bind-mounted host directory, give it to the container with
        `-v <dir>:/data:U`, or use a named volume instead.")
```

Either of these works, and both were verified against the image:

```sh
podman run -d -p 3001:3001 -v same-actor-data:/data -e TMDB_TOKEN=… same-actor   # named volume
podman run -d -p 3001:3001 -v ~/same-data:/data:U -e TMDB_TOKEN=… same-actor     # bind mount
```

`podman stop` is a `SIGTERM`, which commits the last batch and checkpoints the WAL, so a backup
sees one consistent `.db` file and no `-wal` sidecar.

## Request logging and sessions

Every request except assets and `/healthz` becomes one row: timestamp, duration, client
address, session, user agent, method, path, status, and — on failure — an error code and the
message that explains it. Server errors record the *internal* detail (which upstream call
failed) while the response body keeps the generic message.

`sa_sid` is a random 16-byte cookie, `HttpOnly`, `SameSite=Lax`, 30 days. It is not
authentication: it groups requests into a visit, so the log can answer "how many people"
rather than "how many requests".

**`/api/search` and `/api/findout` require it** and answer `428` with a `Set-Cookie` when it is
missing, so a caller can simply retry. Everything else stays anonymous, and each exemption is
deliberate:

| Route | Why |
|---|---|
| `/og/…`, `/title/…`, `/person/…` | Unfurlers fetch these once, with no cookie jar, and will not honour `Set-Cookie`. Requiring one would break every Discord and Slack embed. |
| `/api/trending` | A cached aggregate. There is no visit to attribute. |
| `/api/admin/*` | Carries its own credential. |
| `/healthz` | A probe that needs a cookie is a probe that fails the day cookie logic breaks. |

Logging cannot fail a request: the record is offered to a queue, and if the queue is full it is
dropped and counted in `stats().dropped`.

### Bans

A ban is an address or a CIDR range. **There is no session ban**: a cookie is client-controlled,
so a ban a visitor escapes by pressing ⌘⇧N is worse than none — it looks like enforcement.

The ban list lives in memory and is rebuilt from the database every 30 seconds and immediately
after any change, so enforcement costs no query and a new ban is live on the next request rather
than at the next refresh. Matching prefers the narrowest rule: an exact address beats any range,
and the longest prefix wins among ranges.

A banned client gets `403` with the operator's reason and, for a temporary ban, a `Retry-After`.
`/healthz` is exempt — a ban must never be able to make the container look unhealthy.

Automatic bans are **off by default** and always expire. When enabled they are recorded with
`created_by = 'auto:<rule>'` so a machine's decisions are distinguishable from a human's.

## Public analytics

Two things a visitor sees, both fed by the same store:

- **`GET /api/trending`** — what people searched for in the last day, cached for a minute. Terms
  searched fewer than three times are **not published**: below that the strip would be a log of
  individual people's one-off queries, which does not belong on a public page. The thresholds are
  `const` assertions, so lowering them fails the build rather than a test run.
- **View counts** — "viewed N times" on a comparison, from `/api/findout`'s `views` field.

A "view" is a visitor looking at a comparison once. Deduplication is in memory for 30 minutes per
(session, comparison), because the page render and the `findout` that follows it are one visit,
and a refresh is not an audience. The count is read *before* the write is offered and this view
added manually — the write is batched and asynchronous, so reading afterwards would always miss
it and the first visitor to any comparison would be told it had never been viewed.

`GET /api/trending` is exempt from the session requirement: it is a cached aggregate, it renders
on first paint, and there is no visit to attribute.

## Discovery pages

Three sections — `/discover/top`, `/discover/new`, `/discover/upcoming` — each with a
cast-synergy table. Every section has a second axis, exposed as a control rather than as more
tabs:

| Section | Control | Values |
| --- | --- | --- |
| `top` | Ranked by | `all-time` (default), `trending` |
| `new` | Where | `cinema` (default), `home` — films only; shows have `on-the-air` |
| `upcoming` | — | no choice to offer, so no control |

Each of those exists for both kinds, so nine snapshots in all: `top:movie:all-time`,
`top:movie:trending`, `top:tv:all-time`, `top:tv:trending`, `new:movie:cinema`,
`new:movie:home`, `new:tv:on-the-air`, `upcoming:movie`, `upcoming:tv`. Refreshed into SQLite
every six hours by a background job, so no page ever waits on TMDB and an outage costs
freshness, not availability. A refresh also prunes snapshots whose id is no longer in the list —
otherwise every change to the section model leaves a row behind for ever.

```
GET /api/discover                         → sections, with the newest refresh time in each
GET /api/discover/top?kind=movie&filter=trending
→ { section, heading, blurb, kind, filter, kinds[], filters[], filter_group, refreshed_ts,
    items[], synergy[] }
```

`kind` is `movie` (default) or `tv`; an unknown one is a **422**, and an unknown `filter` is a
422 rather than a quiet fallback — serving the default list for a typo would look like a working
page with the wrong contents. A list that has never been built is a **404**, not an empty page:
an empty list reads as "nothing is popular", which is a lie.

TMDB has no all-time list, so `all-time` is `vote_count.desc` with a floor (5000 films, 2000
shows): cumulative popularity, so a film nobody watched this week still ranks. `home` is release
types 4 and 5 — digital and physical — cut off at today, which is why the refresh substitutes
`{today}` into the query. `TMDB_REGION` (default `US`) is required by the two lists keyed on
release dates; only those send it.

The synergy table is the interesting part. Twenty titles makes the pairwise intersection that
powers the main comparison useless — 190 pairs, most of them empty. What is meaningful at that
scale is an actor appearing in **two or more** of the list, which is what the job computes once
per refresh and stores alongside the list.

The page is a poster shelf rather than a numbered text list, and the synergy rows carry the
actor's portrait and the titles they appear in, each one a link — "the same face again" is a
recognition, not something a column of names conveys. A row stores the title's id and kind as
well as its name, so the link does not have to guess the kind from the list.

## Navigation

One header, `src/components/site-header.tsx`, used by the comparison tool, the sections, and the
dashboard. Each surface supplies its own tabs; the chrome is shared, which is the only way three
independently written headers stop drifting apart.

Five tabs — `Titles | People`, then `Top | New | Upcoming`. The second axis of a section is a
control on the page, not another tab: seven lists across did not fit, and `Films` sitting beside
`Titles` said nothing about what the page was for. The controls are links too, so they work
before any JavaScript has run; with JavaScript they rewrite the query and refetch.

Every tab is a link, including the two that used to be a radio group — mode already cleared the
selection when it changed, so a link behaves identically. `People` points at `/person`, *not*
`/?mode=person`: the mode is read from the path, so the query parameter was silently ignored and
the tab landed on the title comparison.

Below `sm` the navigation takes its own row under the wordmark, where five tabs fit on one line;
a scroll strip there clipped the last label, which reads as broken rather than scrollable.

## Admin API

Everything lives under `/api/admin` and requires `ADMIN_TOKEN`. Two ways to present it:

```sh
# A script: straight bearer token.
curl -H "Authorization: Bearer $ADMIN_TOKEN" localhost:3001/api/admin/overview

# The dashboard: exchange it once for an HttpOnly cookie, so the token never sits in the page.
curl -c jar -X POST localhost:3001/api/admin/session -d '{"token":"…"}' -H 'content-type: application/json'
```

The cookie holds a random session id, not the token, and ids live in memory — restarting the
process signs everyone out. `/session` is outside the auth guard, because presenting the token
is what it does; nesting it behind the guard would make logging in impossible.

| Endpoint | Purpose |
|---|---|
| `GET /overview?window=24h\|7d\|30d` | Counts, p95, top paths, top countries, database size. |
| `GET /logs` | Filters: `ip`, `session`, `country`, `status` (`404` or `4xx`), `kind`, `path` prefix, `q`, cursor `before_id`. |
| `GET /logs/{id}` | One row in full. |
| `GET /stats/timeseries?metric=&window=` | `requests`, `errors`, `visitors`, `searches` per day, with no holes. |
| `GET /stats/geo?days=` | Countries by requests and errors. |
| `GET /stats/searches?window=` | Top terms **and zero-result terms** — what people wanted and the catalogue lacked. |
| `GET /stats/comparisons` | Most viewed comparison sets. |
| `GET /suspicious?window=` | Scored clients, each with the reasons that produced the score. |
| `GET`/`POST /bans`, `DELETE /bans/{id}` | Manage bans. A target that is neither an address nor a range is refused, because a ban that matches nothing would still appear in the list. |
| `GET /health` | Rows, commits, dropped records, database size, geo age. |

A ban created through the API is enforced on the next request, not at the next refresh.

`/api/admin` is exempt from ban enforcement: an operator who banned their own address would
otherwise lock themselves out of the only screen that can undo it. The credential is the gate
there, and it is the stronger one.

### Country lookup

DB-IP Lite's country database is downloaded into the data directory at startup and refreshed
daily. It is not baked into the image — it goes stale monthly, and a build-time network
dependency is a build that fails for reasons unrelated to the code.

Two things worth knowing, both verified against the live service by an ignored test
(`cargo test --lib geo::live -- --ignored`):

- **The download `403`s without a `User-Agent`.** The client sets one explicitly. Without it
  geo silently stays empty, which looks identical to an outage.
- The file is served gzip-compressed with no `Content-Encoding`, so it is decoded here; a
  mirror serving a plain `.mmdb` also works.

Geolocation applies to **country only**. Failures degrade to `country = NULL` and a log line —
never to a failed start.

## TMDB attribution

Required by [TMDB's terms](https://www.themoviedb.org/about/logos-attribution): the logo, the
sentence *"This product uses the TMDB API but is not endorsed or certified by TMDB."*, both
inside a Credits-type section, with the logo less prominent than this app's own mark. The
footer carries all three; the share card carries the logo.

The logo is linked from TMDB's CDN rather than vendored, so it is always their unmodified
asset. Note the asset path is `/assets/v4/logos/v2/…` — the `/assets/2/v4/…` path in the
previous implementation 403s, so that logo had been broken without anyone noticing.

The image route shares the API's per-IP rate limit, because each uncached card costs two
upstream calls plus a rasterisation; rendered cards are cached for an hour. The pages
themselves are never rate limited — a human must always be able to open a link.

## Tests

```sh
./scripts/check.sh              # every gate, each reported separately
./scripts/check.sh backend      # rustfmt, clippy, tests
./scripts/check.sh frontend     # biome, tsc, build

./scripts/cargo.sh cargo test     # unit + HTTP-level tests against a stub TMDB
./scripts/cargo.sh cargo clippy --all-targets -- -D warnings
```

`check.sh` exists because chaining these with `&&` is a trap: when `cargo fmt --check` fails
it short-circuits everything after it, so clippy quietly never runs against the code you just
wrote — which is exactly how two lints reached CI with "clippy clean" in the commit message.

The HTTP tests cover the failure modes that broke the previous version: a selection kind that
is not in the enum never reaches a URL, a single selection is a 422 rather than a crash, and
an upstream 5xx is a 502 for that one request while the process keeps serving.

## Credits

This product uses the TMDB API but is not endorsed or certified by TMDB.

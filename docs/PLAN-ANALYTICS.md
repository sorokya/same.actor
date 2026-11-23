# Analytics, admin, and discovery — plan

Companion to `docs/REWRITE-PLAN.md`. Adds three things on top of the shipped rewrite:

1. **A request log** — IP, session, user agent, geo, timestamp, request, status, error code and
   message, in a persistent store.
2. **An admin API and dashboard** — read the logs, manage bans, find suspicious clients, see
   trends and charts.
3. **Public analytics and discovery pages** — trending searches, view counts per comparison,
   and Top/Upcoming/New Release pages with a cast-synergy table.

The rewrite's non-goal "no database" is lifted here. Nothing else about the product changes:
it is still a client-rendered comparison tool on one origin, one binary.

---

## 1. Decisions

Settled by the five up-front questions:

| # | Decision | Choice |
|---|----------|--------|
| Q1 | Admin auth | Single token in `ADMIN_TOKEN`, sent as a header. No third-party dependency. |
| Q2 | IP storage | Raw IP in the log row; 90 days of detail, then rolled into daily aggregates that drop the IP. |
| Q3 | Geo-IP | DB-IP Lite, **country level only**, no account or key, CC-BY attribution. |
| Q4 | Sessions | First-party cookie `sa_sid`, random id, `HttpOnly`, `SameSite=Lax`, 30-day expiry — **required on search and findout** (§4.1). |
| Q5 | Discovery table | Ranked list plus actors appearing in 2+ of the N, sorted by how many, capped and paginated. |

Settled here, flagged because they are judgement calls rather than answers:

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | `rusqlite` with `bundled`, one writer connection, a small read pool. Not `sqlx`. | `bundled` compiles SQLite in, so the image needs no system library and the version is pinned by `Cargo.lock`. A single writer task removes `SQLITE_BUSY` as a class instead of configuring around it. Migrations are `user_version` steps, ~40 lines — the same call as the hand-rolled rate limiter. |
| D2 | Charts hand-rolled as SVG. No chart library. | The whole SPA is 38 kB. Chart.js would triple it to draw four charts. `og.rs` already generates SVG; the same approach in Preact is ~150 lines for line, bar and sparkline. |
| D3 | The dashboard lives in the same SPA at `/admin`, lazy-loaded. | A second app to build, deploy and version for one user is not worth it. |
| D4 | Bans key on a single IP or a CIDR range. **No session bans.** | A cookie is client-controlled: a banned visitor clears it. A ban you can escape by pressing ⌘⇧N is worse than no ban, because it looks like enforcement. |
| D5 | Geo resolves inside the writer task, never on the request path. | The mmdb read is a memory-mapped lookup measured in microseconds, but there is no reason for it to be in the request's critical path. |
| D6 | Discovery lists are fetched by a background job into SQLite; pages never call TMDB. | "Top movies" is the most requested page shape; upstream latency must not be in front of it, and 5 lists × 20 titles is a trivial periodic cost. |
| D7 | Assets, `/healthz` and share-card images are **not** logged by default. `LOG_ASSETS=1` turns assets on. | One page load is one row, not five. This is the difference between 20 MB/day and 100 MB/day (see §9). |
| D8 | View-count dedupe is in memory, 30 minutes, per (session, comparison). | A DB round trip per page view to dedupe is worse than losing the count on restart. Documented, not hidden. |
| D9 | Trending only publishes terms searched ≥3 times in the window. | Stops the page from exposing a single person's one-off search, and stops it being a list of typos. |
| D10 | The admin token is exchanged for a short-lived `HttpOnly` cookie. | The dashboard then never holds the token in JavaScript, so an XSS in the app cannot lift a permanent credential. |
| D11 | Error `code` and `message` are captured via a response extension set in `AppError::into_response`. | The logging middleware sees the final response, not the `Result`. Putting the error on the response is how it reaches the log without re-parsing JSON bodies. |
| D12 | Upcoming/New Releases use `TMDB_REGION`, default `US`. | TMDB needs a region for release dates and cannot infer it. |
| D13 | If `ADMIN_TOKEN` is unset, admin routes 404. | A forgotten env var must not mean an open dashboard. |
| D14 | Country only, no city column, and no `GEO_LEVEL` switch. | 4 MB instead of 60 MB, one lookup, one column. Adding city later is a migration, not a redesign. |
| D15 | A session cookie is **required** on the two endpoints that call TMDB — `POST /api/findout` and `GET /api/search` — answering `428` with `Set-Cookie` when missing. Everything else is exempt. | Those are the requests that cost an upstream call and represent a visit; forcing a cookie onto a cached read of `/api/trending` or onto an admin call carrying its own token is ceremony, not attribution. See §4.1. |

---

## 2. Architecture

```
                       request
                          │
   ┌──────────────────────▼───────────────────────────────────┐
   │ TraceLayer                                               │
   │ observe      ← times the request, sends a log record     │
   │ enforce_bans ← 403 on a live ban (in-memory cache)       │
   │ rate_limit   ← 429 with Retry-After                      │
   │ routes       ← api / pages / admin                       │
   └──────────────────────┬───────────────────────────────────┘
                          │  mpsc, bounded 8192, never awaited
                 ┌────────▼─────────┐
                 │  writer task     │  batches ≤256 rows or 250 ms,
                 │  (one conn)      │  one transaction per batch
                 └────────┬─────────┘
                 ┌────────▼─────────┐        ┌──────────────┐
                 │  same-actor.db   │◄───────┤ read pool    │ 4 conns, WAL
                 │  WAL             │        └──────┬───────┘
                 └────────┬─────────┘               │
                 ┌────────▼─────────┐        ┌──────▼───────┐
                 │ nightly job      │        │ admin +      │
                 │ rollup, retention│        │ public reads │
                 └──────────────────┘        └──────────────┘
```

Logging is **fire and forget**: if the queue is full the record is dropped and a counter
increments. A slow disk must never turn into slow pages, and the alternative — backpressure
onto requests — is how an analytics feature takes down the thing it measures.

`build_app` keeps its current shape; the new layers slot in around the existing rate limit.

---

## 3. Storage

### 3.1 Store

- `rusqlite = { version = "0.43", features = ["bundled"] }`
- Pragmas at open: `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`,
  `foreign_keys=ON`, `temp_store=MEMORY`, `mmap_size=268435456`, `journal_size_limit`,
  `wal_autocheckpoint=1000`.
- `DB_PATH`, default `/data/same-actor.db`. Parent directory created at startup; the file
  lives on a volume so it survives the container.
- Migrations: `PRAGMA user_version` stepped through an array of `&str` statements, each in a
  transaction, applied on startup. Idempotent, no extra crate.
- Read pool: `Vec<Connection>` behind a `tokio::sync::Mutex`, `spawn_blocking` per query.
  Four connections; admin traffic is one human, public analytics is cached.
- Writer: a single `Connection` owned by a task that never shares it. Only that task writes.

### 3.2 Schema (`user_version` 1)

```sql
CREATE TABLE requests (
  id          INTEGER PRIMARY KEY,
  ts          INTEGER NOT NULL,      -- unix seconds, UTC
  duration_ms INTEGER NOT NULL,
  ip          TEXT,                  -- NULL after rollup
  session     TEXT,
  ua          TEXT,
  method      TEXT NOT NULL,
  path        TEXT NOT NULL,
  status      INTEGER NOT NULL,
  code        TEXT,                  -- AppError code on failure, e.g. 'upstream_error'
  message     TEXT,                  -- human message on failure
  kind        TEXT NOT NULL,         -- api | page | card | admin | asset | health
  country     TEXT                   -- ISO-3166-1 alpha-2, NULL when unknown
);
CREATE INDEX requests_ts         ON requests(ts);
CREATE INDEX requests_ip_ts      ON requests(ip, ts);
CREATE INDEX requests_session_ts ON requests(session, ts);  -- no session column is NULL
                                                             -- for /api rows; see §4.1
CREATE INDEX requests_status_ts  ON requests(status, ts);

CREATE TABLE searches (
  id        INTEGER PRIMARY KEY,
  ts        INTEGER NOT NULL,
  mode      TEXT NOT NULL,        -- title | person
  term      TEXT NOT NULL,        -- as typed, truncated to 120 chars
  term_norm TEXT NOT NULL,        -- lowercased, trimmed, whitespace collapsed
  results   INTEGER NOT NULL,
  session   TEXT,
  ip        TEXT
);
CREATE INDEX searches_ts        ON searches(ts);
CREATE INDEX searches_norm_ts   ON searches(term_norm, ts);

CREATE TABLE comparisons (
  key        TEXT PRIMARY KEY,    -- 'title:movie-603,movie-604'
  mode       TEXT NOT NULL,
  picks_json TEXT NOT NULL,       -- [{id, kind, label}] for display
  label      TEXT NOT NULL,       -- 'The Matrix + The Matrix Reloaded'
  views      INTEGER NOT NULL DEFAULT 0,
  cards      INTEGER NOT NULL DEFAULT 0,
  first_seen INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL
);
CREATE INDEX comparisons_views ON comparisons(views DESC);

CREATE TABLE bans (
  id         INTEGER PRIMARY KEY,
  created_ts INTEGER NOT NULL,
  expires_ts INTEGER,             -- NULL = permanent
  target     TEXT NOT NULL,       -- an address, or a CIDR range
  kind       TEXT NOT NULL,       -- ip | cidr
  reason     TEXT,
  created_by TEXT NOT NULL,       -- 'admin' | 'auto:<rule>'
  hits       INTEGER NOT NULL DEFAULT 0,
  last_hit   INTEGER
);
CREATE UNIQUE INDEX bans_target ON bans(target);

CREATE TABLE daily_requests (
  day        TEXT NOT NULL,
  kind       TEXT NOT NULL,
  country    TEXT NOT NULL DEFAULT '',
  status_cls INTEGER NOT NULL,    -- 2 | 3 | 4 | 5
  requests   INTEGER NOT NULL,
  errors     INTEGER NOT NULL,
  sessions   INTEGER NOT NULL,    -- distinct
  p95_ms     INTEGER NOT NULL,
  PRIMARY KEY (day, kind, country, status_cls)
) WITHOUT ROWID;

CREATE TABLE daily_searches (
  day       TEXT NOT NULL,
  mode      TEXT NOT NULL,
  term_norm TEXT NOT NULL,
  searches  INTEGER NOT NULL,
  PRIMARY KEY (day, mode, term_norm)
) WITHOUT ROWID;

CREATE TABLE daily_comparisons (
  day   TEXT NOT NULL,
  key   TEXT NOT NULL,
  views INTEGER NOT NULL,
  PRIMARY KEY (day, key)
) WITHOUT ROWID;

CREATE TABLE lists (
  id         TEXT PRIMARY KEY,    -- 'movie:popular', 'movie:upcoming', 'tv:on_the_air', …
  fetched_ts INTEGER NOT NULL,
  payload    TEXT NOT NULL        -- JSON: [{id, title, year, poster, overview, rating}]
);

CREATE TABLE list_synergy (
  list_id     TEXT PRIMARY KEY,
  computed_ts INTEGER NOT NULL,
  payload     TEXT NOT NULL       -- JSON: [{person_id, name, count, titles:[]}]
);
```

Two deliberate omissions: no `users` table (there are no accounts), and no per-row foreign
keys between `searches`/`requests` (the volume does not justify the write cost, and rollup
deletes rows independently).

### 3.3 Retention

A nightly job at 03:00 local, plus once at startup if the newest aggregate is stale:

1. For each day older than `LOG_RETENTION_DAYS` (default 90) that still has raw rows:
   compute the day's aggregates from raw (idempotent upsert into `daily_*`), then delete the
   raw rows. The aggregates never contain an IP or a session id — that is the point of the
   cutover.
2. `PRAGMA incremental_vacuum` and `PRAGMA optimize`.

Aggregates are kept indefinitely; they are a few hundred rows per day.

---

## 4. Request logging

Middleware `observe`, outermost inside `TraceLayer`:

1. Start an `Instant`, resolve the client IP (`client_ip`, already exists), read `sa_sid`,
   resolve `kind` from the path.
2. Run the rest of the stack.
3. Read the final status and the `ErrorDetail { code, message }` extension if present.
4. Build a `RequestLog` and `try_send` it to the writer.

**Cookie.** Minted on page responses when absent: 16 random bytes, base64url, `HttpOnly`,
`SameSite=Lax`, `Path=/`, `Max-Age=2592000`, `Secure` when `PUBLIC_ORIGIN` is `https`.
`getrandom` for the bytes — one small dependency, no `rand` tree. On the API it is mandatory
(§4.1).

**Geo.** In the writer task, `geo.rs` looks the IP up in a memory-mapped DB-IP Lite **country**
mmdb and attaches the ISO code. Private and loopback addresses resolve to nothing, which is
correct and also means local development logs clean landscape.

**The geo database** is *not* baked into the image (it goes stale monthly). A background task
downloads the current month's file into `/data/`, verifies the gzip container and the mmdb
header, and reloads it on swap. No file, no network, or a failed check all mean "no geo",
logged once — never a startup failure.

Two facts verified against the live service, both of which would otherwise cost an afternoon:

- **The download 403s without a `User-Agent`.** `https://download.db-ip.com/free/dbip-country-lite-{YYYY-MM}.mmdb.gz`
  returns `206` with a normal UA and `403` with an empty one. The downloader must set a UA
  explicitly — the same class of silent failure as the broken TMDB logo URL, except this one
  degrades to "no geo forever" instead of a missing image.
- **Country-level is 4.2 MB compressed** (city-level is 60 MB, which is why it was dropped —
  D14). The downloader only ever asks for `dbip-country-lite-{YYYY-MM}.mmdb.gz`.

```rust
pub struct RequestLog {
    pub ts: i64,
    pub duration_ms: u32,
    pub ip: Option<IpAddr>,
    pub session: Option<String>,
    pub ua: Option<String>,
    pub method: Method,
    pub path: String,
    pub status: u16,
    pub code: Option<&'static str>,
    pub message: Option<String>,
    pub kind: Kind,
}
```

### 4.1 The session cookie is required on the API

`/api/*` without a `sa_sid` cookie is refused:

```
428 Precondition Required
Set-Cookie: sa_sid=<id>; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000
{"error":{"code":"session_required","message":"this API requires a session cookie; retry with the cookie from this response"}}
```

`428` rather than `400` because the request is well-formed and the caller can fix it by
repeating with the cookie it was just handed — which is exactly what `curl -c jar -b jar` does
on its second call, and what the SPA already does because it loaded a page first.

**Crawler-facing routes are exempt.** `/og/{mode}/{picks}.png`, `/title/{picks}`,
`/person/{picks}` and `/healthz` never require a cookie. Discord, Slack, Twitter and every
other unfurler fetch the share card once, with no cookie jar and no intention of honouring
`Set-Cookie`. Requiring a cookie there would break every embed in exchange for nothing.

Consequences worth stating plainly:

- The SPA's `fetch` calls are same-origin, so cookies ride along by default; no code change.
- `SameSite=Lax` means the cookie is *not* sent on a cross-site `POST`, so requiring it does
  not accidentally open a CSRF hole on `POST /api/findout`.
- A scripted scraper now has to hold state. Combined with IP bans (§5) that is a real speed
  bump, not a security boundary — a scraper with a cookie jar is trivially written.
- Every existing HTTP test that calls `/api` needs a cookie; that is a genuine contract change
  and the reason phase 2 ships before anything reads these rows.

---

## 5. Bans

- Model: `Ban { target, kind, expires_at, reason, created_by }`. Kinds `ip` and `cidr` only —
  there is no session ban, by D4.
- Enforcement: before the rate limit, so a banned client gets a `403` and a reason rather than
  a `429` that invites a retry. The ban set is held in memory and refreshed every 30 s and on
  every mutation, so enforcement costs no DB read per request.
- Matching: exact address, or a CIDR range covering it (v4 and v6, `/24` and `/48` the likely
  defaults). Longest prefix wins for reporting, so the log shows the narrowest ban that caught
  the request rather than just "banned".
- `hits` and `last_hit` increment on the write path, batched like other writes.
- Expired bans are ignored by the matcher and purged by the nightly job.
- Auto-ban rules exist but default **off**: `AUTOBAN_REQUESTS` (n requests / window) and
  `AUTOBAN_ERRORS` (n 4xx / window). Enabled, a matched rule writes `created_by='auto:<rule>'`
  so the admin can tell a machine's decisions from their own.

---

## 6. Admin API

All under `/api/admin`, `ADMIN_TOKEN` required, constant-time compare, `Cache-Control:
no-store`. Unset token ⇒ 404 on the whole subtree.

| Method | Path | Purpose |
|---|---|---|
| POST | `/session` | Exchange the token for a 12 h `HttpOnly` admin cookie. |
| GET | `/overview` | Totals for 24 h / 7 d / 30 d: requests, errors, unique IPs, sessions, p95, top paths, top countries. |
| GET | `/logs` | Filter by time range, ip, session, country, status, kind, path prefix, free text; cursor paginated. |
| GET | `/logs/{id}` | One row, full detail including UA and message. |
| GET | `/stats/timeseries` | `metric=requests\|errors\|visitors\|searches`, `window=24h\|7d\|30d`, `bucket=hour\|day`. |
| GET | `/stats/geo` | Countries and cities, by requests and by errors. |
| GET | `/stats/searches` | Top terms and zero-result terms (the latter is free product insight). |
| GET | `/stats/comparisons` | Most viewed comparison sets, with their labels. |
| GET | `/suspicious` | Scored clients with their reasons. |
| GET | `/bans` · POST `/bans` · DELETE `/bans/{id}` | Manage bans. |
| GET | `/health` | DB size, rows/day, queue depth, dropped records, geo age, cache hit rate. |

### 6.1 Suspicious clients

A score, not a mystery number — every row carries the signals that produced it:

| Signal | Weight | Why |
|---|---|---|
| Requests per IP above the 99th percentile for the window | 3 | Raw volume |
| ≥50 distinct title/person ids requested per session | 3 | Walking the id space |
| 4xx ratio > 40% with ≥20 requests | 2 | Probing |
| Empty user agent, or a UA claiming a browser that never loads assets | 2 | Scripted |
| Identical paths in a mechanical cadence (fixed interval) | 2 | Not a human |
| >1 country in a session within 30 minutes | 1 | Proxy hopping |
| ≥10 rate-limit rejections | 2 | Already told to stop |

Suggested action is shown per row (ban / ignore) but **nothing is applied automatically** —
the dashboard links each signal to the filtered log query that produced it, so the decision is
made on evidence.

---

## 7. Public analytics

- `GET /api/trending` → `{ window: "24h", terms: [{term, count}] }`, cached 60 s, terms with
  fewer than 3 searches excluded (D9).
- View counts: increment `comparisons.views` when a comparison page renders or `findout`
  succeeds, deduped in memory for 30 minutes per (session, key) (D8). The page shows
  "viewed N times" next to the count heading; the share card does not — it is a share card,
  not a leaderboard.
- Home page gets a "Trending searches" strip, clickable into a real search.
- No visitor counts, no per-IP anything, ever, on the public side.

---

## 8. Discovery pages

Lists, refreshed every 6 hours by the background job:

| Route | TMDB source | Region |
|---|---|---|
| `/discover/top-movies` | `movie/popular` | — |
| `/discover/top-tv` | `tv/popular` | — |
| `/discover/new-movies` | `movie/now_playing` | `TMDB_REGION` |
| `/discover/upcoming` | `movie/upcoming` | `TMDB_REGION` |
| `/discover/on-the-air` | `tv/on_the_air` | — |

Each page: the ranked list (poster, title, year, rating), then the synergy table.

**The synergy table** is the interesting part. Twenty titles means the pairwise intersection
that powers the main feature is useless — 190 pairs, mostly empty. Instead:

- Fetch credits for the N titles (N = 20, TMDB's page size) during the refresh job.
- Build `person → set of titles`; keep people in **2 or more**.
- Sort by count descending, then name; cap at 250 rows; paginate 50.
- Cache the whole result in `list_synergy`, keyed by list id, recomputed only when the list
  snapshot changes.
- Each row links to that person's TMDB profile (`https://www.themoviedb.org/person/{id}`) in a
  new tab, `rel="noreferrer noopener"`. There is no `/person/{id}` page on this site yet, and
  phase 8 does not add one — see §14.

Cost: 5 lists × 20 credit calls every 6 hours, against the existing in-memory cache. The
refresh job is the only thing that pays it.

---

## 9. Size, budget, and failure modes

- **Rows.** ~400 bytes/row including indexes. 20k req/day with assets excluded ≈ 8 MB/day,
  ≈ 720 MB at 90 days. Assets logged would be ~4× that, which is why D7 defaults them off.
- **Writes.** Batched, one transaction per 250 ms. At 20k req/day the writer is idle; the
  design is sized for a spike, where a single-writer task with a bounded queue degrades by
  dropping log rows rather than by slowing requests.
- **Failure modes.** Disk full or corrupt DB: the writer logs once per minute and requests
  keep being served — analytics must never be load-bearing. `ADMIN_TOKEN` unset: admin 404s.
  Geo missing: country is NULL and the dashboard says so instead of showing an empty map.
- **Backups.** The DB joins the existing restic set. `sqlite3 .backup` is not needed: WAL plus
  a stop-the-writer checkpoint on shutdown is enough at this size, and restic snapshots a
  consistent file because the job runs outside a write batch.

---

## 10. Frontend

New routes in the existing SPA, all client-side and shareable:

- `/trending` — trending searches, most viewed comparisons
- `/discover/:list` — the five discovery pages
- `/admin` — dashboard, **dynamically imported** so the public bundle does not grow

Admin dashboard tabs: Overview (numbers + sparkline row), Logs (filter bar + virtualised table
+ row detail), Suspicious (score, signals, one-click filtered log), Bans (list + create form),
Health (queue depth, dropped counts, DB size, geo age).

Charts are SVG components: `LineChart`, `BarChart`, `Sparkline`. No chart library (D2).
Estimated admin chunk ~14 kB gzipped; the public bundle gains the trending strip only.

Login: a token field posting to `/api/admin/session`; the cookie is `HttpOnly`, so nothing
sensitive lives in `localStorage` (D10).

One new failure state to handle: a `428 session_required` from search or findout means the
visitor has cookies blocked. The UI says so in plain words ("this app needs cookies enabled")
rather than showing an empty result set, because an empty result set is what it will look like
to them either way and it is the wrong explanation.

---

## 11. Phases

Each phase leaves the app working and shippable.

**Status:** phase 1 done — `store/` with migrations, the writer thread, the read pool and 14
tests; `DB_PATH`, `LOG_RETENTION_DAYS` and `LOG_ASSETS` in config; the container now puts the
database on a `/data` volume. Phases 2–8 not started.

| Phase | Content | Size |
|---|---|---|
| 1 | `store/` — open, pragmas, migrations, writer task, read pool, connection tests | M |
| 2 | `observe` middleware, `sa_sid` cookie, `geo.rs` + DB-IP downloader, `requests` writes | M |
| 3 | Bans — store, in-memory cache, enforcement layer, auto-ban rules (off by default) | M |
| 4 | Rollups, retention job, aggregates, `PRAGMA optimize` | S |
| 5 | Admin API — auth, overview, logs, stats, suspicious, bans, health | L |
| 6 | Admin dashboard — the chunk, the charts, the tables | L |
| 7 | Public analytics — trending endpoint + strip, view counts on comparison pages | S |
| 8 | Discovery — list refresh job, synergy computation, five pages, person links | M |

Sequencing note: 1–4 are invisible to users and can land independently of 5–8. Phase 5 should
not start before 2 has run in production for a few days, so the dashboard is built against
real data shapes rather than guesses.

---

## 12. Testing

Extending the existing 76 tests; every phase adds its own.

- **Store.** Migration from empty and re-run (idempotent); WAL and pragmas actually set;
  writer batching inserts in one transaction; read-after-write visible; a full queue drops and
  counts instead of blocking (assert request latency is unaffected).
- **Logging.** One request ⇒ exactly one row; a 502 carries `code` and `message`; a 429 is
  logged; assets and `/healthz` are absent by default and present with `LOG_ASSETS=1`; the
  cookie is `HttpOnly` and not set twice.
- **Session requirement.** `search` and `findout` without a cookie ⇒ `428` with
  `code: session_required` *and* a `Set-Cookie`; repeating with that cookie ⇒ `200` and the
  log row carries it. `/api/trending`, `/og/…`, `/title/…` and `/healthz` all succeed **without**
  a cookie — the exemption is the part that breaks embeds if it regresses, so it gets a test
  each rather than one representative.
- **Bans.** Banned IP ⇒ 403 with the reason; expired ban ignored; CIDR match on v4 and v6, and a
  `/24` ban does not catch an address outside it; hit counter increments; cache reflects a
  create and a delete immediately.
- **Admin.** No token ⇒ 401; wrong token ⇒ 401; unset `ADMIN_TOKEN` ⇒ 404; the token never
  appears in a log row or an error body; admin responses are `no-store`.
- **Rollup.** 100 raw rows roll up to aggregates that match, and the raw rows are gone; a
  second run does not double-count; no IP or session survives the cutover.
- **Suspicious.** A scripted-scraper fixture ranks first with the id-walking reason; an honest
  browser session does not appear at all.
- **Synergy.** 20 fixtures ⇒ only people in 2+; sorted by count; capped; stable across runs.
- **Trending.** Below-threshold terms excluded; case and whitespace normalise to one row.

---

## 13. What this is not

- No accounts, no per-user history, no saved comparisons.
- No third-party analytics (Plausible, Umami, GA). The data stays in the one SQLite file.
- No Postgres, no Redis, no queue broker. One file, one writer.
- No user-facing visitor counts or "people are looking at this right now".
- No geolocation on the public side, ever.

---

## 14. Resolved: no person pages

Settled — synergy rows link out to `https://www.themoviedb.org/person/{id}` and this site grows
no `/person/{id}` route. Phase 8 therefore adds five list pages and no sixth page type, and
inherits TMDB's own attribution obligations for that outbound link (already covered by the
Credits block from the last release).

If a `/person/{id}` profile ever becomes worth building, it is a follow-up with its own plan.

# same.actor — code review

Reviewed at `a005feb` ("Fix default image for people", 2025-08-19), branch `rewrite`.

Scope: 40 tracked files, ~700 lines of hand-written JS/JSX. Two deployables — a Vite/React
SPA (`src/`, rsync'd to a static host) and an Express API (`api/`, shipped as a container).

Findings marked **[live]** were reproduced against the deployed production API at
`https://same.actor`, not just inferred from source.

---

## What works

The core idea is small and the data model is honest. Two TMDB credit calls in, an
intersection over them out — no database, no user accounts, no state. The API contract is
three fields per search result and four per match, and the frontend consumes exactly that.
There is nothing here that needs to be more complicated than it is, and the rewrite should
resist the urge to add.

The UI also holds up better than its code: debounced search, chip-style selection,
click-to-remove, a results matrix with one column per selected title. That's the right shape
for the problem and worth preserving in an overhaul rather than reinventing.

---

## Critical

### C1 — Client-supplied `mediaType` is interpolated into a URL path **[live]**

`api/controllers/findOut.js:125-131`

```js
async function getCredits({ id, mediaType }) {
  const api = getCreditsApi(mediaType);
  const url = `https://api.themoviedb.org/3/${mediaType}/${id}/${api}?language=en-US`;
```

`mediaType` comes straight off the request body. `findOut()` validates only the *outer*
`body.mediaType` (`'film' | 'actor'`), and only checks that a person isn't mixed with a
title. The per-selection `mediaType` used to build the URL is never whitelisted, so an
attacker chooses the upstream path.

```
POST /api/findOut
{"mediaType":"film","selection":[{"id":603,"mediaType":"../../account"}]}
→ HTTP 502
```

`getCreditsApi()` falls through to `'credits'`, so the URL becomes
`https://api.themoviedb.org/3/../../account/603/credits`, which normalises to
`https://api.themoviedb.org/account/603/credits`. That lands on a different TMDB endpoint
with a different auth requirement — the request the user's API key wasn't scoped for.
Today it just 404s; the point is that the request is being constructed from unvalidated
input, and the whole `/3/<path>` surface is reachable. `id` has the same problem: it is
interpolated raw and never checked to be a number.

### C2 — Any malformed request kills the API process **[live]**

`api/controllers/findOut.js:138-147` returns `null` on a failed upstream call:

```js
if (!response.ok) { console.error(...); return null; }
const data = await response.json();
if (!data.cast) { return null; }
```

Both callers then dereference it without checking (`:35-39`, `:82-87`):

```js
const credits = await Promise.all(selection.map((s) => getCredits(s)));
credits.forEach((credit) => { credit.films.forEach(...) });   // credit === null → TypeError
```

Nothing catches it. `findOutActor`/`findOutFilm` are `async` but are invoked from a `switch`
inside `findOut`, which does not `await` or return the rejection to Express 4 — and Express 4
does not catch async rejections. The rejection is unhandled, and since Node 15 an unhandled
rejection terminates the process by default.

Reproduced with a well-formed request that simply has one selection instead of two:

```
POST /api/findOut {"mediaType":"actor","selection":[{"id":603,"mediaType":"movie"}]}
→ HTTP 502 in 0.25s     (then valid requests 200 again — the container restarted)
```

Two conclusions. First, a four-line unauthenticated request restarts the API and drops every
in-flight request on the box. Second, the same path means *any* TMDB hiccup — a 429, a 5xx, a
title with no `cast` key — takes the service down instead of returning an error. The restart
policy is the only thing keeping the site up, which is why nobody has noticed. Both
selection-count (no server-side minimum of 2) and upstream-failure handling collapse into
this one bug.

---

## High

### H1 — Rate limiting counts the reverse proxy, not the visitor

`api/index.js:3-8` configures `express-rate-limit` with no `keyGenerator` and no
`app.set('trust proxy', …)`. The default key is `req.ip`, which behind openresty/Caddy is
the proxy's socket address. Every visitor therefore shares one 5-requests-per-second bucket.
It also means `X-Forwarded-For` is ignored for logging, so the logs can't tell you who did
what.

Observed ceiling from a single vantage point is exactly 5 req/s for the whole site
(`RateLimit-Policy: 5;w=1`), with a `429` on the sixth. Two people searching at once will
starve each other.

### H2 — A 429 is silently swallowed into a no-op

`src/utils/search.js:12` and `src/utils/getMatches.js:15` both `return null` on 429, and both
callers treat `null` as "nothing happened":

- `Search.jsx:22` — `if (results) setSearchResults(results)`, so a rate-limited keystroke
  leaves the previous query's dropdown on screen with no indication it's stale.
- `App.jsx:33` — `if (matches) setResults(matches)`, so "Find out!" silently does nothing.
  The user presses the button and gets no spinner, no error, no change.

A null return that means "failed" is being consumed as "empty".

### H3 — Search responses can arrive out of order and overwrite each other

`src/components/Search.jsx:17-28`

```js
if (debouncedTerm) { results = await search({ term, mediaType }) }
...
}, [debouncedTerm, mediaType]);
```

The effect re-runs on `debouncedTerm` but sends `term` — read from the closure at the moment
the effect was created. So the payload and the trigger disagree. There is also no
`AbortController`, no request sequence number, and no guard before `setSearchResults`. Two
requests in flight resolve in whatever order the network gives them; the older response wins
if it lands second, and the user sees results for a query they've already typed past.

### H4 — Switching mode can compute results in the old mode

`src/App.jsx:26-36` — `getResults` is a `useCallback` with deps `[selection, loading]` but
reads `mediaType` in its body. Change the radio from Movies & TV to Actors, press "Find out!",
and the callback can still close over `'film'` — you get actor-mode selections sent with a
film-mode discriminator, or the reverse.

### H5 — Frontend assumes every response is JSON

`Search.jsx` and `App.jsx` both call `response.json()` unconditionally. The API returns
`res.status(400).send('No selection provided')` as `text/html`, and Express's default 500
handler returns an HTML error page. `.json()` throws on both, unhandled inside an async
effect. Combined with C2, an API restart during a search surfaces as a silent console error.

### H6 — No cache in front of TMDB

Every keystroke past three characters and every "Find out!" click is one to five live upstream
requests. "matrix" typed slowly is ~7 requests; pressing "Find out!" twice is 4 more. There is
no `Cache-Control` on the responses and no server-side memoisation, so the free-tier TMDB quota
is spent on repeated identical queries. Adding a short TTL cache would take most of the load
off the rate limit that H1/H2 then stop tripping.

---

## Medium

| # | Where | Issue |
|---|---|---|
| M1 | `findOut.js:155` | `new Date(c.release_date \|\| c.first_air_date).getFullYear()` — when both are `undefined`, `new Date(undefined)` is Invalid Date, `getFullYear()` is `NaN`, and `JSON.stringify` emits `null`. A `NaN` year leaks to the client as a null that the type doesn't admit. |
| M2 | `Results.jsx:36` | `mediaType === 'person'` — `mediaType` is only ever `'film'` or `'actor'`. Dead condition, so result years never render in actor mode. |
| M3 | `Button.jsx:24-25` | `if (props.disabled) { }` — empty block, dead code. |
| M4 | `Dropdown.jsx:20` | `item.overview.length` — throws if `overview` is absent. The search handler also emits `"Known for: "` with a dangling label when `known_for` is empty. |
| M5 | `Dropdown.jsx:6`, `Selected.jsx:5` | `onClick` on `<li>` with no `role`, no `tabIndex`, no key handler. Unreachable by keyboard. `biome.json` turns off `a11y/useKeyWithClickEvents` to silence the warning. |
| M6 | `Header.jsx`, `Dropdown.jsx`, `Results.jsx` | `<img>` with no `width`/`height` and no `loading="lazy"` — layout shift on every result row, and 16 full-size TMDB thumbnails fetched at once. |
| M7 | `findOut.js:11` | `selection.length > 5` is checked but there is no `>= 2`. One selection is a crash (C2), not a validation error. |
| M8 | `findOut.js:161-170` | TV characters are joined with `', '` into a single string. The client can't distinguish a character from a list, and film rows have the raw string. Contract asymmetry that surfaces in the table. |
| M9 | `search.js:76-95` | `getImgUrl({media_type:'person', ...d})` relies on the spread order to force `'person'`. Implicit and fragile; also duplicates the `w154` URL construction in three places with a hardcoded `https://same.actor/` fallback. |
| M10 | `api/index.js` | Hardcoded port 3001, `dotenv` loaded in production, no `/healthz`, no `HEALTHCHECK` in the `Dockerfile`, no graceful shutdown, no structured logging, `X-Powered-By` still on. |

---

## Structural

**S1 — Split deployment, no contract.** `deploy.yml` rsyncs `dist/` to `github@leek.cafe` on
push; `docker-publish.yml` separately builds and signs the API image. The frontend and API
version independently, and nothing checks that the JSON shapes match. The only artefact
binding them is a comment-free `fetch` call. A single container serving both — which is what
the Rust rewrite enables for free — removes this failure mode entirely.

**S2 — No tests.** There is no test file, no test runner, and no CI step that would run one.
C1, C2, H3 and H4 are all one-line assertions that nothing was in a position to make. A test
suite is the highest-value thing the rewrite can add.

**S3 — No types.** Plain JS with no `checkJs`. `mediaType` carries two different meanings
(`'film' | 'actor'` at the API boundary, `'movie' | 'tv' | 'person'` inside TMDB mapping) with
no distinction between them. That naming collision is the direct cause of M2, and it's why
`Results.jsx` compares against a value that can never occur.

**S4 — Dependency drift.** Seven stale Dependabot PRs, a `package-lock.json` at
`lockfileVersion 3` with Express 4 / `node-fetch` 2 / `body-parser` (all superseded), and
`dependabot.yml` watches `/` only — `api/` has never been updated automatically. `eslint` and
`biome` are both configured, doing overlapping jobs, and `biome` is the one wired to
`npm run lint`.

**S5 — Accessibility disabled rather than fixed.** `biome.json` sets
`a11y/useKeyWithClickEvents: "off"`. The fix is a `<button>`; the config just mutes the
symptom.

---

## Findings to carry into the rewrite

1. Whitelist every enum that reaches a URL. `mediaType` and `id` become typed newtypes, not
   `String`.
2. No error path may produce a panic in a handler. Upstream failure → typed `Result`, mapped
   to a status code. A rewrite in Rust makes this structural: the borrow checker won't stop
   you dereferencing `Option`, but `?` and a single error type will.
3. Cache upstream responses. It's the difference between a rate limit that matters and one
   that doesn't.
4. Rate limit on the real client IP, and make the trust boundary explicit config.
5. Errors travel to the UI as errors — a discriminated union, not `null`.
6. Keep the interaction model: debounced search, chips, comparison matrix, click to remove.
   Overhaul the presentation, not the flow.

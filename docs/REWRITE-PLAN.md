# Rewrite plan

Companion to `docs/REVIEW.md`. Three workstreams — Rust backend, frontend dependency
upgrade, UI overhaul — plus a container-only build workflow.

---

## 1. Goals and non-goals

**Goals**

- Eliminate every Critical/High finding in the review, structurally rather than by patching.
- One deployable: a single Rust binary serving the SPA and the API on one origin, one version.
- A test suite that would have caught C1, C2, H3 and H4.
- Frontend on current dependencies (React 19, Vite 8, Tailwind 4, Biome 2) with types.
- An interface that holds up on a phone and is usable with a keyboard.
- Every build and every test runs in a container. Nothing but `podman` on the host.

**Non-goals**

- No database, no accounts, no saved comparisons, no social features.
- No change to the product's shape. It's a two-to-five-way comparison tool; that stays.
- No SSR/Next.js. It's a client-rendered single page and doesn't need a server render.
- No new upstream provider. TMDB stays.

---

## 2. Target architecture

```
┌─────────────────────────────────────────────────────┐
│  same-actor  (single binary, distroless, ~12 MB)    │
│                                                     │
│  GET  /                    SPA (embedded or /dist)   │
│  GET  /assets/*            hashed static assets      │
│  GET  /api/search          TMDB multi/person search  │
│  POST /api/findout         credit intersection       │
│  GET  /healthz             liveness                  │
│  GET  /readyz              TMDB reachability         │
│                                                     │
│  tower middleware:                                   │
│    TraceLayer → CORS → rate limit → cache → routes   │
└─────────────────────────────────────────────────────┘
                        │
                        ▼
              api.themoviedb.org/3
```

Two containers in dev (`spa` watcher + `api`), one in production. The existing split between
a static host and a signed API image goes away, and with it S1.

**Contract.** The wire shape stays as close to today's as is useful, so the frontend can be
migrated independently of the backend, but with the bugs fixed:

- `GET /api/search?term=&mode=title|person` → `SearchResult[]`
- `POST /api/findout` `{mode, selection:[{id, kind}]}` → `Match[]`

The `mode`/`kind` rename separates the two senses of `mediaType` that caused M2/S3:
`mode` is what the *user* is comparing (`title` vs `person`), `kind` is what a *selection*
is (`movie` | `tv` | `person`). TMDB's own vocabulary is used for `kind` and nowhere else.

- Errors are always `{"error":{"code","message"}}` with a real HTTP status — never a bare
  `text/html` body, never a 200 with a null.
- `/api/findout` requires 2–5 selections, server-side, and returns 422 otherwise.

---

## 3. Backend rewrite — Rust

### Crates

| Crate | Version | Role |
|---|---|---|
| `axum` | 0.8 | router, extractors |
| `tokio` | 1.54 | runtime (rt-multi-thread, signal) |
| `reqwest` | 0.13 | upstream client (rustls, json, gzip) |
| `serde` / `serde_json` | 1 | wire types |
| `tower` / `tower-http` | 0.5 / 0.7 | trace, compression, static files |
| `tower-governor` | 0.8 | per-IP rate limiting (keyed on real client IP) |
| `moka` | 0.12 | TTL cache in front of TMDB |
| `tracing` / `tracing-subscriber` | 0.1 | structured logs |
| `thiserror` | 2 | error enum |

### Layout

```
api-rs/
├── Cargo.toml
├── Dockerfile
└── src/
    ├── main.rs          bootstrap, config, router, graceful shutdown
    ├── config.rs        env parsing, typed, fail-fast
    ├── error.rs         AppError → IntoResponse, one place maps errors to status codes
    ├── state.rs         AppState { client, config, cache }
    ├── tmdb/
    │   ├── mod.rs       client wrapper, retry, timeout, auth
    │   ├── types.rs     wire structs (serde, unknown fields ignored)
    │   └── mapping.rs   TMDB types → domain types, the only place dates/images are built
    ├── domain.rs        Selection, Kind, Mode, SearchResult, Match  (serialised)
    ├── routes/
    │   ├── mod.rs
    │   ├── search.rs
    │   ├── findout.rs
    │   └── health.rs
    └── intersect.rs     pure functions: intersect_films, intersect_cast  ← unit-tested
```

The intersection logic moves into `intersect.rs` as pure functions over already-fetched
credits. That's what makes it testable without a network, and it's where the old
`forEach`-and-`Array.some` O(n²) collapses into a `HashMap` pass.

### Error model

```rust
pub enum AppError {
    Validation(ValidationError),   // 400 / 422 — client's fault, message is safe to echo
    Upstream(UpstreamError),       // 502 — TMDB failed; detail logged, never returned
    RateLimited { retry_after: u64 }, // 429 + Retry-After
    NotFound,                      // 404 — TMDB 404 for a bad id
    Internal(anyhow::Error),       // 500
}
```

Every handler returns `Result<Json<T>, AppError>`. There is no path from a handler to a
process abort: no `unwrap`, no `expect`, no indexing, and `unsafe` denied at the crate root.
On top of that, `debug_assert`-free production behaviour comes from the type system — `id` is
a `NonZeroU32`, `kind` is a closed enum deserialised by serde, so C1's path injection cannot
be represented.

### Specific fixes mapped to findings

| Finding | Fix |
|---|---|
| C1 | `Kind`/`Mode`/`SelectionId` are enums and `NonZeroU32`; serde rejects anything else with a 422 before a URL exists. |
| C2 | `?` everywhere into `AppError`; `.cast` accessed through a typed struct where a missing field is a `Validation`/`Upstream` error, not a `null`. Panic = abort configured to abort only on genuine bugs, and caught by a `CatchPanicLayer` so a bug in one handler is a 500, not an outage. |
| H1 | `tower-governor` keyed on the peer IP, plus `TRUSTED_PROXY` config so `X-Forwarded-For` is honoured only from the expected hop. |
| H2 | `429` reaches the UI as a typed error with `Retry-After`, surfaced as "Slow down a moment". |
| H6 | `moka` cache, 10 min TTL, 10k entries, keyed on the upstream URL; `Cache-Control: public, max-age=60` on search, `max-age=600` on findout. Negative caching off. |
| M1 | Dates parsed with `time` and mapped to `Option<u16>`; no `NaN`, no null-in-a-number-field. |
| M7 | `2..=5` enforced in the deserialiser. |
| M8 | `character: Vec<String>` for TV, string for film, normalised to `Vec<String>` in the contract. |
| M10 | Typed config from env with startup failure on missing `TMDB_KEY`; `/healthz`; `HEALTHCHECK`; `axum::serve(..).with_graceful_shutdown(signal)`; JSON logs. |

### Tests

- `intersect.rs` — pure-function unit tests, including the empty-credits and
  single-selection cases that crash today.
- Route tests with a stub TMDB: an in-process axum server serving canned JSON, injected via
  `TMDB_BASE_URL`. Covers upstream 500, upstream 404, malformed body, missing `cast`.
- Deserialisation tests: `mediaType: "../../account"` → 422; `id: "603abc"` → 422;
  1 selection → 422; 6 selections → 422. **These are the regression tests for C1 and C2.**
- Rate-limit test: six rapid requests → the sixth is 429 with `Retry-After`.

---

## 4. Frontend dependency upgrade

| Package | From | To | Notes |
|---|---|---|---|
| `react`, `react-dom` | 18.2 | 19.3 | `createRoot` already used; drop `React` default imports, `StrictMode` stays |
| `vite` | 5.2 | 8.3 | Rollup → Rolldown; `build.target` defaults are fine |
| `@vitejs/plugin-react` | 4.2 | 6.3 | |
| `tailwindcss` | 3.4 | 4.3 | **Breaking**: CSS-first config. Delete `tailwind.config.js` and `postcss.config.js`, add `@import "tailwindcss"` plus `@theme` in `index.css`, use `@tailwindcss/vite` plugin. No `autoprefixer` (built in). |
| `@biomejs/biome` | 1.7 | 2.6 | Schema rewrite; `organizeImports` → `assist.actions.source.organizeImports`; nursery rules promoted |
| `react-icons` | 5.2 | 5.7 | |
| `react-error-boundary` | 4.0 | 6.1 | |
| `@uidotdev/usehooks` | 2.4 | **removed** | Used for one hook; reimplement `useDebounce` in ~12 lines |
| `eslint*` | 8.x | **removed** | Biome is already the configured linter; two linters, one wired up |
| `typescript` | — | 7.0 | New. `strict`, `noUncheckedIndexedAccess` |

`autoprefixer`, `postcss` and `eslint-plugin-react*` all leave the tree.

Also: `dependabot.yml` gains a second entry for the Rust crate, and the npm entry is scoped
to `/`.

---

## 5. UI overhaul

Direction: **credits roll**. The app answers "what do these titles/people share?" — that's a
film-credits question, so the interface takes its language from the thing itself: monospace
numerals, hairline rules, high-contrast type on near-black, poster thumbnails as the only
saturated colour. Restrained, dense, legible. Not another gradient-button SaaS page.

**Layout**

```
┌──────────────────────────────────────────────────────┐
│  same.actor                          [title|person]  │  sticky, hairline underline
├──────────────────────────────────────────────────────┤
│  ┌────────────────────────────────────────────────┐  │
│  │ 🔍  Search films and TV…                       │  │  single input, autofocus
│  └────────────────────────────────────────────────┘  │
│   ┌ results popover: poster · title · year · ▸ ┐     │  keyboard-navigable
│   └────────────────────────────────────────────┘     │
│                                                      │
│   [ The Matrix 1999 ×] [ The Matrix Reloaded ×]      │  chips, click/⌫ removes
│                                                      │
│   ┌ Find what's shared ────────────────────────┐     │
│   │                                            │     │
│   │  Keanu Reeves        Neo    Neo            │     │  matrix: person × title
│   │  Carrie-Anne Moss    Trinity  Trinity      │     │  sticky column headers
│   │                                            │     │
│   └────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────┘
```

**Behaviour changes** (all traceable to a review finding)

- Search results in a popover anchored under the input rather than pushing the layout down.
- `AbortController` per request keyed on the debounced term; stale responses dropped (H3).
- Results are a discriminated union — `{status: 'idle'|'loading'|'ok'|'error', …}` — so a
  429 renders "Slow down a moment" and a 502 renders "TMDB is unhappy" (H2, H5).
- Selection and mode live in the URL (`?mode=title&pick=movie:603,tv:1396`), so a comparison
  is shareable and the back button works. This was impossible before (S1-adjacent).
- Results matrix becomes a `<table>` with proper `scope` headers, sticky first column on
  mobile, and cards below `sm` — the old free-floating grid doesn't reflow.
- Posters get `width`/`height` and `loading="lazy"` (M6).
- Everything clickable is a `<button>` (M5); `a11y/useKeyWithClickEvents` goes back on.
- `prefers-reduced-motion` respected; focus rings visible on every interactive element.

**Files**

```
src/
├── main.tsx
├── app.tsx                 shell, layout
├── routes/                 (no router dep: a 40-line hash-less URL state hook)
├── components/
│   ├── mode-switch.tsx
│   ├── search-field.tsx
│   ├── search-popover.tsx
│   ├── selection-chips.tsx
│   ├── result-matrix.tsx
│   ├── poster.tsx          img with fallback + dimensions
│   ├── skeleton.tsx
│   └── ui/                 button, chip, kbd
├── hooks/
│   ├── use-debounced.ts
│   ├── use-url-state.ts
│   └── use-search.ts
├── lib/
│   ├── api.ts              typed client, throws on non-2xx
│   └── types.ts            mirrors api-rs/domain.rs
└── index.css               tailwind 4 @theme tokens
```

---

## 6. Container workflow

Host has `podman` only — no `cargo`, no Node on the PATH for this project. Everything runs
through two wrapper scripts so the container image is pinned in exactly one place:

```sh
# scripts/c.sh — run a command in the pinned toolchain
podman run --rm -it \
  -v "$PWD":/w:Z -v same-actor-cargo:/usr/local/cargo/registry -w /w \
  -e CARGO_TARGET_DIR=/w/.containers/target \
  docker.io/library/rust:1-slim-trixie "$@"
```

- `rust:1-slim-trixie` for `cargo build/test/clippy/fmt`, with named volumes for the registry
  and `target/` so rebuilds aren't from scratch.
- `node:22-alpine` (matching the existing `api/Dockerfile` base) for `npm ci`, `vite build`,
  `biome check`.
- `podman-compose` (or a `Makefile`) brings up `spa` on :3000 with `/api` proxied to the Rust
  API on :3001 for local dev.

Nothing is installed on the host, so the build is reproducible and the host stays clean.

---

## 7. Phases

| Phase | Work | Done when |
|---|---|---|
| **0** | Toolchain containers, `scripts/c.sh`, branch | `cargo --version` and `node -v` both run in containers from a clean host |
| **1** | `api-rs` skeleton: config, error, health, router | `cargo test` passes in container; `/healthz` returns 200 |
| **2** | TMDB client + mapping + `intersect.rs` | unit tests for intersection pass, incl. 1-selection and empty-credits |
| **3** | `search` + `findout` routes, cache, rate limit | route tests green against stub TMDB; C1/C2 regressions covered |
| **4** | Static serving + single Dockerfile | one container serves SPA at `/` and API at `/api`; one binary, one image |
| **5** | Frontend deps: React 19, Vite 8, Tailwind 4, Biome 2, TS | `npm ci && npm run build` in container; `biome check` clean |
| **6** | UI overhaul against the new typed client | keyboard-only run-through; 360 px viewport; Lighthouse a11y ≥ 95 |
| **7** | CI: `cargo test` + `cargo clippy -D warnings` + `biome check` + `vite build`; replace the two workflows with one build-and-push | one workflow, one image |
| **8** | Cutover: point the domain at the container, retire the rsync deploy | production serving the new binary |

Phases 0–4 are backend and independent of 5–6, so they can run in parallel with a second pair
of hands.

## 8. Risks

- **TMDB key shape.** The current code sends `Bearer $TMDB_KEY`; that requires a v4 read
  token. If the deployed key is a v3 key the rewrite must send `api_key=` instead. Config
  should accept either and pick based on a single explicit env var, not sniffing.
- **Tailwind 4 is a config rewrite, not a version bump.** Budget real time for `@theme`
  tokens; the utility-class surface is compatible but the config file is not.
- **Vite 8 / Rolldown** is new enough that a plugin incompatibility is plausible. The only
  plugin in use is React's, which is first-party, so risk is low.
- **Cache correctness.** A wrong TTL on `findout` can serve stale credit data after a TMDB
  edit. 10 minutes is short enough that it can't matter for this product.
- **Scope.** The UI overhaul is the only phase with no objective completion test. Its
  acceptance criteria (keyboard-only, 360 px, a11y score) are deliberately mechanical.

---

## 9. Outcome

Phases 0–7 are implemented on the `rewrite` branch. Phase 8 (cutover) is deliberately left
to you: it changes what the domain serves.

| Phase | State |
|---|---|
| 0 Toolchain containers | done — `scripts/cargo.sh`, `scripts/node.sh`, `docker/toolchain.Dockerfile` |
| 1 Skeleton | done — `api-rs/src/{main,lib,config,error,state}.rs`, `/healthz` |
| 2 TMDB + intersection | done — `api-rs/src/tmdb/`, `api-rs/src/intersect.rs` |
| 3 Routes, cache, rate limit | done — 48 tests, 15 of them HTTP-level |
| 4 Static serving + Dockerfile | done — one image, SPA + API |
| 5 Frontend deps | done — React 19.3, Vite 8.3, Tailwind 4.3, Biome 2.6, TypeScript 7.0 |
| 6 UI overhaul | done — verified at 1280 px and 390 px |
| 7 CI | done — one workflow replacing `deploy.yml` and `docker-publish.yml` |
| 8 Cutover | **not done** — needs your go-ahead |

### Deviations from the plan above, and why

1. **Rate limiting is hand-rolled, not `tower-governor`.** The requirements are four lines of
   behaviour — per-IP key, fixed window, `429`, truthful `Retry-After`. Owning them is ~70
   lines including tests, and it removed a dependency whose middleware signature I would
   otherwise be guessing at. `api-rs/src/ratelimit.rs`.

2. **No `/readyz`.** A readiness probe that pings TMDB restarts a healthy process during
   somebody else's outage. `/healthz` reports whether *this* process is serving; that is the
   question a container orchestrator can actually act on.

3. **Tailwind runs as a PostCSS plugin, not `@tailwindcss/vite`.** With Vite 8, Lightning CSS
   inlines and minifies the stylesheet before the Tailwind Vite plugin's transform runs, which
   produced a stylesheet containing the raw `@theme` block and **zero generated utilities** —
   it looked like a successful build. `postcss.config.mjs` keeps Tailwind inside the CSS
   pipeline where it wins regardless of bundler. Verified by asserting that `.bg-ink` exists
   in the emitted CSS.

4. **`postcss.config.mjs` exists after all.** §4 says to delete it. What gets deleted is
   `autoprefixer`; the file itself is needed for the Tailwind plugin. Net dependency count is
   still three lower than before (no `autoprefixer`, `postcss` is still there, ESLint is gone).

5. **The comparison runs automatically.** The "Find out!" button was removed once results
   started rendering on load from a shared URL: with results already on screen, a button that
   says "Find out" is ambiguous — it looked like the results were stale or that pressing it
   did something else. Selection-change now drives the comparison, and the chip row shows
   "comparing N of 5" so the state is legible. A 300 ms-stable selection and a 10-minute
   cache mean this does not cost more upstream calls.

6. **`append_to_response` instead of separate credits calls.** One call per selection returns
   metadata *and* credits (`/movie/603?append_to_response=credits`), and `findout` echoes the
   resolved `selections` back. This was not in the plan and turned out to be necessary: the
   deep-linkable URL stores only `id`/`kind`, so without it a refreshed page could not label
   its own columns. It also halves the upstream request count.

7. **No `panic = "abort"` in the release profile.** §3 relies on `CatchPanicLayer` to contain
   a panicking handler. That layer cannot catch an abort, so the two were contradictory. The
   profile unwinds; a bug in one handler is a 500, not an outage.

8. **The old `api/` tree is deleted**, along with `deploy.yml` and `docker-publish.yml`. There
   is one image and one workflow now; leaving the previous implementation in the tree would
   invite somebody to patch it.

### Numbers

- The binary is **6.4 MB**; the image is **116 MB**. The plan's "< 20 MB" was a guess about a
  distroless/static target. Debian slim plus `curl` (needed for a real `HEALTHCHECK`) accounts
  for most of it. Static-linking against musl would get the image to roughly 15 MB if that
  ever matters more than the healthcheck.
- **48 tests** (32 unit, 16 HTTP) where the previous tree had none.
- Frontend production assets: 77.6 kB JS + 5.0 kB CSS gzipped.
- One upstream call per selection for `findout`, down from one per credits endpoint plus no
  metadata — and cached for ten minutes.


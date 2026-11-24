# Rust-rendered pages with progressive enhancement

**Status: done.** Phases 1–4 are in: browse pages, comparison pages with the picker island, the
dashboard, and the cleanup that deleted the single-page app. The measurements below are the ones
the decisions were made on, kept so the reasoning survives the code.

## The shape

Rust renders HTML for every page. Preact survives as **one island** — the search box and
selection chips, where instant results are the whole point. Everything else is a link or a form
that already works, optionally intercepted to avoid a round trip.

Nothing about this needs a framework for the 90% case, which is why the measurements below
matter: the cheapest option is not the one with the most familiar name.

## Crate research

| Crate | Version | Notes |
| --- | --- | --- |
| **askama** | 0.16.1 | Compile-time Jinja-like templates. MSRV 1.88 (toolchain here is 1.100). Templates become Rust code, so a typo is a compile error, not a 500 at 3am. |
| **askama_web** | 0.16.0 | `#[derive(WebTemplate)]` → `IntoResponse` for axum 0.8. Optional: `Html(t.render()?)` is two lines and keeps our own error type. |
| maud | 0.27.0 | Macro-based HTML in Rust. Elegant, but last release ~2 years ago and markup in Rust string literals makes anything large unreadable. |
| minijinja | 2.x | Runtime templates: hot reload without recompiling. Costs a runtime parse per render (cached) and loses compile-time checking — the thing we want most. |
| tera | 1.x | Same trade as minijinja, heavier, and its filter set is aimed at content sites. |

**Pick askama**, the same as your starter. The deciding property is that a malformed template
fails the build.

Verified rather than assumed: a throwaway crate with `askama 0.16` on the pinned toolchain image
builds and renders inheritance, `{% include %}`, filters and escaping. Two details from the
spike:

* Escaping produces **numeric entities** (`&#60;` rather than `&lt;`). Correct HTML, but HTML
  assertions in tests must look for the rendered text or the numeric form, not the named entity.
* `askama 0.16` has MSRV **1.88**, while `api-rs/Cargo.toml` declares `rust-version = "1.85"`.
  The declared MSRV becomes wrong the moment it is added and should be bumped in the same
  commit.

### Front end, measured

Bundled with esbuild, minified, gzip -9, in the pinned Node container:

| Bundle | Raw | Gzip |
| --- | --- | --- |
| Alpine 3.17.5 + alpine-ajax 0.12.7 | 63.6 KiB | **22.61 KiB** |
| Alpine 3.17.5 core, no ajax | 54.6 KiB | 19.45 KiB |
| alpine-ajax alone | 8.9 KiB | 3.59 KiB |
| Preact 11 island (runtime + hooks + jsx) | 13.2 KiB | **5.57 KiB** |
| *the whole current app, for scale* | 50.9 KiB | *17.38 KiB* |

Alpine plus alpine-ajax is **larger than the entire current Preact application**. That is not an
argument against Alpine in general — it is an argument against paying 22.6 KiB for what this
site would use it for, which is intercepting five links on the discover pages.

**Recommendation:** Preact for the island (already a dependency, 5.57 KiB), and a ~40-line
vanilla enhancer for fragment swaps. Alpine becomes worth its weight when the admin dashboard
moves, which is a separate decision (see Open questions).

## Architecture

```
api-rs/
  templates/
    layouts/base.html        # <head>, header, footer, blocks
    pages/{home,compare,discover,not_found}.html
    partials/{picker,chips,results,poster_grid,synergy,controls}.html
  src/views/                 # one #[derive(Template)] struct per template
  src/render.rs              # render(template) -> Result<Html<String>, AppError>
```

* **Routing becomes explicit.** `/`, `/person`, `/title/{slug}`, `/person/{slug}`,
  `/discover/{section}`, `/admin`, `/assets/*`, the API. The SPA fallback disappears, and with it
  the whole "serve index.html for anything that is not a file" contract.
* **The header is one template.** The drift fixed by hand in `SiteHeader` cannot recur: there is
  one include.
* **OG tags are part of the page.** `og.rs`'s string-splicing into the SPA shell goes away; the
  head is rendered with the data it describes. The PNG route keeps its own logic.
* **URL parsing moves to Rust.** `src/lib/url.ts` (`parsePicks`, `toPath`, slug handling) exists
  because the browser had to agree with the server. After this the server is the only parser;
  the island only *builds* URLs, so the formats cannot diverge.
* **Tailwind**: v4's automatic detection already scans sibling directories outside `src/` — a
  spike with a class that appears only in a template produced the utility without any
  configuration. `@source "../templates";` is still added, because relying on a detector whose
  rules are "everything not gitignored near the CSS file" is how a class silently vanishes after
  a directory move. Verification stays explicit: grep the built CSS for a class used only in a
  template.
* **Admin stays as it is** — a separate Preact chunk, lazily loaded, authenticated, no SEO, no
  share links, no first-paint requirement. Moving it is a rewrite of the biggest component for
  no user-visible gain. Revisit only if it starts being changed often.

### Fragment protocol

One handler per page, two renderings:

```
GET /title/movie-603-the-matrix,movie-604-the-matrix-reloaded
  → full page, results included
GET /title/… with header `X-Fragment: results`
  → the results region only, plus `Vary: X-Fragment`
```

The island recomputes the URL, requests the fragment, and swaps one element. That is the whole
of the client-side "rendering" that remains, and the results markup exists in exactly one place
(the template). Without JavaScript, every page still works; only the picker is inert, and it
already says so in a `<noscript>`.

This is your `AlpineRequest` pattern, narrowed: one target per page instead of an open-ended
header list, because we have one swap region rather than five.

## What gets deleted

* `state.rs` shell loading, the 32-character floor, and its three tests — templates are compiled
  in, so "the shell is missing" stops being a state the program can be in.
* `src/main.tsx` path-based surface switching; `app.tsx`; `discover.tsx`;
  `components/{site-header,site-nav,result-matrix,selection-chips,search-panel}.tsx` (the last two
  become the island).
* `src/hooks/use-url-state.ts`, `src/lib/url.ts` parsing half.
* `index.html` and the Vite HTML entry: Vite builds `assets/*.js|css` only.

## Phases

**Phase 1 — foundation, and the discover pages.** askama + `askama_web`, base layout with the
existing header/footer markup ported verbatim, `render()` helper, explicit routes, Tailwind
`@source`. `/discover/{section}` becomes server-rendered, including the filter controls as real
links. Accept: `curl` shows the list with no JavaScript; the HTML contains the poster grid,
synergy rows and both control groups; the existing 6 gates still pass; the SPA still serves the
tool pages, which are untouched.

**Phase 2 — the comparison pages and the island.** Rust parses the URL and renders
`/title/{slug}` and `/person/{slug}` from the same `findout` internals. Extract the search box
and chips into a Preact island mounted on `[data-island=picker]`; the results region comes from
`X-Fragment: results`. Delete the SPA paths, `index.html`, and the URL parsing in TS. Accept: a
shared link paints results before any JavaScript runs; typing in the box still answers instantly;
selecting a title updates the URL and the results without a reload.

**Phase 3 — cleanup and docs.** Remove dead modules, update `README.md` (architecture, the
fragment protocol, the Tailwind `@source` trap), and record the grep check for template classes.

## Testing

* askama renders at compile time, so template syntax and field names are already checked by
  `cargo build`.
* New render tests assert **content**, not shape: a comparison page contains the shared actor
  names and the TMDB hrefs; a discover page contains the synergy titles as links; the fragment
  request returns the results without `<html>`.
* `scripts/check.sh` is unchanged — six gates, run independently.

## Risks

1. **Tailwind class discovery from templates** — silent, and the failure looks like "the page is
   unstyled in places" in production only. Auto-detection covered it in the spike; `@source`
   plus the grep check in phase 1 make it deliberate.
2. **Askama recompiles the crate on any template change** — noisy during development, invisible
   in CI. `cargo watch -w templates` in the dev loop.
3. **Two places own the same class strings** (island and template). Shared CSS variables and the
   same utility names keep them close; the island is small enough to review as a unit.
4. **Fragment responses can be cached wrongly** if a proxy ignores `Vary`. The app sends
   `Cache-Control: no-store` on fragment responses.

## Open questions

1. **Alpine, given 22.61 KiB gzip?** My recommendation is no for now — Preact for the picker, a
   small enhancer for the swap, and revisit Alpine with the admin dashboard, where declarative
   tables and filters would actually pay for the runtime.
2. **Admin in this migration or not?** My recommendation is not: it is already built, it is
   behind a login, and rewriting it is the largest single piece of work here for no visible gain.

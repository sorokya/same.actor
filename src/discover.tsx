/**
 * `/discover/:slug` — a ranked list, and the actors who keep turning up in it.
 *
 * Presented as artwork rather than a numbered text list. The films are a poster shelf, because
 * that is how anyone recognises a film; the actors are cards with faces, because "the same actor
 * again" is a recognition, not something a name in a wide table conveys.
 *
 * Every title and every actor links to its TMDB page. This site has no person pages, and adding
 * one to serve a link from a list page is a page type this feature does not need.
 */

import { useEffect, useState } from 'preact/hooks';
import { Poster } from './components/poster';
import { SiteHeader } from './components/site-header';
import { SiteNav } from './components/site-nav';
import { LISTS, listBySlug } from './lib/lists';
import { absoluteTime, relativeTime } from './lib/time';

type Kind = 'movie' | 'tv';

interface Item {
  id: number;
  name: string;
  year: number | null;
  poster: string | null;
  overview: string | null;
  rating: number | null;
}

interface SynergyRow {
  person_id: number;
  name: string;
  image: string | null;
  count: number;
  titles: string[];
}

interface DiscoverPage {
  slug: string;
  heading: string;
  blurb: string;
  kind: Kind;
  refreshed_ts: number;
  items: Item[];
  synergy: SynergyRow[];
}

/** How many actor cards to show before "show more". */
const VISIBLE = 24;

function titleHref(kind: Kind, id: number): string {
  return `https://www.themoviedb.org/${kind}/${id}`;
}

function personHref(id: number): string {
  return `https://www.themoviedb.org/person/${id}`;
}

export function Discover({ slug }: { slug: string }) {
  const [page, setPage] = useState<DiscoverPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);

  const known = listBySlug(slug);

  useEffect(() => {
    const controller = new AbortController();
    setPage(null);
    setError(null);
    setExpanded(false);

    fetch(`/api/discover/${encodeURIComponent(slug)}`, {
      signal: controller.signal,
      headers: { accept: 'application/json' },
    })
      .then(async (response) => {
        if (!response.ok) {
          throw new Error(
            response.status === 404
              ? 'This list has not been built yet. It refreshes every few hours.'
              : `Could not load this list (${response.status}).`,
          );
        }
        return (await response.json()) as DiscoverPage;
      })
      .then(setPage, (problem: unknown) => {
        if (problem instanceof DOMException && problem.name === 'AbortError') return;
        setError(problem instanceof Error ? problem.message : 'Could not load this list.');
      });

    return () => controller.abort();
  }, [slug]);

  const rows = page?.synergy ?? [];
  const shown = expanded ? rows : rows.slice(0, VISIBLE);
  const inEveryTitle = rows.filter((row) => row.count === (page?.items.length ?? 0)).length;

  return (
    <>
      <SiteHeader>
        <SiteNav slug={slug} />
      </SiteHeader>

      <main className="mx-auto w-full max-w-3xl flex-1 px-4 pt-8 pb-24">
        <h1 className="text-2xl tracking-tight text-paper">{known?.heading ?? 'List'}</h1>
        <p className="mt-1 max-w-xl text-sm text-dim">{known?.blurb ?? page?.blurb ?? ''}</p>

        {error ? (
          <p
            role="alert"
            className="mt-6 rounded-md border border-danger/40 bg-danger/10 px-4 py-3 text-sm text-paper"
          >
            {error}
          </p>
        ) : null}

        {!page && !error ? (
          <div className="mt-8 grid grid-cols-3 gap-x-3 gap-y-5 sm:grid-cols-6" aria-hidden="true">
            {[0, 1, 2, 3, 4, 5].map((cell) => (
              <div
                key={cell}
                className="aspect-[2/3] animate-pulse rounded-md border border-line-soft bg-raise"
              />
            ))}
          </div>
        ) : null}

        {page ? (
          <>
            <p className="label mt-3">
              {page.items.length} titles · refreshed{' '}
              <span title={absoluteTime(page.refreshed_ts)}>{relativeTime(page.refreshed_ts)}</span>
            </p>

            <section className="mt-8" aria-labelledby="the-list">
              <h2 id="the-list" className="label mb-3">
                The list
              </h2>

              <ol className="grid grid-cols-3 gap-x-3 gap-y-5 sm:grid-cols-6">
                {page.items.map((item, index) => (
                  <li key={item.id}>
                    <a
                      href={titleHref(page.kind, item.id)}
                      target="_blank"
                      rel="noreferrer noopener"
                      className="group block"
                    >
                      <div className="relative">
                        <Poster src={item.poster} alt="" fluid />
                        <span className="tnum absolute left-1 top-1 rounded bg-ink/80 px-1.5 py-0.5 font-mono text-[10px] text-paper backdrop-blur">
                          {index + 1}
                        </span>
                      </div>

                      <p className="mt-1.5 line-clamp-2 text-xs leading-snug text-paper group-hover:text-accent">
                        {item.name}
                      </p>
                      <p className="tnum mt-0.5 font-mono text-[10px] text-dim">
                        {item.year ?? '—'}
                        {item.rating ? (
                          <span className="text-accent/80"> · {item.rating.toFixed(1)}</span>
                        ) : null}
                      </p>
                    </a>
                  </li>
                ))}
              </ol>
            </section>

            <section className="mt-12" aria-labelledby="synergy">
              <div className="mb-3 flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <h2 id="synergy" className="label">
                  Actors in two or more
                </h2>
                {inEveryTitle > 0 ? (
                  <p className="label">
                    {inEveryTitle} appear{inEveryTitle === 1 ? 's' : ''} in every title
                  </p>
                ) : null}
              </div>

              {rows.length === 0 ? (
                <p className="text-xs text-dim">Nobody appears twice in this list.</p>
              ) : (
                <ul className="grid gap-1.5 sm:grid-cols-2">
                  {shown.map((row) => (
                    <li
                      key={row.person_id}
                      className="flex items-center gap-2.5 rounded-md border border-line bg-raise/40 p-2"
                    >
                      {/* The portrait is decorative here: the name beside it is the link, and
                          two links to the same place would be noise for a screen reader. */}
                      <span className="shrink-0" aria-hidden="true">
                        <Poster src={row.image} alt="" shape="avatar" width={36} height={36} />
                      </span>

                      <div className="min-w-0 flex-1">
                        <div className="flex items-baseline justify-between gap-2">
                          <a
                            href={personHref(row.person_id)}
                            target="_blank"
                            rel="noreferrer noopener"
                            className="truncate text-[13px] text-paper hover:text-accent"
                          >
                            {row.name}
                          </a>
                          <span
                            className="tnum shrink-0 rounded-full border border-line px-1.5 font-mono text-[10px] text-dim"
                            title={`Appears in ${row.count} of these titles`}
                          >
                            in {row.count}
                          </span>
                        </div>
                        <p className="truncate text-[11px] text-dim/80">{row.titles.join(' · ')}</p>
                      </div>
                    </li>
                  ))}
                </ul>
              )}

              {rows.length > VISIBLE ? (
                <button
                  type="button"
                  onClick={() => setExpanded(!expanded)}
                  className="mt-3 rounded border border-line px-3 py-1.5 text-xs text-dim hover:text-paper"
                >
                  {expanded ? 'Show fewer' : `Show all ${rows.length}`}
                </button>
              ) : null}
            </section>

            <nav aria-label="Other lists" className="mt-12 border-t border-line pt-4">
              <p className="label mb-2">More lists</p>
              <ul className="flex flex-wrap gap-x-4 gap-y-1.5">
                {LISTS.filter((list) => list.slug !== slug).map((list) => (
                  <li key={list.slug}>
                    <a
                      href={`/discover/${list.slug}`}
                      className="text-xs text-dim hover:text-paper"
                    >
                      {list.label}
                    </a>
                  </li>
                ))}
              </ul>
            </nav>
          </>
        ) : null}
      </main>
    </>
  );
}

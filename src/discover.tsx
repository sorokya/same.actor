/**
 * `/discover/:slug` — a ranked list, and the actors who keep turning up in it.
 *
 * The synergy table is the point. Twenty titles makes a pairwise intersection useless (190 pairs,
 * most empty); what is interesting at this scale is the actor who appears in two or more of them,
 * which is what the server computes once per list refresh.
 *
 * Person rows link to TMDB. This site has no `/person/{id}` page, and adding one to serve a link
 * from a list page is a page type this feature does not need.
 */

import { useEffect, useState } from 'preact/hooks';
import { Poster } from './components/poster';

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
  count: number;
  titles: string[];
}

interface DiscoverPage {
  slug: string;
  heading: string;
  blurb: string;
  refreshed_ts: number;
  items: Item[];
  synergy: SynergyRow[];
}

const LISTS: { slug: string; label: string }[] = [
  { slug: 'top-movies', label: 'Top films' },
  { slug: 'top-tv', label: 'Top shows' },
  { slug: 'new-movies', label: 'In cinemas' },
  { slug: 'upcoming', label: 'Coming soon' },
  { slug: 'on-the-air', label: 'On the air' },
];

/** How many synergy rows to show before "show more". */
const VISIBLE = 40;

export function Discover({ slug }: { slug: string }) {
  const [page, setPage] = useState<DiscoverPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);

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

  return (
    <div className="mx-auto w-full max-w-3xl px-4 pb-16">
      <header className="flex flex-wrap items-baseline justify-between gap-3 border-b border-line pb-3 pt-6">
        <a href="/" className="text-sm font-medium tracking-tight text-paper">
          same<span className="text-accent">.</span>actor
        </a>
        <nav aria-label="Lists" className="flex flex-wrap gap-1">
          {LISTS.map((list) => (
            <a
              key={list.slug}
              href={`/discover/${list.slug}`}
              aria-current={list.slug === slug ? 'page' : undefined}
              className={`rounded px-2 py-1 text-xs transition-colors ${
                list.slug === slug
                  ? 'bg-raise-2 text-paper'
                  : 'text-dim hover:bg-raise hover:text-paper'
              }`}
            >
              {list.label}
            </a>
          ))}
        </nav>
      </header>

      {error ? (
        <p
          role="alert"
          className="mt-8 rounded border border-danger/40 bg-raise/40 p-3 text-sm text-dim"
        >
          {error}
        </p>
      ) : null}

      {!page && !error ? <p className="mt-8 font-mono text-xs text-dim">Loading…</p> : null}

      {page ? (
        <>
          <h1 className="mt-8 text-2xl tracking-tight text-paper">{page.heading}</h1>
          <p className="mt-1 text-sm text-dim">{page.blurb}</p>
          <p className="label mt-2">
            {page.items.length} titles · refreshed{' '}
            {new Date(page.refreshed_ts * 1000).toLocaleString()}
          </p>

          <section className="mt-8">
            <h2 className="label mb-3">The list</h2>
            <ol className="space-y-px">
              {page.items.map((item, index) => (
                <li
                  key={item.id}
                  className="flex items-center gap-3 rounded px-1 py-1.5 hover:bg-raise/60"
                >
                  <span className="tnum w-6 shrink-0 text-right font-mono text-[11px] text-dim">
                    {index + 1}
                  </span>
                  <Poster src={item.poster} alt="" width={30} height={45} />
                  <div className="min-w-0">
                    <p className="truncate text-sm text-paper">
                      {item.name}
                      {item.year ? (
                        <span className="ml-1.5 text-xs text-dim">{item.year}</span>
                      ) : null}
                    </p>
                    {item.overview ? (
                      <p className="truncate text-[11px] text-dim">{item.overview}</p>
                    ) : null}
                  </div>
                  {/* `abbr` rather than a span: it is a real element that carries a title, so
                      the number has an accessible name without an `aria-label` on an element
                      that does not support one. */}
                  {item.rating ? (
                    <abbr
                      className="tnum ml-auto shrink-0 font-mono text-[11px] text-accent/80 no-underline"
                      title="TMDB rating out of 10"
                    >
                      {item.rating.toFixed(1)}
                    </abbr>
                  ) : null}
                </li>
              ))}
            </ol>
          </section>

          <section className="mt-10">
            <h2 className="label mb-1">Actors in two or more of these films</h2>
            <p className="mb-3 text-[11px] text-dim">
              Actors who appear more than once across this list, ordered by how many titles they
              turn up in.
            </p>
            {rows.length === 0 ? (
              <p className="text-xs text-dim">Nobody appears twice in this list.</p>
            ) : (
              <>
                <div className="overflow-x-auto rounded border border-line">
                  <table className="w-full border-collapse text-left">
                    <thead>
                      <tr className="border-b border-line">
                        <th className="label px-2 py-1.5 font-normal">Actor</th>
                        <th className="label px-2 py-1.5 font-normal">Films</th>
                        <th className="label px-2 py-1.5 font-normal">Which</th>
                      </tr>
                    </thead>
                    <tbody>
                      {shown.map((row) => (
                        <tr key={row.person_id} className="border-b border-line-soft align-top">
                          <td className="px-2 py-1.5">
                            <a
                              href={`https://www.themoviedb.org/person/${row.person_id}`}
                              target="_blank"
                              rel="noreferrer noopener"
                              className="text-sm text-paper hover:text-accent"
                            >
                              {row.name}
                            </a>
                          </td>
                          <td className="tnum px-2 py-1.5 text-xs text-dim">{row.count}</td>
                          <td className="px-2 py-1.5 text-[11px] text-dim">
                            {row.titles.join(' · ')}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>

                {rows.length > VISIBLE ? (
                  <button
                    type="button"
                    onClick={() => setExpanded(!expanded)}
                    className="mt-3 rounded border border-line px-3 py-1.5 text-xs text-dim hover:text-paper"
                  >
                    {expanded ? 'Show fewer' : `Show all ${rows.length}`}
                  </button>
                ) : null}
              </>
            )}
          </section>
        </>
      ) : null}
    </div>
  );
}

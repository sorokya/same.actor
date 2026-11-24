/**
 * `/discover/:section` — a ranked list, and the actors who keep turning up in it.
 *
 * Three sections, each split by kind, and the top and new sections split again by what "top"
 * and "new" mean. That second axis is a control on the page rather than another tab: five lists
 * in the navigation was already one more than fits across, and "Films" sitting beside "Titles"
 * told a reader nothing about what the page was for.
 *
 * The controls are links. With JavaScript they rewrite the query and refetch; without it they are
 * ordinary navigation, which is the direction the front end is heading.
 *
 * Everything that is a title or an actor links to TMDB — this site has no person pages, and a
 * title on an actor's card is exactly where a reader wants to go and look.
 */

import { useCallback, useEffect, useState } from 'preact/hooks';
import { Poster } from './components/poster';
import { SiteHeader } from './components/site-header';
import { SiteNav } from './components/site-nav';
import { SECTIONS, type SectionKey, sectionByKey } from './lib/lists';
import { absoluteTime, relativeTime } from './lib/time';

interface Item {
  id: number;
  name: string;
  year: number | null;
  poster: string | null;
  overview: string | null;
  rating: number | null;
}

interface TitleRef {
  id: number;
  kind: string;
  name: string;
}

interface SynergyRow {
  person_id: number;
  name: string;
  image: string | null;
  count: number;
  titles: TitleRef[];
}

interface Choice {
  value: string;
  label: string;
}

interface DiscoverPage {
  section: SectionKey;
  heading: string;
  blurb: string;
  kind: string;
  filter: string;
  kinds: Choice[];
  filters: Choice[];
  filter_group: string;
  refreshed_ts: number;
  items: Item[];
  synergy: SynergyRow[];
}

/** How many actor cards to show before "show more". */
const VISIBLE = 24;

function hrefFor(section: string, kind: string, filter: string): string {
  const params = new URLSearchParams({ kind });
  if (filter !== '') params.set('filter', filter);
  return `/discover/${section}?${params.toString()}`;
}

function readQuery(): { kind: string; filter: string } {
  const params = new URLSearchParams(window.location.search);
  return { kind: params.get('kind') ?? '', filter: params.get('filter') ?? '' };
}

function ControlGroup({
  label,
  choices,
  active,
  hrefFor,
  onPick,
}: {
  label: string;
  choices: Choice[];
  active: string;
  /** The real URL for a value, so the control is a working link without JavaScript. */
  hrefFor: (value: string) => string;
  onPick: (value: string) => void;
}) {
  return (
    <div className="flex items-center gap-1">
      <span className="label mr-1 hidden sm:inline">{label}</span>
      {choices.map((choice) => (
        <a
          key={choice.value}
          href={hrefFor(choice.value)}
          onClick={(event) => {
            // A modified click is a link like any other; only a plain click is intercepted.
            if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return;
            event.preventDefault();
            onPick(choice.value);
          }}
          aria-current={active === choice.value ? 'true' : undefined}
          className={`rounded-full border px-2.5 py-1 font-mono text-[11px] uppercase tracking-[0.1em] transition-colors ${
            active === choice.value
              ? 'border-paper bg-paper text-ink'
              : 'border-line text-dim hover:border-dim hover:text-paper'
          }`}
        >
          {choice.label}
        </a>
      ))}
    </div>
  );
}

export function Discover({ section }: { section: SectionKey }) {
  const [page, setPage] = useState<DiscoverPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [query, setQuery] = useState(readQuery);

  const known = sectionByKey(section);

  // The controls push history entries, so the back button walks through filters.
  useEffect(() => {
    const onPopState = () => setQuery(readQuery());
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    setPage(null);
    setError(null);
    setExpanded(false);

    const params = new URLSearchParams();
    if (query.kind !== '') params.set('kind', query.kind);
    if (query.filter !== '') params.set('filter', query.filter);
    const search = params.toString();

    fetch(`/api/discover/${section}${search === '' ? '' : `?${search}`}`, {
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
  }, [section, query.kind, query.filter]);

  const select = useCallback(
    (kind: string, filter: string) => {
      setQuery({ kind, filter });
      window.history.pushState(null, '', hrefFor(section, kind, filter));
    },
    [section],
  );

  const rows = page?.synergy ?? [];
  const shown = expanded ? rows : rows.slice(0, VISIBLE);
  const inEveryTitle = rows.filter((row) => row.count === (page?.items.length ?? 0)).length;

  return (
    <>
      <SiteHeader>
        <SiteNav section={section} />
      </SiteHeader>

      <main className="mx-auto w-full max-w-3xl flex-1 px-4 pt-8 pb-24">
        <h1 className="text-2xl tracking-tight text-paper">
          {page?.heading ?? known?.heading ?? 'List'}
        </h1>
        <p className="mt-1 max-w-xl text-sm text-dim">{page?.blurb ?? known?.blurb ?? ''}</p>

        <div className="mt-5 flex flex-wrap items-center gap-x-5 gap-y-2">
          <ControlGroup
            label="Type"
            choices={
              page?.kinds ?? [
                { value: 'movie', label: 'Films' },
                { value: 'tv', label: 'Shows' },
              ]
            }
            active={page?.kind ?? (query.kind === '' ? 'movie' : query.kind)}
            hrefFor={(kind) => hrefFor(section, kind, '')}
            onPick={(kind) => select(kind, '')}
          />

          {/* Only when there is a choice to make: a one-option control is a dead button. */}
          {page && page.filters.length > 1 ? (
            <>
              <span aria-hidden="true" className="h-4 w-px bg-line" />
              <ControlGroup
                label={page.filter_group}
                choices={page.filters}
                active={page.filter}
                hrefFor={(filter) => hrefFor(section, page.kind, filter)}
                onPick={(filter) => select(page.kind, filter)}
              />
            </>
          ) : null}
        </div>

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
            <p className="label mt-4">
              {page.items.length} titles · refreshed{' '}
              <span title={absoluteTime(page.refreshed_ts)}>{relativeTime(page.refreshed_ts)}</span>
            </p>

            <section className="mt-6" aria-labelledby="the-list">
              <h2 id="the-list" className="label mb-3">
                The list
              </h2>

              <ol className="grid grid-cols-3 gap-x-3 gap-y-5 sm:grid-cols-6">
                {page.items.map((item, index) => (
                  <li key={item.id}>
                    <a
                      href={`https://www.themoviedb.org/${page.kind}/${item.id}`}
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
                      className="flex items-start gap-2.5 rounded-md border border-line bg-raise/40 p-2"
                    >
                      {/* The portrait is decorative here: the name beside it is the link, and
                          two links to the same place would be noise for a screen reader. */}
                      <span className="shrink-0" aria-hidden="true">
                        <Poster src={row.image} alt="" shape="avatar" width={36} height={36} />
                      </span>

                      <div className="min-w-0 flex-1">
                        <div className="flex items-baseline justify-between gap-2">
                          <a
                            href={`https://www.themoviedb.org/person/${row.person_id}`}
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

                        <p className="mt-0.5 text-[11px] leading-snug">
                          {row.titles.map((title, index) => (
                            <span key={title.id}>
                              {index > 0 ? <span className="text-dim/50"> · </span> : null}
                              <a
                                href={`https://www.themoviedb.org/${title.kind}/${title.id}`}
                                target="_blank"
                                rel="noreferrer noopener"
                                className="text-dim/80 hover:text-paper hover:underline"
                              >
                                {title.name}
                              </a>
                            </span>
                          ))}
                        </p>
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

            <nav aria-label="Other sections" className="mt-12 border-t border-line pt-4">
              <p className="label mb-2">Browse</p>
              <ul className="flex flex-wrap gap-x-4 gap-y-1.5">
                {SECTIONS.filter((entry) => entry.key !== section).map((entry) => (
                  <li key={entry.key}>
                    <a
                      href={`/discover/${entry.key}`}
                      className="text-xs text-dim hover:text-paper"
                    >
                      {entry.heading}
                    </a>
                  </li>
                ))}
                <li>
                  <a href="/" className="text-xs text-dim hover:text-paper">
                    Compare titles
                  </a>
                </li>
                <li>
                  <a href="/person" className="text-xs text-dim hover:text-paper">
                    Compare people
                  </a>
                </li>
              </ul>
            </nav>
          </>
        ) : null}
      </main>
    </>
  );
}

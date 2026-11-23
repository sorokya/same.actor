import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { CheckIcon, ShareIcon, TrashIcon } from './components/icons';
import { ResultMatrix } from './components/result-matrix';
import { SearchPanel } from './components/search-panel';
import { SelectionChips } from './components/selection-chips';
import { TrendingStrip } from './components/trending-strip';
import { Button, ModeSwitch } from './components/ui';
import { useSearch } from './hooks/use-search';
import { useUrlState } from './hooks/use-url-state';
import { describe, findOut } from './lib/api';
import {
  MAX_SELECTIONS,
  MIN_SELECTIONS,
  type Mode,
  type PersonMatch,
  type SearchResult,
  type Selection,
  type SelectionMeta,
  type TitleMatch,
} from './lib/types';
import type { NameFor } from './lib/url';

type ResultsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | {
      status: 'ok';
      selections: SelectionMeta[];
      matches: ReadonlyArray<PersonMatch | TitleMatch>;
      views?: number | undefined;
    }
  | { status: 'error'; message: string };

/**
 * TMDB's approved logo, served unmodified from their CDN as their brand rules require.
 * See https://www.themoviedb.org/about/logos-attribution
 */
const TMDB_LOGO_URL =
  'https://www.themoviedb.org/assets/v4/logos/v2/blue_short-8e7b30f73a4020692ccca9c88bafe5dcb6f8a62a4c6bc55cd9ba82bb2cd95f6c.svg';

const COPY: Record<Mode, { heading: string; sub: string }> = {
  title: {
    heading: 'What do these films share?',
    sub: 'Pick two to five films or shows. Every actor credited in at least two of them comes back, with their roles.',
  },
  person: {
    heading: 'What do these people share?',
    sub: 'Pick two to five actors. Every title crediting at least two of them comes back, with their roles.',
  },
};

export function App() {
  const [{ mode, picks }, navigate, replaceUrl] = useUrlState();
  const [term, setTerm] = useState('');
  const [results, setResults] = useState<ResultsState>({ status: 'idle' });
  const [shared, setShared] = useState<'idle' | 'copied' | 'failed'>('idle');

  const searchState = useSearch(mode, term);
  const inFlight = useRef<AbortController | null>(null);
  /** Search results the user has clicked, so chips have a label before the server replies. */
  const known = useRef(new Map<number, SearchResult>());
  /** id → display name, for writing readable URLs. */
  const names = useRef(new Map<number, string>());

  useEffect(() => {
    return () => inFlight.current?.abort();
  }, []);

  const nameFor: NameFor = useCallback((pick: Selection) => names.current.get(pick.id) ?? null, []);

  const runFindOut = useCallback(() => {
    if (picks.length < MIN_SELECTIONS) return;

    // One comparison at a time. A new selection replaces the previous request rather than
    // racing it, so the visible result is always the one for the current selection.
    inFlight.current?.abort();
    const controller = new AbortController();
    inFlight.current = controller;

    setResults({ status: 'loading' });

    findOut(mode, picks, controller.signal)
      .then((response) => {
        if (controller.signal.aborted) return;
        setResults({
          status: 'ok',
          selections: response.selections,
          matches: response.matches,
          views: response.views,
        });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setResults({ status: 'error', message: describe(error) });
      });
  }, [mode, picks]);

  // Results follow the selection: adding or removing an entry re-runs the comparison, so
  // there is no separate "compute" step to get out of sync with what is selected. A shared
  // link renders its results with no click at all.
  useEffect(() => {
    if (picks.length < MIN_SELECTIONS) {
      setResults({ status: 'idle' });
      return;
    }
    runFindOut();
  }, [picks.length, runFindOut]);

  const chipMeta: SelectionMeta[] =
    results.status === 'ok'
      ? results.selections
      : picks.flatMap((pick) => {
          const found = known.current.get(pick.id);
          if (found === undefined) return [];
          return [
            {
              id: found.id,
              kind: found.kind,
              name: found.name,
              year: found.year,
              image: found.image,
            },
          ];
        });

  // Once names are known, upgrade `/title/movie-603,movie-604` to
  // `/title/movie-603-the-matrix,movie-604-the-matrix-reloaded` in place, and keep the tab
  // title in step so a shared link reads sensibly in history.
  const labels = chipMeta.map((entry) => `${entry.id}:${entry.name}`).join('|');

  // Keyed on `labels`, the stable string projection of chipMeta; depending on the array
  // identity would re-run this on every render.
  useEffect(() => {
    names.current = new Map(chipMeta.map((entry) => [entry.id, entry.name]));

    if (picks.length > 0) {
      replaceUrl({ mode, picks }, (pick) => names.current.get(pick.id) ?? null);
    }

    document.title =
      chipMeta.length > 1
        ? `${chipMeta.map((entry) => entry.name).join(' vs ')} — same.actor`
        : 'same.actor — is it the same actor?';
  }, [labels, mode, picks, replaceUrl]);

  const addPick = useCallback(
    (result: SearchResult) => {
      known.current.set(result.id, result);
      setTerm('');

      if (picks.some((pick) => pick.id === result.id)) return;
      if (picks.length >= MAX_SELECTIONS) return;

      navigate({ mode, picks: [...picks, { id: result.id, kind: result.kind }] }, nameFor);
    },
    [mode, picks, navigate, nameFor],
  );

  const removePick = useCallback(
    (selection: Selection) => {
      navigate({ mode, picks: picks.filter((pick) => pick.id !== selection.id) }, nameFor);
    },
    [mode, picks, navigate, nameFor],
  );

  const changeMode = useCallback(
    (next: Mode) => {
      if (next === mode) return;
      setTerm('');
      setResults({ status: 'idle' });
      navigate({ mode: next, picks: [] });
    },
    [mode, navigate],
  );

  const clearAll = useCallback(() => {
    setTerm('');
    setResults({ status: 'idle' });
    navigate({ mode, picks: [] });
  }, [mode, navigate]);

  const share = useCallback(async () => {
    const url = window.location.href;

    // Mobile browsers get the real share sheet; that is the point of the button there.
    if (typeof navigator.share === 'function') {
      try {
        await navigator.share({ title: document.title, url });
        return;
      } catch (error) {
        // Dismissing the sheet is not a failure worth reporting.
        if (error instanceof DOMException && error.name === 'AbortError') return;
      }
    }

    try {
      await navigator.clipboard.writeText(url);
      setShared('copied');
      window.setTimeout(() => setShared('idle'), 2000);
    } catch {
      setShared('failed');
      window.setTimeout(() => setShared('idle'), 4000);
    }
  }, []);

  const ready = picks.length >= MIN_SELECTIONS;
  const copy = COPY[mode];

  return (
    <div className="flex min-h-screen flex-col">
      <header className="sticky top-0 z-20 border-b border-line bg-ink/85 backdrop-blur">
        <div className="mx-auto flex w-full max-w-3xl items-center justify-between gap-4 px-4 py-3">
          <a
            href="/"
            className="font-mono text-[13px] uppercase tracking-[0.2em] text-paper transition-colors hover:text-accent"
          >
            same<span className="text-accent">.</span>actor
          </a>
          <ModeSwitch mode={mode} onChange={changeMode} />
        </div>
      </header>

      <main className="mx-auto w-full max-w-3xl flex-1 px-4 pt-10 pb-24">
        <div className="relative">
          {/* The only purely decorative colour on the page: a warm wash behind the heading.
              Everything else is a hue attached to a specific selection. */}
          <div
            aria-hidden="true"
            className="pointer-events-none absolute -top-20 -left-28 h-56 w-[26rem] rounded-full bg-[#e8b14b]/10 blur-3xl"
          />
          <div
            aria-hidden="true"
            className="pointer-events-none absolute -top-10 right-0 h-40 w-64 rounded-full bg-[#3ecfc0]/10 blur-3xl"
          />

          <h1 className="relative text-[clamp(1.6rem,4vw,2.1rem)] leading-tight font-normal tracking-tight text-paper">
            {copy.heading}
          </h1>
          <p className="relative mt-2 max-w-lg text-sm leading-relaxed text-dim">{copy.sub}</p>
        </div>

        <div className="relative mt-8">
          <SearchPanel
            mode={mode}
            term={term}
            onTermChange={setTerm}
            state={searchState}
            onPick={addPick}
            canAdd={picks.length < MAX_SELECTIONS}
          />

          {/* Directly under the input, where a first-time visitor is already looking. */}
          {picks.length === 0 && results.status === 'idle' ? (
            <TrendingStrip onPick={setTerm} />
          ) : null}
        </div>

        <div className="mt-6 space-y-5">
          <SelectionChips selections={picks} meta={chipMeta} onRemove={removePick} />

          <div className="flex flex-wrap items-center gap-3">
            {picks.length > 0 ? (
              <>
                <Button variant="ghost" onClick={share}>
                  {shared === 'copied' ? <CheckIcon size={13} /> : <ShareIcon size={13} />}
                  {shared === 'copied' ? 'Link copied' : 'Share'}
                </Button>
                <Button variant="danger" onClick={clearAll}>
                  <TrashIcon size={13} />
                  Clear
                </Button>
              </>
            ) : null}

            {shared === 'failed' ? (
              <span className="label text-danger" role="status">
                Copy failed — use the address bar
              </span>
            ) : null}

            {!ready && picks.length > 0 ? (
              <span className="label">Add one more to compare</span>
            ) : null}
          </div>
        </div>

        <section className="mt-8" aria-busy={results.status === 'loading'}>
          {results.status === 'loading' ? (
            <div className="space-y-2" role="status" aria-label="Loading results">
              {[0, 1, 2, 3].map((row) => (
                <div
                  key={row}
                  className="h-14 animate-pulse rounded-md border border-line-soft bg-raise"
                />
              ))}
            </div>
          ) : null}

          {results.status === 'error' ? (
            <div
              role="alert"
              className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-danger/40 bg-danger/10 px-4 py-3"
            >
              <p className="text-sm text-paper">{results.message}</p>
              <Button variant="ghost" onClick={runFindOut}>
                Try again
              </Button>
            </div>
          ) : null}

          {results.status === 'ok' ? (
            <div>
              <div className="mb-3 flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <h2 className="text-xs text-dim">
                  {mode === 'title'
                    ? `${results.matches.length} ${results.matches.length === 1 ? 'actor' : 'actors'} in at least two of these titles`
                    : `${results.matches.length} ${results.matches.length === 1 ? 'title' : 'titles'} featuring at least two of these people`}
                </h2>
                <div className="flex items-baseline gap-3">
                  {/* Only once it is worth saying. "Viewed 1 time" is noise about yourself. */}
                  {results.views && results.views > 1 ? (
                    <p className="label">viewed {results.views.toLocaleString()} times</p>
                  ) : null}
                  {results.matches.length > 1 ? <p className="label">most shared first</p> : null}
                </div>
              </div>
              <ResultMatrix mode={mode} selections={results.selections} matches={results.matches} />
            </div>
          ) : null}

          {results.status === 'idle' && picks.length > 0 ? (
            <p className="text-sm text-dim">
              {ready ? 'Comparing your selection…' : 'Add one more to compare. Two is the minimum.'}
            </p>
          ) : null}
        </section>
      </main>

      <footer className="border-t border-line">
        <div className="mx-auto w-full max-w-3xl px-4 py-5">
          <nav aria-label="Browse" className="mb-5">
            <p className="label mb-2">Browse by list</p>
            <ul className="flex flex-wrap gap-x-4 gap-y-1.5">
              {[
                ['top-movies', 'Top films'],
                ['top-tv', 'Top shows'],
                ['new-movies', 'In cinemas'],
                ['upcoming', 'Coming soon'],
                ['on-the-air', 'On the air'],
              ].map(([slug, label]) => (
                <li key={slug}>
                  <a href={`/discover/${slug}`} className="text-xs text-dim hover:text-paper">
                    {label}
                  </a>
                </li>
              ))}
            </ul>
          </nav>

          {/* TMDB's attribution terms require their logo, the notice below, and both to sit
              in a Credits-type section — and the logo to be less prominent than our own
              mark. The wordmark above is 13px; this sits at 10px and lower contrast. */}
          <section aria-labelledby="credits-heading">
            <h2 id="credits-heading" className="label">
              Credits
            </h2>

            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2">
              <a
                href="https://www.themoviedb.org/"
                target="_blank"
                rel="noreferrer noopener"
                className="inline-flex items-center opacity-70 transition-opacity hover:opacity-100"
              >
                <img
                  src={TMDB_LOGO_URL}
                  alt="The Movie Database"
                  height={10}
                  className="h-2.5 w-auto"
                  loading="lazy"
                  decoding="async"
                />
              </a>
              <p className="text-xs text-dim">
                Cast, crew and title data from TMDB. This product uses the TMDB API but is not
                endorsed or certified by TMDB.
              </p>
            </div>

            <p className="mt-3 text-[11px] text-dim/80">
              © {new Date().getFullYear()} Richard Leek
            </p>
          </section>
        </div>
      </footer>
    </div>
  );
}

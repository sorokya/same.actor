import { useCallback, useEffect, useRef, useState } from 'react';
import { FaTrash } from 'react-icons/fa';
import { ResultMatrix } from './components/result-matrix';
import { SearchPanel } from './components/search-panel';
import { SelectionChips } from './components/selection-chips';
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

type ResultsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | {
      status: 'ok';
      selections: SelectionMeta[];
      matches: ReadonlyArray<PersonMatch | TitleMatch>;
    }
  | { status: 'error'; message: string };

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
  const [{ mode, picks }, setUrlState] = useUrlState();
  const [term, setTerm] = useState('');
  const [results, setResults] = useState<ResultsState>({ status: 'idle' });

  const searchState = useSearch(mode, term);
  const inFlight = useRef<AbortController | null>(null);
  /** Search results the user has clicked, so chips have a label before the server replies. */
  const known = useRef(new Map<number, SearchResult>());

  useEffect(() => {
    return () => inFlight.current?.abort();
  }, []);

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

  const addPick = useCallback(
    (result: SearchResult) => {
      known.current.set(result.id, result);
      setTerm('');

      if (picks.some((pick) => pick.id === result.id)) return;
      if (picks.length >= MAX_SELECTIONS) return;

      // The effect above re-runs the comparison; no local results bookkeeping needed.
      setUrlState({ mode, picks: [...picks, { id: result.id, kind: result.kind }] });
    },
    [mode, picks, setUrlState],
  );

  const removePick = useCallback(
    (selection: Selection) => {
      setUrlState({ mode, picks: picks.filter((pick) => pick.id !== selection.id) });
    },
    [mode, picks, setUrlState],
  );

  const changeMode = useCallback(
    (next: Mode) => {
      if (next === mode) return;
      setTerm('');
      setResults({ status: 'idle' });
      setUrlState({ mode: next, picks: [] });
    },
    [mode, setUrlState],
  );

  const clearAll = useCallback(() => {
    setTerm('');
    setResults({ status: 'idle' });
    setUrlState({ mode, picks: [] });
  }, [mode, setUrlState]);

  const chipMeta: SelectionMeta[] =
    results.status === 'ok'
      ? results.selections
      : picks.flatMap((pick) => {
          const found = known.current.get(pick.id);
          if (!found) return [];
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

  const ready = picks.length >= MIN_SELECTIONS;
  const copy = COPY[mode];

  return (
    <div className="flex min-h-screen flex-col">
      <header className="border-b border-line">
        <div className="mx-auto flex w-full max-w-3xl items-center justify-between gap-4 px-4 py-3">
          <a href="/" className="font-mono text-[13px] uppercase tracking-[0.2em] text-paper">
            same<span className="text-accent">.</span>actor
          </a>
          <ModeSwitch mode={mode} onChange={changeMode} />
        </div>
      </header>

      <main className="mx-auto w-full max-w-3xl flex-1 px-4 pt-10 pb-24">
        <h1 className="text-[clamp(1.5rem,4vw,2rem)] leading-tight font-normal tracking-tight text-paper">
          {copy.heading}
        </h1>
        <p className="mt-2 max-w-lg text-sm leading-relaxed text-dim">{copy.sub}</p>

        <div className="mt-8">
          <SearchPanel
            mode={mode}
            term={term}
            onTermChange={setTerm}
            state={searchState}
            onPick={addPick}
            canAdd={picks.length < MAX_SELECTIONS}
          />
        </div>

        <div className="mt-6 space-y-5">
          <SelectionChips selections={picks} meta={chipMeta} onRemove={removePick} />

          <div className="flex flex-wrap items-center gap-3">
            {picks.length > 0 ? (
              <Button variant="danger" onClick={clearAll}>
                <FaTrash size={10} aria-hidden="true" />
                Clear
              </Button>
            ) : null}

            {!ready ? (
              <span className="label">
                {picks.length === 0
                  ? 'Nothing selected yet'
                  : `${MIN_SELECTIONS - picks.length} more to compare`}
              </span>
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
                {results.matches.length > 1 ? <p className="label">most shared first</p> : null}
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
        <div className="mx-auto flex w-full max-w-3xl flex-wrap items-center justify-between gap-2 px-4 py-4">
          <p className="text-[11px] text-dim">© {new Date().getFullYear()} Richard Leek</p>
          <p className="text-[11px] text-dim">
            This product uses the TMDB API but is not endorsed or certified by TMDB.
          </p>
        </div>
      </footer>
    </div>
  );
}

/**
 * The picker island — the only component-driven part of the site.
 *
 * It owns the search box, the selected chips and the Share button. It does *not* render the
 * results: those come back from the server as HTML, so the table exists in exactly one
 * template and a shared link looks the same whether it was painted by the server or swapped in
 * after a click.
 *
 * Without JavaScript the page still shows the current selection and the results; only picking
 * needs the island, and the page says so.
 */

import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { MIN_TERM_LENGTH, useSearch } from '../hooks/use-search';
import type { Mode, SearchResult, Selection, SelectionMeta } from '../lib/types';
import { MAX_SELECTIONS } from '../lib/types';
import { toPath } from '../lib/url';
import { Poster } from './poster';
import { SelectionChips } from './selection-chips';
import { Button } from './ui';

export interface Bootstrap {
  readonly mode: Mode;
  /** The selection the server already rendered into the page. */
  readonly picks: readonly SelectionMeta[];
}

interface PickerProps {
  readonly bootstrap: Bootstrap;
}

function metaFrom(result: SearchResult): SelectionMeta {
  return {
    id: result.id,
    kind: result.kind,
    name: result.name,
    year: result.year,
    image: result.image,
  };
}

export function Picker({ bootstrap }: PickerProps) {
  const mode = bootstrap.mode;
  const [meta, setMeta] = useState<readonly SelectionMeta[]>(bootstrap.picks);
  const [term, setTerm] = useState('');
  const [highlight, setHighlight] = useState(0);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  const search = useSearch(mode, term);
  const results = search.status === 'ok' ? search.results : [];
  const full = meta.length >= MAX_SELECTIONS;

  const picks: Selection[] = meta.map((entry) => ({ id: entry.id, kind: entry.kind }));

  useEffect(() => setHighlight(0), [term]);

  /**
   * Rewrite the URL, then ask the server for the results region and swap it in.
   *
   * The fetch is a navigation the browser could have done on its own — same URL, same HTML —
   * so a failure just performs that navigation instead of leaving a stale page behind a new
   * address.
   */
  const commit = useCallback(
    async (next: readonly SelectionMeta[]) => {
      const nextPicks = next.map((entry) => ({ id: entry.id, kind: entry.kind }));
      const path = toPath(
        { mode, picks: nextPicks },
        (pick) => next.find((entry) => entry.id === pick.id)?.name ?? null,
      );

      setMeta(next);
      window.history.pushState(null, '', path);
      setBusy(true);

      try {
        const response = await fetch(path, {
          headers: { 'X-Fragment': 'results' },
          credentials: 'same-origin',
        });
        if (!response.ok) throw new Error(`fragment request failed: ${response.status}`);

        const html = await response.text();
        const target = document.getElementById('results');
        if (target === null) throw new Error('no results region');
        target.innerHTML = html;
      } catch {
        window.location.assign(path);
      } finally {
        setBusy(false);
      }
    },
    [mode],
  );

  const add = (result: SearchResult) => {
    if (full || meta.some((entry) => entry.id === result.id)) return;
    setTerm('');
    input.current?.focus();
    void commit([...meta, metaFrom(result)]);
  };

  const remove = (selection: Selection) => {
    void commit(meta.filter((entry) => entry.id !== selection.id));
  };

  const clear = () => {
    void commit([]);
  };

  const share = async () => {
    try {
      await navigator.clipboard.writeText(window.location.href);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // Clipboard access can be refused; selecting the address bar is a fair fallback.
      window.prompt('Copy this link', window.location.href);
    }
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape') {
      setTerm('');
      return;
    }
    if (results.length === 0) return;

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setHighlight((current) => Math.min(current + 1, results.length - 1));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setHighlight((current) => Math.max(current - 1, 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const choice = results[highlight];
      if (choice !== undefined) add(choice);
    }
  };

  const placeholder = mode === 'title' ? 'Search films and shows…' : 'Search actors and directors…';

  return (
    <div>
      <div className="relative">
        <input
          ref={input}
          type="search"
          value={term}
          disabled={full}
          placeholder={full ? 'Five is the maximum. Remove one to add another.' : placeholder}
          autocomplete="off"
          spellcheck={false}
          aria-label={placeholder}
          onInput={(event) => setTerm(event.currentTarget.value)}
          onKeyDown={onKeyDown}
          className="w-full rounded-md border border-line bg-raise px-3 py-2.5 text-sm text-paper placeholder:text-dim/70 focus:border-dim focus:outline-none disabled:opacity-60"
        />

        {term.trim().length >= MIN_TERM_LENGTH && search.status !== 'idle' ? (
          <ul
            id="picker-results"
            className="absolute z-30 mt-1 max-h-80 w-full overflow-y-auto rounded-md border border-line bg-raise-2 shadow-xl"
          >
            {search.status === 'loading' ? (
              <li className="px-3 py-2 text-xs text-dim">Searching…</li>
            ) : null}

            {search.status === 'error' ? (
              <li className="px-3 py-2 text-xs text-danger">{search.message}</li>
            ) : null}

            {search.status === 'ok' && results.length === 0 ? (
              <li className="px-3 py-2 text-xs text-dim">Nothing matched that.</li>
            ) : null}

            {results.map((result, index) => (
              <li key={`${result.kind}-${result.id}`}>
                <button
                  type="button"
                  onClick={() => add(result)}
                  onMouseEnter={() => setHighlight(index)}
                  className={`flex w-full items-center gap-2.5 px-2 py-1.5 text-left ${
                    index === highlight ? 'bg-raise' : ''
                  }`}
                >
                  <Poster
                    src={result.image}
                    alt=""
                    shape={result.kind === 'person' ? 'avatar' : 'poster'}
                    width={28}
                    height={result.kind === 'person' ? 28 : 40}
                  />
                  <span className="min-w-0">
                    <span className="block truncate text-[13px] text-paper">{result.name}</span>
                    <span className="tnum block font-mono text-[10px] text-dim">
                      {result.year ?? '—'}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        ) : null}
      </div>

      <div className="mt-4">
        <SelectionChips selections={picks} meta={meta} onRemove={remove} />
      </div>

      {picks.length > 0 ? (
        <div className="mt-3 flex items-center gap-2">
          <Button onClick={share} disabled={busy}>
            {copied ? 'Copied' : 'Share'}
          </Button>
          <Button variant="ghost" onClick={clear} disabled={busy}>
            Clear
          </Button>
          {busy ? <span className="text-xs text-dim">Comparing…</span> : null}
        </div>
      ) : null}
    </div>
  );
}

import { type KeyboardEvent as ReactKeyboardEvent, useEffect, useRef, useState } from 'react';
import { FaSearch, FaTimes } from 'react-icons/fa';
import { MIN_TERM_LENGTH, type SearchState } from '../hooks/use-search';
import type { Mode, SearchResult } from '../lib/types';
import { Poster } from './poster';

interface SearchPanelProps {
  mode: Mode;
  term: string;
  onTermChange: (term: string) => void;
  state: SearchState;
  onPick: (result: SearchResult) => void;
  /** False when the comparison is already at its maximum. */
  canAdd: boolean;
}

const PLACEHOLDER: Record<Mode, string> = {
  title: 'Search films and shows…',
  person: 'Search actors and directors…',
};

export function SearchPanel({ mode, term, onTermChange, state, onPick, canAdd }: SearchPanelProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  /** The term the user dismissed. Dismissal is per-term, so typing again reopens. */
  const [dismissedTerm, setDismissedTerm] = useState<string | null>(null);

  const results = state.status === 'ok' ? state.results : [];
  const open =
    dismissedTerm !== term && term.trim().length >= MIN_TERM_LENGTH && state.status !== 'idle';

  // "/" focuses search from anywhere, Escape leaves it.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || target?.isContentEditable === true) return;
      event.preventDefault();
      inputRef.current?.focus();
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  const focusOption = (index: number) => {
    const options = optionRefs.current;
    if (options.length === 0) return;
    const clamped = Math.max(0, Math.min(index, options.length - 1));
    options[clamped]?.focus();
  };

  const onInputKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown' && results.length > 0) {
      event.preventDefault();
      focusOption(0);
      return;
    }
    if (event.key === 'Enter' && results.length > 0 && open) {
      event.preventDefault();
      const first = results[0];
      if (first) onPick(first);
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      setDismissedTerm(term);
    }
  };

  const onOptionKeyDown = (index: number) => (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      focusOption(index + 1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      if (index === 0) inputRef.current?.focus();
      else focusOption(index - 1);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      setDismissedTerm(term);
      inputRef.current?.focus();
    }
  };

  const countMessage =
    state.status === 'loading'
      ? 'Searching'
      : state.status === 'ok'
        ? `${state.results.length} ${state.results.length === 1 ? 'result' : 'results'}`
        : '';

  return (
    <div className="relative">
      <div className="relative flex items-center">
        <FaSearch
          aria-hidden="true"
          className="pointer-events-none absolute left-3.5 text-dim"
          size={13}
        />
        <input
          ref={inputRef}
          type="search"
          value={term}
          onChange={(event) => onTermChange(event.target.value)}
          onKeyDown={onInputKeyDown}
          placeholder={PLACEHOLDER[mode]}
          aria-label={PLACEHOLDER[mode]}
          aria-describedby="search-hint"
          maxLength={100}
          autoComplete="off"
          spellCheck={false}
          // The visible label is the placeholder; the hint below carries the rest.
          className="w-full rounded-md border border-line bg-raise py-3 pr-10 pl-9 text-[15px] text-paper placeholder:text-dim/80 focus:border-line focus:bg-raise-2 [&::-webkit-search-cancel-button]:hidden"
        />
        <span
          aria-hidden="true"
          className="pointer-events-none absolute right-3 hidden rounded border border-line px-1.5 py-0.5 font-mono text-[10px] text-dim sm:block"
        >
          /
        </span>
      </div>

      <p id="search-hint" className="mt-1.5 label">
        {term.trim().length > 0 && term.trim().length < MIN_TERM_LENGTH
          ? `Type ${MIN_TERM_LENGTH - term.trim().length} more character${MIN_TERM_LENGTH - term.trim().length === 1 ? '' : 's'}`
          : canAdd
            ? mode === 'title'
              ? 'Add two to five films or shows'
              : 'Add two to five people'
            : 'That is the maximum of five'}
      </p>

      {/* Announced to screen readers without stealing focus. */}
      <span aria-live="polite" className="sr-only">
        {open ? countMessage : ''}
      </span>

      {open ? (
        <div className="absolute inset-x-0 top-full z-30 mt-2 max-h-[min(60vh,26rem)] overflow-y-auto rounded-md border border-line bg-raise shadow-[0_16px_48px_rgba(0,0,0,0.6)]">
          {state.status === 'loading' ? (
            <ul className="divide-y divide-line-soft">
              {[0, 1, 2].map((row) => (
                <li key={row} className="flex items-center gap-3 p-3">
                  <span className="h-[54px] w-9 shrink-0 animate-pulse rounded-[3px] bg-raise-2" />
                  <span className="flex-1 space-y-2">
                    <span className="block h-3 w-1/2 animate-pulse rounded bg-raise-2" />
                    <span className="block h-2.5 w-3/4 animate-pulse rounded bg-raise-2" />
                  </span>
                </li>
              ))}
            </ul>
          ) : null}

          {state.status === 'error' ? (
            <p className="p-4 text-sm text-danger">{state.message}</p>
          ) : null}

          {state.status === 'ok' && results.length === 0 ? (
            <p className="p-4 text-sm text-dim">
              Nothing matched “{state.term}”. Try a shorter search.
            </p>
          ) : null}

          {state.status === 'ok' && results.length > 0 ? (
            <ul className="divide-y divide-line-soft">
              {results.map((result, index) => (
                <li key={`${result.kind}-${result.id}`}>
                  <button
                    ref={(node) => {
                      optionRefs.current[index] = node;
                    }}
                    type="button"
                    onKeyDown={onOptionKeyDown(index)}
                    onClick={() => onPick(result)}
                    className="flex w-full items-start gap-3 p-3 text-left transition-colors hover:bg-raise-2 focus-visible:bg-raise-2"
                  >
                    <Poster
                      src={result.image}
                      alt=""
                      shape={result.kind === 'person' ? 'avatar' : 'poster'}
                      width={result.kind === 'person' ? 54 : 36}
                      height={54}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="flex items-baseline gap-2">
                        <span className="truncate text-sm font-medium text-paper">
                          {result.name}
                        </span>
                        {result.year !== null ? (
                          <span className="tnum shrink-0 font-mono text-xs text-dim">
                            {result.year}
                          </span>
                        ) : null}
                        <span className="ml-auto shrink-0 font-mono text-[10px] uppercase tracking-[0.1em] text-dim/70">
                          {result.kind === 'tv' ? 'TV' : result.kind === 'movie' ? 'Film' : 'Actor'}
                        </span>
                      </span>
                      {result.overview !== '' ? (
                        <span className="mt-0.5 line-clamp-2 block text-xs leading-relaxed text-dim">
                          {result.overview}
                        </span>
                      ) : null}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          ) : null}

          <div className="hidden border-t border-line-soft px-3 py-2 sm:block">
            <p className="label flex items-center gap-1.5">
              <FaTimes size={9} aria-hidden="true" /> esc to close · ↑ ↓ to move
            </p>
          </div>
        </div>
      ) : null}
    </div>
  );
}

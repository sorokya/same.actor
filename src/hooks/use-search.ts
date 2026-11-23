import { useEffect, useRef, useState } from 'preact/hooks';
import { describe, search } from '../lib/api';
import type { Mode, SearchResult } from '../lib/types';
import { useDebounced } from './use-debounced';

export const MIN_TERM_LENGTH = 3;

export type SearchState =
  | { readonly status: 'idle' }
  | { readonly status: 'loading'; readonly term: string }
  | { readonly status: 'ok'; readonly term: string; readonly results: SearchResult[] }
  | { readonly status: 'error'; readonly term: string; readonly message: string };

/**
 * Debounced search whose stale responses can never win.
 *
 * The old component keyed its effect on a debounced value but sent the *live* one, with no
 * abort and no sequence guard, so a slow response for an old term could overwrite a fast
 * response for the new one (review H3). Here the effect owns an AbortController and a
 * cancellation flag: when the term changes the in-flight request is aborted and any late
 * resolution is dropped.
 */
export function useSearch(mode: Mode, term: string): SearchState {
  const debounced = useDebounced(term.trim(), 300);
  const [state, setState] = useState<SearchState>({ status: 'idle' });

  // Keep a ref so the effect can tell a real state change from a re-render.
  const latest = useRef('');

  useEffect(() => {
    latest.current = debounced;

    if (debounced.length < MIN_TERM_LENGTH) {
      setState({ status: 'idle' });
      return;
    }

    const controller = new AbortController();
    let cancelled = false;

    setState({ status: 'loading', term: debounced });

    search(debounced, mode, controller.signal)
      .then((response) => {
        if (cancelled || latest.current !== debounced) return;
        setState({ status: 'ok', term: debounced, results: response.results });
      })
      .catch((error: unknown) => {
        if (cancelled || latest.current !== debounced) return;
        setState({ status: 'error', term: debounced, message: describe(error) });
      });

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [debounced, mode]);

  return state;
}

import { useCallback, useEffect, useState } from 'preact/hooks';
import { type NameFor, readFromLocation, toPath, type UrlState } from '../lib/url';

/**
 * Comparison state lives in the URL, so a result is shareable, survives a refresh, and the
 * back button walks through selections.
 *
 * Selection changes push a history entry; the slug upgrade replaces the current one, because
 * the ids-only and slug forms are the same page and shouldn't cost the user two "back"s.
 */
export function useUrlState() {
  const [state, setState] = useState<UrlState>(() =>
    readFromLocation(window.location.pathname, window.location.search),
  );

  useEffect(() => {
    const onPopState = () => {
      setState(readFromLocation(window.location.pathname, window.location.search));
    };
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  const navigate = useCallback((next: UrlState, nameFor?: NameFor) => {
    setState(next);

    const path = toPath(next, nameFor);
    const current = `${window.location.pathname}${window.location.search}`;
    if (path !== current) {
      window.history.pushState(null, '', path);
    }
  }, []);

  /** Rewrite the current entry in place — used to add slugs without growing history. */
  const replace = useCallback((state: UrlState, nameFor: NameFor) => {
    const path = toPath(state, nameFor);
    const current = `${window.location.pathname}${window.location.search}`;
    if (path !== current) {
      window.history.replaceState(null, '', path);
    }
  }, []);

  return [state, navigate, replace] as const;
}

export type { UrlState };

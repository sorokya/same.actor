import { useCallback, useEffect, useState } from 'react';
import { KINDS, type Kind, MAX_SELECTIONS, type Mode, type Selection } from '../lib/types';

/**
 * Comparison state lives in the URL, so a result is shareable and the back button works.
 *
 * `?mode=title&pick=movie:603,tv:1396`
 *
 * This replaces component state that vanished on refresh — there was no way to link someone
 * to a comparison before.
 */

export interface UrlState {
  mode: Mode;
  picks: Selection[];
}

function parseKind(value: string): Kind | null {
  return (KINDS as readonly string[]).includes(value) ? (value as Kind) : null;
}

function parsePicks(raw: string | null): Selection[] {
  if (!raw) return [];

  const picks: Selection[] = [];
  const seen = new Set<number>();

  for (const token of raw.split(',')) {
    const [kindPart, idPart] = token.split(':');
    if (!kindPart || !idPart) continue;

    const kind = parseKind(kindPart);
    const id = Number(idPart);
    if (!kind || !Number.isInteger(id) || id <= 0) continue;
    if (seen.has(id)) continue;

    seen.add(id);
    picks.push({ kind, id });
    if (picks.length === MAX_SELECTIONS) break;
  }

  return picks;
}

export function readFromLocation(search: string): UrlState {
  const params = new URLSearchParams(search);
  const mode: Mode = params.get('mode') === 'person' ? 'person' : 'title';
  const picks = parsePicks(params.get('pick')).filter((pick) =>
    mode === 'person' ? pick.kind === 'person' : pick.kind !== 'person',
  );

  return { mode, picks };
}

function serialize(state: UrlState): string {
  const params = new URLSearchParams();
  params.set('mode', state.mode);
  if (state.picks.length > 0) {
    params.set('pick', state.picks.map((pick) => `${pick.kind}:${pick.id}`).join(','));
  }
  return `?${params}`;
}

export function useUrlState() {
  const [state, setState] = useState<UrlState>(() => readFromLocation(window.location.search));

  useEffect(() => {
    const onPopState = () => setState(readFromLocation(window.location.search));
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  /** Mutate and push a history entry, so back undoes a selection. */
  const update = useCallback((next: UrlState) => {
    setState(next);
    const url = `${window.location.pathname}${serialize(next)}`;
    if (url !== `${window.location.pathname}${window.location.search}`) {
      window.history.pushState(null, '', url);
    }
  }, []);

  return [state, update] as const;
}

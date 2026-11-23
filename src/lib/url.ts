/**
 * Comparison URLs.
 *
 * `?mode=title&pick=movie%3A603` was the old form: `URLSearchParams` escapes the colon, so
 * every shared link carried `%3A` and `%2C` and read like machine output. This module
 * replaces it with a path that reads as prose:
 *
 *   /title/movie-603-the-matrix,movie-604-the-matrix-reloaded
 *   /person/6384-keanu-reeves,530-carrie-anne-moss
 *
 * The id is always present and comes first after its kind, so the slug is decoration: the
 * link stays correct if somebody trims it back to `/title/movie-603,movie-604`. The kind is
 * only written in `title` mode, where a selection can be a film or a show; in `person` mode
 * it would be noise.
 */

import { type Kind, MAX_SELECTIONS, type Mode, type Selection } from './types';

const KINDS: readonly Kind[] = ['movie', 'tv', 'person'];

export interface UrlState {
  readonly mode: Mode;
  readonly picks: Selection[];
}

/**
 * Lowercase, strip accents, and reduce to `[a-z0-9-]`.
 *
 * Names that are not Latin script reduce to an empty string; callers then omit the slug
 * rather than emitting a broken-looking `movie-603-`.
 */
export function slugify(name: string): string {
  return name
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+/, '')
    .replace(/-+$/, '')
    .slice(0, 48)
    .replace(/-+$/, '');
}

export function parsePicks(raw: string | undefined, mode: Mode): Selection[] {
  if (!raw) return [];

  const picks: Selection[] = [];
  const seen = new Set<number>();

  for (const segment of raw.split(',')) {
    if (picks.length === MAX_SELECTIONS) break;

    let rest = segment.trim();
    if (rest === '') continue;

    let kind: Kind = mode === 'person' ? 'person' : 'movie';

    if (mode === 'title') {
      const prefix = /^(movie|tv)-/.exec(rest);
      if (prefix === null) continue;
      kind = prefix[1] as Kind;
      rest = rest.slice(prefix[0].length);
    }

    // `603-the-matrix` and `603` are both valid; the slug is ignored.
    const idMatch = /^(\d{1,10})(?:-|$)/.exec(rest);
    if (idMatch === null) continue;

    const id = Number(idMatch[1]);
    if (!Number.isInteger(id) || id <= 0 || id > 2147483647) continue;
    if (seen.has(id)) continue;

    seen.add(id);
    picks.push({ kind, id });
  }

  return picks;
}

/** The legacy `?mode=&pick=kind:id,…` form, kept so pre-existing links still resolve. */
function parseLegacyQuery(search: string): UrlState | null {
  const params = new URLSearchParams(search);
  const raw = params.get('pick');
  if (raw === null) return null;

  const mode: Mode = params.get('mode') === 'person' ? 'person' : 'title';
  const picks: Selection[] = [];

  for (const token of raw.split(',')) {
    const [kindPart, idPart] = token.split(':');
    if (kindPart === undefined || idPart === undefined) continue;
    if (!(KINDS as readonly string[]).includes(kindPart)) continue;

    const id = Number(idPart);
    if (!Number.isInteger(id) || id <= 0) continue;

    picks.push({ kind: kindPart as Kind, id });
    if (picks.length === MAX_SELECTIONS) break;
  }

  return { mode, picks };
}

export function readFromLocation(pathname: string, search: string): UrlState {
  const segments = pathname.split('/').filter((part) => part !== '');
  const first = segments[0];

  if (first === 'title' || first === 'person') {
    const picks = parsePicks(segments[1], first);
    if (picks.length > 0) return { mode: first, picks };
    return { mode: first, picks: [] };
  }

  const legacy = parseLegacyQuery(search);
  if (legacy !== null) return legacy;

  return { mode: 'title', picks: [] };
}

export type NameFor = (pick: Selection) => string | null;

/**
 * Build the canonical path for a state.
 *
 * With no names available yet this emits `/title/movie-603,movie-604`; the app upgrades it
 * to the slug form once the API has told it what those ids are called.
 */
export function toPath(state: UrlState, nameFor?: NameFor): string {
  if (state.picks.length === 0) {
    return state.mode === 'person' ? '/person' : '/';
  }

  const segments = state.picks.map((pick) => {
    const name = nameFor?.(pick) ?? null;
    const slug = name === null ? '' : slugify(name);
    const kind = state.mode === 'title' ? `${pick.kind}-` : '';
    return slug === '' ? `${kind}${pick.id}` : `${kind}${pick.id}-${slug}`;
  });

  return `/${state.mode}/${segments.join(',')}`;
}

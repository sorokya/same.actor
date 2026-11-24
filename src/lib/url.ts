/**
 * Comparison URLs, built on the client.
 *
 * The server parses these (`api-rs/src/paths.rs`) and now owns that half: this module only
 * writes the path the island navigates to, so there is one parser and no chance of the two
 * disagreeing.
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

import type { Mode, Selection } from './types';

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

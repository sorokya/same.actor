/** Mirrors `api-rs/src/domain.rs`. If you change one, change both. */

export type Mode = 'title' | 'person';
export type Kind = 'movie' | 'tv' | 'person';

export const KINDS: readonly Kind[] = ['movie', 'tv', 'person'];
export const MAX_SELECTIONS = 5;
export const MIN_SELECTIONS = 2;

export interface Selection {
  readonly id: number;
  readonly kind: Kind;
}

export interface SearchResult {
  readonly id: number;
  readonly kind: Kind;
  readonly name: string;
  /** `null` when TMDB has no usable date. */
  readonly year: number | null;
  readonly overview: string;
  /** `null` when there is no artwork; the client owns the placeholder. */
  readonly image: string | null;
}

export interface SearchResponse {
  readonly results: SearchResult[];
}

export interface PersonCredit {
  readonly selection: number;
  readonly character: string[];
}

/** One person who appears in at least two of the selected titles. */
export interface PersonMatch {
  readonly id: number;
  readonly name: string;
  readonly image: string | null;
  readonly credits: PersonCredit[];
}

export interface TitleCredit {
  readonly selection: number;
  readonly character: string | null;
}

/** One title that features at least two of the selected people. */
export interface TitleMatch {
  readonly id: number;
  readonly kind: Kind;
  readonly name: string;
  readonly year: number | null;
  readonly image: string | null;
  readonly credits: TitleCredit[];
}

export interface SelectionMeta {
  readonly id: number;
  readonly kind: Kind;
  readonly name: string;
  readonly year: number | null;
  readonly image: string | null;
}

export type FindOutResponse =
  | {
      readonly mode: 'title';
      readonly selections: SelectionMeta[];
      readonly matches: PersonMatch[];
    }
  | {
      readonly mode: 'person';
      readonly selections: SelectionMeta[];
      readonly matches: TitleMatch[];
    };

export interface ApiErrorBody {
  readonly error: { readonly code: string; readonly message: string };
}

export type AnyMatch = PersonMatch | TitleMatch;

export function isTitle(kind: Kind): boolean {
  return kind === 'movie' || kind === 'tv';
}

/** The selection kind a given mode expects. */
export function kindForMode(mode: Mode): Kind {
  return mode === 'person' ? 'person' : 'movie';
}

export function selectionMatchesMode(selection: Selection, mode: Mode): boolean {
  return mode === 'person' ? selection.kind === 'person' : isTitle(selection.kind);
}

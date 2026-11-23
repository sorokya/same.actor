/**
 * One colour per selection.
 *
 * The point is not decoration: each selected title (or person) gets a hue that follows it
 * from its chip into its column in the results matrix, so a wide table can be read by colour
 * instead of re-reading headers. Five hues, five selections.
 *
 * Class strings are written out in full rather than composed, because Tailwind generates
 * utilities by scanning source text — `text-[${hue}]` produces nothing.
 */

export interface Tint {
  /** Accent text, e.g. the year in a column header. */
  readonly text: string;
  /** 2px bar along the top of a matrix column. */
  readonly bar: string;
  /** Low-opacity wash behind a column header. */
  readonly soft: string;
  /** Border for a chip. */
  readonly chip: string;
  /** Focus/selection ring. */
  readonly ring: string;
}

const TINTS = [
  {
    text: 'text-[#e8b14b]',
    bar: 'bg-[#e8b14b]',
    soft: 'bg-[#e8b14b]/8',
    chip: 'border-[#e8b14b]/45',
    ring: 'ring-[#e8b14b]/50',
  },
  {
    text: 'text-[#3ecfc0]',
    bar: 'bg-[#3ecfc0]',
    soft: 'bg-[#3ecfc0]/8',
    chip: 'border-[#3ecfc0]/45',
    ring: 'ring-[#3ecfc0]/50',
  },
  {
    text: 'text-[#f2799e]',
    bar: 'bg-[#f2799e]',
    soft: 'bg-[#f2799e]/8',
    chip: 'border-[#f2799e]/45',
    ring: 'ring-[#f2799e]/50',
  },
  {
    text: 'text-[#a996ff]',
    bar: 'bg-[#a996ff]',
    soft: 'bg-[#a996ff]/8',
    chip: 'border-[#a996ff]/45',
    ring: 'ring-[#a996ff]/50',
  },
  {
    text: 'text-[#b6d15c]',
    bar: 'bg-[#b6d15c]',
    soft: 'bg-[#b6d15c]/8',
    chip: 'border-[#b6d15c]/45',
    ring: 'ring-[#b6d15c]/50',
  },
] as const satisfies readonly Tint[];

export function tintFor(index: number): Tint {
  // `TINTS` is declared `as const`, so element 0 is a known-present `Tint` and this needs no
  // non-null assertion to satisfy noUncheckedIndexedAccess.
  const slot = ((index % TINTS.length) + TINTS.length) % TINTS.length;
  return TINTS[slot] ?? TINTS[0];
}

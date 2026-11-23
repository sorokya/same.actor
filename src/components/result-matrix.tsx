import type { Mode, PersonMatch, SelectionMeta, TitleMatch } from '../lib/types';
import { Poster } from './poster';

type AnyMatch = PersonMatch | TitleMatch;

interface ResultMatrixProps {
  mode: Mode;
  selections: readonly SelectionMeta[];
  matches: readonly AnyMatch[];
}

/**
 * The character(s) one match has in one selection.
 *
 * `character` is a list for people (a TV actor can hold several roles) and a single string
 * for titles. The old API emitted a comma-joined string for TV and a raw string for film, so
 * the client could not tell a character from a list (review M8).
 */
function charactersFor(match: AnyMatch, selectionId: number): string[] {
  const credit = match.credits.find((entry) => entry.selection === selectionId);
  if (!credit) return [];

  const value = credit.character;
  if (Array.isArray(value)) return value.filter((part) => part.trim() !== '');
  return value !== null && value.trim() !== '' ? [value] : [];
}

function isTitleMatch(match: AnyMatch): match is TitleMatch {
  return 'kind' in match;
}

function tmdbHref(kind: string, id: number): string {
  return `https://www.themoviedb.org/${kind}/${id}`;
}

function rowLabel(match: AnyMatch): {
  name: string;
  year: number | null;
  image: string | null;
  kind: string;
  id: number;
} {
  return isTitleMatch(match)
    ? { name: match.name, year: match.year, image: match.image, kind: match.kind, id: match.id }
    : { name: match.name, year: null, image: match.image, kind: 'person', id: match.id };
}

export function ResultMatrix({ mode, selections, matches }: ResultMatrixProps) {
  if (matches.length === 0) {
    return (
      <div className="rounded-md border border-line bg-raise p-8 text-center">
        <p className="text-sm text-paper">
          {mode === 'title' ? 'No shared cast.' : 'No shared titles.'}
        </p>
        <p className="mt-1 text-xs text-dim">
          Nothing links all of these. Try removing one, or picking a bigger production.
        </p>
      </div>
    );
  }

  const rowHeading = mode === 'title' ? 'Shared cast' : 'Shared titles';

  return (
    <>
      {/* Wide screens: a real table, with a header per selection. */}
      <div className="hidden overflow-x-auto rounded-md border border-line sm:block">
        <table className="w-full border-collapse text-sm">
          <caption className="sr-only">
            {rowHeading}, one column per selected {mode === 'title' ? 'title' : 'person'}
          </caption>

          <thead>
            <tr className="border-b border-line bg-raise">
              <th
                scope="col"
                className="sticky left-0 z-10 border-r border-line-soft bg-raise px-3 py-2 text-left align-bottom"
              >
                <span className="label">{rowHeading}</span>
              </th>
              {selections.map((selection) => (
                <th
                  key={selection.id}
                  scope="col"
                  className="min-w-[9rem] border-l border-line-soft px-3 py-2 text-left align-bottom font-normal"
                >
                  {/* Poster stacked above the label, not beside it: the label's left edge
                      then sits exactly above the cell text, so the column reads as one
                      block instead of two offset ones. */}
                  <a
                    href={tmdbHref(selection.kind, selection.id)}
                    target="_blank"
                    rel="noreferrer noopener"
                    className="block hover:underline"
                  >
                    <Poster
                      src={selection.image}
                      alt=""
                      shape={selection.kind === 'person' ? 'avatar' : 'poster'}
                      width={selection.kind === 'person' ? 26 : 20}
                      height={26}
                      className="mb-1.5"
                    />
                    <span className="block truncate text-xs text-paper">{selection.name}</span>
                    {selection.year !== null ? (
                      <span className="tnum block font-mono text-[11px] text-dim">
                        {selection.year}
                      </span>
                    ) : null}
                  </a>
                </th>
              ))}
            </tr>
          </thead>

          <tbody>
            {matches.map((match) => {
              const entity = rowLabel(match);

              return (
                <tr
                  key={entity.id}
                  className="border-b border-line-soft last:border-0 hover:bg-raise/60"
                >
                  <th
                    scope="row"
                    className="sticky left-0 z-10 border-r border-line-soft bg-ink px-3 py-2 text-left font-normal"
                  >
                    <a
                      href={tmdbHref(entity.kind, entity.id)}
                      target="_blank"
                      rel="noreferrer noopener"
                      className="flex items-center gap-2.5 hover:underline"
                    >
                      <Poster
                        src={entity.image}
                        alt=""
                        shape={mode === 'title' ? 'avatar' : 'poster'}
                        width={mode === 'title' ? 32 : 24}
                        height={36}
                      />
                      <span className="min-w-0">
                        <span className="block truncate text-[13px] text-paper">{entity.name}</span>
                        {entity.year !== null ? (
                          <span className="tnum block font-mono text-[10px] text-dim">
                            {entity.year}
                          </span>
                        ) : null}
                      </span>
                    </a>
                  </th>

                  {selections.map((selection) => {
                    const characters = charactersFor(match, selection.id);
                    return (
                      <td
                        key={selection.id}
                        className="border-l border-line-soft px-3 py-2 align-top"
                      >
                        {characters.length > 0 ? (
                          <span className="text-xs text-dim">{characters.join(', ')}</span>
                        ) : (
                          // An ambiguous blank cell reads as a rendering failure. A dash
                          // states "not credited here" without inventing words.
                          <span aria-hidden="true" className="text-xs text-dim/40">
                            —
                          </span>
                        )}
                        {characters.length === 0 ? (
                          <span className="sr-only">Not credited</span>
                        ) : null}
                      </td>
                    );
                  })}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {/* Narrow screens: the same data as cards. A horizontally scrolling table on a phone is
          technically responsive and practically unusable. */}
      <ul className="space-y-2 sm:hidden">
        {matches.map((match) => {
          const entity = rowLabel(match);

          return (
            <li key={entity.id} className="rounded-md border border-line bg-raise p-3">
              <a
                href={tmdbHref(entity.kind, entity.id)}
                target="_blank"
                rel="noreferrer noopener"
                className="flex items-center gap-2.5"
              >
                <Poster
                  src={entity.image}
                  alt=""
                  shape={mode === 'title' ? 'avatar' : 'poster'}
                  width={mode === 'title' ? 36 : 28}
                  height={44}
                />
                <span className="min-w-0">
                  {/* Two lines, not one: a truncated title on a card has no tooltip rescue. */}
                  <span className="line-clamp-2 text-sm text-paper">{entity.name}</span>
                  {entity.year !== null ? (
                    <span className="tnum block font-mono text-[10px] text-dim">{entity.year}</span>
                  ) : null}
                </span>
              </a>

              <dl className="mt-2 space-y-1 border-t border-line-soft pt-2">
                {selections.map((selection) => {
                  const characters = charactersFor(match, selection.id);
                  return (
                    <div key={selection.id} className="flex gap-2 text-xs">
                      <dt className="w-2/5 min-w-0 truncate text-dim">{selection.name}</dt>
                      {/* Roles wrap. A TV credit can be a sentence, and clipping it hides
                          exactly the detail the user came for. */}
                      <dd className="w-3/5 min-w-0 break-words text-paper">
                        {characters.length > 0 ? characters.join(', ') : '—'}
                      </dd>
                    </div>
                  );
                })}
              </dl>
            </li>
          );
        })}
      </ul>
    </>
  );
}

import { tintFor } from '../lib/palette';
import type { Selection, SelectionMeta } from '../lib/types';
import { CloseIcon } from './icons';
import { Poster } from './poster';

interface SelectionChipsProps {
  selections: readonly Selection[];
  /** Resolved labels, once the server has told us what these ids are. */
  meta: readonly SelectionMeta[];
  onRemove: (selection: Selection) => void;
}

function labelFor(selection: Selection, meta: readonly SelectionMeta[]): string | null {
  const found = meta.find((entry) => entry.id === selection.id);
  if (found === undefined) return null;
  return found.year === null ? found.name : `${found.name} (${found.year})`;
}

export function SelectionChips({ selections, meta, onRemove }: SelectionChipsProps) {
  if (selections.length === 0) return null;

  return (
    <div>
      <h2 className="label mb-2">Comparing {selections.length} of 5</h2>
      <ul className="flex flex-wrap gap-2">
        {selections.map((selection, index) => {
          const label = labelFor(selection, meta);
          const image = meta.find((entry) => entry.id === selection.id)?.image ?? null;
          // The chip carries the same hue as its column in the results matrix below.
          const tint = tintFor(index);

          return (
            <li
              key={selection.id}
              className={`inline-flex max-w-full items-center gap-2.5 overflow-hidden rounded border bg-raise py-1 pr-2 transition-colors hover:bg-raise-2 ${tint.chip}`}
            >
              <span aria-hidden="true" className={`h-9 w-[3px] shrink-0 ${tint.bar}`} />

              {/* The artwork opens the entity on TMDB. A link rather than part of the button:
                  nesting one interactive element inside another is invalid, and the name
                  beside it already does the removing. */}
              <a
                href={`https://www.themoviedb.org/${selection.kind}/${selection.id}`}
                target="_blank"
                rel="noreferrer noopener"
                aria-label={`Open ${label ?? `#${selection.id}`} on TMDB`}
                title={`Open ${label ?? `#${selection.id}`} on TMDB`}
                className="my-0.5 shrink-0 rounded-[3px] outline-offset-2 transition-shadow hover:ring-1 hover:ring-accent/50"
              >
                {/* Artwork is the point of a film comparison, so it gets enough size to be
                    recognisable rather than a 22px thumbnail. */}
                <Poster
                  src={image}
                  alt=""
                  shape={selection.kind === 'person' ? 'avatar' : 'poster'}
                  width={34}
                  height={selection.kind === 'person' ? 34 : 50}
                />
              </a>

              <button
                type="button"
                onClick={() => onRemove(selection)}
                title={label === null ? `Remove #${selection.id}` : `Remove ${label}`}
                className="group flex min-w-0 items-center gap-2 text-left"
              >
                <span className="max-w-[13rem] truncate text-[13px] text-paper">
                  {label ?? `#${selection.id}`}
                </span>
                <CloseIcon size={11} className="shrink-0 text-dim group-hover:text-danger" />
                <span className="sr-only">Remove</span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

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
            <li key={selection.id} className="contents">
              <button
                type="button"
                onClick={() => onRemove(selection)}
                title={label === null ? `Remove #${selection.id}` : `Remove ${label}`}
                className={`group inline-flex max-w-full items-center gap-2 overflow-hidden rounded border bg-raise py-1 pr-2 text-left transition-colors hover:bg-raise-2 ${tint.chip}`}
              >
                <span aria-hidden="true" className={`h-6 w-[3px] shrink-0 ${tint.bar}`} />
                <Poster
                  src={image}
                  alt=""
                  shape={selection.kind === 'person' ? 'avatar' : 'poster'}
                  width={22}
                  height={22}
                />
                <span className="truncate text-xs text-paper">{label ?? `#${selection.id}`}</span>
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

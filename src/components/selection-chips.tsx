import { FaTimes } from 'react-icons/fa';
import type { Selection, SelectionMeta } from '../lib/types';
import { Poster } from './poster';

interface SelectionChipsProps {
  selections: readonly Selection[];
  /** Resolved labels, once the server has told us what these ids are. */
  meta: readonly SelectionMeta[];
  onRemove: (selection: Selection) => void;
}

function labelFor(selection: Selection, meta: readonly SelectionMeta[]): string | null {
  const found = meta.find((entry) => entry.id === selection.id);
  if (!found) return null;
  return found.year === null ? found.name : `${found.name} (${found.year})`;
}

export function SelectionChips({ selections, meta, onRemove }: SelectionChipsProps) {
  if (selections.length === 0) return null;

  return (
    <div>
      <h2 className="label mb-2">Comparing {selections.length} of 5</h2>
      <ul className="flex flex-wrap gap-2">
        {selections.map((selection) => {
          const label = labelFor(selection, meta);
          const image = meta.find((entry) => entry.id === selection.id)?.image ?? null;

          return (
            <li key={selection.id} className="contents">
              <button
                type="button"
                onClick={() => onRemove(selection)}
                title={label === null ? `Remove #${selection.id}` : `Remove ${label}`}
                className="group inline-flex max-w-full items-center gap-2 rounded border border-line bg-raise py-1 pr-2 pl-1 text-left transition-colors hover:border-danger/60 hover:bg-raise-2"
              >
                <Poster
                  src={image}
                  alt=""
                  shape={selection.kind === 'person' ? 'avatar' : 'poster'}
                  width={22}
                  height={22}
                />
                <span className="truncate text-xs text-paper">{label ?? `#${selection.id}`}</span>
                <FaTimes
                  aria-hidden="true"
                  size={11}
                  className="shrink-0 text-dim group-hover:text-danger"
                />
                <span className="sr-only">Remove</span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

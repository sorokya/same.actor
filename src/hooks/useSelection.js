import { useCallback, useMemo, useState } from 'react';

export function useSelection({ setTerm, setSearchResults, setResults }) {
  const [selection, setSelection] = useState([]);

  const addSelection = useCallback(
    (item) => {
      if (selection.includes(item)) {
        return;
      }

      setSelection([...selection, item]);
      setTerm('');
      setSearchResults([]);
    },
    [selection],
  );

  const removeSelection = useCallback(
    (item) => {
      setSelection(selection.filter((i) => i !== item));
      setResults(null);
    },
    [selection],
  );

  const clearSelection = useCallback(() => {
    setSelection([]);
  }, [setSelection]);

  return { selection, addSelection, removeSelection, clearSelection };
}

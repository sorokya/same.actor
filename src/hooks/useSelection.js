import { useCallback, useMemo, useState } from "react";

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

  const mediaTypes = useMemo(() => {
    const types = [];

    selection.forEach((item) => {
      if (!types.includes(item.mediaType)) {
        types.push(item.mediaType);
      }
    });

    if (types.length === 0) {
      return ['person', 'movie', 'tv'];
    }

    if (types.length === 1) {
      switch (types[0]) {
        case 'movie':
          types.push('tv');
          break;
        case 'tv':
          types.push('movie');
          break;
      }
    }

    return types;
  }, [selection]);


  return { selection, mediaTypes, addSelection, removeSelection, clearSelection };
}

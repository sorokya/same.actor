import { useDebounce } from '@uidotdev/usehooks';
import { useCallback, useEffect, useMemo, useState } from 'react';

export function Search() {
  const [term, setTerm] = useState('');
  const [_isLoading, setIsLoading] = useState(false);
  const [results, setResults] = useState([]);
  const [finalResults, setFinalResults] = useState([]);
  const [selection, setSelection] = useState([]);
  const debouncedTerm = useDebounce(term, 300);

  const onChange = useCallback((e) => {
    setTerm(e.target.value);
  }, []);

  const addSelection = useCallback(
    (item) => {
      if (selection.includes(item)) {
        return;
      }

      setSelection([...selection, item]);
      setTerm('');
      setResults([]);
    },
    [selection],
  );

  const removeSelection = useCallback(
    (item) => {
      setSelection(selection.filter((i) => i !== item));
      setFinalResults([]);
    },
    [selection],
  );

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

  const findOut = useCallback(async () => {
    let results = [];
    setIsLoading(true);
    try {
      const response = await fetch('/api/findOut', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          selection: selection.map((item) => ({
            id: item.id,
            mediaType: item.mediaType,
          })),
        }),
      });

      const data = await response.json();
      results = data;
    } catch (e) {
      console.error(e);
    }

    setFinalResults(results);
    setIsLoading(false);
  }, [selection]);

  useEffect(() => {
    const search = async () => {
      let results = [];
      setIsLoading(true);
      if (debouncedTerm) {
        try {
          const response = await fetch(`/api/search?term=${debouncedTerm}`);
          const data = await response.json();
          results = data;
        } catch (e) {
          console.error(e);
        }
      }

      setIsLoading(false);
      setResults(results);
    };

    search();
  }, [debouncedTerm]);

  return (
    <div>
      <form className="flex gap-1 p-1">
        <input
          type="text"
          placeholder="Shrek (2001)"
          value={term}
          onChange={onChange}
          className="block w-full rounded-md border-0 p-1.5 text-gray-900 shadow-sm ring-1 ring-gray-300 ring-inset placeholder:text-gray-400 sm:text-sm sm:leading-6 focus:ring-2 focus:ring-indigo-400 focus:ring-inset"
        />
      </form>
      {selection?.length ? (
        <>
          <Selected items={selection} removeSelection={removeSelection} />
          {selection.length > 1 ? (
            <button
              onClick={findOut}
              className="p-1 ml-1 text-2xl bg-indigo-400 rounded hover:bg-indigo-500 hover:cursor-pointer"
            >
              Find out!!!
            </button>
          ) : null}
        </>
      ) : null}
      {results?.length ? (
        <Dropdown
          items={results.filter((item) => mediaTypes.includes(item.mediaType))}
          addSelection={addSelection}
        />
      ) : null}
      {finalResults?.length ? (
        <Results selection={selection} results={finalResults} />
      ) : null}
    </div>
  );
}

function Dropdown({ items, addSelection }) {
  return (
    <ul className="mt-2">
      {items.map((item) => (
        <li
          key={item.id}
          className="flex p-1 hover:cursor-pointer even:bg-slate-50 hover:bg-slate-100"
          onClick={() => addSelection(item)}
        >
          <img
            src={item.imgUrl}
            alt={`${item.name} poster`}
            className="inline-block h-20"
          />
          <div className="ml-1 inline-block">
            <span className="text-xl">
              {item.name}
              {item.releaseYear ? ` (${item.releaseYear})` : null}
            </span>
            <p className="text-gray-700">
              {item.overview.length > 150
                ? `${item.overview.substring(0, 150)}...`
                : item.overview}
            </p>
          </div>
        </li>
      ))}
    </ul>
  );
}

function Selected({ items, removeSelection }) {
  return (
    <ul className="mb-2">
      {items.map((item) => (
        <li
          key={item.id}
          onClick={() => removeSelection(item)}
          className="inline-block mx-1 p-1 bg-slate-300 rounded hover:cursor-pointer relative"
        >
          <div>
            {item.name}
            {item.releaseYear ? ` (${item.releaseYear})` : null}
          </div>
        </li>
      ))}
    </ul>
  );
}

/*
 [
 {id: 123, name: 'Bryan Cranston', roles: [
 'Walter White', null, 'Something'
 ]}
 ]
*/

function Results({ selection, results }) {
  if (results.length === 0) {
    return '<h1>No matches</h1>';
  }

  return (
    <table className="w-full text-sm text-left rtl:text-right text-gray-500 dark:text-gray-400">
      <thead className="text-xs text-gray-700 uppercase bg-gray-50 dark:bg-gray-700 dark:text-gray-400">
        <tr>
          <th className="px-6 py-3">{' '}</th>
          {selection.map((item) => (
            <th key={item.id} className="px-6 py-3">
              {item.name}
              {item.releaseYear ? ` (${item.releaseYear})` : null}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {results.map((result) => (
          <tr key={result.id}>
            <td className="px-6 py-4">{result.name}</td>
            {selection.map((item) => {
              const match = result.media.find((m) => m.id === item.id);
              if (match) {
                return (<td key={item.id} className="px-6 py-4">{match.character}</td>);
              } else {
                return (<td key={item.id} className="px-6 py-4" />);
              }
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

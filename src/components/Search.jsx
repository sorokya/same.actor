import { useDebounce } from '@uidotdev/usehooks';
import { useCallback, useEffect } from 'react';
import { search } from '../utils/search';

export function Search({ setSearchResults, setLoading, term, setTerm, mediaType }) {
  const debouncedTerm = useDebounce(term, 300);

  const onChange = useCallback((e) => {
    setTerm(e.target.value);
  }, []);


  useEffect(() => {
    const doSearch = async () => {
      let results = [];
      setLoading(true);
      if (debouncedTerm) {
        results = await search({ term, mediaType })
      }

      setLoading(false);
      setSearchResults(results);
    };

    doSearch();
  }, [debouncedTerm, mediaType]);

  return (
    <input
      type="text"
      placeholder={mediaType === 'film' ? 'Enter a movie or tv show name' : 'Enter an actor name'}
      maxLength={100}
      value={term}
      onChange={onChange}
      className="block w-full h-10 rounded-md border-0 p-1.5 text-gray-900 shadow-sm ring-1 ring-gray-300 ring-inset placeholder:text-gray-400 sm:text-sm sm:leading-6 focus:ring-2 focus:ring-indigo-400 focus:ring-inset"
    />
  );
}

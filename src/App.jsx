import { Header } from './components/Header';
import { Search } from './components/Search';
import { Results } from './components/Results';
import { Dropdown } from './components/Dropdown';
import { useSelection } from './hooks/useSelection';
import { getMatches } from './utils/getMatches';
import { useCallback, useMemo, useState } from 'react';
import { Selected } from './components/Selected';
import { Button } from './components/Button';

import { FaTrash, FaSearch } from 'react-icons/fa'
import { Footer } from './components/Footer';
import { MediaType } from './components/MediaType';

function App() {
  const [term, setTerm] = useState('');
  const [searchResults, setSearchResults] = useState([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [results, setResults] = useState(null);
  const [resultsLoading, setResultsLoading] = useState(false);
  const [mediaType, setMediaType] = useState('film');

  const { selection, addSelection, removeSelection, clearSelection } = useSelection({ setTerm, setSearchResults, setResults });

  const loading = useMemo(() => searchLoading || resultsLoading, [searchLoading, resultsLoading]);

  const getResults = useCallback(async () => {
    if (selection.length < 2 || loading) {
      return;
    }

    setResultsLoading(true);
    const matches = await getMatches({ selection, mediaType });
    if (matches) {
      setResults(matches);
    }
    setResultsLoading(false);
  }, [selection, loading]);

  return (
    <>
      <div className="flex flex-col min-h-screen">
        <div className="w-5/6 m-auto flex-grow">
          <Header />
          <div className="w-full md:w-2/3 m-auto">
            <MediaType mediaType={mediaType} setMediaType={(value) => { setMediaType(value); setResults(null); setSearchResults([]); clearSelection(); }} />
            <Search setSearchResults={setSearchResults} setLoading={setSearchLoading} mediaType={mediaType} term={term} setTerm={setTerm} />
          </div>
          <div className="w-full md:w-2/3 mx-auto mt-2">
            <Selected selection={selection} removeSelection={removeSelection} />
          </div>
          <div className="w-full text-center mt-2 mb-4">
            <Button onClick={getResults} disabled={selection.length < 2} title={selection.length < 2 ? 'Select at least 2 items' : null}>
              <FaSearch className="inline" />
              {' '}
              Find out!
            </Button>
            <Button onClick={() => { setTerm(''); setResults(null); setSearchResults([]); clearSelection(); }} variant={"danger"}>
              <FaTrash className="inline" />
              {' '}
              Clear
            </Button>
          </div>

          <Dropdown items={searchResults} addSelection={addSelection} />
          <Results selection={selection} results={results} mediaType={mediaType} />
        </div>
        <Footer />
      </div>
    </>
  );
}

export default App;

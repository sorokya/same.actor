export function Results({ selection, results }) {
  if (selection.length === 0 || !results) {
    return null;
  }

  if (results.length === 0) {
    return <h1 className="text-center text-2xl">😞 No matches found</h1>;
  }

  return (
    <table className="w-full text-sm text-left text-slate-100 mb-40">
      <thead className="text-md font-bold uppercase bg-slate-600 border-b">
        <tr>
          <th className="px-6 py-3">{' '}</th>
          {selection.map((item) => (
            <th key={item.id} className="px-6 py-3">
              <a href={`https://www.themoviedb.org/${item.mediaType}/${item.id}`} target="_blank">
                {item.name}
                {item.releaseYear ? ` (${item.releaseYear})` : null}
              </a>
            </th>
          ))}
        </tr>
      </thead>
      <tbody className="bg-slate-700">
        {results.map((result) => (
          <tr key={result.id} className="border-b">
            <td className="px-6 py-4">
              <a href={`https://www.themoviedb.org/person/${result.id}`} target="_blank">
                <img
                  src={result.imgUrl}
                  alt={`${result.name} poster`}
                  className="inline-block h-20 mr-1"
                />
                {result.name}
              </a>
            </td>

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

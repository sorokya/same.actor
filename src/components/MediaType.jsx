import { useCallback } from "react";

export function MediaType({ mediaType, setMediaType }) {
  const onChange = useCallback((e) => {
    setMediaType(e.target.value);
  }, [setMediaType]);

  return (
    <div className="flex flex-row gap-4">
      <label>
        <input type="radio" value="film" checked={mediaType === 'film'} onChange={onChange} />
        {' '}
        Movies &amp; TV Shows
      </label>
      <label>
        <input type="radio" value="actor" checked={mediaType === 'actor'} onChange={onChange} />
        {' '}
        Actors
      </label>
    </div>
  );
}

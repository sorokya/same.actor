/**
 * What people are searching for right now.
 *
 * Deliberately quiet: it is a hint for a first-time visitor who does not know what to type, not
 * a leaderboard. Terms come from the server already filtered — anything searched fewer than
 * three times in the last day is not published, so this cannot expose one person's query.
 */

import { useEffect, useState } from 'preact/hooks';
import { trending } from '../lib/api';

interface Term {
  term: string;
  searches: number;
}

export function TrendingStrip({ onPick }: { onPick?: (term: string) => void }) {
  const [terms, setTerms] = useState<Term[]>([]);

  useEffect(() => {
    const controller = new AbortController();

    trending(controller.signal).then(
      (response) => setTerms(response.terms),
      () => {
        // A missing strip is not worth telling anyone about.
      },
    );

    return () => controller.abort();
  }, []);

  if (terms.length === 0) return null;

  return (
    <div className="mt-4">
      <p className="label mb-2">What people are looking for — tap one to start</p>
      <ul className="flex flex-wrap gap-1.5">
        {terms.map((entry) => (
          <li key={entry.term}>
            {onPick ? (
              <button
                type="button"
                onClick={() => onPick(entry.term)}
                className="rounded border border-line px-2 py-1 text-xs text-dim transition-colors hover:border-line-soft hover:text-paper"
              >
                {entry.term}
              </button>
            ) : (
              <span className="rounded border border-line px-2 py-1 text-xs text-dim">
                {entry.term}
              </span>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

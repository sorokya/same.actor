import { useEffect, useState } from 'preact/hooks';

/** Debounce a value. Replaces `@uidotdev/usehooks` for the one hook this app used. */
export function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(timer);
  }, [value, delayMs]);

  return debounced;
}

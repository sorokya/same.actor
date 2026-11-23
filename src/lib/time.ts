/**
 * Times, in the way a person reads them.
 *
 * A refresh timestamp rendered as `11/23/2026, 6:39:54 PM` reads like debug output. "Refreshed
 * 2 hours ago" is the thing the reader actually wants to know, and the exact moment is still
 * available in a `title` for anyone who needs it.
 */

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** `just now`, `12 minutes ago`, `3 hours ago`, `2 days ago`, then a date. */
export function relativeTime(unix: number, now: number = Date.now() / 1000): string {
  const seconds = Math.max(0, Math.round(now - unix));

  if (seconds < 45) return 'just now';
  if (seconds < HOUR) {
    const minutes = Math.round(seconds / MINUTE);
    return `${minutes} minute${minutes === 1 ? '' : 's'} ago`;
  }
  if (seconds < DAY) {
    const hours = Math.round(seconds / HOUR);
    return `${hours} hour${hours === 1 ? '' : 's'} ago`;
  }
  if (seconds < 7 * DAY) {
    const days = Math.round(seconds / DAY);
    return `${days} day${days === 1 ? '' : 's'} ago`;
  }

  return new Date(unix * 1000).toLocaleDateString();
}

/** The exact moment, for a `title` attribute beside a relative time. */
export function absoluteTime(unix: number): string {
  return new Date(unix * 1000).toLocaleString();
}

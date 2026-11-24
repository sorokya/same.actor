/**
 * Navigating without a full page load.
 *
 * Every page is rendered by the server, so a "navigation" is: fetch the next page, swap the
 * parts that differ, and put the URL in history. That keeps the site working when this script
 * is missing or fails — every link is still a link, and every failure falls back to asking the
 * browser to do what it was going to do anyway.
 *
 * The swap replaces `[data-swap]`, the header (which carries the active tab) and the title, and
 * then dispatches an event so an island on the new page can mount itself.
 */

export const SWAPPED = 'sameactor:swapped';

const SWAP = '[data-swap]';

/** Only same-origin GETs to pages we render are ours to intercept. */
export function isInternalPage(url: URL, current: URL): boolean {
  if (url.origin !== current.origin) return false;
  if (url.pathname.startsWith('/api/') || url.pathname.startsWith('/og/')) return false;
  if (url.pathname.startsWith('/admin')) return false; // the dashboard stays script-free
  if (url.pathname.startsWith('/assets/')) return false;
  // A file with an extension is a download, not a page.
  const last = url.pathname.split('/').pop() ?? '';
  return !last.includes('.');
}

function replace(selector: string, next: Document): boolean {
  const current = document.querySelector(selector);
  const incoming = next.querySelector(selector);

  if (current === null || incoming === null) return false;

  current.replaceWith(document.importNode(incoming, true));
  return true;
}

/**
 * Take the new document apart and put it in this one.
 *
 * Returns false when the incoming page is not the same shape as the current one, which is the
 * signal to let the browser load it normally.
 */
export function swapDocument(next: Document): boolean {
  if (!replace(SWAP, next)) return false;
  // The header is outside the swap region because it is identical on every page except for
  // one attribute; replacing it is cheaper than reconciling that attribute by hand.
  replace('header[data-chrome]', next);

  if (next.title !== '') document.title = next.title;

  // A keyboard or screen-reader user has no other way to know the page changed: the document
  // did not reload, so nothing announced itself. Focusing the new main is the cheap version of
  // that announcement, and it must not scroll — the navigation decides where we land.
  const main = document.querySelector(SWAP);
  if (main instanceof HTMLElement) {
    main.setAttribute('tabindex', '-1');
    main.focus({ preventScroll: true });
  }

  document.dispatchEvent(new CustomEvent(SWAPPED));
  return true;
}

/** Fetch a page and swap it in. Falls back to a real navigation on any surprise. */
export async function navigate(url: string, options: { push?: boolean } = {}): Promise<void> {
  const push = options.push ?? true;

  try {
    const response = await fetch(url, {
      headers: { accept: 'text/html' },
      credentials: 'same-origin',
    });

    if (!response.ok || !(response.headers.get('content-type') ?? '').includes('text/html')) {
      throw new Error(`not a page: ${response.status}`);
    }

    const html = await response.text();
    const next = new DOMParser().parseFromString(html, 'text/html');

    if (!swapDocument(next)) throw new Error('the page has a different shape');

    if (push) {
      window.history.pushState(null, '', url);
      window.scrollTo(0, 0);
    }
    // Going back, the browser restores the scroll itself — but only if we leave it alone.
  } catch {
    // Whatever went wrong, the browser can do this correctly.
    if (push) window.location.assign(url);
    else window.location.reload();
  }
}

/** Should this click be left to the browser? */
export function isPlainClick(event: MouseEvent): boolean {
  return (
    event.button === 0 &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.shiftKey &&
    !event.altKey &&
    !event.defaultPrevented
  );
}

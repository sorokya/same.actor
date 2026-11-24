/**
 * Progressive enhancement for navigation, loaded on every rendered page.
 *
 * Links stay links: a click that we can serve (left button, no modifiers, same origin, one of
 * our page routes) is fetched and swapped in instead of reloading the document. Back and
 * forward do the same, so the URL in the address bar never disagrees with the page.
 *
 * Nothing here is required for the site to work. If this file is absent, broken, blocked, or
 * the response is not a page, the browser navigates the way it always did.
 */
import { isInternalPage, isPlainClick, navigate } from './lib/nav';

const current = (): URL => new URL(window.location.href);

document.addEventListener('click', (event) => {
  if (!isPlainClick(event)) return;

  const anchor = (event.target as Element | null)?.closest?.('a[href]');
  if (!(anchor instanceof HTMLAnchorElement)) return;
  if (anchor.target !== '' && anchor.target !== '_self') return;
  if (anchor.hasAttribute('download')) return;
  // Anything with an explicit fragment is a scroll, not a navigation.
  if (anchor.href.includes('#')) return;

  const url = new URL(anchor.href);
  if (!isInternalPage(url, current())) return;
  if (url.pathname === window.location.pathname && url.search === window.location.search) return;

  event.preventDefault();
  void navigate(url.toString());
});

window.addEventListener('popstate', () => {
  // No push: the entry is already in history, this is the browser moving through it.
  void navigate(window.location.href, { push: false });
});

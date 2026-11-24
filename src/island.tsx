/**
 * The island entry point.
 *
 * The server renders the page, including the current selection and the results. This mounts on
 * top of that markup to make picking interactive — one element, on the pages that have a
 * picker, and nothing at all on the browse pages or the dashboard.
 *
 * Mounting is a named function because a boosted navigation replaces the markup underneath:
 * when the document is swapped, whatever was mounted is gone, and the new page may have a
 * picker with a different selection. `boost.ts` announces each swap; this listens.
 */

import { render } from 'preact';
import { type Bootstrap, Picker } from './components/picker';
import { SWAPPED } from './lib/nav';
import './index.css';

function mount(): void {
  const target = document.getElementById('picker');
  if (target === null) return; // A browse page: nothing to enhance.

  // The bootstrap rides in a `data-` attribute: the browser un-escapes the attribute before we
  // see it, so a title containing a quote parses exactly like any other.
  let bootstrap: Bootstrap = { mode: 'title', picks: [] };
  try {
    const raw = target.dataset.state;
    if (raw !== undefined && raw !== '') bootstrap = JSON.parse(raw) as Bootstrap;
  } catch {
    // A malformed state means the page falls back to a picker with nothing selected, which is
    // still a working page rather than a blank one.
  }

  // The server rendered a fallback here: the selection as links, for the page with no script.
  // `render` mounts into an empty container, so without this the fallback stays underneath the
  // island for the life of the page — still naming the selection the *document* was built with,
  // after the island has changed it. That is why Clear looked like it did nothing.
  target.replaceChildren();
  render(<Picker bootstrap={bootstrap} />, target);
}

mount();

document.addEventListener(SWAPPED, () => {
  // Preact renders into the element it was given; the fetched page brought a fresh one.
  mount();
});

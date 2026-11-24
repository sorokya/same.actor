/**
 * The island entry point.
 *
 * The server renders the page, including the current selection and the results. This mounts on
 * top of that markup to make picking interactive — one element, on the pages that have a
 * picker, and nothing at all on the browse pages or the dashboard.
 */

import { render } from 'preact';
import { type Bootstrap, Picker } from './components/picker';
import './index.css';

const mount = document.getElementById('picker');
if (mount !== null) {
  // The bootstrap rides in a `data-` attribute: the browser un-escapes the attribute before we
  // see it, so a title containing a quote parses exactly like any other.
  let bootstrap: Bootstrap = { mode: 'title', picks: [] };
  try {
    const raw = mount.dataset.state;
    if (raw !== undefined && raw !== '') bootstrap = JSON.parse(raw) as Bootstrap;
  } catch {
    // A malformed state means the page falls back to a picker with nothing selected, which is
    // still a working page rather than a blank one.
  }

  render(<Picker bootstrap={bootstrap} />, mount);
}

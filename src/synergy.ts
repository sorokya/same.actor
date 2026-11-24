/**
 * The synergy island: hover highlighting, pinning, and filter swaps on the browse pages.
 *
 * Plain DOM, no framework. The matrix is already rendered by the server and the only things that
 * change on interaction are classes and one region of HTML — a component tree would be a second
 * copy of markup that already exists, and a bundle three times the size for the privilege.
 *
 * Everything here is enhancement. Without this file:
 *
 *   - the chips and the cameo switch are links, and clicking one loads the page with the filter
 *     in the URL;
 *   - the panel is reachable by editing `?actor=` / `?title=` in the address bar;
 *   - hover does nothing, which loses nothing a reader needed.
 *
 * With it, a filter change fetches only the matrix region and swaps it, so the page keeps its
 * scroll position and its focus, and a click on a row pins that actor without a reload.
 */

/** The region this island owns and the header the server answers it with. */
const FRAGMENT = 'synergy';

interface Matrix {
  readonly root: HTMLElement;
  readonly rows: HTMLElement[];
  readonly dots: HTMLElement[];
  readonly columns: HTMLElement[];
}

let current: Matrix | null = null;

function read(root: HTMLElement): Matrix {
  const all = <T extends Element>(selector: string): T[] =>
    Array.from(root.querySelectorAll<T>(selector));

  return {
    root,
    rows: all<HTMLElement>('.sync-row'),
    dots: all<HTMLElement>('.sync-dot'),
    columns: all<HTMLElement>('.sync-col'),
  };
}

function titles(row: HTMLElement): string[] {
  return (row.dataset.titles ?? '').split(/\s+/).filter((id) => id !== '');
}

/** Drop every highlight. Called on leave, on blur, and before applying a new one. */
function clear(matrix: Matrix): void {
  for (const row of matrix.rows) row.classList.remove('off');
  for (const dot of matrix.dots) dot.classList.remove('off');
  for (const column of matrix.columns) column.classList.remove('on');
}

/**
 * Light up one title's actors and dim the rest.
 *
 * Column *and* row: hovering the poster highlights the people in that title, which is the
 * question a poster asks. The dimming matters more than the highlight — with twenty columns the
 * eye needs the rest to recede before a run is visible at all.
 */
function focusColumn(matrix: Matrix, id: string): void {
  clear(matrix);

  for (const column of matrix.columns) {
    column.classList.toggle('on', column.dataset.col === id);
  }

  for (const row of matrix.rows) {
    row.classList.toggle('off', !titles(row).includes(id));
  }
}

/** Light up one actor's run and the titles in it. */
function focusRow(matrix: Matrix, row: HTMLElement): void {
  clear(matrix);

  const ids = new Set(titles(row));
  row.classList.add('on');

  for (const dot of matrix.dots) {
    dot.classList.toggle('off', !ids.has(dot.dataset.col ?? ''));
  }

  for (const column of matrix.columns) {
    column.classList.toggle('on', ids.has(column.dataset.col ?? ''));
  }
}

/**
 * Swap the matrix region in after a filter or pin change.
 *
 * `history.pushState` for a filter (it is a place worth going back to), `replaceState` for a pin
 * (it is the same page, one detail of it selected). On any surprise the link does what links do.
 */
async function swap(href: string, push: boolean): Promise<void> {
  const response = await fetch(href, {
    headers: { 'X-Fragment': FRAGMENT },
    credentials: 'same-origin',
  });

  if (!response.ok) throw new Error(`fragment request failed: ${response.status}`);

  const html = await response.text();
  const root = document.querySelector<HTMLElement>('[data-synergy]');
  if (root === null) throw new Error('no matrix to replace');

  // The server answers a fragment with the section and nothing else; anything larger means the
  // request was not understood, and replacing the page with it would be worse than navigating.
  if (!html.includes(`data-${FRAGMENT}`)) throw new Error('not a fragment');

  root.outerHTML = html;

  if (push) window.history.pushState(null, '', href);
  else window.history.replaceState(null, '', href);

  mount();
}

/** The current URL with one query parameter set, or removed when it is already set to that. */
function toggleParam(name: string, value: string): string {
  const url = new URL(window.location.href);
  if (url.searchParams.get(name) === value) url.searchParams.delete(name);
  else url.searchParams.set(name, value);

  return `${url.pathname}${url.search}`;
}

function mount(): void {
  const root = document.querySelector<HTMLElement>('[data-synergy]');
  if (root === null) {
    current = null;
    return;
  }

  current = read(root);
}

const active = (): Matrix | null => current;

document.addEventListener('pointerover', (event) => {
  const matrix = active();
  if (matrix === null) return;

  const target = event.target as Element | null;
  if (!(target instanceof Element) || !matrix.root.contains(target)) return;

  const column = target.closest<HTMLElement>('.sync-col');
  if (column !== null && column.dataset.col !== undefined) {
    focusColumn(matrix, column.dataset.col);
    return;
  }

  const row = target.closest<HTMLElement>('.sync-row');
  if (row !== null) focusRow(matrix, row);
});

// Hover is a mouse gesture; focus is the keyboard's version of the same question, so it gets the
// same answer. Tab through the matrix and the runs light up in turn.
document.addEventListener('focusin', (event) => {
  const matrix = active();
  if (matrix === null) return;

  const target = event.target;
  if (!(target instanceof Element) || !matrix.root.contains(target)) return;

  const row = target.closest<HTMLElement>('.sync-row');
  if (row !== null) focusRow(matrix, row);
});

document.addEventListener('pointerout', (event) => {
  const matrix = active();
  if (matrix === null) return;

  const target = event.target as Element | null;
  const leaving = target?.closest?.('.sync-row, .sync-col');
  if (leaving === null || leaving === undefined) return;

  const next = event.relatedTarget;
  if (next instanceof Node && leaving.contains(next)) return;

  clear(matrix);
});

/**
 * A click on a row pins that actor; a click on a name still opens TMDB.
 *
 * The name is the part of the row that is a link, and it says where it goes. Everything else in
 * the row is the selection, so one element carries both without nesting one interactive thing
 * inside another.
 */
document.addEventListener('click', (event) => {
  const target = event.target as Element | null;
  if (target?.closest('a') !== null && target?.closest('a') !== undefined) return;

  const row = target?.closest?.('.sync-row');
  if (!(row instanceof HTMLElement) || row.dataset.actor === undefined) return;
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return;

  event.preventDefault();
  void swap(toggleParam('actor', row.dataset.actor), false);
});

/** A filter chip is a link; the island makes it a swap. */
document.addEventListener('click', (event) => {
  const target = event.target as Element | null;
  const chip = target?.closest<HTMLAnchorElement>('a[data-chip]');
  if (chip === null || chip === undefined) return;
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return;

  event.preventDefault();
  void swap(chip.getAttribute('href') ?? '', true);
});

/**
 * The boost script swaps whole pages on Back and Forward. When it does, the matrix underneath is
 * a new element, and this island's cached references would point at dead nodes.
 */
window.addEventListener('popstate', () => {
  window.setTimeout(mount, 100);
});

mount();

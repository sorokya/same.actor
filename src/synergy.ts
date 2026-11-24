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
 *
 * The same file serves the co-star grid: there the hover names the two actors a square is about,
 * which is the one thing the server-rendered picture cannot say on its own.
 */

/** The region this island owns and the header the server answers it with. */
const FRAGMENT = 'synergy';

/** Announced by the link booster after it swaps a page in. */
const SWAPPED = 'sameactor:swapped';

interface Matrix {
  readonly root: HTMLElement;
  readonly rows: HTMLElement[];
  readonly dots: HTMLElement[];
  readonly columns: HTMLElement[];
  /** The pairs grid's actor labels and squares, empty on the matrix. */
  readonly names: HTMLElement[];
  readonly pairs: HTMLElement[];
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
    names: all<HTMLElement>('.pair-name'),
    pairs: all<HTMLElement>('[data-pair-a]'),
  };
}

function titles(row: HTMLElement): string[] {
  return (row.dataset.titles ?? '').split(/\s+/).filter((id) => id !== '');
}

/** Drop every highlight. Called on leave, on blur, and before applying a new one. */
function clear(matrix: Matrix): void {
  for (const row of matrix.rows) row.classList.remove('off', 'on');
  for (const dot of matrix.dots) dot.classList.remove('off');
  for (const column of matrix.columns) column.classList.remove('on');
  for (const name of matrix.names) name.classList.remove('off', 'on');
}

/**
 * Name both actors of a square.
 *
 * The grid has no header row: a column is an actor whose name appears on the diagonal's row, so
 * the only actor on screen with the square is the one whose row you are already on. Lighting up
 * both labels is what makes a square mean a pair rather than a cell.
 */
function focusPair(matrix: Matrix, one: string, other: string): void {
  clear(matrix);

  for (const name of matrix.names) {
    const id = name.dataset.actor ?? '';
    const involved = id === one || id === other;
    name.classList.toggle('on', involved);
    name.classList.toggle('off', !involved);
  }
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

  const cell = target.closest<HTMLElement>('[data-pair-a]');
  if (cell !== null) {
    focusPair(matrix, cell.dataset.pairA ?? '', cell.dataset.pairB ?? '');
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

  const cell = target.closest<HTMLElement>('[data-pair-a]');
  if (cell !== null) {
    focusPair(matrix, cell.dataset.pairA ?? '', cell.dataset.pairB ?? '');
    return;
  }

  const row = target.closest<HTMLElement>('.sync-row');
  if (row !== null) focusRow(matrix, row);
});

document.addEventListener('pointerout', (event) => {
  const matrix = active();
  if (matrix === null) return;

  const target = event.target as Element | null;
  const leaving = target?.closest?.('.sync-row, .sync-col, [data-pair-a]');
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
 * Any page swap replaces the matrix, and cached references to it would point at dead nodes: the
 * delegation below asks `current`, which is rebuilt here. Back and forward count too.
 */
document.addEventListener(SWAPPED, () => window.setTimeout(mount, 0));
window.addEventListener('popstate', () => window.setTimeout(mount, 100));

mount();

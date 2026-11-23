/**
 * The site's tabs: what you are comparing, then the lists you can browse.
 *
 * All of them are links, including `Titles` and `People`. That is a change from the radio group
 * this replaced: switching mode already cleared the selection, so a link did the same thing —
 * and a link works before any JavaScript has run, which is the direction the front end is
 * heading. The strip scrolls horizontally on a narrow screen rather than wrapping into a second
 * row that would push the page down.
 */

import { useEffect, useRef } from 'preact/hooks';
import { LISTS } from '../lib/lists';
import type { Mode } from '../lib/types';

interface SiteNavProps {
  /** The current mode, when this surface is the comparison tool. */
  mode?: Mode;
  /** The current list slug, when this surface is a discovery page. */
  slug?: string;
}

function Tab({
  href,
  active,
  children,
}: {
  href: string;
  active: boolean;
  children: preact.ComponentChildren;
}) {
  return (
    <a
      href={href}
      aria-current={active ? 'page' : undefined}
      className={`shrink-0 whitespace-nowrap rounded-full px-2.5 py-1 font-mono text-[11px] uppercase tracking-[0.12em] transition-colors ${
        active ? 'bg-paper text-ink' : 'text-dim hover:bg-raise hover:text-paper'
      }`}
    >
      {children}
    </a>
  );
}

export function SiteNav({ mode, slug }: SiteNavProps) {
  const strip = useRef<HTMLElement>(null);

  // On a phone the strip scrolls, and the tab you are on can be past the right edge — landing on
  // "On air" with no sign of which list is open is worse than a little scroll. Only the strip is
  // scrolled, never the page.
  useEffect(() => {
    const nav = strip.current;
    const active = nav?.querySelector<HTMLElement>('[aria-current="page"]');
    if (!nav || !active) return;
    const offset = active.offsetLeft - 8;
    if (offset > 0) nav.scrollLeft = offset;
  }, [slug, mode]);

  // No `justify-end`: with a horizontally scrollable strip it anchors the content to the right,
  // so a phone opens with the *last* tabs visible and "Titles" scrolled out of reach.
  return (
    <nav
      ref={strip}
      aria-label="Sections"
      className="-mx-1 flex max-w-full items-center gap-0.5 overflow-x-auto px-1 py-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
    >
      <Tab href="/" active={mode === 'title'}>
        Titles
      </Tab>
      <Tab href="/?mode=person" active={mode === 'person'}>
        People
      </Tab>

      <span aria-hidden="true" className="mx-1.5 h-4 w-px shrink-0 bg-line" />

      {LISTS.map((list) => (
        <Tab key={list.slug} href={`/discover/${list.slug}`} active={slug === list.slug}>
          {list.label}
        </Tab>
      ))}
    </nav>
  );
}

/**
 * The site chrome: sticky, hairlined, wordmark on the left, whatever navigation the surface needs
 * on the right.
 *
 * Extracted because three surfaces had grown their own copy and drifted — the discovery pages had
 * a different wordmark size and no sticky behaviour, the dashboard another. One component means
 * one header.
 */

import type { ComponentChildren } from 'preact';

interface SiteHeaderProps {
  children: ComponentChildren;
  /** Must match the page's own container, or the wordmark will not line up with the content. */
  width?: string;
}

export function SiteHeader({ children, width = 'max-w-3xl' }: SiteHeaderProps) {
  return (
    <header className="sticky top-0 z-20 border-b border-line bg-ink/85 backdrop-blur">
      {/* Below `sm` the navigation gets its own row: five tabs beside the wordmark do not fit on
          a phone, and a scroll strip clips the last label, which reads as broken rather than
          scrollable. Given the full width they fit on one line. */}
      <div
        className={`mx-auto flex w-full ${width} flex-col gap-2 px-4 py-2.5 sm:flex-row sm:items-center sm:justify-between sm:gap-4 sm:py-3`}
      >
        <a
          href="/"
          className="shrink-0 font-mono text-[13px] uppercase tracking-[0.2em] text-paper transition-colors hover:text-accent"
        >
          same<span className="text-accent">.</span>actor
        </a>

        <div className="min-w-0">{children}</div>
      </div>
    </header>
  );
}

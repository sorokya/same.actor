/**
 * The three browse sections.
 *
 * Shared by the header tabs and the discovery page, so a section can never be linked from the
 * navigation and then rejected by the page.
 *
 * The second axis — all time versus trending, cinemas versus at home — is deliberately not here:
 * the API returns the filters a section actually has for the chosen kind, and a section with one
 * option would otherwise render a control that does nothing.
 */

export type SectionKey = 'top' | 'new' | 'upcoming';

export interface SiteSection {
  readonly key: SectionKey;
  /** Short enough for a one-line tab strip: the wordmark plus five tabs at 390px. */
  readonly label: string;
  readonly heading: string;
  readonly blurb: string;
}

export const SECTIONS: readonly SiteSection[] = [
  {
    key: 'top',
    label: 'Top',
    heading: 'Top',
    blurb: 'Ranked by how much they have been watched, and the actors who keep turning up.',
  },
  {
    key: 'new',
    label: 'New',
    heading: 'New',
    blurb: 'Out now, and the casts they share.',
  },
  {
    key: 'upcoming',
    label: 'Upcoming',
    heading: 'Upcoming',
    blurb: 'Not out yet. The same actors, in things nobody has seen.',
  },
];

export function sectionByKey(key: string): SiteSection | undefined {
  return SECTIONS.find((section) => section.key === key);
}

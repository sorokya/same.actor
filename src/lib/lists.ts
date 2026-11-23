/**
 * The list pages, in one place.
 *
 * Shared by the header tabs and the discovery page itself, so a slug can never be linked from the
 * navigation and then rejected by the page.
 */

export interface SiteList {
  readonly slug: string;
  /**
   * Short enough to sit in a one-line tab strip with the two tool tabs. The full name is
   * `heading`, which the page itself uses — "Top films" and "Coming soon" do not fit, and a
   * tab strip whose last label is clipped reads as broken rather than scrollable.
   */
  readonly label: string;
  readonly heading: string;
  readonly blurb: string;
}

export const LISTS: readonly SiteList[] = [
  {
    slug: 'top-movies',
    label: 'Films',
    heading: 'Top films',
    blurb: 'What everyone is watching, and the actors who keep turning up in it.',
  },
  {
    slug: 'top-tv',
    label: 'Shows',
    heading: 'Top shows',
    blurb: 'The most-watched series right now, and the faces shared between them.',
  },
  {
    slug: 'new-movies',
    label: 'Cinemas',
    heading: 'In cinemas',
    blurb: 'Showing this week, and the casts they have in common.',
  },
  {
    slug: 'upcoming',
    label: 'Upcoming',
    heading: 'Coming soon',
    blurb: 'Not out yet. The same actors, in films nobody has seen.',
  },
  {
    slug: 'on-the-air',
    label: 'On air',
    heading: 'On the air',
    blurb: 'Series with an episode this week.',
  },
];

export function listBySlug(slug: string): SiteList | undefined {
  return LISTS.find((list) => list.slug === slug);
}

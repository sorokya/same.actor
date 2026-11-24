//! `/discover/{section}` — the browse pages.
//!
//! Rendered from the stored snapshot, so the page is complete in the first response: the poster
//! shelf, the synergy rows, both control groups. There is no script on these pages at all — the
//! controls are links, and the long tail of actors is a `<details>` element.

use askama::Template;

use crate::discovery::{Item, Section, SynergyRow, TitleRef, Variant};
use crate::domain::Kind;

use super::{absolute_time, relative_time, Head};

/// One option in a control group, rendered as a link so it works without JavaScript.
pub struct Choice {
    pub value: String,
    pub label: String,
    pub href: String,
    pub active: bool,
}

pub struct Poster {
    pub href: String,
    pub image: String,
    pub name: String,
    /// Pre-formatted: a template should not be deciding how to round a rating.
    pub meta: String,
    pub rank: usize,
}

pub struct TitleLink {
    pub href: String,
    pub name: String,
}

pub struct Card {
    pub href: String,
    pub name: String,
    pub image: String,
    pub count: usize,
    pub titles: Vec<TitleLink>,
}

pub struct OtherSection {
    pub heading: String,
    pub href: String,
}

/// Everything a browse page renders, in one value.
///
/// Grouped because a page needs all of it or none of it, and eight positional arguments is a
/// call site where the compiler cannot tell `kind` from `section` if they get swapped.
pub struct Snapshot<'a> {
    pub section: Section,
    pub kind: Kind,
    pub variant: &'a Variant,
    pub refreshed_ts: i64,
    pub items: &'a [Item],
    pub synergy: &'a [SynergyRow],
}

#[derive(Template)]
#[template(path = "pages/discover.html")]
pub struct DiscoverPage {
    pub head: Head,
    pub section: String,
    pub heading: String,
    pub blurb: String,
    pub kinds: Vec<Choice>,
    pub filters: Vec<Choice>,
    pub filter_group: String,
    pub refreshed_relative: String,
    pub refreshed_absolute: String,
    pub item_count: usize,
    pub items: Vec<Poster>,
    /// The first screenful of actor cards.
    pub rows: Vec<Card>,
    /// The rest, behind a `<details>`: 250 cards in one list is a page nobody scrolls.
    pub more_rows: Vec<Card>,
    pub more_count: usize,
    pub in_every: usize,
    pub others: Vec<OtherSection>,
}

/// How many actor cards before the rest fold away.
const VISIBLE: usize = 24;

fn kind_key(kind: Kind) -> &'static str {
    match kind {
        Kind::Movie => "movie",
        Kind::Tv => "tv",
        Kind::Person => "person",
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Movie => "Films",
        Kind::Tv => "Shows",
        Kind::Person => "People",
    }
}

pub fn title_href(kind: Kind, id: u32) -> String {
    format!("https://www.themoviedb.org/{}/{id}", kind_key(kind))
}

fn poster(item: &Item, kind: Kind, rank: usize) -> Poster {
    let mut meta = match item.year {
        Some(year) => year.to_string(),
        None => "—".to_owned(),
    };

    if let Some(rating) = item.rating {
        meta.push_str(&format!(" · {rating:.1}"));
    }

    Poster {
        href: title_href(kind, item.id),
        image: item.poster.clone().unwrap_or_default(),
        name: item.name.clone(),
        meta,
        rank,
    }
}

fn card(row: &SynergyRow) -> Card {
    Card {
        href: format!("https://www.themoviedb.org/person/{}", row.person_id),
        name: row.name.clone(),
        image: row.image.clone().unwrap_or_default(),
        count: row.count,
        titles: row
            .titles
            .iter()
            .map(|title: &TitleRef| TitleLink {
                href: title_href(title.kind, title.id),
                name: title.name.clone(),
            })
            .collect(),
    }
}

impl DiscoverPage {
    /// Build the page from a loaded snapshot.
    pub fn new(head: Head, snapshot: &Snapshot<'_>, now: i64, other_sections: &[Section]) -> Self {
        let Snapshot {
            section,
            kind,
            variant,
            refreshed_ts,
            items,
            synergy,
        } = *snapshot;

        let href = |kind: Kind, filter: &str| {
            let mut url = format!("/discover/{}?kind={}", section.key(), kind_key(kind));
            if !filter.is_empty() {
                url.push_str(&format!("&filter={filter}"));
            }
            url
        };

        let kinds = [Kind::Movie, Kind::Tv]
            .into_iter()
            .map(|value| Choice {
                value: kind_key(value).to_owned(),
                label: kind_label(value).to_owned(),
                // Switching the kind resets the filter: "At home" means nothing for shows.
                href: href(value, ""),
                active: value == kind,
            })
            .collect();

        let filters = crate::discovery::variants(section, kind)
            .filter(|candidate| !candidate.filter.is_empty())
            .map(|candidate| Choice {
                value: candidate.filter.to_owned(),
                label: candidate.filter_label.to_owned(),
                href: href(kind, candidate.filter),
                active: candidate.filter == variant.filter,
            })
            .collect();

        let cards: Vec<Card> = synergy.iter().map(card).collect();
        let (rows, more_rows) = if cards.len() > VISIBLE {
            let mut cards = cards;
            let rest = cards.split_off(VISIBLE);
            (cards, rest)
        } else {
            (cards, Vec::new())
        };

        let in_every = synergy
            .iter()
            .filter(|row| row.count == items.len())
            .count();

        Self {
            head,
            section: section.key().to_owned(),
            heading: format!("{} {}", section.heading(), kind_label(kind).to_lowercase()),
            blurb: section.blurb().to_owned(),
            kinds,
            filters,
            filter_group: section.filter_group().to_owned(),
            refreshed_relative: relative_time(refreshed_ts, now),
            refreshed_absolute: absolute_time(refreshed_ts),
            item_count: items.len(),
            items: items
                .iter()
                .enumerate()
                .map(|(index, item)| poster(item, kind, index + 1))
                .collect(),
            rows,
            more_count: synergy.len(),
            more_rows,
            in_every,
            others: other_sections
                .iter()
                .filter(|other| **other != section)
                .map(|other| OtherSection {
                    heading: other.heading().to_owned(),
                    href: format!("/discover/{}", other.key()),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32, name: &str, year: Option<u16>, rating: Option<f64>) -> Item {
        Item {
            id,
            name: name.to_owned(),
            year,
            poster: Some(format!("/{id}.jpg")),
            overview: None,
            rating,
        }
    }

    fn row(person_id: u32, name: &str, titles: &[(u32, &str)], count: usize) -> SynergyRow {
        SynergyRow {
            person_id,
            name: name.to_owned(),
            image: None,
            count,
            titles: titles
                .iter()
                .map(|(id, name)| TitleRef {
                    id: *id,
                    kind: Kind::Movie,
                    name: (*name).to_owned(),
                })
                .collect(),
        }
    }

    fn build(items: &[Item], rows: &[SynergyRow]) -> DiscoverPage {
        let variant = crate::discovery::by_id("top:movie:all-time").expect("variant");
        DiscoverPage::new(
            Head::new("Top films", "desc", "top"),
            &Snapshot {
                section: Section::Top,
                kind: Kind::Movie,
                variant,
                refreshed_ts: 1_795_500_000,
                items,
                synergy: rows,
            },
            1_795_500_000,
            &Section::ALL,
        )
    }

    #[test]
    fn the_two_kinds_and_the_sections_filters_are_both_offered() {
        let page = build(&[item(603, "The Matrix", Some(1999), Some(8.2))], &[]);

        let kinds: Vec<&str> = page.kinds.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(kinds, ["movie", "tv"]);
        assert!(page.kinds[0].active, "the requested kind is marked");

        let filters: Vec<&str> = page.filters.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(filters, ["all-time", "trending"]);
        assert_eq!(page.filter_group, "Ranked by");
    }

    /// A control's href has to be the page it is offering, or clicking it is a no-op.
    #[test]
    fn every_control_links_to_its_own_page() {
        let page = build(&[], &[]);

        for choice in page.kinds.iter().chain(page.filters.iter()) {
            assert!(
                choice.href.starts_with("/discover/top?kind="),
                "{}",
                choice.href
            );
        }

        assert_eq!(page.kinds[1].href, "/discover/top?kind=tv");
        assert_eq!(
            page.filters[1].href,
            "/discover/top?kind=movie&filter=trending"
        );
    }

    #[test]
    fn ratings_and_years_are_pre_formatted() {
        let page = build(&[item(603, "The Matrix", Some(1999), Some(8.25))], &[]);
        assert_eq!(page.items[0].meta, "1999 · 8.2");

        let page = build(&[item(605, "Undated", None, None)], &[]);
        assert_eq!(page.items[0].meta, "—", "no year and no rating is a dash");
    }

    /// Titles inside a card carry their own kind, so a show in a show list links as a show.
    #[test]
    fn card_titles_link_to_the_right_tmdb_kind() {
        let page = build(
            &[item(603, "The Matrix", Some(1999), None)],
            &[row(6384, "Keanu Reeves", &[(603, "The Matrix")], 2)],
        );

        assert_eq!(
            page.rows[0].titles[0].href,
            "https://www.themoviedb.org/movie/603"
        );
        assert_eq!(page.rows[0].href, "https://www.themoviedb.org/person/6384");
    }

    #[test]
    fn the_tail_of_the_table_folds_behind_details() {
        let items = vec![item(1, "One", None, None), item(2, "Two", None, None)];
        let rows: Vec<SynergyRow> = (0..30)
            .map(|id| row(id, "Someone", &[(1, "One"), (2, "Two")], 2))
            .collect();

        let page = build(&items, &rows);
        assert_eq!(page.rows.len(), VISIBLE);
        assert_eq!(page.more_rows.len(), 30 - VISIBLE);
        assert_eq!(page.more_count, 30);
    }

    /// "4 appear in every title" is only true if the count matches the list length.
    #[test]
    fn only_people_in_every_title_are_counted_there() {
        let items = vec![item(1, "One", None, None), item(2, "Two", None, None)];
        let rows = vec![
            row(1, "In Both", &[(1, "One"), (2, "Two")], 2),
            row(2, "In One", &[(1, "One")], 1),
        ];

        let page = build(&items, &rows);
        assert_eq!(page.in_every, 1);
    }
}

//! `/discover/{section}` — the browse pages.
//!
//! Rendered from the stored snapshot, so the page is complete in the first response: the poster
//! shelf, the synergy rows, both control groups. There is no script on these pages at all — the
//! controls are links, and the long tail of actors is a `<details>` element.

use askama::Template;

use std::collections::HashMap;

use crate::discovery::{Item, Section, SynergyRow, Variant};
use crate::domain::Kind;
use crate::store::franchises::Placement;
use crate::synergy::{self, Filter};

use super::{absolute_time, relative_time, Head};

/// One option in a control group, rendered as a link so it works without JavaScript.
pub struct Choice {
    pub value: String,
    pub label: String,
    pub href: String,
    pub active: bool,
}

/// One franchise's run of columns, for the matrix header.
#[derive(Clone)]
pub struct GroupView {
    pub name: String,
    pub colour: &'static str,
    /// How many columns it spans.
    pub span: usize,
}

/// A title, as a column of the matrix.
#[derive(Clone)]
pub struct ColumnView {
    pub id: u32,
    pub href: String,
    pub name: String,
    pub poster: String,
    /// 1-based grid column; the actor names take the first one.
    pub position: usize,
    pub colour: &'static str,
    pub group_name: String,
    pub pinned: bool,
    /// "1999 · 8.2", or the date when the list is about one. The matrix has no room for it on
    /// the header, so it rides in the tooltip: the year is not worth a column, but losing it
    /// would be losing information the old poster grid had.
    pub meta: String,
}

/// An actor, as a row.
#[derive(Clone)]
pub struct RowView {
    pub id: u32,
    pub href: String,
    pub name: String,
    pub image: String,
    /// Grid columns this actor's dots occupy, 1-based.
    pub at: Vec<usize>,
    /// The run line, as the two grid lines it spans.
    pub run_start: usize,
    pub run_end: usize,
    pub has_run: bool,
    pub colour: &'static str,
    /// Ticks: "603,604", so the island can light up a row without a payload.
    pub data: String,
    pub pinned: bool,
}

/// A franchise the reader can switch off.
#[derive(Clone)]
pub struct ChipView {
    pub name: String,
    pub colour: &'static str,
    /// Whether its columns are currently shown.
    pub shown: bool,
    /// The same page with this franchise flipped.
    pub href: String,
}

#[derive(Clone)]
pub struct PanelEntry {
    pub href: String,
    pub name: String,
    pub image: String,
    pub detail: String,
    pub colour: &'static str,
}

/// The side panel: bridges by default, or whatever the URL pinned.
#[derive(Clone)]
pub struct PanelView {
    /// Whether the panel is worth drawing at all. Bridges with nothing in them is a box that
    /// says "no" — the matrix should take the space instead.
    pub show: bool,
    pub heading: String,
    pub note: String,
    pub entries: Vec<PanelEntry>,
    pub related_heading: String,
    pub related: Vec<PanelEntry>,
    pub reset: String,
}

/// The synergy block on its own, for the island to swap in after a filter change.
#[derive(Template)]
#[template(path = "fragments/synergy.html")]
pub struct SynergyFragment {
    pub synergy: SynergyView,
}

/// What the URL asked to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Bridges,
    Actor(u32),
    Title(u32),
}

/// Everything the matrix needs that is not in the stored list.
pub struct Derived {
    pub placements: HashMap<(String, u32), Placement>,
    pub overrides: HashMap<(String, u32, u32), bool>,
    pub filter: Filter,
    pub focus: Focus,
}

#[derive(Clone)]
pub struct SynergyView {
    pub groups: Vec<GroupView>,
    pub columns: Vec<ColumnView>,
    pub rows: Vec<RowView>,
    pub chips: Vec<ChipView>,
    pub cameo_hidden: bool,
    pub cameo_href: String,
    /// Actors the row cap left out.
    pub hidden_actors: usize,
    pub panel: PanelView,
    pub empty: String,
    /// The page with every synergy filter dropped, for the empty state's reset link.
    pub reset: String,
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
    pub synergy: SynergyView,
}

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

/// The same page with one synergy filter flipped.
///
/// `base` already carries the section's kind and ranking, so this only has to append the synergy
/// parameters, and it drops the ones that would be at their default: a URL that says
/// `cameos=1&hide=` is a URL with two lies in it.
fn with_filters(base: &str, filter: &Filter, cameos: bool) -> String {
    let mut url = base.to_owned();
    let mut separator = if url.contains('?') { '&' } else { '?' };

    let mut hidden: Vec<&str> = filter.hide.iter().map(String::as_str).collect();
    hidden.sort_unstable();

    if !hidden.is_empty() {
        url.push(separator);
        separator = '&';
        url.push_str("hide=");
        url.push_str(&hidden.join(","));
    }

    if cameos {
        url.push(separator);
        url.push_str("cameos=0");
    }

    url
}

/// What a column's tooltip says: the date the list is about, or the year and rating.
fn column_meta(column: &synergy::Column) -> String {
    if let Some(date) = column.date.as_deref().and_then(crate::views::format_date) {
        return date;
    }

    let mut meta = match column.year {
        Some(year) => year.to_string(),
        None => "—".to_owned(),
    };

    if let Some(rating) = column.rating {
        meta.push_str(&format!(" · {rating:.1}"));
    }

    meta
}

/// Build the matrix view: the columns, the rows, the chips, and the panel the URL asked for.
fn synergy_view(
    kind: Kind,
    matrix: &synergy::Matrix,
    derived: &Derived,
    base: &str,
) -> SynergyView {
    // A header row that says "Other" over every column is answering a question nobody asked:
    // when nothing belongs to a curated franchise there is no grouping to announce, so the row
    // is dropped entirely.
    let any_franchise = matrix.groups.iter().any(|group| !group.slug.is_empty());

    let groups: Vec<GroupView> = if any_franchise {
        matrix
            .groups
            .iter()
            .map(|group| GroupView {
                name: group.name.clone(),
                colour: group.colour,
                span: group.span,
            })
            .collect()
    } else {
        Vec::new()
    };

    let columns: Vec<ColumnView> = matrix
        .columns
        .iter()
        .map(|column| ColumnView {
            id: column.id,
            href: title_href(column.kind, column.id),
            name: column.name.clone(),
            poster: column.poster.clone().unwrap_or_default(),
            position: column.position,
            colour: matrix.groups[column.group].colour,
            group_name: matrix.groups[column.group].name.clone(),
            pinned: derived.focus == Focus::Title(column.id),
            meta: column_meta(column),
        })
        .collect();

    let rows: Vec<RowView> = matrix
        .actors
        .iter()
        .map(|actor| {
            let first = actor.titles.first().copied().unwrap_or(0);
            let last = actor.titles.last().copied().unwrap_or(0);
            let colour = matrix
                .columns
                .get(first)
                .map(|column| matrix.groups[column.group].colour)
                .unwrap_or(synergy::UNGROUPED);

            // Space separated: CSS `~=` and `classList`-free matching want word boundaries, and
            // the island splits on whitespace.
            let data = actor
                .titles
                .iter()
                .filter_map(|index| matrix.columns.get(*index))
                .map(|column| column.id.to_string())
                .collect::<Vec<_>>()
                .join(" ");

            RowView {
                id: actor.person_id,
                href: format!("https://www.themoviedb.org/person/{}", actor.person_id),
                name: actor.name.clone(),
                image: actor.image.clone().unwrap_or_default(),
                at: actor.titles.iter().map(|index| index + 2).collect(),
                run_start: first + 2,
                run_end: last + 3,
                has_run: actor.titles.len() > 1,
                colour,
                data,
                pinned: derived.focus == Focus::Actor(actor.person_id),
            }
        })
        .collect();

    let chips: Vec<ChipView> = matrix
        .franchises
        .iter()
        .filter(|group| !group.slug.is_empty())
        .map(|group| {
            let mut flipped = derived.filter.clone();
            if !flipped.hide.remove(&group.slug) {
                flipped.hide.insert(group.slug.clone());
            }

            ChipView {
                name: group.name.clone(),
                colour: group.colour,
                shown: !derived.filter.hides(&group.slug),
                href: with_filters(base, &flipped, derived.filter.hide_cameos),
            }
        })
        .collect();

    let cameo_href = with_filters(base, &derived.filter, !derived.filter.hide_cameos);

    let panel = panel_view(kind, matrix, derived, base);

    let empty = if matrix.columns.is_empty() {
        "Every title in this list is hidden by the franchise filter.".to_owned()
    } else if matrix.actors.is_empty() {
        "Nobody appears in two of the titles left.".to_owned()
    } else {
        String::new()
    };

    SynergyView {
        groups,
        columns,
        rows,
        chips,
        cameo_hidden: derived.filter.hide_cameos,
        cameo_href,
        hidden_actors: matrix.hidden_actors,
        panel,
        empty,
        reset: base.to_owned(),
    }
}

/// The side panel: a pinned actor or title when the URL names one, bridges otherwise.
fn panel_view(kind: Kind, matrix: &synergy::Matrix, derived: &Derived, base: &str) -> PanelView {
    let person = |person_id: u32| format!("https://www.themoviedb.org/person/{person_id}");

    let detail = |titles: usize| match titles {
        1 => "1 title".to_owned(),
        count => format!("{count} titles"),
    };

    match derived.focus {
        Focus::Actor(person_id) => {
            let their_titles: Vec<PanelEntry> = matrix
                .actors
                .iter()
                .find(|actor| actor.person_id == person_id)
                .map(|actor| {
                    actor
                        .titles
                        .iter()
                        .filter_map(|index| matrix.columns.get(*index))
                        .map(|column| {
                            let group = &matrix.groups[column.group];
                            PanelEntry {
                                href: title_href(column.kind, column.id),
                                name: column.name.clone(),
                                image: column.poster.clone().unwrap_or_default(),
                                detail: group.name.clone(),
                                colour: group.colour,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            let related: Vec<PanelEntry> = synergy::co_stars(matrix, person_id)
                .into_iter()
                .map(|(id, name, count)| PanelEntry {
                    href: person(id),
                    name,
                    image: String::new(),
                    detail: format!("{count} together"),
                    colour: synergy::UNGROUPED,
                })
                .collect();

            let name = matrix
                .actors
                .iter()
                .find(|actor| actor.person_id == person_id)
                .map(|actor| actor.name.clone())
                .unwrap_or_else(|| format!("#{person_id}"));

            PanelView {
                show: true,
                heading: name,
                note: their_titles.len().to_string() + " in this list",
                entries: their_titles,
                related_heading: "Shares two or more with".to_owned(),
                related,
                reset: base.to_owned(),
            }
        }

        Focus::Title(media_id) => {
            let column = matrix.columns.iter().find(|column| column.id == media_id);
            let group = column.map(|column| &matrix.groups[column.group]);

            let entries: Vec<PanelEntry> = matrix
                .actors
                .iter()
                .filter(|actor| {
                    actor
                        .titles
                        .iter()
                        .filter_map(|index| matrix.columns.get(*index))
                        .any(|found| found.id == media_id)
                })
                .map(|actor| PanelEntry {
                    href: person(actor.person_id),
                    name: actor.name.clone(),
                    image: actor.image.clone().unwrap_or_default(),
                    detail: detail(actor.titles.len()),
                    colour: group
                        .map(|group| group.colour)
                        .unwrap_or(synergy::UNGROUPED),
                })
                .collect();

            PanelView {
                show: true,
                heading: column.map(|column| column.name.clone()).unwrap_or_default(),
                note: detail(entries.len()) + " shared with this list",
                entries,
                related_heading: String::new(),
                related: Vec::new(),
                reset: base.to_owned(),
            }
        }

        Focus::Bridges => {
            let entries: Vec<PanelEntry> = synergy::bridges(matrix)
                .into_iter()
                .map(|(id, name, count)| {
                    let colour = matrix
                        .actors
                        .iter()
                        .find(|actor| actor.person_id == id)
                        .and_then(|actor| actor.titles.first())
                        .and_then(|index| matrix.columns.get(*index))
                        .map(|column| matrix.groups[column.group].colour)
                        .unwrap_or(synergy::UNGROUPED);

                    PanelEntry {
                        href: person(id),
                        name,
                        image: String::new(),
                        detail: detail(count),
                        colour,
                    }
                })
                .collect();

            let _ = kind;

            PanelView {
                show: !entries.is_empty(),
                heading: "Bridges".to_owned(),
                note: "Actors who cross from one franchise to another".to_owned(),
                entries,
                related_heading: String::new(),
                related: Vec::new(),
                reset: base.to_owned(),
            }
        }
    }
}

impl DiscoverPage {
    /// Build the page from a loaded snapshot.
    pub fn new(head: Head, snapshot: &Snapshot<'_>, now: i64, derived: &Derived) -> Self {
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

        // The list a kind shows when no filter is asked for, which is the page the bare href
        // already means.
        let default_filter = |kind: Kind| {
            crate::discovery::resolve(section, kind, None).map(|variant| variant.filter)
        };

        let kinds = [Kind::Movie, Kind::Tv]
            .into_iter()
            .map(|value| Choice {
                value: kind_key(value).to_owned(),
                label: kind_label(value).to_owned(),
                // Carry the filter across when the other kind has one by that name: "Trending
                // shows" is the same question as "Trending films", and dropping back to All time
                // made the control look like it had a mind of its own. A filter that does not
                // exist for that kind ("In cinemas" for shows), or one that is that kind's
                // default anyway, falls back to the bare path — the same page, without a query
                // string saying it twice.
                href: match crate::discovery::resolve(section, value, Some(variant.filter)) {
                    Some(found) if Some(found.filter) != default_filter(value) => {
                        href(value, found.filter)
                    }
                    _ => href(value, ""),
                },
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

        let matrix = synergy::build(
            kind,
            items,
            synergy,
            &derived.placements,
            &derived.overrides,
            &derived.filter,
        );

        let view = synergy_view(kind, &matrix, derived, &href(kind, variant.filter));

        Self {
            head,
            section: section.key().to_owned(),
            heading: variant.heading().to_owned(),
            blurb: variant.blurb().to_owned(),
            kinds,
            filters,
            filter_group: section.filter_group().to_owned(),
            refreshed_relative: relative_time(refreshed_ts, now),
            refreshed_absolute: absolute_time(refreshed_ts),
            item_count: items.len(),
            synergy: view,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::TitleRef;

    fn item(id: u32, name: &str, year: Option<u16>, rating: Option<f64>) -> Item {
        Item {
            id,
            name: name.to_owned(),
            year,
            date: None,
            poster: Some(format!("/{id}.jpg")),
            overview: None,
            rating,
            collection_id: None,
            director_ids: Vec::new(),
            keyword_ids: Vec::new(),
        }
    }

    fn derived() -> Derived {
        Derived {
            placements: HashMap::new(),
            overrides: HashMap::new(),
            filter: Filter::default(),
            focus: Focus::Bridges,
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
                    order: None,
                })
                .collect(),
        }
    }

    fn build(items: &[Item], rows: &[SynergyRow]) -> DiscoverPage {
        build_with(items, rows, derived())
    }

    fn build_with(items: &[Item], rows: &[SynergyRow], derived: Derived) -> DiscoverPage {
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
            &derived,
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

        assert_eq!(
            page.kinds[1].href, "/discover/top?kind=tv",
            "all time is what shows show by default, so the bare path is the same page"
        );
        assert_eq!(
            page.filters[1].href,
            "/discover/top?kind=movie&filter=trending"
        );
    }

    /// Switching kind keeps the filter: "Trending films" and "Trending shows" are one question
    /// asked of two libraries, and landing on All time felt like the control resetting itself.
    #[test]
    fn switching_kind_keeps_the_filter_it_can_keep() {
        let trending = DiscoverPage::new(
            Head::new("t", "d", "title"),
            &Snapshot {
                section: Section::Top,
                kind: Kind::Movie,
                variant: crate::discovery::resolve(Section::Top, Kind::Movie, Some("trending"))
                    .expect("trending"),
                refreshed_ts: 0,
                items: &[],
                synergy: &[],
            },
            0,
            &derived(),
        );

        let shows = trending
            .kinds
            .iter()
            .find(|choice| choice.value == "tv")
            .expect("a shows choice");
        assert_eq!(shows.href, "/discover/top?kind=tv&filter=trending");

        // A filter that means nothing for the other kind falls back to that kind's default
        // rather than offering a page that does not exist.
        let cinemas = DiscoverPage::new(
            Head::new("t", "d", "new"),
            &Snapshot {
                section: Section::New,
                kind: Kind::Movie,
                variant: crate::discovery::resolve(Section::New, Kind::Movie, Some("cinema"))
                    .expect("cinema"),
                refreshed_ts: 0,
                items: &[],
                synergy: &[],
            },
            0,
            &derived(),
        );

        let shows = cinemas
            .kinds
            .iter()
            .find(|choice| choice.value == "tv")
            .expect("a shows choice");
        assert_eq!(shows.href, "/discover/new?kind=tv");
    }

    #[test]
    fn a_column_links_to_tmdb_with_the_lists_own_kind() {
        let page = build(
            &[item(603, "The Matrix", Some(1999), None)],
            &[row(6384, "Keanu Reeves", &[(603, "The Matrix")], 2)],
        );

        assert_eq!(
            page.synergy.columns[0].href,
            "https://www.themoviedb.org/movie/603"
        );
        assert_eq!(page.synergy.columns[0].meta, "1999");
    }

    /// The matrix has no room for a date, but it must not lose one either.
    #[test]
    fn a_dated_list_carries_the_date_into_the_tooltip() {
        let mut dated = item(603, "The Matrix", Some(1999), None);
        dated.date = Some("2026-12-01".to_owned());

        let page = build(&[dated], &[]);
        assert_eq!(page.synergy.columns[0].meta, "1 Dec 2026");
    }

    #[test]
    fn rows_carry_the_grid_positions_the_template_draws_from() {
        let items = vec![
            item(1, "One", None, None),
            item(2, "Two", None, None),
            item(3, "Three", None, None),
        ];
        let rows = vec![row(10, "Present", &[(1, "One"), (3, "Three")], 2)];

        let page = build(&items, &rows);
        let drawn = &page.synergy.rows[0];

        assert_eq!(
            drawn.at,
            vec![2, 4],
            "name column is first, then one per title"
        );
        assert_eq!(
            (drawn.run_start, drawn.run_end),
            (2, 5),
            "the run spans first to last"
        );
        assert!(drawn.has_run);
        assert_eq!(
            drawn.data, "1 3",
            "space separated: the island splits on whitespace"
        );
    }

    #[test]
    fn the_row_cap_is_reported_rather_than_silently_dropping_people() {
        let items = vec![item(1, "One", None, None), item(2, "Two", None, None)];
        let rows: Vec<SynergyRow> = (0..crate::synergy::MAX_ROWS as u32 + 4)
            .map(|id| row(id, "Someone", &[(1, "One"), (2, "Two")], 2))
            .collect();

        let page = build(&items, &rows);
        assert_eq!(page.synergy.rows.len(), crate::synergy::MAX_ROWS);
        assert_eq!(page.synergy.hidden_actors, 4);
    }

    #[test]
    fn a_chip_links_to_the_same_page_with_that_franchise_flipped() {
        let items = vec![
            item(1, "Iron Man", None, None),
            item(2, "The Avengers", None, None),
        ];
        let mut derived = derived();
        derived.placements = [
            (
                ("movie".to_owned(), 1),
                Placement {
                    franchise_id: 1,
                    slug: "marvel".to_owned(),
                    name: "Marvel".to_owned(),
                    sort_order: 0,
                },
            ),
            (
                ("movie".to_owned(), 2),
                Placement {
                    franchise_id: 1,
                    slug: "marvel".to_owned(),
                    name: "Marvel".to_owned(),
                    sort_order: 0,
                },
            ),
        ]
        .into_iter()
        .collect();

        let page = build_with(&items, &[], derived);
        assert_eq!(page.synergy.chips.len(), 1, "one franchise, one chip");
        assert!(page.synergy.chips[0].shown, "on by default");
        assert!(
            page.synergy.chips[0].href.ends_with("hide=marvel"),
            "switching it off is a URL: {}",
            page.synergy.chips[0].href
        );
    }

    #[test]
    fn a_filtered_away_list_says_so_instead_of_drawing_nothing() {
        let items = vec![
            item(1, "Iron Man", None, None),
            item(2, "The Avengers", None, None),
        ];
        let mut derived = derived();
        derived.placements = [(
            ("movie".to_owned(), 1),
            Placement {
                franchise_id: 1,
                slug: "marvel".to_owned(),
                name: "Marvel".to_owned(),
                sort_order: 0,
            },
        )]
        .into_iter()
        .collect();
        derived.filter.hide.insert("marvel".to_owned());

        let page = build_with(&items, &[], derived);
        assert!(
            !page.synergy.empty.is_empty(),
            "an empty state, not a blank grid"
        );
        assert_eq!(page.synergy.columns.len(), 1);
    }

    /// A pinned actor is the panel's subject, and the row says so for the reader and the island.
    #[test]
    fn a_pinned_actor_selects_the_panel_and_marks_the_row() {
        let items = vec![item(1, "One", None, None), item(2, "Two", None, None)];
        let rows = vec![
            row(10, "Pinned", &[(1, "One"), (2, "Two")], 2),
            row(11, "Other", &[(1, "One"), (2, "Two")], 2),
        ];

        let mut derived = derived();
        derived.focus = Focus::Actor(10);

        let page = build_with(&items, &rows, derived);

        assert_eq!(page.synergy.panel.heading, "Pinned");
        assert!(page.synergy.rows.iter().any(|row| row.pinned));
        assert_eq!(
            page.synergy.rows.iter().filter(|row| row.pinned).count(),
            1,
            "only the pinned one"
        );
        assert_eq!(
            page.synergy.panel.related.len(),
            1,
            "the other actor shares both titles with them"
        );
    }

    #[test]
    fn with_nothing_pinned_the_panel_offers_bridges() {
        let items = vec![
            item(1, "Iron Man", None, None),
            item(2, "Pulp Fiction", None, None),
            item(3, "The Avengers", None, None),
        ];
        let rows = vec![row(
            10,
            "Bridges",
            &[(1, "Iron Man"), (2, "Pulp Fiction")],
            2,
        )];

        let mut derived = derived();
        derived.placements = [(
            ("movie".to_owned(), 1),
            Placement {
                franchise_id: 1,
                slug: "marvel".to_owned(),
                name: "Marvel".to_owned(),
                sort_order: 0,
            },
        )]
        .into_iter()
        .collect();

        let page = build_with(&items, &rows, derived);
        assert_eq!(page.synergy.panel.heading, "Bridges");
        assert_eq!(page.synergy.panel.entries.len(), 1);
    }
}

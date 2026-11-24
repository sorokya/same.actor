//! `/title/{picks}` and `/person/{picks}` — the comparison itself.
//!
//! The results are part of the document: a shared link shows its answer before any script runs,
//! and the "6 actors in at least two of these titles" line is in the HTML a crawler sees.
//!
//! The picker on top is an island. It is the one place where instant feedback genuinely needs a
//! component — typing, keyboard navigation, a debounced debounce — and it is the only script on
//! the site.

use askama::Template;

use crate::domain::{Kind, Matches, Mode, PersonMatch, SelectionMeta, TitleMatch};

use super::Head;

/// The palette, mirrored from `src/lib/palette.ts`. A column is identifiable by colour, which
/// matters once there are five of them.
const TINTS: [&str; 5] = ["#e8b14b", "#3ecfc0", "#f2799e", "#a996ff", "#b6d15c"];

fn tint(index: usize) -> &'static str {
    TINTS[index % TINTS.len()]
}

#[derive(Clone)]
pub struct Column {
    pub href: String,
    pub name: String,
    /// Pre-formatted; empty when TMDB has no date.
    pub year: String,
    pub image: String,
    pub person: bool,
    pub tint: &'static str,
}

#[derive(Clone)]
pub struct Cell {
    /// The selection's name, for the mobile card's term column.
    pub label: String,
    /// Empty means "not credited here", which the template renders as a dash.
    pub text: String,
}

#[derive(Clone)]
pub struct Row {
    pub href: String,
    pub name: String,
    pub year: String,
    pub image: String,
    pub person: bool,
    pub cells: Vec<Cell>,
}

/// The results region: the columns, the rows, and the sentence above them.
///
/// Separate from the page because the page and the fragment both render it, and a struct that
/// cannot contain itself is the cheapest way to keep one definition of a result.
#[derive(Clone)]
pub struct Results {
    pub selections: Vec<Column>,
    pub rows: Vec<Row>,
    pub row_heading: String,
    pub count_label: String,
    /// "viewed 1,204 times", or empty. One view is not worth saying.
    pub views_label: String,
    pub most_shared: bool,
    pub empty_heading: String,
    /// "Add one more to compare", while the selection is too small to say anything. It lives
    /// here rather than on the page because the island swaps this region: a hint that stayed
    /// behind after the second pick would be telling the reader something untrue.
    pub hint: String,
}

#[derive(Template)]
#[template(path = "pages/compare.html")]
pub struct ComparePage {
    pub head: Head,
    /// `title` or `person`.
    pub mode: &'static str,
    pub heading: String,
    pub sub: String,
    /// JSON for the island to start from. HTML-escaped into a `data-` attribute, then parsed.
    pub state_json: String,
    /// The canonical comparison URL, for the "copy link" affordance.
    pub share_url: String,
    pub results: Results,
    /// What people have been searching for today. Empty until a term is searched three times.
    pub trending: Vec<TrendLink>,
    /// Starting points for an empty page: things worth typing when nothing is selected.
    pub examples: Vec<TrendLink>,
}

/// One term in a chip strip, linked back into the search box.
pub struct TrendLink {
    pub term: String,
    pub href: String,
    /// One of the hue names in index.css, so a row of chips is not a row of identical pills.
    pub tint: &'static str,
}

/// The five hue names, cycling along a strip of chips.
const TINT_NAMES: [&str; 5] = ["title", "person", "top", "new", "upcoming"];

/// Where a search box lives for this mode: `/` or `/person`.
fn search_path(mode: Mode) -> &'static str {
    match mode {
        Mode::Title => "/",
        Mode::Person => "/person",
    }
}

/// A chip that fills the search box with `term`.
fn trend(term: &str, index: usize, path: &str) -> TrendLink {
    TrendLink {
        term: term.to_owned(),
        href: format!("{path}?q={}", encode_query(term)),
        tint: TINT_NAMES[index % TINT_NAMES.len()],
    }
}

/// What to offer somebody who has not typed anything yet.
///
/// An empty search box asks the reader to remember a title unaided, and the honest answer is
/// "I cannot think of one". These are titles and people with large, well-documented casts, so
/// any two of them produce an answer rather than an apology — which is what an example is for.
fn examples_for(mode: Mode) -> Vec<TrendLink> {
    let path = search_path(mode);
    let terms: &[&str] = match mode {
        Mode::Title => &[
            "The Matrix",
            "Alien",
            "Pulp Fiction",
            "The Wire",
            "Breaking Bad",
            "Dune",
        ],
        Mode::Person => &[
            "Willem Dafoe",
            "Cate Blanchett",
            "Samuel L. Jackson",
            "Tilda Swinton",
            "Brad Pitt",
        ],
    };

    terms
        .iter()
        .enumerate()
        .map(|(index, term)| trend(term, index, path))
        .collect()
}

/// Turn trending terms into chips for the given mode.
pub fn trend_links(terms: &[String], mode: Mode) -> Vec<TrendLink> {
    let path = search_path(mode);
    terms
        .iter()
        .enumerate()
        .map(|(index, term)| trend(term, index, path))
        .collect()
}

/// The results region on its own, for the island to swap in after a selection changes.
#[derive(Template)]
#[template(path = "fragments/results.html")]
pub struct ResultsFragment {
    pub results: Results,
}

/// Percent-encode a search term for a query string. Everything outside an unreserved character
/// goes in as `%XX`, which is always correct and never surprising in a URL.
fn encode_query(term: &str) -> String {
    let mut out = String::with_capacity(term.len());

    for byte in term.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b' ' => {
                if byte == b' ' {
                    out.push('+');
                } else {
                    out.push(byte as char);
                }
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }

    out
}

fn copy(mode: Mode) -> (&'static str, &'static str) {
    match mode {
        Mode::Title => (
            "What do these films share?",
            "Pick two to five films or shows. Every actor credited in at least two of them comes back, with their roles.",
        ),
        Mode::Person => (
            "What do these people share?",
            "Pick two to five actors. Every title crediting at least two of them comes back, with their roles.",
        ),
    }
}

fn mode_key(mode: Mode) -> &'static str {
    match mode {
        Mode::Title => "title",
        Mode::Person => "person",
    }
}

fn tmdb_href(kind: Kind, id: u32) -> String {
    format!("https://www.themoviedb.org/{}/{id}", kind.as_str())
}

fn year_of(year: Option<u16>) -> String {
    year.map(|year| year.to_string()).unwrap_or_default()
}

/// The roles one match has in one selection, already joined for display.
fn characters_for(entry: &Entry, selection_id: u32) -> String {
    match entry {
        Entry::Person(person) => person
            .credits
            .iter()
            .find(|credit| credit.selection == selection_id)
            .map(|credit| {
                credit
                    .character
                    .iter()
                    .map(|part| part.trim())
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        Entry::Title(title) => title
            .credits
            .iter()
            .find(|credit| credit.selection == selection_id)
            .and_then(|credit| credit.character.as_deref())
            .map(|character| character.trim().to_owned())
            .unwrap_or_default(),
    }
}

/// A borrow of either kind of match, so one loop can build both tables.
enum Entry<'a> {
    Person(&'a PersonMatch),
    Title(&'a TitleMatch),
}

impl ComparePage {
    /// The strip is filled by the caller, which has the store: a page can then be built in a
    /// test without one.
    pub fn with_trending(mut self, links: Vec<TrendLink>) -> Self {
        self.trending = links;
        self
    }

    /// Build the page. `matches` is empty when fewer than two selections were made, which is the
    /// empty state rather than an error.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        head: Head,
        mode: Mode,
        selections: &[SelectionMeta],
        matches: &Matches,
        views: Option<u64>,
        share_url: String,
    ) -> Self {
        let (heading, sub) = copy(mode);

        let columns: Vec<Column> = selections
            .iter()
            .enumerate()
            .map(|(index, selection)| Column {
                href: tmdb_href(selection.kind, selection.id),
                name: selection.name.clone(),
                year: year_of(selection.year),
                image: selection.image.clone().unwrap_or_default(),
                person: selection.kind == Kind::Person,
                tint: tint(index),
            })
            .collect();

        let borrowed: Vec<Entry> = match matches {
            Matches::People(people) => people.iter().map(Entry::Person).collect(),
            Matches::Titles(titles) => titles.iter().map(Entry::Title).collect(),
        };

        let rows: Vec<Row> = borrowed
            .iter()
            .map(|entry| {
                let (id, name, year, image, kind, person) = match entry {
                    Entry::Person(person) => (
                        person.id,
                        person.name.clone(),
                        String::new(),
                        person.image.clone(),
                        Kind::Person,
                        true,
                    ),
                    Entry::Title(title) => (
                        title.id,
                        title.name.clone(),
                        year_of(title.year),
                        title.image.clone(),
                        title.kind,
                        false,
                    ),
                };

                Row {
                    href: tmdb_href(kind, id),
                    name,
                    year,
                    image: image.unwrap_or_default(),
                    person,
                    cells: selections
                        .iter()
                        .map(|selection| Cell {
                            label: selection.name.clone(),
                            text: characters_for(entry, selection.id),
                        })
                        .collect(),
                }
            })
            .collect();

        let count = rows.len();
        let count_label = match (mode, count) {
            (_, 0) => String::new(),
            (Mode::Title, n) => format!(
                "{n} {} in at least two of these titles",
                if n == 1 { "actor" } else { "actors" }
            ),
            (Mode::Person, n) => format!(
                "{n} {} featuring at least two of these people",
                if n == 1 { "title" } else { "titles" }
            ),
        };

        let empty_heading = match mode {
            Mode::Title => "No shared cast.",
            Mode::Person => "No shared titles.",
        };

        let hint = match selections.len() {
            0 => String::new(),
            1 => "Add one more to compare. Two is the minimum.".to_owned(),
            _ => String::new(),
        };

        let views_label = match views {
            Some(views) if views > 1 => format!("viewed {views} times"),
            _ => String::new(),
        };

        let row_heading = match mode {
            Mode::Title => "Shared cast",
            Mode::Person => "Shared titles",
        };

        Self {
            head,
            mode: mode_key(mode),
            heading: heading.to_owned(),
            sub: sub.to_owned(),
            state_json: picker_state(mode, selections),
            share_url,
            trending: Vec::new(),
            examples: examples_for(mode),
            results: Results {
                selections: columns,
                rows,
                row_heading: row_heading.to_owned(),
                count_label,
                views_label,
                most_shared: count > 1,
                empty_heading: empty_heading.to_owned(),
                hint,
            },
        }
    }
}

/// The island's starting state: what is picked, and what each pick is called.
///
/// Rendered into a `data-` attribute rather than a `<script type="application/json">` block,
/// because a TMDB title containing `</script>` would otherwise end the element early. Askama
/// escapes the attribute; the browser un-escapes it before `JSON.parse` sees it.
fn picker_state(mode: Mode, selections: &[SelectionMeta]) -> String {
    let picks: Vec<String> = selections
        .iter()
        .map(|selection| {
            format!(
                r#"{{"id":{},"kind":"{}","name":{},"year":{},"image":{}}}"#,
                selection.id,
                selection.kind.as_str(),
                serde_json::to_string(&selection.name).unwrap_or_else(|_| "\"\"".into()),
                selection
                    .year
                    .map(|year| year.to_string())
                    .unwrap_or_else(|| "null".into()),
                serde_json::to_string(&selection.image).unwrap_or_else(|_| "null".into()),
            )
        })
        .collect();

    format!(
        r#"{{"mode":"{}","picks":[{}]}}"#,
        mode_key(mode),
        picks.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PersonCredit, TitleCredit};

    fn meta(id: u32, kind: Kind, name: &str) -> SelectionMeta {
        SelectionMeta {
            id,
            kind,
            name: name.to_owned(),
            year: Some(1999),
            image: None,
        }
    }

    fn head() -> Head {
        Head::new("Test", "desc", "title")
    }

    #[test]
    fn a_person_row_gets_one_cell_per_selection() {
        let selections = vec![
            meta(603, Kind::Movie, "The Matrix"),
            meta(604, Kind::Movie, "Reloaded"),
        ];
        let matches = Matches::People(vec![PersonMatch {
            id: 6384,
            name: "Keanu Reeves".into(),
            image: None,
            credits: vec![
                PersonCredit {
                    selection: 603,
                    character: vec!["Neo".into()],
                },
                PersonCredit {
                    selection: 604,
                    character: vec!["Neo".into()],
                },
            ],
        }]);

        let page = ComparePage::new(
            head(),
            Mode::Title,
            &selections,
            &matches,
            None,
            "/x".into(),
        );

        assert_eq!(page.results.rows.len(), 1);
        assert_eq!(page.results.rows[0].cells.len(), 2);
        assert_eq!(page.results.rows[0].cells[0].text, "Neo");
        assert_eq!(
            page.results.count_label,
            "1 actor in at least two of these titles"
        );
        assert_eq!(page.results.row_heading, "Shared cast");
    }

    /// Several roles in one title is normal for television; they are joined, not dropped.
    #[test]
    fn a_multi_role_credit_is_joined() {
        let selections = vec![
            meta(1399, Kind::Tv, "Game of Thrones"),
            meta(1396, Kind::Tv, "Breaking Bad"),
        ];
        let matches = Matches::People(vec![PersonMatch {
            id: 1,
            name: "Someone".into(),
            image: None,
            credits: vec![PersonCredit {
                selection: 1399,
                character: vec!["Daario".into(), "Second role".into()],
            }],
        }]);

        let page = ComparePage::new(
            head(),
            Mode::Title,
            &selections,
            &matches,
            None,
            "/x".into(),
        );
        assert_eq!(page.results.rows[0].cells[0].text, "Daario, Second role");
        assert_eq!(
            page.results.rows[0].cells[1].text, "",
            "no credit in the other title is an empty cell, not a missing one"
        );
    }

    #[test]
    fn a_title_row_carries_its_kind_into_the_link() {
        let selections = vec![
            meta(6384, Kind::Person, "Keanu Reeves"),
            meta(530, Kind::Person, "Carrie-Anne Moss"),
        ];
        let matches = Matches::Titles(vec![TitleMatch {
            id: 1399,
            kind: Kind::Tv,
            name: "A Show".into(),
            year: Some(2011),
            image: None,
            credits: vec![TitleCredit {
                selection: 6384,
                character: Some("Himself".into()),
            }],
        }]);

        let page = ComparePage::new(
            head(),
            Mode::Person,
            &selections,
            &matches,
            None,
            "/x".into(),
        );

        assert_eq!(
            page.results.rows[0].href,
            "https://www.themoviedb.org/tv/1399"
        );
        assert_eq!(page.results.rows[0].year, "2011");
        assert_eq!(page.results.row_heading, "Shared titles");
        assert!(
            !page.results.rows[0].person,
            "a title row gets a poster, not a portrait"
        );
    }

    #[test]
    fn no_matches_is_stated_rather_than_rendered_as_an_empty_table() {
        let selections = vec![
            meta(603, Kind::Movie, "The Matrix"),
            meta(604, Kind::Movie, "Reloaded"),
        ];
        let page = ComparePage::new(
            head(),
            Mode::Title,
            &selections,
            &Matches::People(Vec::new()),
            None,
            "/x".into(),
        );

        assert!(page.results.rows.is_empty());
        assert_eq!(page.results.empty_heading, "No shared cast.");
        assert_eq!(page.results.count_label, "");
    }

    #[test]
    fn one_selection_is_a_hint_not_a_comparison() {
        let selections = vec![meta(603, Kind::Movie, "The Matrix")];
        let page = ComparePage::new(
            head(),
            Mode::Title,
            &selections,
            &Matches::People(Vec::new()),
            None,
            "/x".into(),
        );

        assert_eq!(
            page.results.hint,
            "Add one more to compare. Two is the minimum."
        );
    }

    /// The island parses this string, so it has to be valid JSON on its own.
    #[test]
    fn the_picker_state_is_json_the_island_can_parse() {
        let selections = vec![meta(603, Kind::Movie, "The \"Matrix\"")]; // a quote in the name
        let state = picker_state(Mode::Title, &selections);
        let parsed: serde_json::Value = serde_json::from_str(&state).expect("valid json");

        assert_eq!(parsed["mode"], "title");
        assert_eq!(parsed["picks"][0]["id"], 603);
        assert_eq!(parsed["picks"][0]["kind"], "movie");
        assert_eq!(
            parsed["picks"][0]["name"], "The \"Matrix\"",
            "the name survives escaping"
        );
    }

    #[test]
    fn an_unknown_selection_id_is_an_empty_cell() {
        let selections = vec![meta(603, Kind::Movie, "The Matrix")];
        let matches = Matches::People(vec![PersonMatch {
            id: 1,
            name: "Someone".into(),
            image: None,
            credits: vec![PersonCredit {
                selection: 999,
                character: vec!["Elsewhere".into()],
            }],
        }]);

        let page = ComparePage::new(
            head(),
            Mode::Title,
            &selections,
            &matches,
            None,
            "/x".into(),
        );
        assert_eq!(page.results.rows[0].cells[0].text, "");
    }

    /// A view count of one is noise about yourself; the label only appears once it is shared.
    #[test]
    fn one_view_is_not_worth_a_label() {
        let selections: Vec<SelectionMeta> = Vec::new();
        let matches = Matches::People(Vec::new());
        assert_eq!(
            ComparePage::new(
                head(),
                Mode::Title,
                &selections,
                &matches,
                Some(1),
                "/x".into()
            )
            .results
            .views_label,
            ""
        );
        assert_eq!(
            ComparePage::new(
                head(),
                Mode::Title,
                &selections,
                &matches,
                Some(42),
                "/x".into()
            )
            .results
            .views_label,
            "viewed 42 times"
        );
    }
}

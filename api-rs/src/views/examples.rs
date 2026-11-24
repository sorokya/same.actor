//! Worked examples for an empty comparison page.
//!
//! The first thing a new visitor sees is a search box and nothing else, which asks them to
//! remember a film and then a second one — the two hardest steps on the site, taken first. These
//! are *combinations*: a click opens a real comparison with two titles already in it.
//!
//! They are derived from a stored discovery snapshot rather than written down. That matters:
//! hard-coded ids and pairings are a maintenance liability that rots quietly — the pair stops
//! making sense, the id moves, and nothing tests it. A snapshot already knows which people appear
//! in which titles, because the synergy table needed the same fact, so an example that means
//! something falls out of it:
//!
//!   - for titles, an actor credited in two of the list's films — their two films, linked
//!   - for people, two actors the list credits in one shared title — those two, linked
//!
//! No upstream calls: this reads one row out of SQLite and parses it.

use crate::discovery::{Item, SynergyRow, TitleRef};
use crate::domain::{Kind, Mode};
use crate::paths;

/// One side of a pair: artwork, a name, and where to read about it.
pub struct Side {
    pub href: String,
    /// Empty when TMDB has no artwork; the template shows a placeholder.
    pub image: String,
    pub name: String,
}

/// A pair worth clicking, and the thing that connects it.
pub struct Example {
    /// The comparison this card opens.
    pub href: String,
    pub left: Side,
    pub right: Side,
    /// What the two sides have in common: a person, or a title.
    pub connector: Side,
    /// Whether the connector should be drawn as a face (`true`) or a poster (`false`).
    pub connector_is_person: bool,
    /// "in both", "both in", ready to sit after the connector's name.
    pub phrase: &'static str,
}

/// The lists used for examples, in order of preference.
///
/// Trending first for titles because a visitor recognises the names; all-time first for people
/// because the biggest casts are the ones most likely to share a title.
fn lists_for(mode: Mode) -> &'static [&'static str] {
    match mode {
        Mode::Title => &[
            "top:movie:trending",
            "top:movie:all-time",
            "top:tv:trending",
        ],
        Mode::Person => &[
            "top:movie:all-time",
            "top:movie:trending",
            "top:tv:all-time",
        ],
    }
}

/// How many cards the empty state shows. Two is a pair; three is a row.
const LIMIT: usize = 2;

/// Read a snapshot and build examples for this mode.
///
/// Returns an empty vector rather than an error when there is nothing to say — no snapshot yet,
/// an unreadable one, or a list whose synergy has no pair in it. An empty state without examples
/// is the page we had before, not a broken page.
pub async fn for_mode(state: &crate::state::AppState, mode: Mode) -> Vec<Example> {
    for id in lists_for(mode) {
        let Ok(Some(stored)) = state.store.list_snapshot((*id).to_owned()).await else {
            continue;
        };

        let Ok(rows) = serde_json::from_str::<Vec<SynergyRow>>(&stored.synergy) else {
            continue;
        };
        let items: Vec<Item> = serde_json::from_str(&stored.payload).unwrap_or_default();

        let examples = match mode {
            Mode::Title => pairs_of_titles(&rows, &items),
            Mode::Person => pairs_of_people(&rows, &items),
        };

        if !examples.is_empty() {
            return examples;
        }
    }

    Vec::new()
}

/// Artwork for a title, from the list's own items: a synergy row keeps only the name.
fn poster_of(items: &[Item], id: u32) -> String {
    items
        .iter()
        .find(|item| item.id == id)
        .and_then(|item| item.poster.clone())
        .unwrap_or_default()
}

/// An actor credited in two of the list's titles, and those two titles.
fn pairs_of_titles(rows: &[SynergyRow], items: &[Item]) -> Vec<Example> {
    let mut out = Vec::new();
    let mut seen: Vec<(u32, u32)> = Vec::new();

    for row in rows {
        if out.len() >= LIMIT {
            break;
        }

        let Some((first, second)) = row
            .titles
            .split_first()
            .and_then(|(head, rest)| rest.first().map(|next| (head, next)))
        else {
            continue;
        };

        // Two entries for the same title would be a pair that compares a film with itself.
        if first.id == second.id || first.kind != second.kind {
            continue;
        }

        // Two actors in the same two films is a real case, and two identical cards side by side
        // reads as a rendering bug. The pair is what the card offers; the actor is how it is
        // explained, so the first explanation wins.
        let key = (first.id.min(second.id), first.id.max(second.id));
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);

        out.push(Example {
            href: paths::comparison(
                Mode::Title,
                &[
                    (first.kind, first.id, first.name.as_str()),
                    (second.kind, second.id, second.name.as_str()),
                ],
            ),
            left: side_of_title(first, items),
            right: side_of_title(second, items),
            connector: Side {
                href: format!("https://www.themoviedb.org/person/{}", row.person_id),
                image: row.image.clone().unwrap_or_default(),
                name: row.name.clone(),
            },
            connector_is_person: true,
            phrase: "in both",
        });
    }

    out
}

/// Two actors the list credits in the same title.
fn pairs_of_people(rows: &[SynergyRow], items: &[Item]) -> Vec<Example> {
    // title id -> (title, the people credited in it), in list order.
    let mut by_title: Vec<(TitleRef, Vec<&SynergyRow>)> = Vec::new();

    for row in rows {
        for title in &row.titles {
            match by_title.iter_mut().find(|(known, _)| known.id == title.id) {
                Some((_, people)) => people.push(row),
                None => by_title.push((title.clone(), vec![row])),
            }
        }
    }

    let mut out = Vec::new();

    let mut seen: Vec<(u32, u32)> = Vec::new();

    for (title, people) in by_title {
        if out.len() >= LIMIT {
            break;
        }

        let (Some(first), Some(second)) = (people.first(), people.get(1)) else {
            continue;
        };

        if first.person_id == second.person_id {
            continue;
        }

        let key = (
            first.person_id.min(second.person_id),
            first.person_id.max(second.person_id),
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);

        out.push(Example {
            href: paths::comparison(
                Mode::Person,
                &[
                    (Kind::Person, first.person_id, first.name.as_str()),
                    (Kind::Person, second.person_id, second.name.as_str()),
                ],
            ),
            left: Side {
                href: format!("https://www.themoviedb.org/person/{}", first.person_id),
                image: first.image.clone().unwrap_or_default(),
                name: first.name.clone(),
            },
            right: Side {
                href: format!("https://www.themoviedb.org/person/{}", second.person_id),
                image: second.image.clone().unwrap_or_default(),
                name: second.name.clone(),
            },
            connector: side_of_title(&title, items),
            connector_is_person: false,
            phrase: "both in",
        });
    }

    out
}

fn side_of_title(title: &TitleRef, items: &[Item]) -> Side {
    Side {
        href: format!(
            "https://www.themoviedb.org/{}/{}",
            title.kind.as_str(),
            title.id
        ),
        image: poster_of(items, title.id),
        name: title.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_items() -> Vec<Item> {
        Vec::new()
    }

    fn row(person: u32, name: &str, titles: &[(Kind, u32, &str)]) -> SynergyRow {
        SynergyRow {
            person_id: person,
            name: name.to_owned(),
            image: Some(format!("/{person}.jpg")),
            count: titles.len(),
            titles: titles
                .iter()
                .map(|(kind, id, title)| TitleRef {
                    id: *id,
                    kind: *kind,
                    name: (*title).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn an_actor_in_two_titles_becomes_a_title_pair() {
        let rows = vec![row(
            1,
            "Keanu Reeves",
            &[
                (Kind::Movie, 603, "The Matrix"),
                (Kind::Movie, 245891, "John Wick"),
            ],
        )];

        let examples = pairs_of_titles(&rows, &no_items());
        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].left.name, "The Matrix");
        assert_eq!(examples[0].right.name, "John Wick");
        assert_eq!(examples[0].connector.name, "Keanu Reeves");
        assert_eq!(
            examples[0].href,
            "/title/movie-603-the-matrix,movie-245891-john-wick"
        );
    }

    /// A pair that compares a film with itself is not a comparison.
    #[test]
    fn one_title_is_not_a_pair() {
        let rows = vec![row(
            1,
            "Keanu Reeves",
            &[
                (Kind::Movie, 603, "The Matrix"),
                (Kind::Movie, 603, "The Matrix"),
            ],
        )];

        assert!(pairs_of_titles(&rows, &no_items()).is_empty());
    }

    /// Two people the list also credits in the same film are a person comparison.
    #[test]
    fn two_people_in_one_title_become_a_person_pair() {
        let rows = vec![
            row(1, "Carrie-Anne Moss", &[(Kind::Movie, 603, "The Matrix")]),
            row(2, "Keanu Reeves", &[(Kind::Movie, 603, "The Matrix")]),
        ];

        let examples = pairs_of_people(&rows, &no_items());
        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].left.name, "Carrie-Anne Moss");
        assert_eq!(examples[0].right.name, "Keanu Reeves");
        assert_eq!(examples[0].connector.name, "The Matrix");
        assert_eq!(examples[0].phrase, "both in");
        assert_eq!(
            examples[0].href, "/person/1-carrie-anne-moss,2-keanu-reeves",
            "person picks carry a bare id, matching src/lib/url.ts"
        );
    }

    /// The same pair twice is two identical cards, which reads as a bug in the layout.
    #[test]
    fn a_pair_is_only_offered_once() {
        let rows = vec![
            row(
                1,
                "Carrie-Anne Moss",
                &[
                    (Kind::Movie, 603, "The Matrix"),
                    (Kind::Movie, 604, "Reloaded"),
                ],
            ),
            row(
                2,
                "Hugo Weaving",
                &[
                    (Kind::Movie, 603, "The Matrix"),
                    (Kind::Movie, 604, "Reloaded"),
                ],
            ),
        ];

        let examples = pairs_of_titles(&rows, &no_items());
        assert_eq!(examples.len(), 1, "one pair, one card");
        assert_eq!(examples[0].connector.name, "Carrie-Anne Moss");
    }

    #[test]
    fn an_empty_list_produces_nothing_rather_than_a_broken_card() {
        assert!(pairs_of_titles(&[], &no_items()).is_empty());
        assert!(pairs_of_people(&[], &no_items()).is_empty());
    }
}

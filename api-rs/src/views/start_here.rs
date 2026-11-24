//! What an empty comparison page offers instead of a bare search box.
//!
//! Two sources, in this order:
//!
//! 1. **Comparisons that worked.** Every comparison the site runs is already recorded, with how
//!    many answers it produced. The newest few of those are the best possible suggestion: real
//!    sets, real counts, artwork already stored, and no upstream call to render.
//! 2. **Curated pairs**, when nothing has been compared yet. A short hand-written list of pairs
//!    known to share a cast, resolved through TMDB search at render time.
//!
//! The order is the point. A visitor's own traffic recommences better than a developer's taste,
//! and it starts working the moment the first comparison happens — not when a discovery refresh
//! has run, which is what this section used to wait for.
//!
//! No TMDB ids are written down here. A curated entry names a search term, and search resolves it
//! to whatever that title is *now*; a hard-coded id rots silently, and a hard-coded id paired with
//! a hard-coded name rots into a lie.

use serde::Deserialize;

use crate::domain::{Kind, Mode};
use crate::paths;
use crate::state::AppState;

/// How many cards the section offers: two rows of two, which is a shape rather than a list
/// that stopped.
const LIMIT: usize = 4;

/// One side of a pair. The whole card is one link, so the side carries no href of its own.
pub struct Side {
    /// Empty when TMDB has no artwork; the template shows a placeholder behind it.
    pub image: String,
    pub name: String,
}

/// A pair worth clicking, and the line under it that says why.
pub struct Example {
    /// The comparison this card opens.
    pub href: String,
    pub left: Side,
    pub right: Side,
    /// The person (title mode) or title (person mode) that connects the two sides. Empty for a
    /// recent comparison, which counts its answers instead of naming a connector.
    pub connector: Side,
    /// Whether the connector is a face (`true`) or a poster (`false`), for the glyph.
    pub connector_is_person: bool,
    /// "in both", or "12 actors in common".
    pub phrase: String,
}

/// A curated starting point: two search terms and the reason they go together.
struct Curated {
    mode: Mode,
    left: &'static str,
    right: &'static str,
    /// Who or what links them. Written by hand, because that is what curation is.
    connector: &'static str,
}

/// Read down, not across: one pair per line, each pair a comparison that is worth a click.
const CURATED: &[Curated] = &[
    Curated {
        mode: Mode::Title,
        left: "The Matrix",
        right: "The Matrix Reloaded",
        connector: "Keanu Reeves in both",
    },
    Curated {
        mode: Mode::Title,
        left: "Alien",
        right: "Aliens",
        connector: "Sigourney Weaver in both",
    },
    Curated {
        mode: Mode::Title,
        left: "Pulp Fiction",
        right: "Reservoir Dogs",
        connector: "Harvey Keitel in both",
    },
    Curated {
        mode: Mode::Title,
        left: "The Bourne Identity",
        right: "The Bourne Supremacy",
        connector: "Matt Damon in both",
    },
    Curated {
        mode: Mode::Person,
        left: "Keanu Reeves",
        right: "Carrie-Anne Moss",
        connector: "Both in The Matrix",
    },
    Curated {
        mode: Mode::Person,
        left: "Samuel L. Jackson",
        right: "John Travolta",
        connector: "Both in Pulp Fiction",
    },
    Curated {
        mode: Mode::Person,
        left: "Cate Blanchett",
        right: "Brad Pitt",
        connector: "Both in Babel",
    },
    Curated {
        mode: Mode::Person,
        left: "Matt Damon",
        right: "Ben Affleck",
        connector: "Both in Good Will Hunting",
    },
];

/// One pick as `picks_json` stores it.
#[derive(Deserialize)]
struct Pick {
    id: u32,
    kind: Kind,
    name: String,
    /// Stored with the row, so a card never asks TMDB for artwork it already had.
    #[serde(default)]
    image: Option<String>,
}

/// A search result reduced to what a card needs.
struct Found {
    id: u32,
    kind: Kind,
    name: String,
    image: String,
}

/// The cards for one mode: recent comparisons first, curated pairs when there are none.
pub async fn for_mode(state: &AppState, mode: Mode) -> Vec<Example> {
    let recent = state
        .store
        .recent_results(mode.as_str().to_owned(), LIMIT)
        .await
        .unwrap_or_default();

    let cards: Vec<Example> = recent
        .iter()
        .filter_map(|row| from_recent(mode, row))
        .collect();

    if cards.is_empty() {
        return curated(state, mode).await;
    }

    cards
}

/// A card for a comparison somebody ran, out of the stored picks.
fn from_recent(mode: Mode, recent: &crate::store::comparisons::Recent) -> Option<Example> {
    let picks: Vec<Pick> = serde_json::from_str(&recent.picks_json).ok()?;
    let (first, second) = (picks.first()?, picks.get(1)?);

    // The link carries every pick the comparison had, not just the two the card names: the count
    // in `phrase` is the count for *that set*, and a link to a smaller set would arrive at a page
    // that contradicts the card that sent you there.
    let segments: Vec<(Kind, u32, &str)> = picks
        .iter()
        .map(|pick| (pick.kind, pick.id, pick.name.as_str()))
        .collect();

    Some(Example {
        href: paths::comparison(mode, &segments),
        left: Side {
            image: first.image.clone().unwrap_or_default(),
            name: first.name.clone(),
        },
        right: Side {
            image: second.image.clone().unwrap_or_default(),
            // More than two picks in the stored set: the card names two and counts the rest, so
            // the line under it cannot look like a two-way comparison that arrived with more.
            name: if picks.len() > 2 {
                format!("{} +{}", second.name, picks.len() - 2)
            } else {
                second.name.clone()
            },
        },
        connector: Side {
            image: String::new(),
            name: String::new(),
        },
        connector_is_person: mode == Mode::Title,
        phrase: counted(mode, recent.matches),
    })
}

/// "12 actors in common", "1 title in common".
fn counted(mode: Mode, matches: i64) -> String {
    let noun = match (mode, matches == 1) {
        (Mode::Title, true) => "actor",
        (Mode::Title, false) => "actors",
        (Mode::Person, true) => "title",
        (Mode::Person, false) => "titles",
    };

    format!("{matches} {noun} in common")
}

async fn curated(state: &AppState, mode: Mode) -> Vec<Example> {
    let mut out = Vec::new();

    for entry in CURATED.iter().filter(|entry| entry.mode == mode) {
        if out.len() >= LIMIT {
            break;
        }

        let (Some(left), Some(right)) = (
            resolve(state, mode, entry.left).await,
            resolve(state, mode, entry.right).await,
        ) else {
            // A curated pair that cannot be resolved is left out rather than shown with a
            // placeholder: the card would be claiming a comparison that is not there.
            continue;
        };

        out.push(Example {
            href: paths::comparison(
                mode,
                &[
                    (left.kind, left.id, left.name.as_str()),
                    (right.kind, right.id, right.name.as_str()),
                ],
            ),
            left: Side {
                image: left.image,
                name: left.name,
            },
            right: Side {
                image: right.image,
                name: right.name,
            },
            connector: Side {
                image: String::new(),
                name: entry.connector.to_owned(),
            },
            connector_is_person: mode == Mode::Title,
            // The connector already reads as a sentence ("Keanu Reeves in both"); the phrase
            // slot is where a recent comparison puts its count instead.
            phrase: String::new(),
        });
    }

    out
}

/// Resolve a search term to the thing it names.
///
/// The name has to match what was asked for. Search is fuzzy by design — "Alien" leads with
/// *Alien* today and could lead with *Alien: Covenant* tomorrow — and a card whose connector line
/// says "Sigourney Weaver in both" is only true if the second side is really *Aliens*.
async fn resolve(state: &AppState, mode: Mode, term: &str) -> Option<Found> {
    let results = state.tmdb.search(mode, term).await.ok()?;

    results
        .into_iter()
        .find(|result| named(result.name.as_str(), term) && kind_fits(mode, result.kind))
        .map(|result| Found {
            id: result.id,
            kind: result.kind,
            name: result.name,
            image: result.image.unwrap_or_default(),
        })
}

fn kind_fits(mode: Mode, kind: Kind) -> bool {
    match mode {
        Mode::Title => matches!(kind, Kind::Movie | Kind::Tv),
        Mode::Person => kind == Kind::Person,
    }
}

/// Case-insensitive, whitespace-trimmed equality: "the matrix" is "The Matrix", "Alien" is not
/// "Alien: Covenant".
fn named(found: &str, wanted: &str) -> bool {
    found.trim().eq_ignore_ascii_case(wanted.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_matches_only_when_it_is_the_name() {
        assert!(named("The Matrix", "The Matrix"));
        assert!(named("the matrix", "  The Matrix "));
        assert!(!named("Alien: Covenant", "Alien"));
        assert!(!named("Alien", "Aliens"));
    }

    #[test]
    fn counting_says_actors_for_titles_and_titles_for_people() {
        assert_eq!(counted(Mode::Title, 12), "12 actors in common");
        assert_eq!(counted(Mode::Title, 1), "1 actor in common");
        assert_eq!(counted(Mode::Person, 5), "5 titles in common");
        assert_eq!(counted(Mode::Person, 1), "1 title in common");
    }

    /// Every curated pair is one mode's business, and both its sides are named for that mode.
    #[test]
    fn curated_pairs_are_complete_and_split_by_mode() {
        for entry in CURATED {
            assert!(
                !entry.left.trim().is_empty(),
                "a curated pair needs a left side"
            );
            assert!(
                !entry.right.trim().is_empty(),
                "a curated pair needs a right side"
            );
            assert!(
                !entry.connector.trim().is_empty(),
                "a curated pair needs a reason"
            );
            assert_ne!(entry.left, entry.right, "a pair of one thing is not a pair");
        }

        for mode in [Mode::Title, Mode::Person] {
            let count = CURATED.iter().filter(|entry| entry.mode == mode).count();
            assert!(
                count >= LIMIT,
                "{mode:?} has fewer curated pairs than cards"
            );
        }
    }

    #[test]
    fn a_stored_comparison_becomes_a_card_with_its_count() {
        let recent = crate::store::comparisons::Recent {
            picks_json: r#"[{"id":603,"kind":"movie","name":"The Matrix"},
                            {"id":604,"kind":"movie","name":"The Matrix Reloaded"}]"#
                .to_owned(),
            matches: 9,
        };

        let card = from_recent(Mode::Title, &recent).expect("two picks make a card");
        assert_eq!(card.left.name, "The Matrix");
        assert_eq!(card.right.name, "The Matrix Reloaded");
        assert_eq!(card.phrase, "9 actors in common");
        assert_eq!(
            card.href,
            "/title/movie-603-the-matrix,movie-604-the-matrix-reloaded"
        );
        assert!(
            card.connector_is_person,
            "the connector is a person in title mode"
        );
    }

    /// A set of three names two and counts the third, rather than pretending to be a pair.
    #[test]
    fn a_third_pick_is_counted_not_hidden() {
        let recent = crate::store::comparisons::Recent {
            picks_json: r#"[{"id":603,"kind":"movie","name":"The Matrix"},
                            {"id":604,"kind":"movie","name":"The Matrix Reloaded"},
                            {"id":605,"kind":"movie","name":"The Matrix Revolutions"}]"#
                .to_owned(),
            matches: 4,
        };

        let card = from_recent(Mode::Title, &recent).expect("three picks make a card");
        assert_eq!(card.right.name, "The Matrix Reloaded +1");
        assert!(
            card.href.ends_with(",movie-605-the-matrix-revolutions"),
            "the link still opens the whole set: {}",
            card.href
        );
    }

    /// One pick is not a comparison, and a card built from it would open a page with no answer.
    #[test]
    fn a_single_stored_pick_is_not_a_card() {
        let recent = crate::store::comparisons::Recent {
            picks_json: r#"[{"id":603,"kind":"movie","name":"The Matrix"}]"#.to_owned(),
            matches: 4,
        };

        assert!(from_recent(Mode::Title, &recent).is_none());
    }

    #[test]
    fn unreadable_picks_are_skipped_rather_than_fatal() {
        let recent = crate::store::comparisons::Recent {
            picks_json: "not json".to_owned(),
            matches: 4,
        };

        assert!(from_recent(Mode::Title, &recent).is_none());
    }
}

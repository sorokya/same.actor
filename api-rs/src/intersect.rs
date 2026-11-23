//! Pure intersection logic.
//!
//! Deliberately free of I/O: given the credits fetched for each selection, produce the
//! matches. That makes the one piece of real logic in this app testable without a network,
//! a database, or a running server — which is what the original `forEach`-and-`.some`
//! version inside a request handler never was.

use std::collections::{HashMap, HashSet};

use crate::domain::{Kind, PersonCredit, PersonMatch, TitleCredit, TitleMatch, MIN_SELECTIONS};

/// One person's appearance in one title, as fetched from TMDB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonClaim {
    pub id: u32,
    pub name: String,
    pub image: Option<String>,
    /// TV shows can credit one actor in several roles. Always a list.
    pub characters: Vec<String>,
}

/// One title's appearance in one person's filmography.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleClaim {
    pub id: u32,
    pub kind: Kind,
    pub name: String,
    pub year: Option<u16>,
    pub image: Option<String>,
    pub character: Option<String>,
}

#[derive(Debug)]
struct Accumulated<C> {
    name: String,
    image: Option<String>,
    credits: Vec<C>,
}

/// People who appear in at least [`MIN_SELECTIONS`] of the selected titles.
///
/// Ordered by how many of the selections they appear in (descending), then by name, so the
/// output is stable and the most-shared people are first.
pub fn intersect_people(per_selection: &[(u32, Vec<PersonClaim>)]) -> Vec<PersonMatch> {
    let mut acc: HashMap<u32, Accumulated<PersonCredit>> = HashMap::new();

    for (selection_id, claims) in per_selection {
        // A person listed twice in the same title (dual role in the raw credits) must not
        // count twice towards the threshold.
        let mut seen_in_this_selection: HashSet<u32> = HashSet::new();

        for claim in claims {
            if !seen_in_this_selection.insert(claim.id) {
                continue;
            }

            let entry = acc.entry(claim.id).or_insert_with(|| Accumulated {
                name: claim.name.clone(),
                image: claim.image.clone(),
                credits: Vec::new(),
            });

            // TMDB is inconsistent about which credit carries a portrait; keep the first.
            if entry.image.is_none() {
                entry.image.clone_from(&claim.image);
            }

            entry.credits.push(PersonCredit {
                selection: *selection_id,
                character: claim.characters.clone(),
            });
        }
    }

    let mut matches: Vec<PersonMatch> = acc
        .into_iter()
        .filter(|(_, a)| a.credits.len() >= MIN_SELECTIONS)
        .map(|(id, a)| PersonMatch {
            id,
            name: a.name,
            image: a.image,
            credits: a.credits,
        })
        .collect();

    for m in &mut matches {
        m.credits.sort_by_key(|c| c.selection);
    }
    matches.sort_by(|a, b| {
        b.credits
            .len()
            .cmp(&a.credits.len())
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    matches
}

/// Titles that feature at least [`MIN_SELECTIONS`] of the selected people.
pub fn intersect_titles(per_selection: &[(u32, Vec<TitleClaim>)]) -> Vec<TitleMatch> {
    let mut acc: HashMap<u32, Accumulated<TitleCredit>> = HashMap::new();

    for (selection_id, claims) in per_selection {
        let mut seen_in_this_selection: HashSet<u32> = HashSet::new();

        for claim in claims {
            if !seen_in_this_selection.insert(claim.id) {
                continue;
            }

            let entry = acc.entry(claim.id).or_insert_with(|| Accumulated {
                name: claim.name.clone(),
                image: claim.image.clone(),
                credits: Vec::new(),
            });

            if entry.image.is_none() {
                entry.image.clone_from(&claim.image);
            }

            entry.credits.push(TitleCredit {
                selection: *selection_id,
                character: claim.character.clone(),
            });
        }
    }

    let mut matches: Vec<TitleMatch> = acc
        .into_iter()
        .filter(|(_, a)| a.credits.len() >= MIN_SELECTIONS)
        .map(|(id, a)| TitleMatch {
            id,
            kind: Kind::Movie, // overwritten below from the claim
            name: a.name,
            year: None,
            image: a.image,
            credits: a.credits,
        })
        .collect();

    // `kind` and `year` live on the claim, not the accumulated credit, so re-derive them
    // from the first claim we saw for that title.
    let metadata: HashMap<u32, (Kind, Option<u16>)> = per_selection
        .iter()
        .flat_map(|(_, claims)| claims.iter())
        .map(|c| (c.id, (c.kind, c.year)))
        .collect();

    for m in &mut matches {
        if let Some((kind, year)) = metadata.get(&m.id) {
            m.kind = *kind;
            m.year = *year;
        }
    }

    for m in &mut matches {
        m.credits.sort_by_key(|c| c.selection);
    }
    matches.sort_by(|a, b| {
        b.credits
            .len()
            .cmp(&a.credits.len())
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: u32, name: &str) -> PersonClaim {
        PersonClaim {
            id,
            name: name.to_owned(),
            image: None,
            characters: vec!["Someone".to_owned()],
        }
    }

    fn title(id: u32, name: &str) -> TitleClaim {
        TitleClaim {
            id,
            kind: Kind::Movie,
            name: name.to_owned(),
            year: Some(1999),
            image: None,
            character: Some("Neo".to_owned()),
        }
    }

    #[test]
    fn a_person_in_both_titles_is_a_match() {
        let result = intersect_people(&[
            (603, vec![person(1, "Keanu"), person(2, "Laurence")]),
            (604, vec![person(1, "Keanu")]),
        ]);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, 1);
        assert_eq!(result[0].credits.len(), 2);
    }

    #[test]
    fn a_person_in_only_one_title_is_not_a_match() {
        let result = intersect_people(&[
            (603, vec![person(1, "Keanu")]),
            (604, vec![person(2, "Laurence")]),
        ]);
        assert!(result.is_empty());
    }

    /// The exact shape that used to return `null` and crash the process.
    #[test]
    fn a_single_selection_yields_no_matches_and_does_not_panic() {
        let result = intersect_people(&[(603, vec![person(1, "Keanu")])]);
        assert!(result.is_empty());
        let result = intersect_titles(&[(603, vec![title(1, "The Matrix")])]);
        assert!(result.is_empty());
    }

    /// An upstream failure used to produce `null` credits. An empty slice must behave.
    #[test]
    fn empty_credit_lists_yield_no_matches() {
        assert!(intersect_people(&[(603, vec![]), (604, vec![])]).is_empty());
        assert!(intersect_titles(&[(603, vec![]), (604, vec![])]).is_empty());
        assert!(intersect_people(&[]).is_empty());
        assert!(intersect_titles(&[]).is_empty());
    }

    #[test]
    fn a_person_listed_twice_in_one_title_counts_once() {
        let result = intersect_people(&[
            (603, vec![person(1, "Keanu"), person(1, "Keanu")]),
            (604, vec![person(2, "Laurence")]),
        ]);
        assert!(
            result.is_empty(),
            "duplicate within one selection must not reach the threshold"
        );
    }

    #[test]
    fn deep_casts_are_intersected_correctly() {
        let a: Vec<PersonClaim> = (0..500).map(|i| person(i, &format!("p{i}"))).collect();
        let b: Vec<PersonClaim> = (250..750).map(|i| person(i, &format!("p{i}"))).collect();

        let result = intersect_people(&[(1, a), (2, b)]);
        assert_eq!(result.len(), 250);
        assert!(result.iter().all(|m| (250..500).contains(&m.id)));
    }

    #[test]
    fn titles_are_deduplicated_across_people() {
        let result = intersect_titles(&[
            (6384, vec![title(603, "The Matrix"), title(604, "Reloaded")]),
            (530, vec![title(603, "The Matrix")]),
        ]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, 603);
        assert_eq!(result[0].name, "The Matrix");
        assert_eq!(result[0].kind, Kind::Movie);
        assert_eq!(result[0].year, Some(1999));
    }
}

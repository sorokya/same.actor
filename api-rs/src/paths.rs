//! Parsing for the human-readable comparison paths.
//!
//! Mirrors `src/lib/url.ts`. `/title/movie-603-the-matrix,tv-1396-breaking-bad` and
//! `/person/6384-keanu-reeves,530-carrie-anne-moss`: the id is authoritative, the slug is
//! decoration, and the kind is only written in `title` mode where a selection can be either.

use crate::domain::{Kind, Mode, Selection, MAX_SELECTIONS};

/// Parse the pick segment of a comparison path.
pub fn parse_picks(raw: &str, mode: Mode) -> Vec<Selection> {
    let mut picks: Vec<Selection> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();

    for segment in raw.split(',') {
        if picks.len() == MAX_SELECTIONS {
            break;
        }

        let mut rest = segment.trim();
        if rest.is_empty() {
            continue;
        }

        let mut kind = if mode == Mode::Person {
            Kind::Person
        } else {
            Kind::Movie
        };

        if mode == Mode::Title {
            if let Some(tail) = rest.strip_prefix("movie-") {
                kind = Kind::Movie;
                rest = tail;
            } else if let Some(tail) = rest.strip_prefix("tv-") {
                kind = Kind::Tv;
                rest = tail;
            } else {
                continue;
            }
        }

        // `603-the-matrix` and `603` are both valid; the slug is ignored. Anything that is
        // not a slug — `603abc` — is refused rather than partially understood.
        let digit_count = rest.chars().take_while(char::is_ascii_digit).count();
        if digit_count == 0 || digit_count > 10 {
            continue;
        }
        let (digits, tail) = rest.split_at(digit_count);
        if !tail.is_empty() && !tail.starts_with('-') {
            continue;
        }

        let Ok(id) = digits.parse::<u32>() else {
            continue;
        };
        if id == 0 || seen.contains(&id) {
            continue;
        }

        seen.push(id);
        picks.push(Selection {
            id: std::num::NonZeroU32::new(id).expect("checked non-zero"),
            kind,
        });
    }

    picks
}

/// The URL path for a set of picks, without slugs — the canonical form for `og:url`.
pub fn canonical(mode: Mode, picks: &[Selection]) -> String {
    let mode_segment = match mode {
        Mode::Title => "title",
        Mode::Person => "person",
    };

    let parts: Vec<String> = picks
        .iter()
        .map(|pick| match mode {
            Mode::Title => match pick.kind {
                Kind::Tv => format!("tv-{}", pick.id),
                _ => format!("movie-{}", pick.id),
            },
            Mode::Person => pick.id.to_string(),
        })
        .collect();

    format!("/{mode_segment}/{}", parts.join(","))
}

/// A title or person name reduced to the shape the client already uses.
///
/// `src/lib/url.ts` normalises with NFKD and strips combining marks; Rust has no such
/// normalisation without a dependency, so the common accented Latin letters are mapped by hand.
/// The two only have to agree well enough that a link does not look broken — the id in front of
/// the slug is what the parser reads, and it is authoritative.
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut dash = false;

    for character in name.chars() {
        let plain = match character {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
            'ñ' | 'ń' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
            'ý' | 'ÿ' => 'y',
            'ž' | 'ź' | 'ż' => 'z',
            'š' | 'ś' => 's',
            'ł' => 'l',
            'ß' => 's',
            other => other,
        };

        if let Some(letter) = plain
            .to_lowercase()
            .next()
            .filter(char::is_ascii_alphanumeric)
        {
            if dash && !out.is_empty() {
                out.push('-');
            }
            out.push(letter);
            dash = false;
        } else {
            dash = true;
        }
    }

    out.truncate(48);
    out.trim_end_matches('-').to_owned()
}

/// The path a comparison of these picks lives at, with slugs.
///
/// Mirrors `toPath` in `src/lib/url.ts`: the kind is written only in title mode, where a pick can
/// be a film or a show, and the id always comes before the slug so trimming the name still works.
pub fn comparison(mode: Mode, picks: &[(Kind, u32, &str)]) -> String {
    let segments: Vec<String> = picks
        .iter()
        .map(|(kind, id, name)| {
            let slug = slugify(name);
            let prefix = match mode {
                Mode::Title => format!("{}-", kind.as_str()),
                Mode::Person => String::new(),
            };

            if slug.is_empty() {
                format!("{prefix}{id}")
            } else {
                format!("{prefix}{id}-{slug}")
            }
        })
        .collect();

    format!("/{}/{}", mode.as_str(), segments.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_paths_carry_their_kind() {
        let picks = parse_picks("movie-603-the-matrix,tv-1396-breaking-bad", Mode::Title);
        assert_eq!(picks.len(), 2);
        assert_eq!(picks[0].id.get(), 603);
        assert_eq!(picks[0].kind, Kind::Movie);
        assert_eq!(picks[1].id.get(), 1396);
        assert_eq!(picks[1].kind, Kind::Tv);
    }

    #[test]
    fn person_paths_omit_the_kind() {
        let picks = parse_picks("6384-keanu-reeves,530-carrie-anne-moss", Mode::Person);
        assert_eq!(picks.len(), 2);
        assert!(picks.iter().all(|p| p.kind == Kind::Person));
        assert_eq!(picks[0].id.get(), 6384);
    }

    #[test]
    fn a_bare_id_is_enough() {
        let picks = parse_picks("movie-603,movie-604", Mode::Title);
        assert_eq!(
            picks.iter().map(|p| p.id.get()).collect::<Vec<_>>(),
            vec![603, 604]
        );
    }

    /// The slug must never be able to change which entity is loaded, and nothing outside the
    /// id may reach TMDB.
    #[test]
    fn junk_is_rejected_rather_than_guessed() {
        assert!(parse_picks("../../account", Mode::Title).is_empty());
        assert!(parse_picks("movie-../../account", Mode::Title).is_empty());
        assert!(parse_picks("603abc", Mode::Person).is_empty());
        assert!(parse_picks("movie-0", Mode::Title).is_empty());
        assert!(parse_picks("", Mode::Title).is_empty());
        assert!(parse_picks("movie-99999999999999", Mode::Title).is_empty());
        // A wrong kind prefix for the mode is refused, not coerced.
        assert!(parse_picks("person-6384", Mode::Title).is_empty());
    }

    #[test]
    fn duplicates_and_overflow_are_cut() {
        assert_eq!(parse_picks("603,603,604", Mode::Person).len(), 2);
        assert_eq!(
            parse_picks("1,2,3,4,5,6,7", Mode::Person).len(),
            MAX_SELECTIONS
        );
    }

    #[test]
    fn canonical_round_trips_through_the_parser() {
        let picks = parse_picks("movie-603-the-matrix,tv-1396-breaking-bad", Mode::Title);
        let path = canonical(Mode::Title, &picks);
        assert_eq!(path, "/title/movie-603,tv-1396");

        let reparsed = parse_picks(path.trim_start_matches("/title/"), Mode::Title);
        assert_eq!(reparsed, picks);
    }
}

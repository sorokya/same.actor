//! Wire types shared with the frontend.
//!
//! Two vocabularies are kept strictly apart, which is the fix for the review's S3:
//!
//! * [`Mode`] is what the *user* is comparing — titles or people.
//! * [`Kind`] is what a single *entity* is, in TMDB's own words — movie, tv, person.
//!
//! The old code overloaded one `mediaType` field for both senses, which is why the frontend
//! ended up comparing against `'person'` in a branch that could only ever see `'film'`.

use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// What the user is comparing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Compare titles → results are people they share.
    #[default]
    Title,
    /// Compare people → results are titles they share.
    Person,
}

/// What one selected entity is. Deserialised by serde from a closed set, so no other value
/// can ever reach a URL. This is the structural fix for the review's C1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Movie,
    Tv,
    Person,
}

impl Kind {
    pub const fn is_title(self) -> bool {
        matches!(self, Kind::Movie | Kind::Tv)
    }
}

/// Maximum selections the comparison accepts, per the original product.
pub const MAX_SELECTIONS: usize = 5;
/// Minimum selections that can meaningfully be compared.
pub const MIN_SELECTIONS: usize = 2;

/// One entry the user picked. `id` is a `NonZeroU32`, so a non-numeric id or a path
/// fragment cannot be represented, let alone interpolated into a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub id: NonZeroU32,
    pub kind: Kind,
}

/// A row in the search dropdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchResult {
    pub id: u32,
    pub kind: Kind,
    pub name: String,
    /// `None` when TMDB has no usable date. Previously this leaked as `NaN` → `null`.
    pub year: Option<u16>,
    pub overview: String,
    /// `None` when there is no artwork. The client owns the placeholder, so it no longer
    /// needs a hardcoded absolute URL back to its own domain.
    pub image: Option<String>,
}

/// One person who appears in at least two of the selected titles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonMatch {
    pub id: u32,
    pub name: String,
    pub image: Option<String>,
    pub credits: Vec<PersonCredit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonCredit {
    /// Which selection this credit came from.
    pub selection: u32,
    /// TV roles can be several. Always a list, so the client stops guessing.
    pub character: Vec<String>,
}

/// One title that features at least two of the selected people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TitleMatch {
    pub id: u32,
    pub kind: Kind,
    pub name: String,
    pub year: Option<u16>,
    pub image: Option<String>,
    pub credits: Vec<TitleCredit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TitleCredit {
    pub selection: u32,
    pub character: Option<String>,
}

/// One title or person the user picked, resolved to a label the client can render.
///
/// Returned alongside the matches so a shared URL is self-describing: the client stores only
/// `id`/`kind`, and gets the display metadata back from the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectionMeta {
    pub id: u32,
    pub kind: Kind,
    pub name: String,
    pub year: Option<u16>,
    pub image: Option<String>,
}

/// Response body for `POST /api/findout`. `matches` serialises untagged because `mode`
/// already discriminates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindOutResponse {
    pub mode: Mode,
    pub selections: Vec<SelectionMeta>,
    pub matches: Matches,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Matches {
    People(Vec<PersonMatch>),
    Titles(Vec<TitleMatch>),
}

/// `GET /api/search` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}

/// Parse the year out of a TMDB date string (`YYYY-MM-DD`).
///
/// TMDB returns `""` for unknown dates, and occasionally omits the field entirely. The old
/// code did `new Date(x).getFullYear() || 'N/A'` in one place and bare `.getFullYear()`
/// in another, producing `NaN` that `JSON.stringify` turned into `null` in a numeric field.
pub fn year_from_date(date: Option<&str>) -> Option<u16> {
    let date = date?;
    let year = date.get(0..4)?;
    let year: u16 = year.parse().ok()?;
    // Reject nonsense like "0000-00-00".
    (1800..=2200).contains(&year).then_some(year)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_parsing_handles_every_tmdb_shape() {
        assert_eq!(year_from_date(Some("1999-03-31")), Some(1999));
        assert_eq!(year_from_date(Some("")), None);
        assert_eq!(year_from_date(None), None);
        assert_eq!(year_from_date(Some("0000-00-00")), None);
        assert_eq!(year_from_date(Some("abcd-ef-gh")), None);
        assert_eq!(year_from_date(Some("199")), None);
        // Never a NaN, never a null in a numeric field.
    }

    #[test]
    fn kind_rejects_values_that_are_not_in_the_set() {
        assert!(serde_json::from_str::<Kind>("\"movie\"").is_ok());
        assert!(serde_json::from_str::<Kind>("\"tv\"").is_ok());
        assert!(serde_json::from_str::<Kind>("\"person\"").is_ok());
        // The exact payload that used to be interpolated into the TMDB URL.
        assert!(serde_json::from_str::<Kind>("\"../../account\"").is_err());
        assert!(serde_json::from_str::<Kind>("\"Movie\"").is_err());
    }

    #[test]
    fn selection_id_must_be_a_positive_integer() {
        assert!(serde_json::from_str::<Selection>(r#"{"id":603,"kind":"movie"}"#).is_ok());
        assert!(serde_json::from_str::<Selection>(r#"{"id":0,"kind":"movie"}"#).is_err());
        assert!(serde_json::from_str::<Selection>(r#"{"id":"603","kind":"movie"}"#).is_err());
        assert!(serde_json::from_str::<Selection>(r#"{"id":"603abc","kind":"movie"}"#).is_err());
        assert!(serde_json::from_str::<Selection>(r#"{"id":-1,"kind":"movie"}"#).is_err());
    }
}

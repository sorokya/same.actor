//! TMDB wire types.
//!
//! Only the fields this service actually uses. Everything is `Option` where TMDB is known to
//! omit fields, and unknown fields are ignored — TMDB adds fields without notice and a strict
//! schema here would turn their additive change into our outage.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SearchMulti {
    #[serde(default)]
    pub results: Vec<MultiResult>,
}

#[derive(Debug, Deserialize)]
pub struct MultiResult {
    pub id: u32,
    pub media_type: Option<String>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub profile_path: Option<String>,
    /// Present on `/search/person` results.
    #[serde(default)]
    pub known_for: Vec<MultiResult>,
}

/// `/person/{id}/combined_credits`.
#[derive(Debug, Deserialize)]
pub struct CombinedCredits {
    #[serde(default)]
    pub cast: Vec<CombinedCastCredit>,
}

#[derive(Debug, Deserialize)]
pub struct CombinedCastCredit {
    pub id: u32,
    pub media_type: Option<String>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub character: Option<String>,
    pub poster_path: Option<String>,
}

/// `/movie/{id}/credits` and `/tv/{id}/aggregate_credits`.
#[derive(Debug, Deserialize)]
pub struct Credits {
    #[serde(default)]
    pub cast: Vec<CastMember>,
    /// Only read for directors, which the admin's franchise suggestions use. A title with no
    /// crew here costs nothing: the field defaults to empty.
    #[serde(default)]
    pub crew: Vec<CrewMember>,
}

#[derive(Debug, Deserialize)]
pub struct CrewMember {
    pub id: u32,
    #[serde(default)]
    pub job: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CastMember {
    pub id: u32,
    pub name: Option<String>,
    /// Movie credits put the role here.
    pub character: Option<String>,
    /// TV aggregate credits put one entry per role here instead.
    #[serde(default)]
    pub roles: Vec<Role>,
    pub profile_path: Option<String>,
    /// Billing position, 0-based: `0` is the name above the title. This is the difference
    /// between a collaboration and a walk-on, and it is the only signal TMDB gives for it.
    #[serde(default)]
    pub order: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct Role {
    pub character: Option<String>,
}

/// `/movie/{id}?append_to_response=credits` and `/tv/{id}?append_to_response=aggregate_credits`.
///
/// One upstream call returns both the title's metadata and its cast. The metadata is what
/// lets a shared URL render its own column headers — the old API only ever returned ids, so
/// a comparison could not survive a refresh.
#[derive(Debug, Deserialize)]
pub struct MediaDetail {
    pub id: u32,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub poster_path: Option<String>,
    pub credits: Option<Credits>,
    pub aggregate_credits: Option<Credits>,
    /// The franchise TMDB thinks this belongs to. A *suggestion* for the admin's queue: a
    /// collection is narrower than a franchise (Iron Man, The Avengers and Guardians are three
    /// collections and one franchise), so this never decides anything on its own.
    #[serde(default)]
    pub belongs_to_collection: Option<Collection>,
    #[serde(default)]
    pub keywords: Option<KeywordList>,
}

#[derive(Debug, Deserialize)]
pub struct Collection {
    pub id: u32,
}

/// TMDB nests keywords differently by media type: films return `keywords`, shows return
/// `results`. One struct reads both, so callers see a single shape.
#[derive(Debug, Default, Deserialize)]
pub struct KeywordList {
    #[serde(default)]
    pub keywords: Vec<Keyword>,
    #[serde(default)]
    pub results: Vec<Keyword>,
}

#[derive(Debug, Deserialize)]
pub struct Keyword {
    pub id: u32,
}

impl KeywordList {
    /// Every keyword id, whichever nest it arrived in, in a stable order and without duplicates.
    pub fn ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .keywords
            .iter()
            .chain(self.results.iter())
            .map(|keyword| keyword.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

impl MediaDetail {
    pub fn display_name(&self) -> String {
        self.title
            .clone()
            .or_else(|| self.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("#{}", self.id))
    }

    pub fn year(&self) -> Option<u16> {
        crate::domain::year_from_date(
            self.release_date
                .as_deref()
                .or(self.first_air_date.as_deref()),
        )
    }

    /// The collection TMDB groups this title under, when it has one.
    pub fn collection_id(&self) -> Option<u32> {
        self.belongs_to_collection
            .as_ref()
            .map(|collection| collection.id)
    }

    pub fn keyword_ids(&self) -> Vec<u32> {
        self.keywords
            .as_ref()
            .map(KeywordList::ids)
            .unwrap_or_default()
    }

    /// Directors of a film. Shows list episode directors per episode rather than in their
    /// aggregate credits, so this is usually empty for them — the suggestion it feeds is a
    /// heuristic, and a heuristic with no signal says nothing.
    pub fn director_ids(&self) -> Vec<u32> {
        let crew = self
            .credits
            .as_ref()
            .or(self.aggregate_credits.as_ref())
            .map(|credits| credits.crew.as_slice())
            .unwrap_or_default();

        let mut ids: Vec<u32> = crew
            .iter()
            .filter(|member| member.job.as_deref() == Some("Director"))
            .map(|member| member.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub fn cast(&self) -> &[CastMember] {
        self.credits
            .as_ref()
            .or(self.aggregate_credits.as_ref())
            .map(|c| c.cast.as_slice())
            .unwrap_or_default()
    }
}

/// `/person/{id}?append_to_response=combined_credits`.
#[derive(Debug, Deserialize)]
pub struct PersonDetail {
    pub id: u32,
    pub name: Option<String>,
    pub profile_path: Option<String>,
    pub combined_credits: Option<CombinedCredits>,
}

impl CastMember {
    /// One actor, one title, possibly several roles. Normalised to a list so the contract
    /// no longer varies by media type (review M8).
    pub fn characters(&self) -> Vec<String> {
        let from_roles: Vec<String> = self
            .roles
            .iter()
            .filter_map(|r| r.character.as_deref())
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_owned)
            .collect();

        if !from_roles.is_empty() {
            return from_roles;
        }

        self.character
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| vec![c.to_owned()])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Films nest keywords under `keywords`, shows under `results`. One reader, both shapes.
    #[test]
    fn keywords_are_read_from_either_nest() {
        let film: MediaDetail =
            serde_json::from_str(r#"{"id":1,"keywords":{"keywords":[{"id":9},{"id":3}]}}"#)
                .expect("a film detail");
        assert_eq!(film.keyword_ids(), [3, 9], "sorted and deduplicated");

        let show: MediaDetail =
            serde_json::from_str(r#"{"id":2,"keywords":{"results":[{"id":5}]}}"#)
                .expect("a show detail");
        assert_eq!(show.keyword_ids(), [5]);

        let none: MediaDetail = serde_json::from_str(r#"{"id":3}"#).expect("a bare detail");
        assert!(none.keyword_ids().is_empty());
        assert!(none.collection_id().is_none());
        assert!(none.director_ids().is_empty());
    }

    #[test]
    fn directors_come_from_the_crew_and_nobody_else() {
        let detail: MediaDetail = serde_json::from_str(
            r#"{"id":603,"credits":{"cast":[{"id":6384,"order":0}],
                 "crew":[{"id":7,"job":"Director"},{"id":8,"job":"Producer"},{"id":7,"job":"Director"}]}}"#,
        )
        .expect("a detail with a crew");

        assert_eq!(
            detail.director_ids(),
            [7],
            "one entry, whatever the credits say"
        );
        assert_eq!(detail.cast()[0].order, Some(0), "billing order is parsed");
    }

    #[test]
    fn a_collection_is_a_suggestion_and_arrives_as_an_id() {
        let detail: MediaDetail = serde_json::from_str(
            r#"{"id":603,"belongs_to_collection":{"id":2344,"name":"The Matrix Collection"}}"#,
        )
        .expect("a detail in a collection");

        assert_eq!(detail.collection_id(), Some(2344));
    }

    #[test]
    fn movie_credits_roles_come_from_character() {
        let m: CastMember = serde_json::from_str(
            r#"{"id":1,"name":"Keanu Reeves","character":"Neo","profile_path":"/x.jpg"}"#,
        )
        .unwrap();
        assert_eq!(m.characters(), vec!["Neo"]);
    }

    #[test]
    fn tv_aggregate_credits_roles_come_from_the_roles_array() {
        let m: CastMember = serde_json::from_str(
            r#"{"id":1,"name":"Actor","roles":[{"character":"A"},{"character":"B"}]}"#,
        )
        .unwrap();
        assert_eq!(m.characters(), vec!["A", "B"]);
    }

    #[test]
    fn a_credit_with_no_role_is_an_empty_list_not_a_placeholder() {
        let m: CastMember = serde_json::from_str(r#"{"id":1,"name":"Self"}"#).unwrap();
        assert!(m.characters().is_empty());
    }

    #[test]
    fn unknown_fields_do_not_break_deserialisation() {
        // TMDB adds fields routinely; an additive change must not be an outage.
        let m: CastMember = serde_json::from_str(
            r#"{"id":1,"name":"X","new_field":{"a":1},"adult":false,"order":3}"#,
        )
        .unwrap();
        assert_eq!(m.id, 1);
    }

    #[test]
    fn missing_cast_is_an_empty_cast_not_an_error() {
        let c: Credits = serde_json::from_str(r#"{"id":603,"crew":[]}"#).unwrap();
        assert!(c.cast.is_empty());
    }
}

/// `/movie/popular`, `/tv/on_the_air`, and friends. One shape covers all of them: the fields
/// that differ (movie vs tv titles and dates) are optional and read through helpers.
#[derive(Debug, Deserialize)]
pub struct ListResponse {
    #[serde(default)]
    pub results: Vec<ListItem>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ListItem {
    pub id: u32,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    /// Present on TV list responses. "What is on air" is a question about the last episode, not
    /// about when the show started.
    pub last_air_date: Option<String>,
    pub poster_path: Option<String>,
    pub overview: Option<String>,
    pub vote_average: Option<f64>,
    /// Only collection parts carry this: `/collection/{id}` mixes films and shows in one list.
    #[serde(default)]
    pub media_type: Option<String>,
}

/// `/collection/{id}`. Parts are list items because they are, in every field that matters.
#[derive(Debug, Deserialize)]
pub struct CollectionDetail {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub parts: Vec<ListItem>,
}

impl ListItem {
    pub fn display_name(&self) -> String {
        self.title
            .clone()
            .or_else(|| self.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("#{}", self.id))
    }

    pub fn year(&self) -> Option<u16> {
        crate::domain::year_from_date(
            self.release_date
                .as_deref()
                .or(self.first_air_date.as_deref()),
        )
    }
}

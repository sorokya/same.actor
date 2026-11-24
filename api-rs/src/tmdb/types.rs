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

#[derive(Debug, Deserialize)]
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

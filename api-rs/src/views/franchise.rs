//! The franchise admin: the list, one franchise, the unassigned queue, and cameo overrides.
//!
//! Everything here is a server-rendered page with plain forms. The dashboard loads no script, and
//! curation is the last place to start: it is a hundred small decisions made once, and a form that
//! posts and redirects is the least machinery that records them.
//!
//! The colour of a franchise is its position in this palette, not a column in the table. A curated
//! palette is how a twenty-column matrix ends up with two purples nobody can tell apart; position
//! cannot.

use askama::Template;

use crate::store::franchises::{Assigned, Franchise};

use super::Head;

/// Eight hues: the site's five tints first, then three that stay distinguishable against them.
const PALETTE: [&str; 8] = [
    "#e8b14b", "#3ecfc0", "#f2799e", "#a996ff", "#b6d15c", "#6fb3e8", "#e8845c", "#9bd1a8",
];

pub fn colour(sort_order: i64) -> &'static str {
    PALETTE[sort_order.rem_euclid(PALETTE.len() as i64) as usize]
}

pub struct FranchiseRow {
    pub slug: String,
    pub name: String,
    pub id: i64,
    pub titles: usize,
    pub colour: &'static str,
}

#[derive(Template)]
#[template(path = "pages/admin/franchises.html")]
pub struct FranchisesPage {
    pub head: Head,
    pub rows: Vec<FranchiseRow>,
    pub unassigned: usize,
    pub error: String,
    pub message: String,
}

impl FranchisesPage {
    pub fn new(head: Head, rows: &[Franchise], unassigned: usize) -> Self {
        Self {
            head,
            rows: rows
                .iter()
                .map(|franchise| FranchiseRow {
                    slug: franchise.slug.clone(),
                    name: franchise.name.clone(),
                    id: franchise.id,
                    titles: franchise.titles,
                    colour: colour(franchise.sort_order),
                })
                .collect(),
            unassigned,
            error: String::new(),
            message: String::new(),
        }
    }
}

/// One title on a franchise page.
pub struct TitleRow {
    pub kind: String,
    pub id: u32,
    pub name: String,
    pub poster: String,
    pub person: bool,
}

#[derive(Template)]
#[template(path = "pages/admin/franchise.html")]
pub struct FranchisePage {
    pub head: Head,
    pub slug: String,
    pub name: String,
    pub id: i64,
    pub colour: &'static str,
    pub rows: Vec<TitleRow>,
    /// The search box's current value, so a search survives being submitted.
    pub query: String,
    pub results: Vec<TitleRow>,
    /// The collection an import would bring in. Empty when there is nothing to offer, because a
    /// template cannot format an `Option` into a URL.
    pub collection_id: String,
    pub collection_name: String,
    pub collection_parts: usize,
    pub error: String,
    pub message: String,
}

impl FranchisePage {
    pub fn new(head: Head, franchise: &Franchise, titles: &[Assigned]) -> Self {
        Self {
            head,
            slug: franchise.slug.clone(),
            name: franchise.name.clone(),
            id: franchise.id,
            colour: colour(franchise.sort_order),
            rows: titles.iter().map(title_row).collect(),
            query: String::new(),
            results: Vec::new(),
            collection_id: String::new(),
            collection_name: String::new(),
            collection_parts: 0,
            error: String::new(),
            message: String::new(),
        }
    }
}

fn title_row<A: TitleLike>(assigned: &A) -> TitleRow {
    TitleRow {
        kind: assigned.kind().to_owned(),
        id: assigned.id(),
        name: assigned.name().to_owned(),
        poster: assigned.poster().unwrap_or_default().to_owned(),
        person: false,
    }
}

/// What a title row needs, whether it came from the database or from a search.
pub trait TitleLike {
    fn kind(&self) -> &str;
    fn id(&self) -> u32;
    fn name(&self) -> &str;
    fn poster(&self) -> Option<&str>;
}

impl TitleLike for Assigned {
    fn kind(&self) -> &str {
        &self.kind
    }

    fn id(&self) -> u32 {
        self.media_id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn poster(&self) -> Option<&str> {
        self.poster.as_deref()
    }
}

/// A suggestion on the queue: one click, and the title joins that franchise.
pub struct Suggestion {
    pub franchise_id: i64,
    pub slug: String,
    pub name: String,
    pub reason: String,
    pub colour: &'static str,
    pub kind: String,
    pub media_id: u32,
    pub title: String,
    pub poster: String,
}

pub struct QueueRow {
    pub kind: String,
    pub id: u32,
    pub name: String,
    pub year: String,
    pub poster: String,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Template)]
#[template(path = "pages/admin/queue.html")]
pub struct QueuePage {
    pub head: Head,
    pub rows: Vec<QueueRow>,
    pub unassigned: usize,
    pub error: String,
    pub message: String,
}

/// One actor on the cameo page: the cutoff's verdict, and the override when there is one.
pub struct CastRow {
    pub person_id: u32,
    pub name: String,
    pub image: String,
    /// Billing position, or empty when TMDB did not say.
    pub order: String,
    pub cameo: bool,
    /// Whether this came from an override rather than the cutoff.
    pub overridden: bool,
}

#[derive(Template)]
#[template(path = "pages/admin/cameos.html")]
pub struct CameosPage {
    pub head: Head,
    pub slug: String,
    pub franchise_name: String,
    pub title_name: String,
    pub kind: String,
    pub media_id: u32,
    pub cutoff: u32,
    pub rows: Vec<CastRow>,
    pub error: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_cycles_rather_than_running_out() {
        assert_eq!(colour(0), PALETTE[0]);
        assert_eq!(colour(7), PALETTE[7]);
        assert_eq!(colour(8), PALETTE[0], "positions wrap");
        assert_eq!(
            colour(-1),
            PALETTE[7],
            "a negative order still lands on a hue"
        );
    }
}

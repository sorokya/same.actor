//! The derived model behind the synergy views.
//!
//! Everything here is computed from two things a refresh already stored — the list's titles and
//! the table of who appears in which of them — plus the curated franchises. No upstream calls, and
//! no state: `build` is a pure function of its inputs, which is what makes the filters cheap
//! enough to run per request instead of caching a matrix per combination.
//!
//! Two decisions worth knowing:
//!
//!   - **Titles are ordered by franchise, then by the list's own ranking.** That is what makes the
//!     staircase appear: a Marvel column sits next to the other Marvel columns, so an actor in
//!     three of them draws a long run and an actor who bridges Marvel and Tarantino draws one that
//!     crosses a block boundary.
//!   - **Actors are ordered by where their run starts.** Reading down the left edge gives the
//!     titles in order, so the page reads as a story rather than as an alphabet.

use std::collections::{HashMap, HashSet};

use crate::discovery::SynergyRow;
use crate::domain::Kind;
use crate::store::franchises::Placement;

/// Billing positions at or beyond this are treated as cameos unless an override says otherwise.
///
/// A starting guess, not a measurement: it was picked to be past the leads and the strong
/// supporting cast of a twenty-title list. Being wrong is cheap — the override table exists — and
/// the cutoff is off by default, so a wrong guess never hides anyone without being asked.
pub const CAMEO_CUTOFF: u32 = 15;

/// The colour of a title in no franchise: neutral, so it reads as "unclassified" rather than as a
/// group somebody chose.
pub const UNGROUPED: &str = "#8a8f98";

/// Eight hues by position, the same palette the admin assigns. Position, not a stored colour: a
/// curated palette is how a twenty-column matrix ends up with two purples nobody can tell apart.
pub const PALETTE: [&str; 8] = [
    "#e8b14b", "#3ecfc0", "#f2799e", "#a996ff", "#b6d15c", "#6fb3e8", "#e8845c", "#9bd1a8",
];

pub fn colour(sort_order: i64) -> &'static str {
    PALETTE[sort_order.rem_euclid(PALETTE.len() as i64) as usize]
}

/// How many actors a page shows.
///
/// The page is about people — "who keeps turning up" — so the limit is on actors, not on titles.
/// Twenty of them bring in every title they appear in, which is what makes a franchise appear as
/// a block of columns rather than being thinned out of existence: the cast is the subject and the
/// titles are evidence. A hundred titles is fine; a hundred *rows* is not a page.
pub const MAX_ROWS: usize = 20;

/// What the reader has switched off.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    /// Franchise slugs to hide. Empty means everything is visible, which is the default state of
    /// a chip that is "on".
    pub hide: HashSet<String>,
    /// Whether to drop billed-late appearances, which is what makes a walk-on look like a
    /// collaboration.
    pub hide_cameos: bool,
    /// Whether to show every actor instead of the first `MAX_ROWS`.
    ///
    /// The cap is a reading decision, not a filter — it hides nothing about the data — but it is
    /// reached the same way, from the URL, so it travels with the rest of the switches.
    pub all: bool,
}

impl Filter {
    pub fn hides(&self, slug: &str) -> bool {
        self.hide.contains(slug)
    }
}

/// One franchise's run of columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    /// Empty for the ungrouped block, which has no page to link to.
    pub slug: String,
    pub colour: &'static str,
    /// How many columns it spans.
    pub span: usize,
}

/// A title, as a column of the matrix.
///
/// `PartialEq` without `Eq`: the rating is an `f64`, and a column is compared in tests rather
/// than used as a key.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub id: u32,
    pub kind: Kind,
    pub name: String,
    pub poster: Option<String>,
    /// The date this list is about ("1 Dec 2026"), when it is about one.
    pub date: Option<String>,
    pub year: Option<u16>,
    pub rating: Option<f64>,
    /// The franchise group this column belongs to, as an index into `Matrix::groups`.
    pub group: usize,
    /// 1-based grid column for the template: the actor names take the first one.
    pub position: usize,
    /// Whether this column opens a cast cluster outside the curated franchises, which draws a
    /// rule before it: those clusters have no name to label them with.
    pub starts_cluster: bool,
}

/// An actor, as a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub person_id: u32,
    pub name: String,
    pub image: Option<String>,
    /// Column indices this actor appears in, ascending.
    pub titles: Vec<usize>,
    /// "6 in 3 franchises", for the row's tooltip.
    pub bridges: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Matrix {
    /// The groups the columns are drawn in, after filtering.
    pub groups: Vec<Group>,
    /// Every franchise represented in this list, hidden ones included. The filter chips are
    /// built from this: a chip that disappears when it is switched off leaves no way back.
    pub franchises: Vec<Group>,
    pub columns: Vec<Column>,
    pub actors: Vec<Actor>,
    /// How many actors the cap left out.
    pub hidden_actors: usize,
    /// Grid columns where a cast cluster starts, so a rule can be drawn before it.
    pub breaks: Vec<usize>,
}

impl Matrix {
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty() || self.actors.is_empty()
    }
}

/// An appearance of one person in one title, as the stored table records it.
struct Appearance {
    column: usize,
}

/// Build the matrix.
///
/// `overrides` is keyed by `(kind, media id, person id)` and holds the maintainer's cameo flags;
/// `placements` is the curated franchise of each title, keyed the same way minus the person.
pub fn build(
    kind: Kind,
    items: &[crate::discovery::Item],
    rows: &[crate::discovery::SynergyRow],
    placements: &HashMap<(String, u32), Placement>,
    overrides: &HashMap<(String, u32, u32), bool>,
    filter: &Filter,
) -> Matrix {
    let kind_key = kind.as_str();

    // Every franchise the list touches, before any filter is applied.
    let mut franchises: Vec<Group> = Vec::new();
    for item in items {
        let Some(placement) = placements.get(&(kind_key.to_owned(), item.id)) else {
            continue;
        };

        if franchises.iter().any(|group| group.slug == placement.slug) {
            continue;
        }

        franchises.push(Group {
            name: placement.name.clone(),
            slug: placement.slug.clone(),
            colour: colour(placement.sort_order),
            span: 0,
        });
    }

    // Titles that survive the franchise filter, keeping the list's ranking as the tie-break.
    let mut visible: Vec<(usize, Option<&Placement>)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let placement = placements.get(&(kind_key.to_owned(), item.id));

        if let Some(placement) = placement {
            if filter.hides(&placement.slug) {
                continue;
            }
        }

        // `None` sorts after every real franchise: ungrouped titles are the tail, not a block
        // that interrupts the curated ones.
        visible.push((index, placement));
    }

    // Curated titles first, grouped by franchise in the order the franchises were curated.
    let mut curated: Vec<(usize, Option<&Placement>)> = visible
        .iter()
        .copied()
        .filter(|(_, placement)| placement.is_some())
        .collect();
    curated.sort_by_key(|(index, placement)| {
        (
            placement.map(|placement| placement.sort_order).unwrap_or(0),
            *index,
        )
    });

    // Everything no franchise claims: ordered by who shares a cast with whom. The list's own
    // ranking scatters an actor's two credits to opposite ends of the grid, which turns a
    // connection into a line across twenty empty cells.
    let rank: HashMap<u32, usize> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id, index))
        .collect();
    let unclaimed: Vec<u32> = visible
        .iter()
        .filter(|(_, placement)| placement.is_none())
        .map(|(index, _)| items[*index].id)
        .collect();
    let index_of_id: HashMap<u32, usize> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id, index))
        .collect();

    let mut ordered: Vec<(usize, Option<&Placement>, bool)> = curated
        .iter()
        .map(|(index, placement)| (*index, *placement, false))
        .collect();

    for (id, starts_cluster) in order_by_shared_cast(&unclaimed, rows, &rank) {
        let Some(index) = index_of_id.get(&id).copied() else {
            continue;
        };
        ordered.push((index, None, starts_cluster));
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut columns: Vec<Column> = Vec::with_capacity(ordered.len());

    for (visible_index, (item_index, placement, starts_cluster)) in ordered.iter().enumerate() {
        let item = &items[*item_index];
        let placement = *placement;

        let group = match placement {
            Some(placement) => match groups.iter().position(|group| group.slug == placement.slug) {
                Some(existing) => existing,
                None => {
                    groups.push(Group {
                        name: placement.name.clone(),
                        slug: placement.slug.clone(),
                        colour: colour(placement.sort_order),
                        span: 0,
                    });
                    groups.len() - 1
                }
            },
            None => {
                let last = groups.last();
                match last {
                    Some(group) if group.slug.is_empty() => groups.len() - 1,
                    _ => {
                        groups.push(Group {
                            name: "Other".to_owned(),
                            slug: String::new(),
                            colour: UNGROUPED,
                            span: 0,
                        });
                        groups.len() - 1
                    }
                }
            }
        };

        groups[group].span += 1;
        columns.push(Column {
            id: item.id,
            kind,
            name: item.name.clone(),
            poster: item.poster.clone(),
            date: item.date.clone(),
            year: item.year,
            rating: item.rating,
            group,
            position: visible_index + 2,
            starts_cluster: *starts_cluster,
        });
    }

    // The stored table refers to titles by id; map each id to the column it landed in.
    let column_of_id: HashMap<u32, usize> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| (column.id, index))
        .collect();

    let mut actors: Vec<Actor> = Vec::new();

    for row in rows {
        let mut appearances: Vec<Appearance> = Vec::new();

        for title in &row.titles {
            let Some(column) = column_of_id.get(&title.id).copied() else {
                // The title is hidden, or it was not in this list at all.
                continue;
            };

            let override_flag = overrides.get(&(kind_key.to_owned(), title.id, row.person_id));
            let cameo = override_flag
                .copied()
                .unwrap_or_else(|| is_cameo(title.order));

            if filter.hide_cameos && cameo {
                continue;
            }

            appearances.push(Appearance { column });
        }

        // Two titles is what "appears more than once" means; one is an actor passing through.
        if appearances.len() < 2 {
            continue;
        }

        appearances.sort_by_key(|appearance| appearance.column);

        let franchises: HashSet<usize> = appearances
            .iter()
            .filter_map(|appearance| columns.get(appearance.column).map(|column| column.group))
            .collect();

        actors.push(Actor {
            person_id: row.person_id,
            name: row.name.clone(),
            image: row.image.clone(),
            titles: appearances.iter().map(|a| a.column).collect(),
            bridges: franchises.len(),
        });
    }

    // The staircase: whoever starts furthest left is read first.
    actors.sort_by(|a, b| {
        a.titles
            .first()
            .cmp(&b.titles.first())
            .then(b.titles.len().cmp(&a.titles.len()))
            .then(a.name.cmp(&b.name))
    });

    // Twenty people is what a page can read, and the twenty are chosen by credits: on a
    // seventy-nine-actor list, someone in three titles is a better row than someone in two, and
    // the rest are one link away. The cap selects; it does not reorder — the survivors go back
    // into reading order, which the selection sort has just scrambled.
    let hidden_actors = if filter.all {
        0
    } else {
        let total = actors.len();

        actors.sort_by(|a, b| {
            b.titles
                .len()
                .cmp(&a.titles.len())
                .then(a.name.cmp(&b.name))
        });
        actors.truncate(MAX_ROWS);
        actors.sort_by(|a, b| {
            a.titles
                .first()
                .cmp(&b.titles.first())
                .then(b.titles.len().cmp(&a.titles.len()))
                .then(a.name.cmp(&b.name))
        });

        total.saturating_sub(MAX_ROWS)
    };

    // The columns are whatever these actors appear in: nothing is dropped for having a crowd,
    // because a crowd is a franchise and a franchise is the answer to "who keeps turning up".
    let used: HashSet<usize> = actors
        .iter()
        .flat_map(|actor| actor.titles.iter().copied())
        .collect();

    if used.len() == columns.len() {
        return Matrix {
            groups,
            franchises,
            breaks: cluster_breaks(&columns),
            columns,
            actors,
            hidden_actors,
        };
    }

    let mut kept: Vec<usize> = (0..columns.len())
        .filter(|index| used.contains(index))
        .collect();
    kept.sort_unstable();

    // Old column index → new column index, so the dots and the runs move with their titles.
    let moved: HashMap<usize, usize> = kept
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new))
        .collect();

    let mut compacted_groups: Vec<Group> = Vec::new();
    let mut group_moved: HashMap<usize, usize> = HashMap::new();
    let mut compacted_columns: Vec<Column> = Vec::with_capacity(kept.len());

    for (new_index, old_index) in kept.iter().enumerate() {
        let column = &columns[*old_index];

        // Columns keep their group order, so a group's new id is just the order it appears in.
        let group = *group_moved.entry(column.group).or_insert_with(|| {
            let mut group = groups[column.group].clone();
            group.span = 0;
            compacted_groups.push(group);
            compacted_groups.len() - 1
        });

        compacted_groups[group].span += 1;
        compacted_columns.push(Column {
            group,
            position: new_index + 2,
            ..column.clone()
        });
    }

    for actor in &mut actors {
        actor.titles = actor
            .titles
            .iter()
            .filter_map(|index| moved.get(index).copied())
            .collect();
    }

    Matrix {
        groups: compacted_groups,
        franchises,
        breaks: cluster_breaks(&compacted_columns),
        columns: compacted_columns,
        actors,
        hidden_actors,
    }
}

/// The grid columns where an unlabelled cast cluster starts.
fn cluster_breaks(columns: &[Column]) -> Vec<usize> {
    columns
        .iter()
        .filter(|column| column.starts_cluster)
        .map(|column| column.position)
        .collect()
}

/// Order the titles no franchise claims so that titles sharing a cast sit side by side.
///
/// A trending list has no franchises, so curated order says nothing about it, and the ranking
/// puts an actor's two credits wherever it likes — a dot at each end of the grid and a line
/// across twenty empty cells, which reads as noise rather than as a connection.
///
/// Titles are nodes; an edge joins two titles that share a repeat actor, weighted by how many.
/// Connected clusters are ordered largest first — most repeat actors — and inside a cluster the
/// walk starts at the least connected title and always takes its strongest unvisited neighbour.
/// The start matters: it keeps the loose end of a chain on the left, which is where the eye
/// starts, and it stops two clusters from being traversed from the middle outwards.
///
/// Deterministic throughout, ties broken by the list's own rank and then by id: an order that
/// changes between two renders of the same data is a bug you only ever see in a screenshot.
///
/// Returns each title with whether it opens a cluster, so the template can draw the rule between
/// them.
fn order_by_shared_cast(
    titles: &[u32],
    rows: &[SynergyRow],
    rank: &HashMap<u32, usize>,
) -> Vec<(u32, bool)> {
    if titles.is_empty() {
        return Vec::new();
    }

    let allowed: HashSet<u32> = titles.iter().copied().collect();
    let mut weight: HashMap<(u32, u32), usize> = HashMap::new();
    // Each person's titles inside this set, so a cluster can be counted in people, not edges.
    let mut people: Vec<Vec<u32>> = Vec::new();

    for row in rows {
        let mut mine: Vec<u32> = row
            .titles
            .iter()
            .map(|title| title.id)
            .filter(|id| allowed.contains(id))
            .collect();
        mine.sort_unstable();
        mine.dedup();

        if mine.len() < 2 {
            continue;
        }

        for first in 0..mine.len() {
            for second in first + 1..mine.len() {
                *weight.entry((mine[first], mine[second])).or_default() += 1;
            }
        }

        people.push(mine);
    }

    // Connected clusters, by union-find over the edges.
    let mut parent: HashMap<u32, u32> = titles.iter().map(|id| (*id, *id)).collect();

    for (left, right) in weight.keys() {
        let (a, b) = (root(&mut parent, *left), root(&mut parent, *right));
        if a != b {
            // The smaller id wins, so the parent table does not depend on insertion order.
            let (low, high) = if a < b { (a, b) } else { (b, a) };
            parent.insert(high, low);
        }
    }

    let mut clusters: HashMap<u32, Vec<u32>> = HashMap::new();
    for id in titles {
        clusters
            .entry(root(&mut parent, *id))
            .or_default()
            .push(*id);
    }

    // How many repeat actors each cluster holds, which is what orders it against the others.
    let mut in_cluster: HashMap<u32, usize> = HashMap::new();
    for mine in &people {
        if let Some(first) = mine.first() {
            *in_cluster.entry(root(&mut parent, *first)).or_default() += 1;
        }
    }

    let mut order: Vec<(u32, Vec<u32>)> = clusters.into_iter().collect();
    order.sort_by_key(|(id, members)| {
        (
            std::cmp::Reverse(in_cluster.get(id).copied().unwrap_or(0)),
            members
                .iter()
                .map(|id| at(rank, *id))
                .min()
                .unwrap_or(usize::MAX),
            *id,
        )
    });

    let mut neighbours: HashMap<u32, HashMap<u32, usize>> = HashMap::new();
    for ((left, right), count) in &weight {
        neighbours.entry(*left).or_default().insert(*right, *count);
        neighbours.entry(*right).or_default().insert(*left, *count);
    }

    let mut out: Vec<(u32, bool)> = Vec::with_capacity(titles.len());

    for (_, members) in order {
        let mut left: HashSet<u32> = members.iter().copied().collect();
        let mut walked: HashSet<u32> = HashSet::new();
        // The loose end first: fewest connections, then whichever the list ranks higher.
        let mut current = *left
            .iter()
            .min_by_key(|id| {
                (
                    neighbours.get(id).map(HashMap::len).unwrap_or(0),
                    at(rank, **id),
                    **id,
                )
            })
            .expect("a cluster has at least one member");

        let mut first = true;

        loop {
            left.remove(&current);
            walked.insert(current);
            out.push((current, first));
            first = false;

            if left.is_empty() {
                break;
            }

            let next = best_next(&neighbours, &left, current, rank).or_else(|| {
                // A walk can run out of unvisited neighbours — a cycle closes on itself — and has
                // to rejoin the rest of the cluster at whatever links back most strongly.
                left.iter()
                    .min_by_key(|id| {
                        (
                            std::cmp::Reverse(links_into(&neighbours, &walked, **id)),
                            at(rank, **id),
                            **id,
                        )
                    })
                    .copied()
            });

            match next {
                Some(id) => current = id,
                None => break,
            }
        }
    }

    out
}

/// The unvisited title most strongly linked to the one being walked from.
fn best_next(
    neighbours: &HashMap<u32, HashMap<u32, usize>>,
    left: &HashSet<u32>,
    from: u32,
    rank: &HashMap<u32, usize>,
) -> Option<u32> {
    neighbours.get(&from).and_then(|links| {
        links
            .iter()
            .filter(|(id, _)| left.contains(id))
            .min_by_key(|(id, count)| (std::cmp::Reverse(**count), at(rank, **id), **id))
            .map(|(id, _)| *id)
    })
}

/// How strongly a title is linked into the part of its cluster already walked.
fn links_into(
    neighbours: &HashMap<u32, HashMap<u32, usize>>,
    walked: &HashSet<u32>,
    id: u32,
) -> usize {
    neighbours
        .get(&id)
        .map(|links| {
            links
                .iter()
                .filter(|(other, _)| walked.contains(other))
                .map(|(_, count)| *count)
                .sum()
        })
        .unwrap_or(0)
}

/// A title's position in the list, for tie-breaking only.
fn at(rank: &HashMap<u32, usize>, id: u32) -> usize {
    rank.get(&id).copied().unwrap_or(usize::MAX)
}

/// The root of a union-find node, compressing the path as it goes.
fn root(parent: &mut HashMap<u32, u32>, id: u32) -> u32 {
    let mut current = id;
    while parent.get(&current).copied().unwrap_or(current) != current {
        current = parent[&current];
    }

    let mut walk = id;
    while parent.get(&walk).copied().unwrap_or(walk) != walk {
        let next = parent[&walk];
        parent.insert(walk, current);
        walk = next;
    }

    current
}

/// The cutoff's verdict: anyone billed at or beyond it looks like a walk-on.
pub fn is_cameo(order: Option<u32>) -> bool {
    match order {
        // TMDB did not say where they are billed, so there is no evidence of a cameo.
        None => false,
        Some(order) => order >= CAMEO_CUTOFF,
    }
}

/// The people who share two or more titles with one actor, strongest first.
pub fn co_stars(matrix: &Matrix, person_id: u32) -> Vec<(u32, String, usize)> {
    let Some(subject) = matrix.actors.iter().find(|a| a.person_id == person_id) else {
        return Vec::new();
    };

    let theirs: HashSet<usize> = subject.titles.iter().copied().collect();

    let mut shared: Vec<(u32, String, usize)> = matrix
        .actors
        .iter()
        .filter(|actor| actor.person_id != person_id)
        .filter_map(|actor| {
            let count = actor
                .titles
                .iter()
                .filter(|title| theirs.contains(title))
                .count();
            (count >= 2).then(|| (actor.person_id, actor.name.clone(), count))
        })
        .collect();

    shared.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
    shared
}

/// An actor, as an axis of the pairs grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairActor {
    pub person_id: u32,
    pub name: String,
    pub image: Option<String>,
    /// Their own title count — the diagonal, and the answer to "how much of this list is theirs".
    pub total: usize,
}

/// One square of the lower triangle: the later actor's row, the earlier actor's column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairCell {
    pub row: usize,
    pub col: usize,
    pub shared: usize,
}

/// The actor × actor grid, lower triangle only, with the diagonal left to the caller.
///
/// A full matrix would say everything twice — the pair of two actors is one fact, not two — and
/// the triangle is also what makes the grid readable: with actors ordered by where their run
/// starts, a cluster of co-stars shows up as a block against the diagonal.
#[derive(Debug, Clone, Default)]
pub struct Pairs {
    pub actors: Vec<PairActor>,
    /// Every pair, row by row from the top, columns left to right. Zero shares included: the
    /// empty squares are what make the shape of the triangle visible.
    pub cells: Vec<PairCell>,
    /// The largest share in the grid, so the template can scale brightness to the data rather
    /// than to a number nobody has.
    pub most: usize,
}

/// Build the grid from the same rows the matrix draws, in the same order.
pub fn pairs(matrix: &Matrix) -> Pairs {
    let actors: Vec<PairActor> = matrix
        .actors
        .iter()
        .map(|actor| PairActor {
            person_id: actor.person_id,
            name: actor.name.clone(),
            image: actor.image.clone(),
            total: actor.titles.len(),
        })
        .collect();

    let mut cells = Vec::with_capacity(actors.len() * actors.len() / 2);
    let mut most = 0;

    for row in 0..matrix.actors.len() {
        for col in 0..row {
            let shared = shared_titles(&matrix.actors[row].titles, &matrix.actors[col].titles);
            most = most.max(shared);
            cells.push(PairCell { row, col, shared });
        }
    }

    Pairs {
        actors,
        cells,
        most,
    }
}

/// How many titles two actors have in common.
///
/// Both lists are ascending column indices, so this walks them together instead of building a
/// set per pair: a hundred titles and sixty actors is eighteen hundred pairs, and eighteen
/// hundred sets is a lot of allocation to answer a question two cursors can.
fn shared_titles(one: &[usize], other: &[usize]) -> usize {
    let (mut a, mut b) = (0, 0);
    let mut shared = 0;

    while a < one.len() && b < other.len() {
        match one[a].cmp(&other[b]) {
            std::cmp::Ordering::Less => a += 1,
            std::cmp::Ordering::Greater => b += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                a += 1;
                b += 1;
            }
        }
    }

    shared
}

/// The actors who span more than one franchise: the links the flat list could not show.
pub fn bridges(matrix: &Matrix) -> Vec<(u32, String, usize)> {
    let mut out: Vec<(u32, String, usize)> = matrix
        .actors
        .iter()
        .filter(|actor| actor.bridges >= 2)
        .map(|actor| (actor.person_id, actor.name.clone(), actor.titles.len()))
        .collect();

    out.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{Item, SynergyRow, TitleRef};

    fn item(id: u32, name: &str) -> Item {
        Item {
            id,
            name: name.to_owned(),
            year: None,
            date: None,
            poster: None,
            overview: None,
            rating: None,
            collection_id: None,
            director_ids: Vec::new(),
            keyword_ids: Vec::new(),
        }
    }

    fn row(person_id: u32, name: &str, titles: &[(u32, Option<u32>)]) -> SynergyRow {
        SynergyRow {
            person_id,
            name: name.to_owned(),
            image: None,
            count: titles.len(),
            titles: titles
                .iter()
                .map(|(id, order)| TitleRef {
                    id: *id,
                    kind: Kind::Movie,
                    name: format!("Title {id}"),
                    order: *order,
                })
                .collect(),
        }
    }

    fn placement(id: u32, slug: &str, name: &str, sort_order: i64) -> ((String, u32), Placement) {
        (
            ("movie".to_owned(), id),
            Placement {
                franchise_id: sort_order + 1,
                slug: slug.to_owned(),
                name: name.to_owned(),
                sort_order,
            },
        )
    }

    fn build_default(
        items: &[Item],
        rows: &[SynergyRow],
        placements: &HashMap<(String, u32), Placement>,
    ) -> Matrix {
        build(
            Kind::Movie,
            items,
            rows,
            placements,
            &HashMap::new(),
            &Filter::default(),
        )
    }

    /// A list with no franchises — a trending list — used to keep its own ranking, which scatters
    /// an actor's two credits to opposite ends of the grid. Sorting by shared cast is what turns
    /// those two dots into a run you can read.
    #[test]
    fn titles_that_share_a_cast_sit_together() {
        // Ranked 1..6, but the casts pair 1 with 4, 2 with 5, 3 with 6.
        let items = (1..=6).map(|id| item(id, "A title")).collect::<Vec<_>>();
        let rows = vec![
            row(10, "First", &[(1, None), (4, None)]),
            row(11, "Second", &[(2, None), (5, None)]),
            row(12, "Third", &[(3, None), (6, None)]),
        ];

        let matrix = build_default(&items, &rows, &HashMap::new());

        let kept: Vec<u32> = matrix.columns.iter().map(|column| column.id).collect();
        assert_eq!(
            kept,
            [1, 4, 2, 5, 3, 6],
            "each actor's two titles are neighbours, in the order the list ranked the pairs"
        );
        assert!(
            matrix.actors.iter().all(|actor| {
                let gap = actor.titles[1] - actor.titles[0];
                gap == 1
            }),
            "and every run is one column wide"
        );
    }

    /// The biggest cluster comes first: it holds the most repeat actors, so it is what the page
    /// is about.
    #[test]
    fn the_cluster_holding_the_most_people_comes_first() {
        let items = (1..=4).map(|id| item(id, "A title")).collect::<Vec<_>>();
        let rows = vec![
            row(10, "A", &[(3, None), (4, None)]),
            row(11, "B", &[(3, None), (4, None)]),
            row(12, "C", &[(1, None), (2, None)]),
        ];

        let matrix = build_default(&items, &rows, &HashMap::new());

        let kept: Vec<u32> = matrix.columns.iter().map(|column| column.id).collect();
        assert_eq!(kept, [3, 4, 1, 2], "two people beat one");
    }

    /// A chain is walked from its loose end, not from wherever the list happens to rank it.
    ///
    /// Every column has at least one link — a title only becomes a column because somebody
    /// appears in it twice — so the walk never starts on a stray. What it must not do is start
    /// in the middle of a chain, which would put both ends on the right and read backwards.
    #[test]
    fn a_chain_is_walked_from_its_loose_end() {
        // Ranked B, A, C, D; the cast runs A-B, B-C, C-D.
        let items = vec![item(2, "B"), item(1, "A"), item(3, "C"), item(4, "D")];
        let rows = vec![
            row(10, "AB", &[(1, None), (2, None)]),
            row(11, "BC", &[(2, None), (3, None)]),
            row(12, "CD", &[(3, None), (4, None)]),
        ];

        let matrix = build_default(&items, &rows, &HashMap::new());

        let kept: Vec<u32> = matrix.columns.iter().map(|column| column.id).collect();
        assert_eq!(kept, [1, 2, 3, 4], "A to D, not from the middle outwards");
    }

    /// The rules between clusters are drawn where a cluster starts, not before everything.
    #[test]
    fn cluster_breaks_mark_where_a_new_cluster_starts() {
        let items = (1..=4).map(|id| item(id, "A title")).collect::<Vec<_>>();
        let rows = vec![
            row(10, "A", &[(1, None), (2, None)]),
            row(11, "B", &[(3, None), (4, None)]),
        ];

        let matrix = build_default(&items, &rows, &HashMap::new());

        assert_eq!(
            matrix.breaks,
            vec![2, 4],
            "one before the first cluster and one before the second, past the name column"
        );
    }

    /// The cap keeps the people who turn up most, not whoever the staircase listed first: on a
    /// long list those are different twenty, and the three-credit actor is the better row.
    #[test]
    fn the_cap_keeps_the_actors_with_the_most_credits() {
        let items = (1..=6).map(|id| item(id, "A title")).collect::<Vec<_>>();

        let mut rows: Vec<SynergyRow> = (0..MAX_ROWS as u32)
            .map(|index| {
                row(
                    100 + index,
                    &format!("Passing {index:02}"),
                    &[(1, None), (2, None)],
                )
            })
            .collect();
        // Four actors in three titles each, ranked last by every other measure.
        rows.extend((0..4).map(|index| {
            row(
                900 + index,
                &format!("Recurring {index}"),
                &[(3, None), (4, None), (5, None)],
            )
        }));

        let matrix = build_default(&items, &rows, &HashMap::new());

        assert_eq!(matrix.actors.len(), MAX_ROWS);
        let kept: Vec<&str> = matrix
            .actors
            .iter()
            .map(|actor| actor.name.as_str())
            .collect();
        for index in 0..4 {
            assert!(
                kept.contains(&format!("Recurring {index}").as_str()),
                "three titles beats two: kept {kept:?}"
            );
        }
    }

    #[test]
    fn the_actor_cap_lifts_when_the_reader_asks() {
        let items = (1..=40).map(|id| item(id, "A title")).collect::<Vec<_>>();
        let rows = (0..MAX_ROWS as u32 + 3)
            .map(|index| {
                let start = index % 20 + 1;
                row(
                    index,
                    &format!("Actor {index:02}"),
                    &[(start, None), (start + 10, None), (start + 20, None)],
                )
            })
            .collect::<Vec<_>>();

        let capped = build_default(&items, &rows, &HashMap::new());
        assert_eq!(capped.actors.len(), MAX_ROWS);
        assert_eq!(capped.hidden_actors, 3);

        let all = build(
            Kind::Movie,
            &items,
            &rows,
            &HashMap::new(),
            &HashMap::new(),
            &Filter {
                all: true,
                ..Filter::default()
            },
        );

        assert_eq!(all.actors.len(), MAX_ROWS + 3, "everyone, when asked");
        assert_eq!(all.hidden_actors, 0, "and nothing left to announce");
    }

    #[test]
    fn titles_group_by_franchise_and_actors_order_by_where_their_run_starts() {
        let items = vec![
            item(1, "Iron Man"),
            item(2, "Pulp Fiction"),
            item(3, "The Avengers"),
        ];
        let placements: HashMap<_, _> = [
            placement(1, "marvel", "Marvel", 0),
            placement(3, "marvel", "Marvel", 0),
        ]
        .into_iter()
        .collect();

        let rows = vec![
            row(10, "Late starter", &[(2, None), (3, None)]),
            row(11, "From the start", &[(1, None), (3, None)]),
        ];

        let matrix = build_default(&items, &rows, &placements);

        let names: Vec<&str> = matrix.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["Iron Man", "The Avengers", "Pulp Fiction"],
            "Marvel first, ungrouped last"
        );

        let actors: Vec<&str> = matrix.actors.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            actors,
            ["From the start", "Late starter"],
            "leftmost run reads first"
        );

        assert_eq!(matrix.groups.len(), 2);
        assert_eq!(matrix.groups[0].span, 2);
        assert_eq!(matrix.groups[1].span, 1);
        assert_eq!(matrix.groups[1].slug, "", "the ungrouped block has no page");
    }

    #[test]
    fn hiding_a_franchise_drops_its_columns_and_the_actors_left_with_one() {
        let items = vec![
            item(1, "Iron Man"),
            item(2, "Pulp Fiction"),
            item(3, "The Avengers"),
        ];
        let placements: HashMap<_, _> = [
            placement(1, "marvel", "Marvel", 0),
            placement(3, "marvel", "Marvel", 0),
        ]
        .into_iter()
        .collect();

        let rows = vec![
            row(10, "Only Marvel", &[(1, None), (3, None)]),
            row(11, "Bridges", &[(1, None), (2, None)]),
        ];

        let filter = Filter {
            hide: ["marvel".to_owned()].into_iter().collect(),
            hide_cameos: false,
            all: false,
        };

        let matrix = build(
            Kind::Movie,
            &items,
            &rows,
            &placements,
            &HashMap::new(),
            &filter,
        );

        assert_eq!(matrix.actors.len(), 0, "nobody appears twice any more");
        assert_eq!(
            matrix.columns.len(),
            0,
            "and a column with no repeat cast goes with them"
        );
    }

    /// Switching a chip off must leave the chip: a control that vanishes cannot be undone.
    #[test]
    fn a_hidden_franchise_is_still_listed_as_a_chip() {
        let items = vec![
            item(1, "Iron Man"),
            item(2, "The Avengers"),
            item(3, "Pulp Fiction"),
        ];
        let placements: HashMap<_, _> = [
            placement(1, "marvel", "Marvel", 0),
            placement(2, "marvel", "Marvel", 0),
        ]
        .into_iter()
        .collect();

        let filter = Filter {
            hide: ["marvel".to_owned()].into_iter().collect(),
            hide_cameos: false,
            all: false,
        };

        let matrix = build(
            Kind::Movie,
            &items,
            &[],
            &placements,
            &HashMap::new(),
            &filter,
        );

        assert!(matrix.groups.iter().all(|group| group.slug != "marvel"));
        assert_eq!(matrix.columns.len(), 0, "what is left has nothing to show");
        assert_eq!(
            matrix.franchises.len(),
            1,
            "but the franchise is still offered"
        );
        assert_eq!(matrix.franchises[0].slug, "marvel");
    }

    #[test]
    fn hiding_cameos_drops_the_appearance_not_the_actor() {
        let items = vec![item(1, "A"), item(2, "B"), item(3, "C")];
        let rows = vec![row(
            10,
            "Walk-on",
            &[(1, Some(0)), (2, Some(1)), (3, Some(40))],
        )];

        let visible = build_default(&items, &rows, &HashMap::new());
        assert_eq!(visible.actors[0].titles.len(), 3);

        let filter = Filter {
            hide: HashSet::new(),
            hide_cameos: true,
            all: false,
        };

        let filtered = build(
            Kind::Movie,
            &items,
            &rows,
            &HashMap::new(),
            &HashMap::new(),
            &filter,
        );
        assert_eq!(
            filtered.actors[0].titles.len(),
            2,
            "the third appearance is a cameo and goes"
        );
    }

    /// The override is the point: a lead billed late is not a walk-on, and a star in one scene is.
    #[test]
    fn an_override_beats_the_cutoff_in_both_directions() {
        let items = vec![item(1, "A"), item(2, "B")];
        let rows = vec![row(10, "Late lead", &[(1, Some(40)), (2, Some(41))])];

        let filter = Filter {
            hide: HashSet::new(),
            hide_cameos: true,
            all: false,
        };

        let without = build(
            Kind::Movie,
            &items,
            &rows,
            &HashMap::new(),
            &HashMap::new(),
            &filter,
        );
        assert_eq!(without.actors.len(), 0, "the cutoff hides both");

        let overrides: HashMap<(String, u32, u32), bool> = [
            (("movie".to_owned(), 1, 10), false),
            (("movie".to_owned(), 2, 10), false),
        ]
        .into_iter()
        .collect();

        let with = build(
            Kind::Movie,
            &items,
            &rows,
            &HashMap::new(),
            &overrides,
            &filter,
        );
        assert_eq!(with.actors.len(), 1, "'not a cameo' wins over the cutoff");
    }

    #[test]
    fn a_bridge_is_an_actor_spanning_two_franchises() {
        let items = vec![
            item(1, "Iron Man"),
            item(2, "Pulp Fiction"),
            item(3, "The Avengers"),
        ];
        let placements: HashMap<_, _> = [
            placement(1, "marvel", "Marvel", 0),
            placement(3, "marvel", "Marvel", 0),
        ]
        .into_iter()
        .collect();

        let rows = vec![
            row(10, "Yields to nothing", &[(1, None), (3, None)]),
            row(11, "Yields to everything", &[(1, None), (2, None)]),
        ];

        let matrix = build_default(&items, &rows, &placements);
        let links = bridges(&matrix);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].1, "Yields to everything");
    }

    #[test]
    fn co_stars_are_the_people_sharing_two_or_more_titles() {
        let items = vec![item(1, "A"), item(2, "B")];
        let rows = vec![
            row(10, "Subject", &[(1, None), (2, None)]),
            row(11, "Shares two", &[(1, None), (2, None)]),
        ];

        let matrix = build_default(&items, &rows, &HashMap::new());
        let friends = co_stars(&matrix, 10);

        assert_eq!(friends.len(), 1);
        assert_eq!(friends[0].1, "Shares two");
        assert_eq!(friends[0].2, 2);
    }

    /// One shared title is a crossing, not a partnership. Under the shared-cast cap the two
    /// cases cannot sit in the same list — a third actor in a title is what the cap removes —
    /// so this is its own list.
    #[test]
    fn one_shared_title_is_not_a_co_star() {
        let items = vec![item(1, "A"), item(2, "B"), item(3, "C")];
        let rows = vec![
            row(10, "Subject", &[(1, None), (2, None)]),
            row(11, "Crosses once", &[(2, None), (3, None)]),
        ];

        let matrix = build_default(&items, &rows, &HashMap::new());

        assert_eq!(co_stars(&matrix, 10).len(), 0);
    }

    /// The page budgets actors, not titles. A franchise with six recurring faces is the most
    /// interesting thing a list has — a title is not dropped for having a crowd in it.
    #[test]
    fn a_crowded_title_is_kept() {
        let items = (1..=3).map(|id| item(id, "A title")).collect::<Vec<_>>();
        let rows = (0..6)
            .map(|index| {
                row(
                    10 + index,
                    &format!("Actor {index}"),
                    &[(1, None), (2, None), (3, None)],
                )
            })
            .collect::<Vec<_>>();

        let matrix = build_default(&items, &rows, &HashMap::new());

        let kept: Vec<u32> = matrix.columns.iter().map(|column| column.id).collect();
        assert_eq!(
            kept,
            [1, 2, 3],
            "six faces in three films is a franchise, not a crowd"
        );
        assert_eq!(matrix.actors.len(), 6);
    }

    /// Twenty actors, and the titles they appear in — all of them. The rest are counted, not
    /// silently dropped.
    #[test]
    fn the_page_spends_its_budget_on_actors() {
        let items = (1..=40).map(|id| item(id, "A title")).collect::<Vec<_>>();
        // Each actor is in three titles of their own, so every column has exactly one face and
        // nothing is thinned for any other reason.
        let rows = (0..MAX_ROWS as u32 + 5)
            .map(|index| {
                let start = index % 20 + 1;
                let titles = vec![(start, None), (start + 10, None), (start + 20, None)];
                row(index, &format!("Actor {index:02}"), &titles)
            })
            .collect::<Vec<_>>();

        let matrix = build_default(&items, &rows, &HashMap::new());

        assert_eq!(matrix.actors.len(), MAX_ROWS, "twenty people, not sixty");
        assert_eq!(matrix.hidden_actors, 5, "and the rest are counted");
        assert!(
            matrix.columns.len() > MAX_ROWS,
            "twenty actors can bring in more than twenty titles: {}",
            matrix.columns.len()
        );
    }

    #[test]
    fn the_palette_cycles_and_never_runs_out() {
        assert_eq!(colour(0), PALETTE[0]);
        assert_eq!(colour(8), PALETTE[0]);
        assert_eq!(colour(-3), PALETTE[5]);
    }

    /// A title nobody appears in twice is dropped: the list can be long precisely because the
    /// matrix is not.
    #[test]
    fn a_title_with_no_repeat_cast_is_not_a_column() {
        let items = vec![
            item(1, "Shared"),
            item(2, "Shared too"),
            item(3, "Nobody twice"),
        ];
        let rows = vec![row(10, "Someone", &[(1, None), (2, None)])];

        let matrix = build_default(&items, &rows, &HashMap::new());

        let names: Vec<&str> = matrix.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["Shared", "Shared too"],
            "the third title has no story here"
        );
        assert_eq!(
            matrix.actors[0].titles,
            vec![0, 1],
            "and the dots moved with their titles"
        );
        assert!(matrix.columns.iter().all(|c| c.position >= 2));
    }

    #[test]
    fn an_empty_list_is_empty_rather_than_wrong() {
        let matrix = build_default(&[], &[], &HashMap::new());
        assert!(matrix.is_empty());
        assert!(matrix.groups.is_empty());
    }
}

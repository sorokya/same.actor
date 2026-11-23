//! Open Graph share cards.
//!
//! Renders a 1200×630 PNG per comparison so a link to `same.actor/title/…` unfurls into
//! something worth clicking. The card is built as an SVG string and rasterised with resvg;
//! fonts are vendored into the binary so the image is identical in every environment and the
//! runtime needs no font package.

use std::sync::{Arc, OnceLock};

use fontdb::Database;
use resvg::tiny_skia;
use resvg::usvg;

const SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
const SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");
const MONO: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");

pub const WIDTH: u32 = 1200;
pub const HEIGHT: u32 = 630;

/// Palette shared with `src/lib/palette.ts`, so the card matches the site it links to.
pub const TINTS: [&str; 5] = ["#e8b14b", "#3ecfc0", "#f2799e", "#a996ff", "#b6d15c"];

// Colours are constants rather than literals inside the SVG template: a raw string cannot
// hold the sequence `quote-hash` that every `fill="#0b0b0d"` attribute contains.
const INK: &str = "#0b0b0d";
const LINE: &str = "#26262b";
const PAPER: &str = "#ececed";
const DIM: &str = "#98989f";
const ACCENT: &str = TINTS[0];

fn fonts() -> Arc<Database> {
    static DB: OnceLock<Arc<Database>> = OnceLock::new();

    DB.get_or_init(|| {
        let mut db = Database::new();
        for data in [SANS, SANS_BOLD, MONO] {
            db.load_font_data(data.to_vec());
        }
        db.set_sans_serif_family("DejaVu Sans");
        db.set_monospace_family("DejaVu Sans Mono");
        Arc::new(db)
    })
    .clone()
}

/// One entry in the card's list of selections.
pub struct Pick {
    pub name: String,
    pub year: Option<u16>,
}

/// Everything the card says. Built by the route from the same metadata the page uses.
pub struct Card {
    pub heading: String,
    pub picks: Vec<Pick>,
    /// The big number: how many shared credits the comparison found.
    pub count: usize,
    /// What that number counts, e.g. `"ACTORS IN AT LEAST TWO"`.
    pub count_label: &'static str,
    pub mode_label: &'static str,
}

/// Escape text for inclusion in SVG character data.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Truncate to a character budget with an ellipsis.
///
/// SVG has no text wrapping and resvg will happily paint text straight off the canvas, so
/// long titles are cut to a width the layout can guarantee.
fn ellipsize(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn svg(card: &Card) -> String {
    let mut body = String::new();

    body.push_str(&format!(
        r#"<text x="64" y="88" font-family="DejaVu Sans Mono" font-size="26" letter-spacing="4" fill="{PAPER}">SAME<tspan fill="{ACCENT}">.</tspan>ACTOR</text>"#
    ));

    body.push_str(&format!(
        r#"<text x="{x}" y="88" text-anchor="end" font-family="DejaVu Sans Mono" font-size="20" letter-spacing="5" fill="{DIM}">{mode}</text>"#,
        x = WIDTH - 64,
        mode = esc(card.mode_label)
    ));

    body.push_str(&format!(
        r#"<rect x="64" y="116" width="1072" height="1" fill="{LINE}"/>"#
    ));

    // The heading is a question, not a repeat of the list: the list is drawn separately on
    // the right, at a size that survives a 300px thumbnail.
    for (line, y) in wrap(&card.heading, 24).into_iter().take(2).zip([182, 238]) {
        body.push_str(&format!(
            r#"<text x="64" y="{y}" font-family="DejaVu Sans" font-size="46" fill="{PAPER}">{text}</text>"#,
            text = esc(&line)
        ));
    }

    // The count, neutral rather than tinted so it reads as the total rather than as one
    // selection's.
    body.push_str(&format!(
        r#"<text x="64" y="470" font-family="DejaVu Sans" font-weight="bold" font-size="176" fill="{PAPER}">{count}</text>"#,
        count = card.count
    ));

    body.push_str(&format!(
        r#"<text x="70" y="516" font-family="DejaVu Sans Mono" font-size="30" letter-spacing="2" fill="{PAPER}" fill-opacity="0.72">{label}</text>"#,
        label = esc(card.count_label)
    ));

    // The selections, stacked: five rows of 24px text stay legible in a small embed, where
    // five side-by-side pills would not.
    for (index, pick) in card.picks.iter().take(5).enumerate() {
        let tint = TINTS[index % TINTS.len()];
        let y = 156 + index as i64 * 58;

        body.push_str(&format!(
            r#"<rect x="664" y="{bar_y}" width="4" height="30" rx="2" fill="{tint}"/><text x="684" y="{ty}" font-family="DejaVu Sans" font-size="25" fill="{PAPER}">{name}</text>"#,
            bar_y = y - 22,
            ty = y,
            name = esc(&ellipsize(&pick.name, 30))
        ));

        if let Some(year) = pick.year {
            body.push_str(&format!(
                r#"<text x="{x}" y="{ty}" font-family="DejaVu Sans Mono" font-size="19" fill="{DIM}">{year}</text>"#,
                x = WIDTH - 64,
                ty = y,
                year = year
            ));
        }
    }

    body.push_str(&format!(
        r#"<rect x="64" y="576" width="1072" height="1" fill="{LINE}"/><text x="64" y="608" font-family="DejaVu Sans Mono" font-size="17" letter-spacing="2" fill="{DIM}">CREDITS FROM TMDB</text>"#
    ));

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}"><defs><radialGradient id="a" cx="0.08" cy="0" r="0.95"><stop offset="0" stop-color="{ACCENT}" stop-opacity="0.18"/><stop offset="1" stop-color="{ACCENT}" stop-opacity="0"/></radialGradient><radialGradient id="b" cx="0.99" cy="0.06" r="0.72"><stop offset="0" stop-color="{TINTS_1}" stop-opacity="0.15"/><stop offset="1" stop-color="{TINTS_1}" stop-opacity="0"/></radialGradient></defs><rect width="{WIDTH}" height="{HEIGHT}" fill="{INK}"/><rect width="{WIDTH}" height="{HEIGHT}" fill="url(#a)"/><rect width="{WIDTH}" height="{HEIGHT}" fill="url(#b)"/><rect x="0" y="0" width="{WIDTH}" height="6" fill="{ACCENT}"/>{body}</svg>"#,
        TINTS_1 = TINTS[1],
        body = body
    )
}

/// Greedy word wrap. SVG has no wrapping of its own, and a heading that runs off the canvas
/// is worse than one split across two lines.
fn wrap(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

/// Rasterise a card to PNG bytes.
pub fn render(card: &Card) -> Result<Vec<u8>, String> {
    let markup = svg(card);

    let mut options = usvg::Options::default();
    options.fontdb = fonts();
    options.font_family = "DejaVu Sans".to_owned();

    let tree = usvg::Tree::from_str(&markup, &options).map_err(|e| format!("invalid SVG: {e}"))?;

    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT)
        .ok_or_else(|| format!("could not allocate a {WIDTH}x{HEIGHT} pixmap"))?;

    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());

    pixmap
        .encode_png()
        .map_err(|e| format!("could not encode PNG: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Card {
        Card {
            heading: "The Matrix · The Matrix Reloaded".into(),
            picks: vec![
                Pick {
                    name: "The Matrix".into(),
                    year: Some(1999),
                },
                Pick {
                    name: "The Matrix Reloaded".into(),
                    year: Some(2003),
                },
            ],
            count: 33,
            count_label: "ACTORS IN AT LEAST TWO",
            mode_label: "TITLES",
        }
    }

    #[test]
    fn renders_a_png_of_the_expected_size() {
        let png = render(&card()).expect("render");

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
        assert!(png.len() > 5_000, "suspiciously small: {} bytes", png.len());

        let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!((width, height), (WIDTH, HEIGHT));
    }

    /// Text must actually reach the canvas. If font loading failed, resvg draws nothing and
    /// the PNG compresses down to a flat gradient — which a byte count catches.
    #[test]
    fn text_is_actually_rasterised() {
        let with_text = render(&card()).expect("render");

        let mut bare = card();
        bare.heading = String::new();
        bare.picks.clear();
        bare.count_label = "";
        let without_text = render(&bare).expect("render");

        assert!(
            with_text.len() > without_text.len() + 2_000,
            "text did not render: {} vs {} bytes",
            with_text.len(),
            without_text.len()
        );
    }

    #[test]
    fn long_titles_are_truncated_not_overflowed() {
        assert_eq!(ellipsize("short", 10), "short");
        let long = "The Extraordinarily Long And Winding Title Of A Film Nobody Can Fit";
        let cut = ellipsize(long, 12);
        assert_eq!(cut.chars().count(), 12);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn markup_is_escaped() {
        assert_eq!(esc("a & b <c>"), "a &amp; b &lt;c&gt;");
    }

    #[test]
    fn a_card_with_five_picks_and_no_dates_still_renders() {
        let mut card = card();
        card.picks = (0..5)
            .map(|i| Pick {
                name: format!("Pick number {i} with a fairly long name"),
                year: None,
            })
            .collect();
        assert!(render(&card).is_ok());
    }
}

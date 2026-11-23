//! Open Graph share cards.
//!
//! Renders a 1200×630 PNG per comparison so a link to `same.actor/title/…` unfurls into
//! something worth clicking: the titles with their posters, the number of shared credits,
//! and the app's own mark. Built as an SVG string, rasterised with resvg, with TMDB's logo
//! composited on top. Fonts are vendored into the binary so the image is identical in every
//! environment and the runtime needs no font package.

use std::sync::{Arc, OnceLock};

use base64::Engine as _;
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
const RAISE: &str = "#131316";
const LINE: &str = "#26262b";
const PAPER: &str = "#ececed";
const DIM: &str = "#98989f";
const ACCENT: &str = TINTS[0];

// Geometry of the selection column.
const ROW_X: f64 = 636.0;
const ROW_TOP: f64 = 148.0;
const ROW_PITCH: f64 = 84.0;
const POSTER_W: f64 = 52.0;
const POSTER_H: f64 = 78.0;
/// Portraits are square: a circle must fit inside the drawn box, or the clip turns into a
/// stadium shape against the edge of the image.
const AVATAR: f64 = 62.0;
/// Left edge of the name column, measured from the artwork's left edge.
const TEXT_INSET: f64 = 78.0;
/// Rendered width of the attribution mark in the footer.
const LOGO_W: f64 = 148.0;

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

fn options() -> usvg::Options<'static> {
    let mut options = usvg::Options::default();
    options.fontdb = fonts();
    options.font_family = "DejaVu Sans".to_owned();
    options
}

/// One entry in the card's list of selections.
pub struct Pick {
    pub name: String,
    pub year: Option<u16>,
    /// Poster or portrait bytes, already fetched. `None` draws an empty well.
    pub image: Option<Arc<Vec<u8>>>,
    /// Portraits are clipped to a circle, posters are rectangles.
    pub round: bool,
}

/// Everything the card says. Built by the route from the same metadata the page uses.
pub struct Card {
    pub heading: String,
    pub picks: Vec<Pick>,
    /// The big number: how many shared credits the comparison found.
    pub count: usize,
    /// What that number counts, e.g. `"ACTORS IN 2+ OF THESE FILMS"`.
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

/// Greedy word wrap. SVG has no wrapping of its own.
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

fn data_uri(bytes: &[u8]) -> String {
    let mime = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else {
        "image/jpeg"
    };
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn svg(card: &Card) -> String {
    let mut body = String::new();
    let mut defs = String::new();

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
    for (line, y) in wrap(&card.heading, 22).into_iter().take(2).zip([182, 238]) {
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

    // The selections, stacked with their artwork: five rows stay legible in a small embed,
    // where five side-by-side pills would not.
    for (index, pick) in card.picks.iter().take(5).enumerate() {
        let tint = TINTS[index % TINTS.len()];
        let top = ROW_TOP + index as f64 * ROW_PITCH;

        body.push_str(&format!(
            r#"<rect x="{ROW_X}" y="{bar_y}" width="3" height="{POSTER_H}" rx="1.5" fill="{tint}"/>"#,
            bar_y = top + 1.0
        ));

        let image_x = ROW_X + 14.0;
        let clip = format!("p{index}");

        // A poster fills the row; a portrait is a square centred in it.
        let (width, height, image_y) = if pick.round {
            (AVATAR, AVATAR, top + (POSTER_H - AVATAR) / 2.0)
        } else {
            (POSTER_W, POSTER_H, top)
        };

        match &pick.image {
            Some(bytes) => {
                if pick.round {
                    defs.push_str(&format!(
                        r#"<clipPath id="{clip}"><circle cx="{cx}" cy="{cy}" r="{r}"/></clipPath>"#,
                        cx = image_x + width / 2.0,
                        cy = image_y + height / 2.0,
                        r = width / 2.0
                    ));
                } else {
                    defs.push_str(&format!(
                        r#"<clipPath id="{clip}"><rect x="{image_x}" y="{image_y}" width="{width}" height="{height}" rx="3"/></clipPath>"#
                    ));
                }

                body.push_str(&format!(
                    r#"<image x="{image_x}" y="{image_y}" width="{width}" height="{height}" preserveAspectRatio="xMidYMid slice" clip-path="url(#{clip})" href="{href}"/>"#,
                    href = data_uri(bytes)
                ));
            }
            None => {
                // No artwork: keep the grid by drawing the empty well rather than nothing.
                body.push_str(&format!(
                    r#"<rect x="{image_x}" y="{image_y}" width="{width}" height="{height}" rx="3" fill="{RAISE}" stroke="{LINE}"/>"#
                ));
            }
        }

        let text_x = image_x + TEXT_INSET;
        body.push_str(&format!(
            r#"<text x="{text_x}" y="{name_y}" font-family="DejaVu Sans" font-size="25" fill="{PAPER}">{name}</text>"#,
            name_y = top + 34.0,
            name = esc(&ellipsize(&pick.name, 26))
        ));

        if let Some(year) = pick.year {
            body.push_str(&format!(
                r#"<text x="{text_x}" y="{year_y}" font-family="DejaVu Sans Mono" font-size="18" fill="{DIM}">{year}</text>"#,
                year_y = top + 62.0
            ));
        }
    }

    body.push_str(&format!(
        r#"<rect x="64" y="584" width="1072" height="1" fill="{LINE}"/>"#
    ));

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}"><defs>{defs}<radialGradient id="a" cx="0.08" cy="0" r="0.95"><stop offset="0" stop-color="{ACCENT}" stop-opacity="0.18"/><stop offset="1" stop-color="{ACCENT}" stop-opacity="0"/></radialGradient><radialGradient id="b" cx="0.99" cy="0.06" r="0.72"><stop offset="0" stop-color="{TINTS_1}" stop-opacity="0.15"/><stop offset="1" stop-color="{TINTS_1}" stop-opacity="0"/></radialGradient></defs><rect width="{WIDTH}" height="{HEIGHT}" fill="{INK}"/><rect width="{WIDTH}" height="{HEIGHT}" fill="url(#a)"/><rect width="{WIDTH}" height="{HEIGHT}" fill="url(#b)"/><rect x="0" y="0" width="{WIDTH}" height="6" fill="{ACCENT}"/>{body}</svg>"#,
        TINTS_1 = TINTS[1],
        body = body
    )
}

/// Rasterise a card to PNG bytes.
///
/// `logo` is TMDB's own SVG mark. It is composited unmodified, scaled uniformly, and kept
/// well below the size of this app's wordmark — their brand rules ask for exactly that.
pub fn render(card: &Card, logo: Option<&[u8]>) -> Result<Vec<u8>, String> {
    render_pixmap(card, logo)?
        .encode_png()
        .map_err(|e| format!("could not encode PNG: {e}"))
}

/// The rasterised card, before PNG encoding. Exposed to the tests so they can assert on
/// actual pixels rather than on compressed sizes, which is the only way to know artwork
/// reached the canvas.
fn render_pixmap(card: &Card, logo: Option<&[u8]>) -> Result<tiny_skia::Pixmap, String> {
    let tree = usvg::Tree::from_str(&svg(card), &options())
        .map_err(|e| format!("invalid card SVG: {e}"))?;

    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT)
        .ok_or_else(|| format!("could not allocate a {WIDTH}x{HEIGHT} pixmap"))?;

    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());

    if let Some(bytes) = logo {
        if let Err(error) = composite_logo(&mut pixmap, bytes) {
            // A missing attribution mark is worth a log line, not a failed card.
            tracing::warn!(%error, "could not composite the TMDB logo");
        }
    }

    Ok(pixmap)
}

fn composite_logo(pixmap: &mut tiny_skia::Pixmap, logo: &[u8]) -> Result<(), String> {
    let tree = usvg::Tree::from_data(logo, &options()).map_err(|e| format!("logo SVG: {e}"))?;

    // usvg sizes are f32; keep the whole calculation in that type.
    let size = tree.size();
    if size.width() <= 0.0 {
        return Err("logo has zero width".to_owned());
    }

    let target = LOGO_W as f32;
    let scale = target / size.width();
    let height = target * (size.height() / size.width());
    let x = WIDTH as f32 - 64.0 - target;
    let y = HEIGHT as f32 - 30.0 - height;

    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(x, y);

    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(())
}

/// Rewrite a TMDB image URL to the thumbnail size the card needs.
///
/// The site stores `w342` for its own use; a card draws posters at 52px, so the smaller
/// asset is both quicker to fetch and quicker to decode.
pub fn thumb_url(url: &str) -> String {
    let Some((prefix, rest)) = url.split_once("/t/p/") else {
        return url.to_owned();
    };
    let Some((_size, path)) = rest.split_once('/') else {
        return url.to_owned();
    };
    format!("{prefix}/t/p/w185/{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Card {
        Card {
            heading: "What do these films share?".into(),
            picks: vec![
                Pick {
                    name: "The Matrix".into(),
                    year: Some(1999),
                    image: None,
                    round: false,
                },
                Pick {
                    name: "The Matrix Reloaded".into(),
                    year: Some(2003),
                    image: None,
                    round: false,
                },
            ],
            count: 33,
            count_label: "ACTORS IN 2+ OF THESE FILMS",
            mode_label: "TITLES",
        }
    }

    /// A solid 40×60 PNG, to prove artwork reaches the canvas.
    fn swatch() -> Arc<Vec<u8>> {
        let mut pixmap = tiny_skia::Pixmap::new(40, 60).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(200, 40, 40, 255));
        Arc::new(pixmap.encode_png().unwrap())
    }

    #[test]
    fn renders_a_png_of_the_expected_size() {
        let png = render(&card(), None).expect("render");

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
        assert!(png.len() > 5_000, "suspiciously small: {} bytes", png.len());

        let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!((width, height), (WIDTH, HEIGHT));
    }

    /// Text must actually reach the canvas. If font loading failed, resvg draws nothing and
    /// the PNG compresses to a flat gradient — which a byte count catches.
    #[test]
    fn text_is_actually_rasterised() {
        let with_text = render(&card(), None).expect("render");

        let mut bare = card();
        bare.heading = String::new();
        bare.picks.clear();
        bare.count_label = "";
        let without_text = render(&bare, None).expect("render");

        assert!(
            with_text.len() > without_text.len() + 2_000,
            "text did not render: {} vs {} bytes",
            with_text.len(),
            without_text.len()
        );
    }

    /// Centre of the first poster well, where the swatch's red should be.
    fn poster_centre() -> (u32, u32) {
        (
            (ROW_X + 14.0 + POSTER_W / 2.0) as u32,
            (ROW_TOP + POSTER_H / 2.0) as u32,
        )
    }

    fn rgb_at(pixmap: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
        // `pixel` is premultiplied; the swatch is fully opaque so demultiply is a no-op there,
        // and the assertions only ever compare against fully opaque or fully transparent.
        let color = pixmap.pixel(x, y).expect("pixel in bounds").demultiply();
        (color.red(), color.green(), color.blue())
    }

    #[test]
    fn posters_reach_the_canvas() {
        let mut pictured = card();
        for pick in &mut pictured.picks {
            pick.image = Some(swatch());
        }

        let (x, y) = poster_centre();

        // Without artwork the well is a dark placeholder.
        let plain = render_pixmap(&card(), None).expect("render");
        let (r, g, b) = rgb_at(&plain, x, y);
        assert!(
            r < 80 && g < 80 && b < 80,
            "expected an empty well, got ({r},{g},{b})"
        );

        // With artwork it is the swatch's red.
        let with_posters = render_pixmap(&pictured, None).expect("render");
        let (r, g, b) = rgb_at(&with_posters, x, y);
        assert!(
            r > 150 && g < 90 && b < 90,
            "poster did not render: ({r},{g},{b})"
        );
    }

    /// A circle, not a stadium: sample just inside the inscribed radius all the way round
    /// (artwork) and all four corners of the square box (clipped away). A box wider than it
    /// is tall — the bug this replaced — fails the corner samples on the horizontal sides.
    #[test]
    fn a_portrait_is_round_not_stadium_shaped() {
        let mut pictured = card();
        for pick in &mut pictured.picks {
            pick.image = Some(swatch());
            pick.round = true;
        }
        let pixmap = render_pixmap(&pictured, None).expect("render");

        let cx = ROW_X + 14.0 + AVATAR / 2.0;
        let cy = ROW_TOP + (POSTER_H - AVATAR) / 2.0 + AVATAR / 2.0;
        let r = AVATAR / 2.0;

        let is_artwork = |x: f64, y: f64| {
            let (red, green, blue) = rgb_at(&pixmap, x as u32, y as u32);
            red > 150 && green < 90 && blue < 90
        };

        // Eight points at 85% of the radius: all inside the circle.
        for (dx, dy) in [
            (1.0, 0.0),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (0.707, 0.707),
            (-0.707, 0.707),
            (0.707, -0.707),
            (-0.707, -0.707),
        ] {
            assert!(
                is_artwork(cx + dx * r * 0.85, cy + dy * r * 0.85),
                "point ({dx},{dy}) inside the circle should be artwork"
            );
        }

        // The four corners of the square box are outside the inscribed circle.
        for (dx, dy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            assert!(
                !is_artwork(cx + dx * r * 0.96, cy + dy * r * 0.96),
                "corner ({dx},{dy}) must be clipped away"
            );
        }
    }

    #[test]
    fn the_logo_is_composited() {
        // A 2:1 rectangle stands in for the real mark.
        let logo = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect width="200" height="100" fill="#00b3e5"/></svg>"##;

        let without = render(&card(), None).expect("render");
        let with_logo = render(&card(), Some(logo)).expect("render");

        assert!(
            with_logo.len() > without.len(),
            "logo was not drawn: {} vs {} bytes",
            with_logo.len(),
            without.len()
        );
    }

    /// Their brand rules require the mark to be less prominent than ours; the wordmark is
    /// 26px tall, so the logo must stay well under it.
    #[test]
    fn the_logo_stays_smaller_than_our_wordmark() {
        let logo_aspect = 35.52_f32 / 273.42;
        assert!(LOGO_W as f32 * logo_aspect < 26.0);
    }

    #[test]
    fn a_broken_logo_does_not_fail_the_card() {
        let png = render(&card(), Some(b"not an svg at all")).expect("a bad logo must not fail");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
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
    fn thumbnails_are_rewritten_to_the_smaller_size() {
        assert_eq!(
            thumb_url("https://image.tmdb.org/t/p/w342/abc.jpg"),
            "https://image.tmdb.org/t/p/w185/abc.jpg"
        );
        // Anything unexpected passes through untouched rather than being mangled.
        assert_eq!(
            thumb_url("https://example.test/x.jpg"),
            "https://example.test/x.jpg"
        );
    }

    #[test]
    fn data_uris_carry_the_right_type() {
        assert!(data_uri(&swatch()).starts_with("data:image/png;base64,"));
        assert!(data_uri(b"\xff\xd8\xff").starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn a_card_with_five_picks_and_no_dates_still_renders() {
        let mut card = card();
        card.picks = (0..5)
            .map(|i| Pick {
                name: format!("Pick number {i} with a fairly long name"),
                year: None,
                image: None,
                round: false,
            })
            .collect();
        assert!(render(&card, None).is_ok());
    }
}

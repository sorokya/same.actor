//! Render a sample share card to a file, so the design can be looked at without running the
//! server.
//!
//!     ./scripts/cargo.sh cargo run --example og-preview -- /tmp/card.png

use same_actor::og::{render, Card, Pick};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "card.png".into());

    let card = Card {
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
            Pick {
                name: "The Matrix Revolutions".into(),
                year: Some(2003),
                image: None,
                round: false,
            },
        ],
        count: 33,
        count_label: "ACTORS IN AT LEAST TWO",
        mode_label: "TITLES",
    };

    let png = render(&card, None).expect("render");
    std::fs::write(&path, &png).expect("write");
    println!("wrote {path} ({} bytes)", png.len());
}

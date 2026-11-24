//! The 404 page.
//!
//! Every unknown address used to return the single-page shell with a 200, which meant a broken
//! link looked like a working page that had not finished loading. This is a page that says so.

use askama::Template;

use super::Head;

#[derive(Template)]
#[template(path = "pages/not_found.html")]
pub struct NotFoundPage {
    pub head: Head,
}

impl NotFoundPage {
    pub fn new(css: &Option<String>, boost: &Option<String>) -> Self {
        let mut head = Head::new(
            "Nothing here — same.actor",
            "There is no page at this address.",
            "",
        )
        .noindex();

        if let Some(css) = css.as_deref() {
            head = head.with_css(css);
        }
        if let Some(boost) = boost.as_deref() {
            head = head.with_boost(boost);
        }

        Self { head }
    }
}

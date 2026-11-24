//! Filenames for the built front-end assets.
//!
//! Vite emits content-hashed names (`index-Ccu-D8U5.css`), so a template cannot hard-code them.
//! The build writes `dist/.vite/manifest.json` mapping source paths to those names; this reads it
//! once at startup and hands every page the same two URLs.
//!
//! A missing manifest is logged loudly but does not stop the server: an API-only deployment has
//! no assets and must still work, and a page with no stylesheet still has all of its content.
//! That is a deliberate difference from the old behaviour, where a missing shell replaced every
//! page with a blank document.

use std::path::Path;

use serde::Deserialize;

/// Where the island's entry point lives, as Vite records it.
const ISLAND: &str = "src/island.tsx";

#[derive(Debug, Clone, Default)]
pub struct Assets {
    /// Site-wide stylesheet, or `None` when no build is present.
    pub css: Option<String>,
    /// The picker island, loaded only by pages that have one.
    pub island: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    file: String,
    #[serde(default)]
    css: Vec<String>,
    #[serde(default)]
    is_entry: bool,
}

impl Assets {
    /// Read the manifest next to the static files, if there is one.
    pub fn load(static_dir: &Path) -> Self {
        let path = static_dir.join(".vite").join("manifest.json");

        let manifest = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "no asset manifest; pages will render without a stylesheet"
                );
                return Self::default();
            }
        };

        match Self::from_manifest(&manifest) {
            Ok(assets) => {
                tracing::info!(css = ?assets.css, island = ?assets.island, "loaded asset manifest");
                assets
            }
            Err(error) => {
                // A malformed manifest means the build is wrong, not that the file is missing.
                tracing::error!(path = %path.display(), %error, "could not read the asset manifest");
                Self::default()
            }
        }
    }

    /// Parse a Vite manifest. Public so the format is testable without a build.
    pub fn from_manifest(json: &str) -> serde_json::Result<Self> {
        let entries: std::collections::BTreeMap<String, Entry> = serde_json::from_str(json)?;

        let css = entries
            .iter()
            // The stylesheet hangs off whichever entry imported it; in practice there is one.
            .filter(|(key, entry)| entry.is_entry || key.ends_with(".css"))
            .flat_map(|(_, entry)| entry.css.iter())
            .next()
            .or_else(|| entries.values().flat_map(|entry| entry.css.iter()).next())
            .or_else(|| {
                // A CSS-only entry keeps its own path in `file`.
                entries
                    .iter()
                    .find(|(key, _)| key.ends_with(".css"))
                    .map(|(_, entry)| &entry.file)
            })
            .map(|file| format!("/{file}"));

        let island = entries.get(ISLAND).map(|entry| format!("/{}", entry.file));

        Ok(Self { css, island })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
      "index.html": {
        "file": "assets/main-abc123.js",
        "src": "index.html",
        "isEntry": true,
        "css": ["assets/index-Ccu-D8U5.css"]
      },
      "src/island.tsx": {
        "file": "assets/island-def456.js",
        "src": "src/island.tsx",
        "isEntry": true
      }
    }"#;

    #[test]
    fn the_stylesheet_and_the_island_are_both_found() {
        let assets = Assets::from_manifest(MANIFEST).expect("manifest");
        assert_eq!(assets.css.as_deref(), Some("/assets/index-Ccu-D8U5.css"));
        assert_eq!(assets.island.as_deref(), Some("/assets/island-def456.js"));
    }

    /// A CSS-only entry carries its own path rather than a `css` list.
    #[test]
    fn a_css_only_entry_is_used_when_it_is_the_only_one() {
        let assets = Assets::from_manifest(
            r#"{ "src/index.css": { "file": "assets/index-zzz.css", "src": "src/index.css" } }"#,
        )
        .expect("manifest");

        assert_eq!(assets.css.as_deref(), Some("/assets/index-zzz.css"));
        assert!(assets.island.is_none(), "no island was built");
    }

    #[test]
    fn a_manifest_without_a_stylesheet_is_not_an_error() {
        let assets = Assets::from_manifest(r#"{ "src/app.tsx": { "file": "assets/app.js" } }"#)
            .expect("manifest");
        assert!(assets.css.is_none());
    }
}

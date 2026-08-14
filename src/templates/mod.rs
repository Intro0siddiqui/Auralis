//! HTML Partial Serves
//!
//! Serves the new Soft Glass Audio HTMX partials from the ui/partials/ directory.
//! These are static HTML fragments with glassmorphism + neumorphism styling.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

static PARTIALS_DIR: OnceLock<PathBuf> = OnceLock::new();

fn partials_dir() -> &'static PathBuf {
    PARTIALS_DIR.get_or_init(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("ui")
            .join("partials")
    })
}

/// Static compiled-in templates for packaged binaries (Android APK, macOS DMG, Windows MSI, etc.)
const EMBEDDED_PARTIALS: &[(&str, &str)] = &[
    ("albums", include_str!("../../ui/partials/albums.html")),
    ("artists", include_str!("../../ui/partials/artists.html")),
    ("download", include_str!("../../ui/partials/download.html")),
    ("home", include_str!("../../ui/partials/home.html")),
    ("library", include_str!("../../ui/partials/library.html")),
    ("nav", include_str!("../../ui/partials/nav.html")),
    ("player-full", include_str!("../../ui/partials/player-full.html")),
    ("playlists", include_str!("../../ui/partials/playlists.html")),
    ("search", include_str!("../../ui/partials/search.html")),
    ("settings", include_str!("../../ui/partials/settings.html")),
    ("sync", include_str!("../../ui/partials/sync.html")),
];

static PARTIAL_CACHE: OnceLock<HashMap<String, String>> = OnceLock::new();

fn partial_cache() -> &'static HashMap<String, String> {
    PARTIAL_CACHE.get_or_init(|| {
        let mut map = HashMap::new();

        // 1. Populate with compiled-in static fallbacks
        for (name, content) in EMBEDDED_PARTIALS {
            map.insert(name.to_string(), content.to_string());
        }

        // 2. Overlay disk files if available (e.g. during local dev)
        let dir = partials_dir();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("html") {
                    if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            map.insert(name.to_string(), content);
                        }
                    }
                }
            }
        }
        map
    })
}

pub fn get_partial(name: &str) -> Option<String> {
    let clean_name = name
        .trim_start_matches('/')
        .trim_start_matches("partials/")
        .trim_end_matches(".html");

    partial_cache()
        .get(clean_name)
        .or_else(|| partial_cache().get(name))
        .cloned()
}

pub fn all_partial_names() -> Vec<String> {
    partial_cache().keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_partials_loaded() {
        assert!(get_partial("home").is_some());
        assert!(get_partial("library").is_some());
        assert!(get_partial("library.html").is_some());
        assert!(get_partial("/partials/library.html").is_some());
        assert!(get_partial("nav").is_some());
        assert!(get_partial("settings").is_some());
        assert!(get_partial("sync").is_some());
    }
}

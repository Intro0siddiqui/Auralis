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

static PARTIAL_CACHE: OnceLock<HashMap<String, String>> = OnceLock::new();

fn partial_cache() -> &'static HashMap<String, String> {
    PARTIAL_CACHE.get_or_init(|| {
        let mut map = HashMap::new();
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
    partial_cache().get(name).cloned()
}

pub fn all_partial_names() -> Vec<String> {
    partial_cache().keys().cloned().collect()
}

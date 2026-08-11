//! Template Commands
//!
//! Tauri command handlers for serving HTMX partials. These are the
//! primary entry points used by the Soft Glass Audio frontend — every
//! navigation and fragment swap is one of these commands.

use crate::templates::get_partial;

/// Render a full page partial by name.
#[tauri::command]
pub async fn render_template(name: String) -> Result<String, String> {
    get_partial(&name).ok_or_else(|| format!("Template '{name}' not found"))
}

/// Render a small partial by name (used for HTMX swaps).
#[tauri::command]
pub async fn render_partial(name: String, _data: serde_json::Value) -> Result<String, String> {
    get_partial(&name).ok_or_else(|| format!("Partial '{name}' not found"))
}

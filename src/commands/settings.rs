//! Settings Commands
//!
//! Tauri command handlers for application settings.

use crate::templates::render;
use crate::domain::models::Settings;
use crate::templates::SettingsTemplate;

/// Get the current settings.
#[tauri::command]
pub async fn get_settings() -> Result<Settings, String> {
    // TODO: read from SettingsRepository
    Ok(Settings::default())
}

/// Update settings (partial — only provided fields are changed).
#[tauri::command]
pub async fn update_settings(settings: Settings) -> Result<Settings, String> {
    // TODO: persist and broadcast change
    Ok(settings)
}

/// Render the settings page as HTML.
#[tauri::command]
pub async fn render_settings() -> Result<String, String> {
    let settings = Settings::default();
    let tmpl = SettingsTemplate {
        settings: &settings,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

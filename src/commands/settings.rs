//! Settings Commands
//!
//! Tauri command handlers for application settings.

use crate::domain::models::Settings;
use crate::domain::repositories::SettingsRepository;
use crate::infrastructure::database::Database;
use std::sync::Arc;
use tauri::State;

/// Create a settings repository from the database state
fn settings_repo(db: &Database) -> Arc<dyn SettingsRepository> {
    Arc::new(
        crate::infrastructure::database::repositories::SqliteSettingsRepository::new(Arc::new(
            db.clone(),
        )),
    )
}

/// Get the current settings.
#[tauri::command]
pub async fn get_settings(db: State<'_, Database>) -> Result<Settings, String> {
    let repo = settings_repo(&db);

    let settings = repo.get_settings().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch settings");
        format!("Failed to fetch settings: {e}")
    })?;

    Ok(settings)
}

/// Update settings.
#[tauri::command]
pub async fn update_settings(
    db: State<'_, Database>,
    settings: Settings,
) -> Result<Settings, String> {
    validate_settings(&settings).map_err(|e| {
        tracing::error!(error = %e, "Invalid settings");
        e
    })?;

    let repo = settings_repo(&db);

    repo.save_settings(&settings).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to save settings");
        format!("Failed to save settings: {e}")
    })?;

    tracing::info!("Settings updated");
    Ok(settings)
}

/// Validate settings fields.
fn validate_settings(settings: &Settings) -> Result<(), String> {
    if settings.audio.volume < 0.0 || settings.audio.volume > 1.0 {
        return Err("Volume must be between 0.0 and 1.0".to_string());
    }
    if settings.downloads.default_quality < 64 || settings.downloads.default_quality > 320 {
        return Err("Audio quality must be between 64 and 320 kbps".to_string());
    }
    if settings.downloads.max_concurrent == 0 || settings.downloads.max_concurrent > 10 {
        return Err("Max concurrent downloads must be between 1 and 10".to_string());
    }
    if settings.language.len() != 2 && settings.language.len() != 5 {
        return Err("Language code must be 2 (en) or 5 (en-US) characters".to_string());
    }
    Ok(())
}

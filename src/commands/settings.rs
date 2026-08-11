//! Settings Commands
//!
//! Tauri command handlers for application settings.

use crate::domain::models::Settings;
use crate::domain::repositories::SettingsRepository;
use crate::domain::services::SettingsService;
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
    SettingsService::validate_settings(&settings).map_err(|e| {
        tracing::error!(error = %e, "Invalid settings");
        e.to_string()
    })?;

    let repo = settings_repo(&db);

    repo.save_settings(&settings).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to save settings");
        format!("Failed to save settings: {e}")
    })?;

    tracing::info!("Settings updated");
    Ok(settings)
}

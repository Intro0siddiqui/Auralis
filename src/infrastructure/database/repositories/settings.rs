//! SQLite-backed settings repository implementation.

use async_trait::async_trait;
use rusqlite::params;
use std::sync::Arc;
use tracing::{error, info};

use crate::domain::models::*;
use crate::domain::repositories::*;
use crate::infrastructure::database::Database;

// ============================================================================
// Settings Repository
// ============================================================================

/// SQLite-backed settings repository
pub struct SqliteSettingsRepository {
    db: Arc<Database>,
}

impl SqliteSettingsRepository {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SettingsRepository for SqliteSettingsRepository {
    async fn get_settings(&self) -> Result<Settings, Box<dyn std::error::Error + Send + Sync>> {
        let data: Option<String> = {
            let conn = self
                .db
                .connection()
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            conn.query_row("SELECT data FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .ok()
        };

        match data {
            Some(json_str) => {
                let settings: Settings = serde_json::from_str(&json_str).map_err(|e| {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Failed to parse settings: {e}"),
                    )) as Box<dyn std::error::Error + Send + Sync>
                })?;
                Ok(settings)
            }
            None => {
                let default_settings = Settings::default();
                self.save_settings(&default_settings).await.map_err(|e| {
                    error!(error = %e, "Failed to save default settings");
                    e
                })?;
                Ok(default_settings)
            }
        }
    }

    async fn save_settings(
        &self,
        settings: &Settings,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let json_str = serde_json::to_string(settings).map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to serialize settings: {e}"),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;

        conn.execute(
            "INSERT OR REPLACE INTO settings (id, data) VALUES (1, ?)",
            params![json_str],
        )?;

        info!("Settings saved successfully");
        Ok(())
    }
}

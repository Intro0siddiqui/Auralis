//! Settings Service
//!
//! Handles application settings management.

use crate::domain::models::Settings;
use crate::domain::repositories::SettingsRepository;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Settings service for managing application configuration
pub struct SettingsService {
    repository: Arc<dyn SettingsRepository>,
    cache: Arc<tokio::sync::RwLock<Option<Settings>>>,
}

impl SettingsService {
    /// Create a new settings service
    pub fn new(repository: Arc<dyn SettingsRepository>) -> Self {
        Self {
            repository,
            cache: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Initialize cache from repository
    pub async fn init_cache(&self) -> Result<(), SettingsError> {
        let settings = self.repository.get_settings().await.map_err(|e| {
            error!(error = %e, "Failed to load settings");
            SettingsError::DatabaseError(e.to_string())
        })?;

        let mut cache = self.cache.write().await;
        *cache = Some(settings);
        debug!("Settings cache initialized");
        Ok(())
    }

    /// Get current settings
    pub async fn get_settings(&self) -> Result<Settings, SettingsError> {
        let cache = self.cache.read().await;
        match &*cache {
            Some(settings) => Ok(settings.clone()),
            None => {
                drop(cache);
                self.init_cache().await?;
                let cache = self.cache.read().await;
                Ok(cache.as_ref().unwrap().clone())
            }
        }
    }

    /// Update settings
    pub async fn update_settings(&self, settings: Settings) -> Result<Settings, SettingsError> {
        info!("Updating settings");

        // Validate settings
        self.validate_settings(&settings)?;

        self.repository
            .save_settings(&settings)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to save settings");
                SettingsError::DatabaseError(e.to_string())
            })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            *cache = Some(settings.clone());
        }

        info!("Settings updated successfully");
        Ok(settings)
    }

    /// Validate settings
    fn validate_settings(&self, settings: &Settings) -> Result<(), SettingsError> {
        // Validate volume
        if settings.audio.volume < 0.0 || settings.audio.volume > 1.0 {
            return Err(SettingsError::ValidationError(
                "Volume must be between 0.0 and 1.0".to_string(),
            ));
        }

        // Validate quality
        if settings.downloads.default_quality < 64 || settings.downloads.default_quality > 320 {
            return Err(SettingsError::ValidationError(
                "Audio quality must be between 64 and 320 kbps".to_string(),
            ));
        }

        // Validate concurrent downloads
        if settings.downloads.max_concurrent == 0 || settings.downloads.max_concurrent > 10 {
            return Err(SettingsError::ValidationError(
                "Max concurrent downloads must be between 1 and 10".to_string(),
            ));
        }

        // Validate language code
        if settings.language.len() != 2 && settings.language.len() != 5 {
            return Err(SettingsError::ValidationError(
                "Language code must be 2 (en) or 5 (en-US) characters".to_string(),
            ));
        }

        Ok(())
    }

    /// Reset settings to defaults
    pub async fn reset_settings(&self) -> Result<Settings, SettingsError> {
        info!("Resetting settings to defaults");

        let settings = Settings::default();

        self.repository
            .save_settings(&settings)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to reset settings");
                SettingsError::DatabaseError(e.to_string())
            })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            *cache = Some(settings.clone());
        }

        info!("Settings reset to defaults");
        Ok(settings)
    }

    /// Add a library scan path
    pub async fn add_scan_path(&self, path: std::path::PathBuf) -> Result<Settings, SettingsError> {
        let mut settings = self.get_settings().await?;

        if !settings.library.scan_paths.contains(&path) {
            settings.library.scan_paths.push(path);
            self.update_settings(settings).await
        } else {
            Ok(settings)
        }
    }

    /// Remove a library scan path
    pub async fn remove_scan_path(
        &self,
        path: &std::path::Path,
    ) -> Result<Settings, SettingsError> {
        let mut settings = self.get_settings().await?;

        settings.library.scan_paths.retain(|p| p != path);
        self.update_settings(settings).await
    }
}

/// Settings-related errors
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

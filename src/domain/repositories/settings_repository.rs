//! Settings Repository Interface
//!
//! Defines the data access contract for application settings.

use crate::domain::models::Settings;
use async_trait::async_trait;

/// Repository interface for settings data access
#[async_trait]
pub trait SettingsRepository: Send + Sync {
    /// Get current settings
    async fn get_settings(&self) -> Result<Settings, Box<dyn std::error::Error + Send + Sync>>;

    /// Save settings
    async fn save_settings(&self, settings: &Settings) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

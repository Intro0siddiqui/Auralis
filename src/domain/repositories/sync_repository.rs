//! Sync Repository Interface
//!
//! Defines the data access contract for synchronization data.

use crate::domain::models::{PairedDevice, SyncChange};
use async_trait::async_trait;
use uuid::Uuid;

/// Repository interface for sync data access
#[async_trait]
pub trait SyncRepository: Send + Sync {
    // Paired devices

    /// Get all paired devices
    async fn get_paired_devices(&self) -> Result<Vec<PairedDevice>, Box<dyn std::error::Error + Send + Sync>>;

    /// Get a paired device by ID
    async fn get_paired_device(&self, id: Uuid) -> Result<Option<PairedDevice>, Box<dyn std::error::Error + Send + Sync>>;

    /// Save a paired device
    async fn save_paired_device(&self, device: &PairedDevice) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Delete a paired device
    async fn delete_paired_device(&self, id: Uuid) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    // Sync changes

    /// Get all pending sync changes
    async fn get_pending_changes(&self) -> Result<Vec<SyncChange>, Box<dyn std::error::Error + Send + Sync>>;

    /// Save a sync change
    async fn save_change(&self, change: &SyncChange) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Mark a change as applied
    async fn mark_change_applied(&self, id: Uuid) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Clear all pending changes
    async fn clear_changes(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

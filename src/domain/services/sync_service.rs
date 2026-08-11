//! Sync Service
//!
//! Handles P2P synchronization between devices.

use crate::domain::models::{
    ChangeType, DeviceStatus, DeviceType, EntityType, PairedDevice, PairingInfo, SyncChange,
    SyncStatus,
};
use crate::domain::repositories::{SettingsRepository, SyncRepository, TrackRepository};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use uuid::Uuid;

/// Sync service for managing P2P synchronization
pub struct SyncService {
    settings_repository: Arc<dyn SettingsRepository>,
    sync_repository: Arc<dyn SyncRepository>,
    /// Track repository; reserved for applying remote sync changes to local tracks.
    #[allow(dead_code)]
    track_repository: Arc<dyn TrackRepository>,
    paired_devices: Arc<RwLock<HashMap<Uuid, PairedDevice>>>,
    pending_changes: Arc<RwLock<Vec<SyncChange>>>,
    sync_status: Arc<RwLock<SyncStatus>>,
    active_pairing: Arc<RwLock<Option<PairingInfo>>>,
}

impl SyncService {
    /// Create a new sync service
    pub fn new(
        settings_repository: Arc<dyn SettingsRepository>,
        sync_repository: Arc<dyn SyncRepository>,
        track_repository: Arc<dyn TrackRepository>,
    ) -> Self {
        Self {
            settings_repository,
            sync_repository,
            track_repository,
            paired_devices: Arc::new(RwLock::new(HashMap::new())),
            pending_changes: Arc::new(RwLock::new(Vec::new())),
            sync_status: Arc::new(RwLock::new(SyncStatus::default())),
            active_pairing: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize sync state from repository
    pub async fn init(&self) -> Result<(), SyncError> {
        info!("Initializing sync service");

        // Load paired devices
        let devices = self
            .sync_repository
            .get_paired_devices()
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to load paired devices");
                SyncError::DatabaseError(e.to_string())
            })?;

        {
            let mut paired = self.paired_devices.write().await;
            for device in devices {
                paired.insert(device.id, device);
            }
        }

        // Load pending changes
        let changes = self
            .sync_repository
            .get_pending_changes()
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to load pending changes");
                SyncError::DatabaseError(e.to_string())
            })?;

        {
            let mut pending = self.pending_changes.write().await;
            *pending = changes;
        }

        // Update status
        {
            let mut status = self.sync_status.write().await;
            status.enabled = true;
            status.pending_changes = self.pending_changes.read().await.len() as u32;
        }

        info!("Sync service initialized");
        Ok(())
    }

    /// Get all paired devices
    pub async fn get_paired_devices(&self) -> Vec<PairedDevice> {
        let devices = self.paired_devices.read().await;
        devices.values().cloned().collect()
    }

    /// Start pairing process
    pub async fn start_pairing(&self) -> Result<PairingInfo, SyncError> {
        info!("Starting pairing process");

        let _settings = self
            .settings_repository
            .get_settings()
            .await
            .map_err(|e| SyncError::SettingsError(e.to_string()))?;

        // Generate pairing info
        let pairing_info = PairingInfo::generate();

        // Store active pairing
        {
            let mut active = self.active_pairing.write().await;
            *active = Some(pairing_info.clone());
        }

        info!(pin = %pairing_info.pin, "Pairing initiated");
        Ok(pairing_info)
    }

    /// Complete pairing with PIN
    pub async fn complete_pairing(&self, pin: String) -> Result<PairedDevice, SyncError> {
        info!(pin = %pin, "Completing pairing");

        // Check active pairing
        let active = {
            let pairing = self.active_pairing.read().await;
            pairing.clone()
        }
        .ok_or(SyncError::NoActivePairing)?;

        // Validate PIN
        if active.pin != pin {
            return Err(SyncError::InvalidPin);
        }

        if active.is_expired() {
            return Err(SyncError::PairingExpired);
        }

        // TODO: Implement actual network pairing via libp2p
        // For now, create a mock device
        let device = PairedDevice::new("Paired Device".to_string(), DeviceType::Desktop);

        // Save to repository
        self.sync_repository
            .save_paired_device(&device)
            .await
            .map_err(|e| SyncError::DatabaseError(e.to_string()))?;

        // Update cache
        {
            let mut devices = self.paired_devices.write().await;
            devices.insert(device.id, device.clone());
        }

        // Clear active pairing
        {
            let mut active = self.active_pairing.write().await;
            *active = None;
        }

        info!(device_id = %device.id, "Pairing completed");
        Ok(device)
    }

    /// Unpair a device
    pub async fn unpair_device(&self, device_id: Uuid) -> Result<(), SyncError> {
        info!(device_id = %device_id, "Unpairing device");

        // Remove from repository
        self.sync_repository
            .delete_paired_device(device_id)
            .await
            .map_err(|e| SyncError::DatabaseError(e.to_string()))?;

        // Update cache
        {
            let mut devices = self.paired_devices.write().await;
            devices.remove(&device_id);
        }

        info!(device_id = %device_id, "Device unpaired");
        Ok(())
    }

    /// Sync with a specific device
    pub async fn sync_with_device(&self, device_id: Uuid) -> Result<(), SyncError> {
        info!(device_id = %device_id, "Starting sync with device");

        // Get device
        let _device = {
            let devices = self.paired_devices.read().await;
            devices
                .get(&device_id)
                .cloned()
                .ok_or(SyncError::DeviceNotFound(device_id))?
        };

        // Update status
        {
            let mut status = self.sync_status.write().await;
            status.is_syncing = true;
            status.progress = 0.0;
        }

        // TODO: Implement actual sync via libp2p
        // This would involve:
        // 1. Connecting to the device
        // 2. Exchanging library versions
        // 3. Sending/receiving changes
        // 4. Resolving conflicts

        // Simulate sync progress
        for i in 1..=10 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let mut status = self.sync_status.write().await;
            status.progress = i as f32 / 10.0;
        }

        // Update device sync time
        {
            let mut devices = self.paired_devices.write().await;
            if let Some(d) = devices.get_mut(&device_id) {
                d.mark_synced();
                self.sync_repository
                    .save_paired_device(d)
                    .await
                    .map_err(|e| SyncError::DatabaseError(e.to_string()))?;
            }
        }

        // Update status
        {
            let mut status = self.sync_status.write().await;
            status.is_syncing = false;
            status.progress = 1.0;
            status.last_sync = Some(chrono::Utc::now());
            status.connected_devices = self.paired_devices.read().await.len() as u32;
        }

        info!(device_id = %device_id, "Sync completed");
        Ok(())
    }

    /// Get current sync status
    pub async fn get_sync_status(&self) -> SyncStatus {
        let status = self.sync_status.read().await;
        let pending = self.pending_changes.read().await;
        let devices = self.paired_devices.read().await;

        SyncStatus {
            enabled: status.enabled,
            is_syncing: status.is_syncing,
            connected_devices: devices
                .values()
                .filter(|d| d.status == DeviceStatus::Connected)
                .count() as u32,
            pending_changes: pending.len() as u32,
            last_sync: status.last_sync,
            progress: status.progress,
            error: status.error.clone(),
        }
    }

    /// Record a change for sync
    pub async fn record_change(
        &self,
        change_type: ChangeType,
        entity_type: EntityType,
        entity_id: Uuid,
        payload: serde_json::Value,
    ) -> Result<(), SyncError> {
        let change = SyncChange::new(change_type, entity_type, entity_id, payload);

        // Save to repository
        self.sync_repository
            .save_change(&change)
            .await
            .map_err(|e| SyncError::DatabaseError(e.to_string()))?;

        // Update cache
        {
            let mut pending = self.pending_changes.write().await;
            pending.push(change);
        }

        // Update status
        {
            let mut status = self.sync_status.write().await;
            status.pending_changes = self.pending_changes.read().await.len() as u32;
        }

        debug!(entity_type = ?entity_type, entity_id = %entity_id, "Change recorded for sync");
        Ok(())
    }

    /// Clear all pending changes
    pub async fn clear_pending_changes(&self) -> Result<(), SyncError> {
        self.sync_repository
            .clear_changes()
            .await
            .map_err(|e| SyncError::DatabaseError(e.to_string()))?;

        {
            let mut pending = self.pending_changes.write().await;
            pending.clear();
        }

        {
            let mut status = self.sync_status.write().await;
            status.pending_changes = 0;
        }

        info!("Pending changes cleared");
        Ok(())
    }
}

/// Sync-related errors
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("Device not found: {0}")]
    DeviceNotFound(Uuid),

    #[error("No active pairing")]
    NoActivePairing,

    #[error("Invalid PIN")]
    InvalidPin,

    #[error("Pairing expired")]
    PairingExpired,

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Settings error: {0}")]
    SettingsError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Sync conflict: {0}")]
    Conflict(String),
}

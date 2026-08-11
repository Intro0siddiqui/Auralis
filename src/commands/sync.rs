//! Sync Commands
//!
//! Tauri command handlers for P2P device synchronization.

use crate::domain::models::{PairedDevice, PairingInfo, SyncStatus};
use crate::domain::repositories::{SettingsRepository, SyncRepository, TrackRepository};
use crate::domain::services::SyncService;
use crate::infrastructure::database::Database;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

/// Pairing completion request (PIN entered on the pairing device).
#[derive(Debug, Serialize, Deserialize)]
pub struct CompletePairingRequest {
    pub pin: String,
    pub device_name: String,
}

/// Build a SyncService wired to database-backed repositories.
pub fn build_sync_service(db: &Database) -> SyncService {
    use crate::infrastructure::database::repositories::{
        SqliteSettingsRepository, SqliteSyncRepository, SqliteTrackRepository,
    };

    let db = Arc::new(db.clone());
    let settings_repository: Arc<dyn SettingsRepository> =
        Arc::new(SqliteSettingsRepository::new(db.clone()));
    let sync_repository: Arc<dyn SyncRepository> = Arc::new(SqliteSyncRepository::new(db.clone()));
    let track_repository: Arc<dyn TrackRepository> = Arc::new(SqliteTrackRepository::new(db));

    SyncService::new(settings_repository, sync_repository, track_repository)
}

/// Get all paired devices.
#[tauri::command]
pub async fn get_paired_devices(
    service: State<'_, SyncService>,
) -> Result<Vec<PairedDevice>, String> {
    Ok(service.get_paired_devices().await)
}

/// Start a pairing request — returns a PIN and QR code.
#[tauri::command]
pub async fn start_pairing(service: State<'_, SyncService>) -> Result<PairingInfo, String> {
    service.start_pairing().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to start pairing");
        format!("Failed to start pairing: {e}")
    })
}

/// Complete pairing using a PIN supplied by the user.
#[tauri::command]
pub async fn complete_pairing(
    service: State<'_, SyncService>,
    request: CompletePairingRequest,
) -> Result<PairedDevice, String> {
    service.complete_pairing(request.pin).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to complete pairing");
        format!("Failed to complete pairing: {e}")
    })
}

/// Unpair (remove) a device.
#[tauri::command]
pub async fn unpair_device(service: State<'_, SyncService>, id: Uuid) -> Result<(), String> {
    service.unpair_device(id).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to unpair device");
        format!("Failed to unpair device: {e}")
    })
}

/// Trigger a sync with the given device.
#[tauri::command]
pub async fn sync_with_device(
    service: State<'_, SyncService>,
    id: Uuid,
) -> Result<SyncStatus, String> {
    service.sync_with_device(id).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to sync with device");
        format!("Failed to sync with device: {e}")
    })?;

    Ok(service.get_sync_status().await)
}

/// Get the current sync status.
#[tauri::command]
pub async fn get_sync_status(service: State<'_, SyncService>) -> Result<SyncStatus, String> {
    Ok(service.get_sync_status().await)
}

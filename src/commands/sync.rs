//! Sync Commands
//!
//! Tauri command handlers for P2P device synchronization.

use crate::domain::models::{PairedDevice, PairingInfo, SyncStatus};
use crate::templates::render;
use crate::templates::SyncTemplate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Pairing completion request (PIN entered on the pairing device).
#[derive(Debug, Serialize, Deserialize)]
pub struct CompletePairingRequest {
    pub pin: String,
    pub device_name: String,
}

/// Get all paired devices.
#[tauri::command]
pub async fn get_paired_devices() -> Result<Vec<PairedDevice>, String> {
    // TODO: read from SyncRepository
    Ok(Vec::new())
}

/// Start a pairing request — returns a PIN and QR code.
#[tauri::command]
pub async fn start_pairing() -> Result<PairingInfo, String> {
    Ok(PairingInfo::generate())
}

/// Complete pairing using a PIN supplied by the user.
#[tauri::command]
pub async fn complete_pairing(_request: CompletePairingRequest) -> Result<PairedDevice, String> {
    // TODO: validate PIN + persist PairedDevice
    Err("complete_pairing not yet implemented".to_string())
}

/// Unpair (remove) a device.
#[tauri::command]
pub async fn unpair_device(_id: Uuid) -> Result<(), String> {
    // TODO: remove from repository
    Ok(())
}

/// Trigger a sync with the given device.
#[tauri::command]
pub async fn sync_with_device(_id: Uuid) -> Result<SyncStatus, String> {
    // TODO: kick off sync task
    Ok(SyncStatus::default())
}

/// Get the current sync status.
#[tauri::command]
pub async fn get_sync_status() -> Result<SyncStatus, String> {
    Ok(SyncStatus::default())
}

/// Render the sync / devices page.
#[tauri::command]
pub async fn render_sync() -> Result<String, String> {
    let devices: Vec<PairedDevice> = Vec::new();
    let tmpl = SyncTemplate { devices: &devices };
    render(&tmpl).map_err(|e| e.to_string())
}

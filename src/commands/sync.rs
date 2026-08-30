//! Sync Commands
//!
//! Tauri command handlers for P2P device synchronization.

use crate::domain::models::{AudioFormat, PairedDevice, PairingInfo, SyncStatus, Track};
use crate::domain::repositories::{SettingsRepository, SyncRepository, TrackRepository};
use crate::domain::services::{RamTrackBuffer, SyncService};
use crate::infrastructure::database::repositories::SqliteTrackRepository;
use crate::infrastructure::database::Database;
use crate::infrastructure::media::player::AudioPlayer;
use crate::infrastructure::network::SyncEngine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

/// Pairing completion request (PIN entered on the pairing device, optionally with scanned QR payload).
#[derive(Debug, Serialize, Deserialize)]
pub struct CompletePairingRequest {
    pub pin: String,
    pub device_name: String,
    pub qr_payload: Option<String>,
}

/// Build a SyncService wired to database-backed repositories.
pub fn build_sync_service(db: &Database, sync_engine: SyncEngine) -> SyncService {
    use crate::infrastructure::database::repositories::{
        SqliteSettingsRepository, SqliteSyncRepository,
    };

    let db = Arc::new(db.clone());
    let settings_repository: Arc<dyn SettingsRepository> =
        Arc::new(SqliteSettingsRepository::new(db.clone()));
    let sync_repository: Arc<dyn SyncRepository> = Arc::new(SqliteSyncRepository::new(db.clone()));
    let sync_engine = Arc::new(sync_engine);

    SyncService::new(settings_repository, sync_repository, sync_engine)
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

/// Complete pairing using a PIN supplied by the user and optional scanned QR payload data.
#[tauri::command]
pub async fn complete_pairing(
    service: State<'_, SyncService>,
    request: CompletePairingRequest,
) -> Result<PairedDevice, String> {
    service
        .complete_pairing_with_qr(request.pin.clone(), request.device_name, request.qr_payload)
        .await
        .map_err(|e| {
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

/// Connect directly to a peer via IP:Port or Multiaddr (bypasses AP isolation / mobile multicast blocks)
#[tauri::command]
pub async fn connect_peer_address(
    service: State<'_, SyncService>,
    address: String,
) -> Result<String, String> {
    service.connect_address(&address).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to connect to peer address");
        format!("Failed to connect: {e}")
    })
}

/// Request and stream audio file chunks from a peer directly over libp2p binary sub-protocol into RAM buffer with instant playback option.
#[tauri::command]
pub async fn stream_p2p_track_to_ram(
    app_handle: AppHandle,
    service: State<'_, SyncService>,
    player: State<'_, AudioPlayer>,
    peer_id: String,
    track_id: String,
    title: String,
    artist: String,
    album: Option<String>,
    file_extension: String,
    auto_play: bool,
) -> Result<RamTrackBuffer, String> {
    let ram_track = service
        .fetch_and_buffer_track_from_peer(
            &peer_id,
            &track_id,
            title.clone(),
            artist.clone(),
            album,
            file_extension,
        )
        .await
        .map_err(|e| {
            tracing::error!(peer = %peer_id, track_id = %track_id, error = %e, "Failed to stream P2P track to RAM");
            format!("Failed to stream track from peer: {e}")
        })?;

    if auto_play {
        player.play_bytes(ram_track.data.clone()).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to start instant RAM audio playback");
            format!("Instant RAM playback failed: {e}")
        })?;
    }

    let payload = serde_json::json!({
        "track_id": track_id,
        "title": title,
        "artist": artist,
        "auto_play": auto_play,
        "peer_id": peer_id,
    });

    let _ = app_handle.emit("sync:track_received_in_ram", payload);
    Ok(ram_track)
}

/// Buffer an incoming audio track in RAM and trigger instant playback option.
#[tauri::command]
pub async fn receive_ram_track(
    app_handle: AppHandle,
    service: State<'_, SyncService>,
    player: State<'_, AudioPlayer>,
    ram_track: RamTrackBuffer,
    auto_play: bool,
) -> Result<(), String> {
    let track_id = ram_track.track_id.clone();
    let title = ram_track.title.clone();
    let artist = ram_track.artist.clone();
    let data = ram_track.data.clone();

    service.buffer_track_in_ram(ram_track).await;

    if auto_play {
        player.play_bytes(data).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to start RAM audio playback");
            format!("RAM playback failed: {e}")
        })?;
    }

    let payload = serde_json::json!({
        "track_id": track_id,
        "title": title,
        "artist": artist,
        "auto_play": auto_play,
    });

    let _ = app_handle.emit("sync:track_received_in_ram", payload);
    Ok(())
}

/// Save a RAM-buffered track to disk, insert metadata in DB, and clear RAM buffer.
#[tauri::command]
pub async fn save_ram_track(
    service: State<'_, SyncService>,
    db: State<'_, Database>,
    track_id: String,
    downloads_dir: Option<String>,
) -> Result<Track, String> {
    let ram_track = service
        .get_ram_track(&track_id)
        .await
        .ok_or_else(|| format!("RAM buffer not found for track: {track_id}"))?;

    let base_dir = if let Some(dir) = downloads_dir {
        PathBuf::from(dir)
    } else {
        dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Auralis")
    };

    tokio::fs::create_dir_all(&base_dir).await.map_err(|e| {
        format!("Failed to create download directory {base_dir:?}: {e}")
    })?;

    let safe_title = ram_track
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();
    let ext = if ram_track.file_extension.is_empty() {
        "mp3"
    } else {
        ram_track.file_extension.trim_start_matches('.')
    };

    let target_path = base_dir.join(format!("{safe_title}.{ext}"));
    tokio::fs::write(&target_path, &ram_track.data)
        .await
        .map_err(|e| format!("Failed to write RAM track to disk: {e}"))?;

    let target_path_str = target_path.to_string_lossy().to_string();

    let track_format = AudioFormat::from_extension(ext).unwrap_or(AudioFormat::Mp3);
    let mut new_track = Track::new(
        ram_track.title,
        target_path_str.clone(),
        0,
        track_format,
    );
    new_track.artist = Some(ram_track.artist);
    new_track.album = ram_track.album;

    let track_repo = SqliteTrackRepository::new(Arc::new((*db).clone()));
    track_repo
        .insert(&new_track)
        .await
        .map_err(|e| format!("Failed to save track in database: {e}"))?;

    service.discard_ram_track(&track_id).await;

    tracing::info!(track_id = %track_id, path = %target_path_str, "Saved RAM track to disk & SQLite database");
    Ok(new_track)
}

/// Discard a RAM-buffered track without writing any bytes to disk.
#[tauri::command]
pub async fn discard_ram_track(
    service: State<'_, SyncService>,
    track_id: String,
) -> Result<bool, String> {
    let removed = service.discard_ram_track(&track_id).await;
    Ok(removed)
}

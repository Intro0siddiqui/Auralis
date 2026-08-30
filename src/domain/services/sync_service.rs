//! Sync Service
//!
//! Handles P2P synchronization between devices.

use crate::domain::models::{
    ChangeType, DeviceStatus, DeviceType, EntityType, PairedDevice, PairingInfo, SyncChange,
    SyncStatus,
};
use crate::domain::repositories::{SettingsRepository, SyncRepository};
use crate::infrastructure::network::SyncEngine;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Maximum consecutive pairing failures before a peer is locked out.
const PAIRING_MAX_FAILURES: u32 = 5;

/// How long a peer stays locked out after too many pairing failures.
const PAIRING_LOCKOUT: Duration = Duration::from_secs(60);

/// In-memory failed-pairing tracker keyed by peer identifier
/// (`(failure_count, locked_out_until)`).
#[derive(Default)]
struct PairingRateLimiter {
    failures: HashMap<String, (u32, Option<Instant>)>,
}

impl PairingRateLimiter {
    /// Returns `true` while the peer is locked out; an expired lockout resets
    /// the failure counter.
    fn is_locked_out(&mut self, key: &str) -> bool {
        match self.failures.get_mut(key) {
            Some((count, locked_until)) => match *locked_until {
                Some(until) if Instant::now() < until => true,
                _ => {
                    *count = 0;
                    *locked_until = None;
                    false
                }
            },
            None => false,
        }
    }

    fn record_failure(&mut self, key: &str) {
        let entry = self.failures.entry(key.to_string()).or_insert((0, None));
        entry.0 += 1;
        if entry.0 >= PAIRING_MAX_FAILURES {
            *entry = (0, Some(Instant::now() + PAIRING_LOCKOUT));
        }
    }

    fn clear(&mut self, key: &str) {
        self.failures.remove(key);
    }
}

/// Lock the pairing limiter, recovering a poisoned mutex instead of panicking.
fn lock_pairing_limiter(
    limiter: &Mutex<PairingRateLimiter>,
) -> std::sync::MutexGuard<'_, PairingRateLimiter> {
    limiter
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Byte-wise constant-time equality: XOR-accumulates every byte pair and ORs
/// into a single accumulator so timing cannot reveal how many leading bytes
/// matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// An audio track buffered in RAM before deciding to save to disk or discard.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RamTrackBuffer {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub file_extension: String,
    pub data: Vec<u8>,
}

/// Sync service for managing P2P synchronization
pub struct SyncService {
    settings_repository: Arc<dyn SettingsRepository>,
    sync_repository: Arc<dyn SyncRepository>,
    sync_engine: Arc<SyncEngine>,
    paired_devices: Arc<RwLock<HashMap<Uuid, PairedDevice>>>,
    pending_changes: Arc<RwLock<Vec<SyncChange>>>,
    sync_status: Arc<RwLock<SyncStatus>>,
    active_pairing: Arc<RwLock<Option<PairingInfo>>>,
    pairing_rate_limiter: Mutex<PairingRateLimiter>,
    ram_track_buffers: Arc<RwLock<HashMap<String, RamTrackBuffer>>>,
}

impl SyncService {
    /// Create a new sync service
    pub fn new(
        settings_repository: Arc<dyn SettingsRepository>,
        sync_repository: Arc<dyn SyncRepository>,
        sync_engine: Arc<SyncEngine>,
    ) -> Self {
        Self {
            settings_repository,
            sync_repository,
            sync_engine,
            paired_devices: Arc::new(RwLock::new(HashMap::new())),
            pending_changes: Arc::new(RwLock::new(Vec::new())),
            sync_status: Arc::new(RwLock::new(SyncStatus::default())),
            active_pairing: Arc::new(RwLock::new(None)),
            pairing_rate_limiter: Mutex::new(PairingRateLimiter::default()),
            ram_track_buffers: Arc::new(RwLock::new(HashMap::new())),
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
            for device in devices.clone() {
                paired.insert(device.id, device);
            }
        }

        // Hydrate in-memory alias map from persisted peer_id column (survives restarts)
        // Keeps RwLock for runtime speed but DB is source of truth.
        for device in &devices {
            if let Some(peer_str) = &device.peer_id {
                if let Ok(pid) = peer_str.parse::<libp2p::PeerId>() {
                    self.sync_engine
                        .runtime()
                        .register_device_alias(device.id.to_string(), pid)
                        .await;
                    // also cache lowercased variant (register does both, but explicit)
                } else {
                    warn!(device_id = %device.id, peer_id = %peer_str, "Invalid persisted peer_id; skipping hydrate");
                }
            }
        }
        // Also attempt full DB hydration via NetworkRuntime's alias_db if attached
        // (covers devices that may have been updated directly via SQL)
        if let Err(e) = self.sync_engine.runtime().hydrate_aliases().await {
            debug!(error = %e, "Network alias hydration from DB failed (non-fatal)");
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

        // Extract network PeerId and known multiaddrs for instant QR connection
        let peer_id = Some(self.sync_engine.peer_id().to_string());
        let addrs = if let Ok(pid) = self.sync_engine.peer_id().parse::<libp2p::PeerId>() {
            self.sync_engine
                .runtime()
                .addresses_of_alias(&pid.to_string())
                .await
                .into_iter()
                .map(|a| a.to_string())
                .collect()
        } else {
            Vec::new()
        };

        // Generate pairing info with PeerId & Multiaddrs embedded in QR code
        let pairing_info = PairingInfo::generate_with_identity(peer_id, addrs);

        // Store active pairing
        {
            let mut active = self.active_pairing.write().await;
            *active = Some(pairing_info.clone());
        }

        info!("Pairing initiated with network identity embedded in QR");
        Ok(pairing_info)
    }

    /// Complete pairing using PIN and optional scanned QR payload data
    pub async fn complete_pairing_with_qr(
        &self,
        pin: String,
        device_name: String,
        qr_payload: Option<String>,
    ) -> Result<PairedDevice, SyncError> {
        info!(device = %device_name, "Completing pairing with QR/PIN data");

        let limiter_key = device_name.trim().to_lowercase();
        {
            let mut limiter = lock_pairing_limiter(&self.pairing_rate_limiter);
            if limiter.is_locked_out(&limiter_key) {
                warn!(device = %device_name, "Pairing locked out after repeated failures");
                return Err(SyncError::TooManyAttempts);
            }
        }

        // Try extracting embedded identity (PeerId & multiaddrs) from QR payload if scanned
        let mut qr_peer_id: Option<String> = None;
        let mut qr_addrs: Vec<String> = Vec::new();

        if let Some(payload_str) = &qr_payload {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(payload_str) {
                if let Some(pid) = parsed.get("peer_id").and_then(|v| v.as_str()) {
                    qr_peer_id = Some(pid.to_string());
                }
                if let Some(arr) = parsed.get("addrs").and_then(|v| v.as_array()) {
                    for addr in arr {
                        if let Some(a) = addr.as_str() {
                            qr_addrs.push(a.to_string());
                        }
                    }
                }
            }
        }

        // Check active pairing PIN (if local initiator) or QR payload PIN (if scanner)
        let active_opt = {
            let pairing = self.active_pairing.read().await;
            pairing.clone()
        };

        if let Some(active) = active_opt {
            if !constant_time_eq(active.pin.as_bytes(), pin.as_bytes()) {
                warn!(device = %device_name, "Pairing failed: invalid PIN");
                lock_pairing_limiter(&self.pairing_rate_limiter).record_failure(&limiter_key);
                return Err(SyncError::InvalidPin);
            }
            if active.is_expired() {
                return Err(SyncError::PairingExpired);
            }
        }

        // Dial multiaddrs directly (WAN relay or LAN mDNS) if provided
        for addr_str in &qr_addrs {
            if let Err(e) = self.sync_engine.connect(addr_str).await {
                debug!(addr = %addr_str, error = %e, "Instant QR dial attempt logged");
            }
        }

        let device = if let Some(pid_str) = &qr_peer_id {
            PairedDevice::with_peer_id(device_name.clone(), DeviceType::Desktop, pid_str.clone())
        } else {
            PairedDevice::new(device_name.clone(), DeviceType::Desktop)
        };

        if let Some(pid_str) = &qr_peer_id {
            if let Ok(pid) = pid_str.parse::<libp2p::PeerId>() {
                self.sync_engine
                    .runtime()
                    .register_device_alias(device.id.to_string(), pid)
                    .await;
            }
        }

        self.sync_repository
            .save_paired_device(&device)
            .await
            .map_err(|e| SyncError::DatabaseError(e.to_string()))?;

        {
            let mut devices = self.paired_devices.write().await;
            devices.insert(device.id, device.clone());
        }

        {
            let mut active = self.active_pairing.write().await;
            *active = None;
        }

        lock_pairing_limiter(&self.pairing_rate_limiter).clear(&limiter_key);

        info!(device_id = %device.id, peer_id = ?qr_peer_id, "Pairing completed successfully");
        Ok(device)
    }

    /// Complete pairing with PIN (legacy wrapper)
    pub async fn complete_pairing(
        &self,
        pin: String,
        device_name: String,
    ) -> Result<PairedDevice, SyncError> {
        self.complete_pairing_with_qr(pin, device_name, None).await
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

    /// Sync with a specific device using libp2p request-response.
    ///
    /// Connects to the peer, exchanges the queued sync changes, and marks
    /// the device as synced on success. Connection failures are logged but
    /// do not prevent the device from being marked as synced (best-effort).
    pub async fn sync_with_device(&self, device_id: Uuid) -> Result<(), SyncError> {
        info!(device_id = %device_id, "Starting sync with device");

        // Get device
        let device = {
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
            status.error = None;
        }

        // 1. Connect to the peer via libp2p (use ip_address if available, otherwise device id)
        let peer_addr = device
            .ip_address
            .clone()
            .unwrap_or_else(|| device_id.to_string());

        if let Err(e) = self.sync_engine.connect(&peer_addr).await {
            warn!(device_id = %device_id, error = %e, "Failed to connect to peer, attempting sync anyway");
        }

        // 2. Enqueue our pending changes into the SyncEngine queue
        {
            let pending = self.pending_changes.read().await;
            if !pending.is_empty() {
                let payload = serde_json::json!({
                    "changes": pending.iter().map(|c| serde_json::json!({
                        "type": c.change_type,
                        "entity": c.entity_type,
                        "id": c.entity_id.to_string(),
                        "payload": c.payload,
                    })).collect::<Vec<_>>(),
                });
                self.sync_engine.enqueue_sync_payload(payload);
            }
        }

        // Ensure alias map is warmed from persisted peer_id (HIGH fix: survives restarts)
        // If device has a persisted peer_id, make sure the in-memory map has it.
        if let Some(peer_str) = &device.peer_id {
            if let Ok(pid) = peer_str.parse::<libp2p::PeerId>() {
                self.sync_engine
                    .runtime()
                    .register_device_alias(device_id.to_string(), pid)
                    .await;
            }
        }

        // 3. Perform the actual request-response sync via libp2p
        // request_sync resolves via in-memory map, then DB peer_id column, then raw PeerId
        let peer_id = device_id.to_string();
        if let Err(e) = self.sync_engine.request_sync(&peer_id).await {
            warn!(device_id = %device_id, error = %e, "Sync transfer failed; keeping pending changes in queue");
            let mut status = self.sync_status.write().await;
            status.is_syncing = false;
            status.error = Some(e.to_string());
            return Err(SyncError::NetworkError(e.to_string()));
        }

        debug!(device_id = %device_id, "Sync transfer completed");

        // 4. Clear pending changes on the local side only after successful sync
        self.clear_pending_changes().await.ok();

        // Update device sync time (preserve peer_id — don't overwrite persisted alias with None)
        // Resolve any persisted peer_id without holding the write guard across await
        let resolved_peer: Option<String> = {
            let need = {
                let devices = self.paired_devices.read().await;
                devices
                    .get(&device_id)
                    .and_then(|d| d.peer_id.clone())
                    .is_none()
            };
            if need {
                self.sync_engine
                    .runtime()
                    .resolve_peer_id(&device_id.to_string())
                    .await
                    .map(|p| p.to_string())
            } else {
                None
            }
        };
        {
            let mut devices = self.paired_devices.write().await;
            if let Some(d) = devices.get_mut(&device_id) {
                if d.peer_id.is_none() {
                    if let Some(pid_str) = resolved_peer.clone() {
                        d.peer_id = Some(pid_str);
                    }
                }
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

    /// Connect directly to a peer via Multiaddr or IP:Port (fallback when mDNS is unavailable)
    pub async fn connect_address(&self, address: &str) -> Result<String, SyncError> {
        let addr = address.trim();
        if addr.is_empty() {
            return Err(SyncError::NetworkError(
                "Address cannot be empty".to_string(),
            ));
        }

        let multiaddr_str = if addr.starts_with('/') {
            addr.to_string()
        } else if let Some((ip, port)) = addr.split_once(':') {
            format!("/ip4/{ip}/tcp/{port}")
        } else {
            format!("/ip4/{addr}/tcp/4001")
        };

        self.sync_engine
            .connect(&multiaddr_str)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        info!(multiaddr = %multiaddr_str, "Connected direct P2P address");
        Ok(format!("Direct connection established to {multiaddr_str}"))
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

    /// Stream a track's audio chunks directly from a P2P peer over libp2p binary file transfer into RAM buffer.
    pub async fn fetch_and_buffer_track_from_peer(
        &self,
        peer_id_str: &str,
        track_id: &str,
        title: String,
        artist: String,
        album: Option<String>,
        file_extension: String,
    ) -> Result<RamTrackBuffer, SyncError> {
        info!(peer = %peer_id_str, track_id = %track_id, "Fetching binary track chunks over libp2p");
        let mut accumulated_bytes = Vec::new();
        let mut current_chunk = 0u32;

        loop {
            let chunk_response = self
                .sync_engine
                .request_file_chunk(peer_id_str, track_id.to_string(), current_chunk)
                .await
                .map_err(|e| SyncError::NetworkError(format!("Failed to stream file chunk {current_chunk}: {e}")))?;

            if chunk_response.data.is_empty() && chunk_response.total_chunks == 0 {
                return Err(SyncError::NetworkError(format!("Peer returned empty track data for {track_id}")));
            }

            accumulated_bytes.extend_from_slice(&chunk_response.data);

            if chunk_response.is_last || (current_chunk + 1 >= chunk_response.total_chunks) {
                break;
            }
            current_chunk += 1;
        }

        let ram_track = RamTrackBuffer {
            track_id: track_id.to_string(),
            title,
            artist,
            album,
            file_extension,
            data: accumulated_bytes,
        };

        self.buffer_track_in_ram(ram_track.clone()).await;
        Ok(ram_track)
    }

    /// Buffer an incoming track stream in RAM before disk decision
    pub async fn buffer_track_in_ram(&self, ram_track: RamTrackBuffer) {
        info!(track_id = %ram_track.track_id, title = %ram_track.title, len = ram_track.data.len(), "Buffering track in RAM");
        let mut buffers = self.ram_track_buffers.write().await;
        buffers.insert(ram_track.track_id.clone(), ram_track);
    }

    /// Retrieve a RAM-buffered track by track_id
    pub async fn get_ram_track(&self, track_id: &str) -> Option<RamTrackBuffer> {
        let buffers = self.ram_track_buffers.read().await;
        buffers.get(track_id).cloned()
    }

    /// Discard a RAM-buffered track without writing to disk
    pub async fn discard_ram_track(&self, track_id: &str) -> bool {
        let mut buffers = self.ram_track_buffers.write().await;
        let removed = buffers.remove(track_id).is_some();
        if removed {
            info!(track_id = %track_id, "Discarded RAM track buffer with zero disk writes");
        }
        removed
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::database::repositories::{
        SqliteSettingsRepository, SqliteSyncRepository,
    };
    use crate::infrastructure::database::Database;
    use libp2p::identity::Keypair;

    #[tokio::test]
    async fn test_sync_service_init_warms_alias_map() {
        let db_path =
            std::env::temp_dir().join(format!("test_auralis_sync_svc_{}.db", Uuid::new_v4()));
        let db = Database::new(&db_path).unwrap();
        db.run_migrations().unwrap();
        let db_arc = Arc::new(db);

        let settings_repo = Arc::new(SqliteSettingsRepository::new(db_arc.clone()));
        let sync_repo = Arc::new(SqliteSyncRepository::new(db_arc.clone()));

        let sync_engine = Arc::new(SyncEngine::new());
        sync_engine
            .runtime()
            .set_persistent_store(db_arc.clone())
            .await;

        let peer_id = Keypair::generate_ed25519().public().to_peer_id();
        let device = PairedDevice::with_peer_id(
            "Paired Phone".to_string(),
            DeviceType::Mobile,
            peer_id.to_string(),
        );

        sync_repo.save_paired_device(&device).await.unwrap();

        let service = SyncService::new(settings_repo, sync_repo, sync_engine.clone());
        service.init().await.unwrap();

        let resolved = sync_engine
            .runtime()
            .resolve_peer_id(&device.id.to_string())
            .await;
        assert_eq!(resolved, Some(peer_id));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_ram_track_buffering_and_discard() {
        let db_path =
            std::env::temp_dir().join(format!("test_auralis_ram_buf_{}.db", Uuid::new_v4()));
        let db = Database::new(&db_path).unwrap();
        db.run_migrations().unwrap();
        let db_arc = Arc::new(db);

        let settings_repo = Arc::new(SqliteSettingsRepository::new(db_arc.clone()));
        let sync_repo = Arc::new(SqliteSyncRepository::new(db_arc.clone()));
        let sync_engine = Arc::new(SyncEngine::new());

        let service = SyncService::new(settings_repo, sync_repo, sync_engine);

        let ram_track = RamTrackBuffer {
            track_id: "test-track-1".to_string(),
            title: "Test Stream".to_string(),
            artist: "Auralis".to_string(),
            album: None,
            file_extension: "wav".to_string(),
            data: vec![0, 1, 2, 3, 4],
        };

        service.buffer_track_in_ram(ram_track.clone()).await;

        let retrieved = service.get_ram_track("test-track-1").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().data, vec![0, 1, 2, 3, 4]);

        let discarded = service.discard_ram_track("test-track-1").await;
        assert!(discarded);

        let let_after = service.get_ram_track("test-track-1").await;
        assert!(let_after.is_none());

        let _ = std::fs::remove_file(&db_path);
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

    #[error("Too many failed pairing attempts; try again later")]
    TooManyAttempts,

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

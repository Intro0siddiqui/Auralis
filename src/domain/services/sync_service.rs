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
    limiter.unwrap_or_else(std::sync::PoisonError::into_inner)
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

        info!("Pairing initiated (PIN not logged)");
        Ok(pairing_info)
    }

    /// Complete pairing with PIN
    pub async fn complete_pairing(
        &self,
        pin: String,
        device_name: String,
    ) -> Result<PairedDevice, SyncError> {
        info!(device = %device_name, "Completing pairing");

        // Rate limit failed pairing attempts per peer identifier.
        let limiter_key = device_name.trim().to_lowercase();
        {
            let mut limiter = lock_pairing_limiter(&self.pairing_rate_limiter);
            if limiter.is_locked_out(&limiter_key) {
                warn!(device = %device_name, "Pairing locked out after repeated failures");
                return Err(SyncError::TooManyAttempts);
            }
        }

        // Check active pairing
        let active = {
            let pairing = self.active_pairing.read().await;
            pairing.clone()
        }
        .ok_or(SyncError::NoActivePairing)?;

        // Validate PIN (constant-time comparison; failures rate limited)
        if !constant_time_eq(active.pin.as_bytes(), pin.as_bytes()) {
            warn!(device = %device_name, "Pairing failed: invalid PIN");
            lock_pairing_limiter(&self.pairing_rate_limiter).record_failure(&limiter_key);
            return Err(SyncError::InvalidPin);
        }

        if active.is_expired() {
            return Err(SyncError::PairingExpired);
        }

        // TODO: Implement actual network pairing via libp2p
        let device = PairedDevice::new(device_name, DeviceType::Desktop);

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

        // Successful pairing resets the failure counter for this peer.
        lock_pairing_limiter(&self.pairing_rate_limiter).clear(&limiter_key);

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

        // 3. Perform the actual request-response sync via libp2p
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

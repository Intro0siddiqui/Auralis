//! Sync Model
//!
//! P2P synchronization entities for library and playback state sharing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Represents a paired device for P2P sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    /// Unique device identifier
    pub id: Uuid,

    /// Human-readable device name
    pub name: String,

    /// Device type (desktop, mobile)
    pub device_type: DeviceType,

    /// Last known IP address
    pub ip_address: Option<String>,

    /// When this device was first paired
    pub paired_at: DateTime<Utc>,

    /// Last successful sync time
    pub last_sync: Option<DateTime<Utc>>,

    /// Current connection status
    pub status: DeviceStatus,

    /// Shared library version for conflict detection
    pub library_version: u64,

    /// Persisted libp2p PeerId string for this device (if known).
    /// Bridges the UUID `id` (used by the app) to the network's `PeerId`
    /// so aliases survive restarts. `None` until the device is discovered
    /// or explicitly linked via `register_device_alias`.
    #[serde(default)]
    pub peer_id: Option<String>,
}

impl PairedDevice {
    /// Create a new paired device
    pub fn new(name: String, device_type: DeviceType) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            device_type,
            ip_address: None,
            paired_at: Utc::now(),
            last_sync: None,
            status: DeviceStatus::Disconnected,
            library_version: 0,
            peer_id: None,
        }
    }

    /// Create a new paired device with a known PeerId
    pub fn with_peer_id(name: String, device_type: DeviceType, peer_id: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            device_type,
            ip_address: None,
            paired_at: Utc::now(),
            last_sync: None,
            status: DeviceStatus::Disconnected,
            library_version: 0,
            peer_id: Some(peer_id),
        }
    }

    /// Update sync timestamp
    pub fn mark_synced(&mut self) {
        self.last_sync = Some(Utc::now());
        self.status = DeviceStatus::Connected;
    }
}

/// Device type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Desktop,
    Mobile,
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceType::Desktop => write!(f, "desktop"),
            DeviceType::Mobile => write!(f, "mobile"),
        }
    }
}

/// Device connection status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    Connected,
    Connecting,
    Disconnected,
    Error,
}

impl fmt::Display for DeviceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceStatus::Connected => write!(f, "connected"),
            DeviceStatus::Connecting => write!(f, "connecting"),
            DeviceStatus::Disconnected => write!(f, "disconnected"),
            DeviceStatus::Error => write!(f, "error"),
        }
    }
}

/// Synchronization status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    /// Whether sync is enabled
    pub enabled: bool,

    /// Whether currently syncing
    pub is_syncing: bool,

    /// Active device connections
    pub connected_devices: u32,

    /// Pending changes to sync
    pub pending_changes: u32,

    /// Last sync time
    pub last_sync: Option<DateTime<Utc>>,

    /// Current sync progress (0.0 to 1.0)
    pub progress: f32,

    /// Sync error if any
    pub error: Option<String>,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self {
            enabled: true,
            is_syncing: false,
            connected_devices: 0,
            pending_changes: 0,
            last_sync: None,
            progress: 0.0,
            error: None,
        }
    }
}

/// A change to be synchronized
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncChange {
    /// Unique change identifier
    pub id: Uuid,

    /// Type of change
    pub change_type: ChangeType,

    /// Entity type affected
    pub entity_type: EntityType,

    /// Entity identifier
    pub entity_id: Uuid,

    /// Change payload (JSON)
    pub payload: serde_json::Value,

    /// When the change occurred
    pub timestamp: DateTime<Utc>,

    /// Whether this change has been applied
    pub applied: bool,
}

impl SyncChange {
    /// Create a new sync change
    pub fn new(
        change_type: ChangeType,
        entity_type: EntityType,
        entity_id: Uuid,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            change_type,
            entity_type,
            entity_id,
            payload,
            timestamp: Utc::now(),
            applied: false,
        }
    }
}

/// Type of change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    Created,
    Updated,
    Deleted,
}

/// Entity type for sync
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    Track,
    Playlist,
    Settings,
    PlaybackState,
}

/// Pairing request information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingInfo {
    /// Generated pairing PIN
    pub pin: String,

    /// QR code data for quick pairing
    pub qr_data: String,

    /// QR code as base64 image
    pub qr_image: String,

    /// Expiration time
    pub expires_at: DateTime<Utc>,
}

impl PairingInfo {
    /// Generate a new pairing request
    pub fn generate() -> Self {
        use rand::prelude::*;

        let mut rng = rand::rng();
        let pin: String = (0..6)
            .map(|_| rng.random_range(0..10).to_string())
            .collect();

        let qr_data = format!("auralis://pair?pin={}", pin);

        Self {
            pin,
            qr_data: qr_data.clone(),
            qr_image: Self::generate_qr_code(&qr_data),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        }
    }

    /// Generate QR code as base64 PNG
    fn generate_qr_code(data: &str) -> String {
        use base64::Engine;
        use image::ImageEncoder;

        let qr = qrcode::QrCode::new(data.as_bytes()).unwrap();
        let image = qr.render::<image::Luma<u8>>().build();

        let mut buffer = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buffer);
        encoder
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::L8,
            )
            .unwrap();

        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&buffer)
        )
    }

    /// Check if pairing has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_creation() {
        let device = PairedDevice::new("My PC".to_string(), DeviceType::Desktop);
        assert_eq!(device.name, "My PC");
        assert_eq!(device.device_type, DeviceType::Desktop);
        assert_eq!(device.status, DeviceStatus::Disconnected);
    }

    #[test]
    fn test_pairing_info() {
        let info = PairingInfo::generate();
        assert_eq!(info.pin.len(), 6);
        assert!(info.pin.chars().all(|c| c.is_ascii_digit()));
        assert!(!info.is_expired());
    }

    #[test]
    fn test_sync_change() {
        let change = SyncChange::new(
            ChangeType::Created,
            EntityType::Track,
            Uuid::new_v4(),
            serde_json::json!({"title": "Test"}),
        );

        assert_eq!(change.change_type, ChangeType::Created);
        assert!(!change.applied);
    }
}

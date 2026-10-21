//! SQLite-backed sync repository implementation.

use async_trait::async_trait;
use rusqlite::params;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::parse_datetime;
use crate::domain::models::*;
use crate::domain::repositories::*;
use crate::infrastructure::database::Database;

// ============================================================================
// Sync Repository
// ============================================================================

/// SQLite-backed sync repository
pub struct SqliteSyncRepository {
    db: Arc<Database>,
}

impl SqliteSyncRepository {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn row_to_device(row: &rusqlite::Row) -> rusqlite::Result<PairedDevice> {
        let last_sync_str: Option<String> = row.get(5)?;
        let device_type_str: String = row.get(2)?;
        let status_str: String = row.get(6)?;

        // peer_id is at index 8 after migration; handle gracefully for old rows / SELECT *
        // and also via column name lookup to survive column reordering.
        let peer_id: Option<String> = row
            .get::<_, Option<String>>("peer_id")
            .ok()
            .flatten()
            .or_else(|| row.get::<_, Option<String>>(8).ok().flatten());

        Ok(PairedDevice {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_else(|_| Uuid::new_v4()),
            name: row.get(1)?,
            device_type: match device_type_str.as_str() {
                "mobile" => DeviceType::Mobile,
                _ => DeviceType::Desktop,
            },
            ip_address: row.get(3)?,
            paired_at: parse_datetime(&row.get::<_, String>(4)?),
            last_sync: last_sync_str.as_deref().map(parse_datetime),
            status: match status_str.as_str() {
                "connected" => DeviceStatus::Connected,
                "connecting" => DeviceStatus::Connecting,
                "error" => DeviceStatus::Error,
                _ => DeviceStatus::Disconnected,
            },
            library_version: row.get::<_, i64>(7)? as u64,
            peer_id,
        })
    }

    fn row_to_change(row: &rusqlite::Row) -> rusqlite::Result<SyncChange> {
        let change_type_str: String = row.get(1)?;
        let entity_type_str: String = row.get(2)?;
        let payload_str: String = row.get(4)?;

        Ok(SyncChange {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_else(|_| Uuid::new_v4()),
            change_type: match change_type_str.as_str() {
                "updated" => ChangeType::Updated,
                "deleted" => ChangeType::Deleted,
                _ => ChangeType::Created,
            },
            entity_type: match entity_type_str.as_str() {
                "playlist" => EntityType::Playlist,
                "settings" => EntityType::Settings,
                "playback_state" => EntityType::PlaybackState,
                _ => EntityType::Track,
            },
            entity_id: Uuid::parse_str(&row.get::<_, String>(3)?)
                .unwrap_or_else(|_| Uuid::new_v4()),
            payload: serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null),
            timestamp: parse_datetime(&row.get::<_, String>(5)?),
            applied: row.get::<_, i32>(6)? != 0,
        })
    }
}

#[async_trait]
impl SyncRepository for SqliteSyncRepository {
    async fn get_paired_devices(
        &self,
    ) -> Result<Vec<PairedDevice>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare("SELECT * FROM paired_devices ORDER BY paired_at DESC")?;

        let devices = stmt
            .query_map([], Self::row_to_device)?
            .filter_map(|r| match r {
                Ok(d) => Some(d),
                Err(e) => {
                    warn!(error = %e, "Failed to map paired device row; skipping");
                    None
                }
            })
            .collect();

        Ok(devices)
    }

    async fn get_paired_device(
        &self,
        id: Uuid,
    ) -> Result<Option<PairedDevice>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare("SELECT * FROM paired_devices WHERE id = ?")?;

        let device = stmt
            .query_row(params![id.to_string()], Self::row_to_device)
            .ok();

        Ok(device)
    }

    async fn save_paired_device(
        &self,
        device: &PairedDevice,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        conn.execute(
            r#"INSERT OR REPLACE INTO paired_devices (
                id, name, device_type, ip_address, paired_at, last_sync, status, library_version, peer_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            params![
                device.id.to_string(),
                device.name,
                device.device_type.to_string(),
                device.ip_address,
                device.paired_at.to_rfc3339(),
                device.last_sync.map(|d| d.to_rfc3339()),
                device.status.to_string(),
                device.library_version as i64,
                device.peer_id,
            ],
        )?;

        info!(device_id = %device.id, name = %device.name, "Paired device saved");
        Ok(())
    }

    async fn delete_paired_device(
        &self,
        id: Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        conn.execute(
            "DELETE FROM paired_devices WHERE id = ?",
            params![id.to_string()],
        )?;
        info!(device_id = %id, "Paired device deleted");
        Ok(())
    }

    async fn get_pending_changes(
        &self,
    ) -> Result<Vec<SyncChange>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt =
            conn.prepare("SELECT * FROM sync_changes WHERE applied = 0 ORDER BY timestamp ASC")?;

        let changes = stmt
            .query_map([], Self::row_to_change)?
            .filter_map(|r| match r {
                Ok(c) => Some(c),
                Err(e) => {
                    warn!(error = %e, "Failed to map sync change row; skipping");
                    None
                }
            })
            .collect();

        Ok(changes)
    }

    async fn save_change(
        &self,
        change: &SyncChange,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        conn.execute(
            r#"INSERT OR REPLACE INTO sync_changes (
                id, change_type, entity_type, entity_id, payload, timestamp, applied
            ) VALUES (?, ?, ?, ?, ?, ?, ?)"#,
            params![
                change.id.to_string(),
                match change.change_type {
                    ChangeType::Created => "created",
                    ChangeType::Updated => "updated",
                    ChangeType::Deleted => "deleted",
                },
                match change.entity_type {
                    EntityType::Track => "track",
                    EntityType::Playlist => "playlist",
                    EntityType::Settings => "settings",
                    EntityType::PlaybackState => "playback_state",
                },
                change.entity_id.to_string(),
                change.payload.to_string(),
                change.timestamp.to_rfc3339(),
                change.applied as i32,
            ],
        )?;

        debug!(change_id = %change.id, "Sync change saved");
        Ok(())
    }

    async fn mark_change_applied(
        &self,
        id: Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        conn.execute(
            "UPDATE sync_changes SET applied = 1 WHERE id = ?",
            params![id.to_string()],
        )?;
        Ok(())
    }

    async fn clear_changes(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        conn.execute("DELETE FROM sync_changes WHERE applied = 1", [])?;
        info!("Applied sync changes cleared");
        Ok(())
    }
}

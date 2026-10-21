//! SQLite-backed playlist repository implementation.

use async_trait::async_trait;
use chrono::Utc;
use rusqlite::params;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

use super::parse_datetime;
use crate::domain::models::*;
use crate::domain::repositories::*;
use crate::infrastructure::database::Database;

// ============================================================================
// Playlist Repository
// ============================================================================

/// SQLite-backed playlist repository
pub struct SqlitePlaylistRepository {
    db: Arc<Database>,
}

impl SqlitePlaylistRepository {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn row_to_playlist(row: &rusqlite::Row) -> rusqlite::Result<Playlist> {
        let track_ids_json: String = row.get(6)?;
        let track_ids: Vec<Uuid> = serde_json::from_str(&track_ids_json).unwrap_or_default();
        let smart_criteria_json: Option<String> = row.get(7)?;
        let smart_criteria = smart_criteria_json
            .and_then(|s| serde_json::from_str::<SmartPlaylistCriteria>(&s).ok());

        Ok(Playlist {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_else(|_| Uuid::new_v4()),
            name: row.get(1)?,
            description: row.get(2)?,
            created_at: parse_datetime(&row.get::<_, String>(3)?),
            updated_at: parse_datetime(&row.get::<_, String>(4)?),
            is_smart: row.get::<_, i32>(5)? != 0,
            track_ids,
            smart_criteria,
        })
    }
}

#[async_trait]
impl PlaylistRepository for SqlitePlaylistRepository {
    async fn find_all(&self) -> Result<Vec<Playlist>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare("SELECT * FROM playlists ORDER BY updated_at DESC")?;

        let playlists = stmt
            .query_map([], Self::row_to_playlist)?
            .filter_map(|r| match r {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(error = %e, "Failed to map playlist row; skipping");
                    None
                }
            })
            .collect();

        Ok(playlists)
    }

    async fn find_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<Playlist>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare("SELECT * FROM playlists WHERE id = ?")?;

        let playlist = stmt
            .query_row(params![id.to_string()], Self::row_to_playlist)
            .ok();

        Ok(playlist)
    }

    async fn find_by_name(
        &self,
        name: &str,
    ) -> Result<Option<Playlist>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare("SELECT * FROM playlists WHERE name = ?")?;

        let playlist = stmt.query_row(params![name], Self::row_to_playlist).ok();

        Ok(playlist)
    }

    async fn insert(
        &self,
        playlist: &Playlist,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let track_ids_json = serde_json::to_string(&playlist.track_ids).unwrap_or_default();
        let smart_criteria_json = playlist
            .smart_criteria
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());

        conn.execute(
            r#"INSERT INTO playlists (
                id, name, description, created_at, updated_at, is_smart, track_ids, smart_criteria
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
            params![
                playlist.id.to_string(),
                playlist.name,
                playlist.description,
                playlist.created_at.to_rfc3339(),
                playlist.updated_at.to_rfc3339(),
                playlist.is_smart as i32,
                track_ids_json,
                smart_criteria_json,
            ],
        )?;
        Ok(())
    }

    async fn update(
        &self,
        playlist: &Playlist,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let track_ids_json = serde_json::to_string(&playlist.track_ids).unwrap_or_default();
        let smart_criteria_json = playlist
            .smart_criteria
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());

        conn.execute(
            r#"UPDATE playlists SET
                name = ?, description = ?, updated_at = ?, is_smart = ?,
                track_ids = ?, smart_criteria = ?
            WHERE id = ?"#,
            params![
                playlist.name,
                playlist.description,
                Utc::now().to_rfc3339(),
                playlist.is_smart as i32,
                track_ids_json,
                smart_criteria_json,
                playlist.id.to_string(),
            ],
        )?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        conn.execute(
            "DELETE FROM playlists WHERE id = ?",
            params![id.to_string()],
        )?;
        Ok(())
    }

    async fn count(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM playlists", [], |row| row.get(0))?;
        Ok(count as u64)
    }
}

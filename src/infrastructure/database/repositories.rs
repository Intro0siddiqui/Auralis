//! Database Repository Implementations
//!
//! SQLite-backed repository implementations.

use crate::domain::models::*;
use crate::domain::repositories::*;
use crate::infrastructure::database::Database;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::params;
use std::sync::Arc;
use tracing::{debug, error, info};
use uuid::Uuid;

// ============================================================================
// Track Repository
// ============================================================================

/// SQLite-backed track repository
pub struct SqliteTrackRepository {
    db: Arc<Database>,
}

impl SqliteTrackRepository {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn row_to_track(row: &rusqlite::Row) -> rusqlite::Result<Track> {
        let last_played_str: Option<String> = row.get(17)?;
        Ok(Track {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_else(|_| Uuid::new_v4()),
            title: row.get(1)?,
            artist: row.get(2)?,
            album: row.get(3)?,
            album_artist: row.get(4)?,
            genre: row.get(5)?,
            year: row.get(6)?,
            track_number: row.get(7)?,
            disc_number: row.get(8)?,
            duration_secs: row.get(9)?,
            file_path: row.get(10)?,
            file_size: row.get(11)?,
            format: parse_format(&row.get::<_, String>(12)?),
            bitrate: row.get(13)?,
            sample_rate: row.get(14)?,
            album_art_path: row.get(15)?,
            date_added: parse_datetime(&row.get::<_, String>(16)?),
            last_played: last_played_str.as_deref().map(parse_datetime),
            play_count: row.get(18)?,
            is_downloaded: row.get::<_, i32>(19)? != 0,
            source_url: row.get(20)?,
        })
    }
}

#[async_trait]
impl TrackRepository for SqliteTrackRepository {
    async fn find_all(
        &self,
        filter: TrackFilter,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let mut sql = String::from("SELECT * FROM tracks WHERE 1=1");
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(search) = &filter.search {
            sql.push_str(" AND (title LIKE ? OR artist LIKE ? OR album LIKE ?)");
            let pattern = format!("%{}%", search);
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern));
        }

        if let Some(artist) = &filter.artist {
            sql.push_str(" AND artist LIKE ?");
            params_vec.push(Box::new(format!("%{}%", artist)));
        }

        if let Some(album) = &filter.album {
            sql.push_str(" AND album LIKE ?");
            params_vec.push(Box::new(format!("%{}%", album)));
        }

        if filter.downloaded_only {
            sql.push_str(" AND is_downloaded = 1");
        }

        let order_field = match filter.sort_by.unwrap_or(TrackSortField::DateAdded) {
            TrackSortField::Title => "title",
            TrackSortField::Artist => "artist",
            TrackSortField::Album => "album",
            TrackSortField::DateAdded => "date_added",
            TrackSortField::LastPlayed => "last_played",
            TrackSortField::PlayCount => "play_count",
            TrackSortField::Duration => "duration_secs",
            TrackSortField::Year => "year",
        };

        sql.push_str(&format!(
            " ORDER BY {} {}",
            order_field,
            if filter.sort_desc { "DESC" } else { "ASC" }
        ));

        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }

        if let Some(offset) = filter.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;

        let tracks = stmt
            .query_map(params_refs.as_slice(), Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    async fn find_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare("SELECT * FROM tracks WHERE id = ?")?;

        let track = stmt
            .query_row(params![id.to_string()], Self::row_to_track)
            .ok();

        Ok(track)
    }

    async fn find_by_path(
        &self,
        path: &str,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare("SELECT * FROM tracks WHERE file_path = ?")?;

        let track = stmt.query_row(params![path], Self::row_to_track).ok();

        Ok(track)
    }

    async fn find_by_artist(
        &self,
        artist: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt =
            conn.prepare("SELECT * FROM tracks WHERE artist LIKE ? ORDER BY album, track_number")?;

        let tracks = stmt
            .query_map(params![format!("%{}%", artist)], Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    async fn find_by_album(
        &self,
        album: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare(
            "SELECT * FROM tracks WHERE album LIKE ? ORDER BY disc_number, track_number",
        )?;

        let tracks = stmt
            .query_map(params![format!("%{}%", album)], Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT * FROM tracks WHERE title LIKE ? OR artist LIKE ? OR album LIKE ? LIMIT 50",
        )?;

        let tracks = stmt
            .query_map(params![&pattern, &pattern, &pattern], Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    async fn insert(&self, track: &Track) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        conn.execute(
            r#"INSERT INTO tracks (
                id, title, artist, album, album_artist, genre, year,
                track_number, disc_number, duration_secs, file_path,
                file_size, format, bitrate, sample_rate, album_art_path,
                date_added, last_played, play_count, is_downloaded, source_url
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            params![
                track.id.to_string(),
                track.title,
                track.artist,
                track.album,
                track.album_artist,
                track.genre,
                track.year,
                track.track_number,
                track.disc_number,
                track.duration_secs,
                track.file_path,
                track.file_size,
                track.format.to_string(),
                track.bitrate,
                track.sample_rate,
                track.album_art_path,
                track.date_added.to_rfc3339(),
                track.last_played.map(|d| d.to_rfc3339()),
                track.play_count,
                track.is_downloaded as i32,
                track.source_url,
            ],
        )?;
        Ok(())
    }

    async fn update(&self, track: &Track) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        conn.execute(
            r#"UPDATE tracks SET
                title = ?, artist = ?, album = ?, album_artist = ?, genre = ?, year = ?,
                track_number = ?, disc_number = ?, duration_secs = ?, file_path = ?,
                file_size = ?, format = ?, bitrate = ?, sample_rate = ?, album_art_path = ?,
                date_added = ?, last_played = ?, play_count = ?, is_downloaded = ?, source_url = ?
            WHERE id = ?"#,
            params![
                track.title,
                track.artist,
                track.album,
                track.album_artist,
                track.genre,
                track.year,
                track.track_number,
                track.disc_number,
                track.duration_secs,
                track.file_path,
                track.file_size,
                track.format.to_string(),
                track.bitrate,
                track.sample_rate,
                track.album_art_path,
                track.date_added.to_rfc3339(),
                track.last_played.map(|d| d.to_rfc3339()),
                track.play_count,
                track.is_downloaded as i32,
                track.source_url,
                track.id.to_string(),
            ],
        )?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        conn.execute("DELETE FROM tracks WHERE id = ?", params![id.to_string()])?;
        Ok(())
    }

    async fn delete_many(
        &self,
        ids: Vec<Uuid>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let ids_str: Vec<String> = ids.iter().map(|u| u.to_string()).collect();
        let placeholders = ids_str.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        conn.execute(
            &format!("DELETE FROM tracks WHERE id IN ({})", placeholders),
            rusqlite::params_from_iter(&ids_str),
        )?;
        Ok(())
    }

    async fn count(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    async fn total_duration(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let duration: i64 = conn.query_row(
            "SELECT COALESCE(SUM(duration_secs), 0) FROM tracks",
            [],
            |row| row.get(0),
        )?;
        Ok(duration as u64)
    }

    async fn recent(
        &self,
        limit: u32,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare("SELECT * FROM tracks ORDER BY date_added DESC LIMIT ?")?;
        let tracks = stmt
            .query_map(params![limit], Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    async fn most_played(
        &self,
        limit: u32,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare(
            "SELECT * FROM tracks WHERE play_count > 0 ORDER BY play_count DESC LIMIT ?",
        )?;
        let tracks = stmt
            .query_map(params![limit], Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    async fn set_favorite(
        &self,
        id: &str,
        is_favorite: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        conn.execute(
            "UPDATE tracks SET is_favorite = ? WHERE id = ?",
            params![is_favorite as i32, id],
        )?;
        Ok(())
    }
}

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
            .filter_map(|r| r.ok())
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

// ============================================================================
// Settings Repository
// ============================================================================

/// SQLite-backed settings repository
pub struct SqliteSettingsRepository {
    db: Arc<Database>,
}

impl SqliteSettingsRepository {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SettingsRepository for SqliteSettingsRepository {
    async fn get_settings(&self) -> Result<Settings, Box<dyn std::error::Error + Send + Sync>> {
        let data: Option<String> = {
            let conn = self
                .db
                .connection()
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            conn.query_row("SELECT data FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .ok()
        };

        match data {
            Some(json_str) => {
                let settings: Settings = serde_json::from_str(&json_str).map_err(|e| {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Failed to parse settings: {e}"),
                    )) as Box<dyn std::error::Error + Send + Sync>
                })?;
                Ok(settings)
            }
            None => {
                let default_settings = Settings::default();
                self.save_settings(&default_settings).await.map_err(|e| {
                    error!(error = %e, "Failed to save default settings");
                    e
                })?;
                Ok(default_settings)
            }
        }
    }

    async fn save_settings(
        &self,
        settings: &Settings,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let json_str = serde_json::to_string(settings).map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to serialize settings: {e}"),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;

        conn.execute(
            "INSERT OR REPLACE INTO settings (id, data) VALUES (1, ?)",
            params![json_str],
        )?;

        info!("Settings saved successfully");
        Ok(())
    }
}

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
            .filter_map(|r| r.ok())
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
                id, name, device_type, ip_address, paired_at, last_sync, status, library_version
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
            params![
                device.id.to_string(),
                device.name,
                device.device_type.to_string(),
                device.ip_address,
                device.paired_at.to_rfc3339(),
                device.last_sync.map(|d| d.to_rfc3339()),
                device.status.to_string(),
                device.library_version as i64,
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
            .filter_map(|r| r.ok())
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

// ============================================================================
// Helper functions
// ============================================================================

pub fn parse_format(s: &str) -> AudioFormat {
    match s.to_lowercase().as_str() {
        "mp3" => AudioFormat::Mp3,
        "flac" => AudioFormat::Flac,
        "aac" => AudioFormat::Aac,
        "ogg" => AudioFormat::Ogg,
        "wav" => AudioFormat::Wav,
        "m4a" => AudioFormat::M4a,
        _ => AudioFormat::Mp3,
    }
}

pub fn parse_datetime(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_track_repository_crud() {
        let db_path = std::env::temp_dir().join(format!("test_auralis_repo_{}.db", Uuid::new_v4()));
        let db = Database::new(&db_path).unwrap();
        db.run_migrations().unwrap();
        let db_arc = Arc::new(db);
        let repo = SqliteTrackRepository::new(db_arc);

        let track = Track::new(
            "Test Favorite Track".to_string(),
            "/path/to/test.mp3".to_string(),
            200,
            AudioFormat::Mp3,
        );
        repo.insert(&track).await.unwrap();

        let track_id_str = track.id.to_string();
        repo.set_favorite(&track_id_str, true).await.unwrap();

        let found = repo.find_by_id(track.id).await.unwrap().unwrap();
        assert_eq!(found.title, "Test Favorite Track");

        repo.set_favorite(&track_id_str, false).await.unwrap();

        let _ = std::fs::remove_file(&db_path);
    }
}

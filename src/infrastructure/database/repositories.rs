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
use uuid::Uuid;

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
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
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
}

// Helper functions
fn parse_format(s: &str) -> AudioFormat {
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

fn parse_datetime(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

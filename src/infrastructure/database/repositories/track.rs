//! SQLite-backed track repository implementation.

use async_trait::async_trait;
use rusqlite::params;
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

use super::{parse_datetime, parse_format};
use crate::domain::models::*;
use crate::domain::repositories::*;
use crate::infrastructure::database::Database;

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
            file_size: row.get::<_, i64>(11)? as u64,
            format: parse_format(&row.get::<_, String>(12)?),
            bitrate: row.get(13)?,
            sample_rate: row.get(14)?,
            album_art_path: row.get(15)?,
            date_added: parse_datetime(&row.get::<_, String>(16)?),
            last_played: last_played_str.as_deref().map(parse_datetime),
            play_count: row.get(18)?,
            is_downloaded: row.get::<_, i32>(19)? != 0,
            source_url: row.get(20)?,
            is_favorite: row
                .get::<_, Option<i32>>(21)
                .unwrap_or_default()
                .unwrap_or(0)
                != 0,
            mtime: row.get::<_, Option<i64>>(22)?.unwrap_or(0),
        })
    }

    /// Get summary of all albums with track counts and artwork (aggregated via SQLite)
    pub async fn get_albums_summary(
        &self,
    ) -> Result<Vec<AlbumSummary>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare(
            "SELECT album, artist, album_art_path, COUNT(*) as track_count, MIN(id) as first_track_id \
             FROM tracks \
             WHERE album IS NOT NULL AND album != '' \
             GROUP BY album \
             ORDER BY album COLLATE NOCASE ASC",
        )?;

        let albums = stmt
            .query_map([], |row| {
                Ok(AlbumSummary {
                    album: row.get(0)?,
                    artist: row.get(1)?,
                    album_art_path: row.get(2)?,
                    track_count: row.get::<_, i64>(3)? as u32,
                    first_track_id: row.get(4)?,
                })
            })?
            .filter_map(|r| match r {
                Ok(a) => Some(a),
                Err(e) => {
                    warn!(error = %e, "Failed to map album summary row; skipping");
                    None
                }
            })
            .collect();

        Ok(albums)
    }

    /// Get summary of all artists with track counts (aggregated via SQLite)
    pub async fn get_artists_summary(
        &self,
    ) -> Result<Vec<ArtistSummary>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut stmt = conn.prepare(
            "SELECT artist, COUNT(*) as track_count, MIN(id) as first_track_id \
             FROM tracks \
             WHERE artist IS NOT NULL AND artist != '' \
             GROUP BY artist \
             ORDER BY artist COLLATE NOCASE ASC",
        )?;

        let artists = stmt
            .query_map([], |row| {
                Ok(ArtistSummary {
                    artist: row.get(0)?,
                    track_count: row.get::<_, i64>(1)? as u32,
                    first_track_id: row.get(2)?,
                })
            })?
            .filter_map(|r| match r {
                Ok(a) => Some(a),
                Err(e) => {
                    warn!(error = %e, "Failed to map artist summary row; skipping");
                    None
                }
            })
            .collect();

        Ok(artists)
    }
}

/// Build the `WHERE` clause and ordered param bindings shared by `find_all`
/// and `count_filtered`. Returns the SQL fragment (starting with `WHERE`) and
/// the params to bind in the same order.
///
/// NOTE: `LIKE '%term%'` with a leading `%` defeats the `idx_tracks_title`
/// (and `idx_tracks_artist`/`album`) B-tree indexes — SQLite must do a full
/// scan for that clause. For short `term` values we could use a prefix
/// pattern `term%` that *does* use the index, but that changes semantics
/// (substring vs prefix). The correct future fix is an `FTS5` virtual table
/// or a `COLLATE NOCASE` trigram index, which requires a schema migration.
/// We intentionally do **not** migrate now (per task constraints) and keep
/// the substring search, but the comment documents the trade-off. If we
/// ever re-enable prefix optimization for `term.len() > 2`, it would be:
/// `title LIKE 'term%' OR title LIKE '%term%'` with a `UNION` or `OR` — not
/// done here to avoid surprising results.
/// Escape user input for use inside a SQL `LIKE` pattern.
///
/// `%`, `_`, and the escape character itself are prefixed with `\`, so a
/// search for `50%` matches the literal string `50%` instead of acting as a
/// wildcard. Every `LIKE` clause built from this helper must carry
/// `ESCAPE '\'` — without it the backslashes are literal characters and the
/// escaping silently stops working. That clause is load-bearing, not noise.
pub(crate) fn escape_like_pattern(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\\' || c == '%' || c == '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn track_filter_where(filter: &TrackFilter) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut sql = String::from("WHERE 1=1");
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(search) = &filter.search {
        // Leading `%` defeats index — see doc comment above. No migration now.
        // `ESCAPE '\'` pairs with `escape_like_pattern`: it is what makes the
        // backslash an escape character rather than a literal. Do not drop it.
        sql.push_str(
            " AND (title LIKE ? ESCAPE '\\' OR artist LIKE ? ESCAPE '\\' OR album LIKE ? ESCAPE '\\')",
        );
        let pattern = format!("%{}%", escape_like_pattern(search));
        params_vec.push(Box::new(pattern.clone()));
        params_vec.push(Box::new(pattern.clone()));
        params_vec.push(Box::new(pattern));
    }

    if let Some(artist) = &filter.artist {
        sql.push_str(" AND artist LIKE ? ESCAPE '\\'");
        params_vec.push(Box::new(format!("%{}%", escape_like_pattern(artist))));
    }

    if let Some(album) = &filter.album {
        sql.push_str(" AND album LIKE ? ESCAPE '\\'");
        params_vec.push(Box::new(format!("%{}%", escape_like_pattern(album))));
    }

    if filter.downloaded_only {
        sql.push_str(" AND is_downloaded = 1");
    }

    if let Some(fav) = filter.is_favorite {
        sql.push_str(" AND is_favorite = ?");
        params_vec.push(Box::new(if fav { 1 } else { 0 }));
    }

    (sql, params_vec)
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

        let (where_sql, mut params_vec) = track_filter_where(&filter);
        let mut sql = String::from("SELECT * FROM tracks ");
        sql.push_str(&where_sql);

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
            sql.push_str(" LIMIT ?");
            params_vec.push(Box::new(limit));
        }

        if let Some(offset) = filter.offset {
            sql.push_str(" OFFSET ?");
            params_vec.push(Box::new(offset));
        }

        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;

        let tracks = stmt
            .query_map(params_refs.as_slice(), Self::row_to_track)?
            .filter_map(|r| match r {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!(error = %e, "Failed to map track row; skipping");
                    None
                }
            })
            .collect();

        Ok(tracks)
    }

    async fn count_filtered(
        &self,
        filter: TrackFilter,
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let (where_sql, params_vec) = track_filter_where(&filter);
        let sql = format!("SELECT COUNT(*) FROM tracks {where_sql}");
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn.query_row(&sql, params_refs.as_slice(), |r| r.get(0))?;
        Ok(count.max(0) as u64)
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

    async fn find_by_paths(
        &self,
        paths: &[&str],
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let mut unique_paths: Vec<&str> = paths.to_vec();
        unique_paths.sort_unstable();
        unique_paths.dedup();

        let mut track_map = std::collections::HashMap::with_capacity(unique_paths.len());
        const CHUNK_SIZE: usize = 500;

        for chunk in unique_paths.chunks(CHUNK_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("SELECT * FROM tracks WHERE file_path IN ({})", placeholders);
            let params_refs: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_refs.as_slice(), Self::row_to_track)?;
            for r in rows {
                match r {
                    Ok(t) => {
                        track_map.insert(t.file_path.clone(), t);
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to map track row in find_by_paths; skipping");
                    }
                }
            }
        }

        let mut result = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(track) = track_map.get(*path) {
                result.push(track.clone());
            }
        }

        Ok(result)
    }

    async fn find_by_ids(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let mut unique_ids: Vec<Uuid> = ids.to_vec();
        unique_ids.sort_unstable();
        unique_ids.dedup();

        let mut track_map = std::collections::HashMap::with_capacity(unique_ids.len());
        const CHUNK_SIZE: usize = 500;
        let ids_str: Vec<String> = unique_ids.iter().map(|u| u.to_string()).collect();

        for chunk in ids_str.chunks(CHUNK_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("SELECT * FROM tracks WHERE id IN ({})", placeholders);
            let params_refs: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_refs.as_slice(), Self::row_to_track)?;
            for r in rows {
                match r {
                    Ok(t) => {
                        track_map.insert(t.id, t);
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to map track row in find_by_ids; skipping");
                    }
                }
            }
        }

        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(track) = track_map.get(id) {
                result.push(track.clone());
            } else {
                warn!(id = %id, "Track in playlist no longer exists");
            }
        }

        Ok(result)
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
            .filter_map(|r| match r {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!(error = %e, "Failed to map track row in find_by_artist; skipping");
                    None
                }
            })
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
            .filter_map(|r| match r {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!(error = %e, "Failed to map track row in find_by_album; skipping");
                    None
                }
            })
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
            .filter_map(|r| match r {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!(error = %e, "Failed to map track row in search; skipping");
                    None
                }
            })
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
                date_added, last_played, play_count, is_downloaded, source_url, is_favorite, mtime
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
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
                track.file_size as i64,
                track.format.to_string(),
                track.bitrate,
                track.sample_rate,
                track.album_art_path,
                track.date_added.to_rfc3339(),
                track.last_played.map(|d| d.to_rfc3339()),
                track.play_count,
                track.is_downloaded as i32,
                track.source_url,
                track.is_favorite as i32,
                track.mtime,
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
                date_added = ?, last_played = ?, play_count = ?, is_downloaded = ?, source_url = ?,
                is_favorite = ?, mtime = ?
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
                track.file_size as i64,
                track.format.to_string(),
                track.bitrate,
                track.sample_rate,
                track.album_art_path,
                track.date_added.to_rfc3339(),
                track.last_played.map(|d| d.to_rfc3339()),
                track.play_count,
                track.is_downloaded as i32,
                track.source_url,
                track.is_favorite as i32,
                track.mtime,
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
        if ids.is_empty() {
            return Ok(());
        }
        let mut conn = self
            .db
            .connection()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        // Chunk to avoid SQLITE_MAX_VARIABLE_NUMBER (default 999, some
        // builds 32766). 500 is safely below both and keeps the SQL string
        // small for the query planner.
        const CHUNK_SIZE: usize = 500;
        let ids_str: Vec<String> = ids.iter().map(|u| u.to_string()).collect();
        let tx = conn.transaction()?;
        for chunk in ids_str.chunks(CHUNK_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("DELETE FROM tracks WHERE id IN ({})", placeholders);
            tx.execute(&sql, rusqlite::params_from_iter(chunk))?;
            debug!(count = chunk.len(), "Deleted chunk of tracks");
        }
        tx.commit()?;
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
            .filter_map(|r| match r {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!(error = %e, "Failed to map track row in recent; skipping");
                    None
                }
            })
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
            .filter_map(|r| match r {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!(error = %e, "Failed to map track row in most_played; skipping");
                    None
                }
            })
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

    async fn get_albums_summary(
        &self,
    ) -> Result<Vec<AlbumSummary>, Box<dyn std::error::Error + Send + Sync>> {
        SqliteTrackRepository::get_albums_summary(self).await
    }

    async fn get_artists_summary(
        &self,
    ) -> Result<Vec<ArtistSummary>, Box<dyn std::error::Error + Send + Sync>> {
        SqliteTrackRepository::get_artists_summary(self).await
    }
}

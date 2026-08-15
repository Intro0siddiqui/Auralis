//! Playlist Commands
//!
//! Tauri command handlers for playlist management.

use crate::domain::models::{
    Playlist, SmartPlaylistCriteria, SmartSortField, Track, TrackFilter, TrackSortField,
};
use crate::domain::repositories::{PlaylistRepository, TrackRepository};
use crate::infrastructure::database::Database;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

/// Playlist creation request
#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePlaylistRequest {
    pub name: String,
    pub description: Option<String>,
    pub track_ids: Option<Vec<Uuid>>,
}

/// Playlist update request
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdatePlaylistRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Reorder request
#[derive(Debug, Serialize, Deserialize)]
pub struct ReorderRequest {
    pub track_ids: Vec<Uuid>,
}

/// Create a playlist repository from the database state
fn playlist_repo(db: &Database) -> Arc<dyn PlaylistRepository> {
    Arc::new(
        crate::infrastructure::database::repositories::SqlitePlaylistRepository::new(Arc::new(
            db.clone(),
        )),
    )
}

/// Create a track repository from the database state
fn track_repo(db: &Database) -> Arc<dyn TrackRepository> {
    Arc::new(
        crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
            db.clone(),
        )),
    )
}

pub const SMART_FAVORITES_ID: &str = "smart_favorites";
pub const SMART_RECENT_ID: &str = "smart_recent";
pub const SMART_MOST_PLAYED_ID: &str = "smart_most_played";

pub const SMART_FAVORITES_UUID: Uuid = Uuid::from_u128(1);
pub const SMART_RECENT_UUID: Uuid = Uuid::from_u128(2);
pub const SMART_MOST_PLAYED_UUID: Uuid = Uuid::from_u128(3);

/// Build virtual built-in smart playlists populated with real tracks.
async fn build_smart_playlists(tracks_repo: &Arc<dyn TrackRepository>) -> Vec<Playlist> {
    let mut smart = Vec::with_capacity(3);

    // 1. Favorites
    let fav_filter = TrackFilter {
        is_favorite: Some(true),
        sort_by: Some(TrackSortField::DateAdded),
        sort_desc: true,
        ..Default::default()
    };
    let fav_tracks = tracks_repo.find_all(fav_filter).await.unwrap_or_default();
    let mut fav_pl = Playlist::new("Favorites".to_string());
    fav_pl.id = SMART_FAVORITES_UUID;
    fav_pl.description = Some("Your favorite liked tracks".to_string());
    fav_pl.is_smart = true;
    fav_pl.track_ids = fav_tracks.into_iter().map(|t| t.id).collect();
    smart.push(fav_pl);

    // 2. Recently Added
    let recent_tracks = tracks_repo.recent(100).await.unwrap_or_default();
    let mut recent_pl = Playlist::new("Recently Added".to_string());
    recent_pl.id = SMART_RECENT_UUID;
    recent_pl.description = Some("Recently added tracks in your library".to_string());
    recent_pl.is_smart = true;
    recent_pl.smart_criteria = Some(SmartPlaylistCriteria {
        sort_by: SmartSortField::DateAdded,
        sort_desc: true,
        limit: 100,
        ..Default::default()
    });
    recent_pl.track_ids = recent_tracks.into_iter().map(|t| t.id).collect();
    smart.push(recent_pl);

    // 3. Most Played
    let most_played_tracks = tracks_repo.most_played(100).await.unwrap_or_default();
    let mut most_played_pl = Playlist::new("Most Played".to_string());
    most_played_pl.id = SMART_MOST_PLAYED_UUID;
    most_played_pl.description = Some("Your most frequently played tracks".to_string());
    most_played_pl.is_smart = true;
    most_played_pl.smart_criteria = Some(SmartPlaylistCriteria {
        sort_by: SmartSortField::PlayCount,
        sort_desc: true,
        limit: 100,
        ..Default::default()
    });
    most_played_pl.track_ids = most_played_tracks.into_iter().map(|t| t.id).collect();
    smart.push(most_played_pl);

    smart
}

/// Load tracks for the given playlist, preserving playlist order.
async fn tracks_for_playlist(repo: &Arc<dyn TrackRepository>, track_ids: &[Uuid]) -> Vec<Track> {
    let mut tracks = Vec::with_capacity(track_ids.len());
    for id in track_ids {
        match repo.find_by_id(*id).await {
            Ok(Some(track)) => tracks.push(track),
            Ok(None) => tracing::warn!(id = %id, "Track in playlist no longer exists"),
            Err(e) => {
                tracing::error!(id = %id, error = %e, "Failed to fetch track for playlist")
            }
        }
    }
    tracks
}

/// Get all playlists (including built-in Smart Playlists).
#[tauri::command]
pub async fn get_playlists(db: State<'_, Database>) -> Result<Vec<Playlist>, String> {
    let pl_repo = playlist_repo(&db);
    let tr_repo = track_repo(&db);

    let user_playlists = pl_repo.find_all().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch playlists");
        format!("Failed to fetch playlists: {e}")
    })?;

    let mut all_playlists = build_smart_playlists(&tr_repo).await;
    for pl in user_playlists {
        if pl.id != SMART_FAVORITES_UUID
            && pl.id != SMART_RECENT_UUID
            && pl.id != SMART_MOST_PLAYED_UUID
        {
            all_playlists.push(pl);
        }
    }

    Ok(all_playlists)
}

/// Get a single playlist with its tracks (dynamically resolved for smart playlists).
#[tauri::command]
pub async fn get_playlist(
    db: State<'_, Database>,
    id: String,
) -> Result<Option<(Playlist, Vec<Track>)>, String> {
    let tr_repo = track_repo(&db);

    if id == SMART_FAVORITES_ID || id == SMART_FAVORITES_UUID.to_string() {
        let fav_filter = TrackFilter {
            is_favorite: Some(true),
            sort_by: Some(TrackSortField::DateAdded),
            sort_desc: true,
            ..Default::default()
        };
        let tracks = tr_repo.find_all(fav_filter).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch favorites tracks");
            format!("Failed to fetch favorites tracks: {e}")
        })?;

        let mut playlist = Playlist::new("Favorites".to_string());
        playlist.id = SMART_FAVORITES_UUID;
        playlist.description = Some("Your favorite liked tracks".to_string());
        playlist.is_smart = true;
        playlist.track_ids = tracks.iter().map(|t| t.id).collect();
        return Ok(Some((playlist, tracks)));
    }

    if id == SMART_RECENT_ID || id == SMART_RECENT_UUID.to_string() {
        let tracks = tr_repo.recent(100).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch recent tracks");
            format!("Failed to fetch recent tracks: {e}")
        })?;

        let mut playlist = Playlist::new("Recently Added".to_string());
        playlist.id = SMART_RECENT_UUID;
        playlist.description = Some("Recently added tracks in your library".to_string());
        playlist.is_smart = true;
        playlist.smart_criteria = Some(SmartPlaylistCriteria {
            sort_by: SmartSortField::DateAdded,
            sort_desc: true,
            limit: 100,
            ..Default::default()
        });
        playlist.track_ids = tracks.iter().map(|t| t.id).collect();
        return Ok(Some((playlist, tracks)));
    }

    if id == SMART_MOST_PLAYED_ID || id == SMART_MOST_PLAYED_UUID.to_string() {
        let tracks = tr_repo.most_played(100).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch most played tracks");
            format!("Failed to fetch most played tracks: {e}")
        })?;

        let mut playlist = Playlist::new("Most Played".to_string());
        playlist.id = SMART_MOST_PLAYED_UUID;
        playlist.description = Some("Your most frequently played tracks".to_string());
        playlist.is_smart = true;
        playlist.smart_criteria = Some(SmartPlaylistCriteria {
            sort_by: SmartSortField::PlayCount,
            sort_desc: true,
            limit: 100,
            ..Default::default()
        });
        playlist.track_ids = tracks.iter().map(|t| t.id).collect();
        return Ok(Some((playlist, tracks)));
    }

    let parsed_id = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return Ok(None),
    };

    let pl_repo = playlist_repo(&db);
    let playlist = pl_repo.find_by_id(parsed_id).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch playlist");
        format!("Failed to fetch playlist: {e}")
    })?;

    match playlist {
        Some(playlist) => {
            let tracks = tracks_for_playlist(&tr_repo, &playlist.track_ids).await;
            Ok(Some((playlist, tracks)))
        }
        None => Ok(None),
    }
}

/// Create a new playlist.
#[tauri::command]
pub async fn create_playlist(
    db: State<'_, Database>,
    request: CreatePlaylistRequest,
) -> Result<Playlist, String> {
    let repo = playlist_repo(&db);

    let mut playlist = Playlist::new(request.name);
    playlist.description = request.description;
    if let Some(track_ids) = request.track_ids {
        playlist.add_tracks(track_ids);
    }

    repo.insert(&playlist).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to create playlist");
        format!("Failed to create playlist: {e}")
    })?;

    tracing::info!(id = %playlist.id, name = %playlist.name, "Playlist created");
    Ok(playlist)
}

/// Update playlist metadata.
#[tauri::command]
pub async fn update_playlist(
    db: State<'_, Database>,
    id: Uuid,
    request: UpdatePlaylistRequest,
) -> Result<Playlist, String> {
    let repo = playlist_repo(&db);

    let mut playlist = repo
        .find_by_id(id)
        .await
        .map_err(|e| format!("Failed to fetch playlist: {e}"))?
        .ok_or_else(|| format!("Playlist not found: {id}"))?;

    if let Some(name) = request.name {
        playlist.name = name;
    }
    if let Some(description) = request.description {
        playlist.description = Some(description);
    }
    playlist.updated_at = chrono::Utc::now();

    repo.update(&playlist).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to update playlist");
        format!("Failed to update playlist: {e}")
    })?;

    tracing::info!(id = %playlist.id, "Playlist updated");
    Ok(playlist)
}

/// Delete a playlist.
#[tauri::command]
pub async fn delete_playlist(db: State<'_, Database>, id: Uuid) -> Result<(), String> {
    let repo = playlist_repo(&db);

    repo.delete(id).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to delete playlist");
        format!("Failed to delete playlist: {e}")
    })?;

    tracing::info!(id = %id, "Playlist deleted");
    Ok(())
}

/// Add tracks to a playlist.
#[tauri::command]
pub async fn add_tracks_to_playlist(
    db: State<'_, Database>,
    playlist_id: Uuid,
    track_ids: Vec<Uuid>,
) -> Result<Playlist, String> {
    let repo = playlist_repo(&db);

    let mut playlist = repo
        .find_by_id(playlist_id)
        .await
        .map_err(|e| format!("Failed to fetch playlist: {e}"))?
        .ok_or_else(|| format!("Playlist not found: {playlist_id}"))?;

    playlist.add_tracks(track_ids);

    repo.update(&playlist).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to add tracks to playlist");
        format!("Failed to add tracks to playlist: {e}")
    })?;

    tracing::info!(id = %playlist.id, "Tracks added to playlist");
    Ok(playlist)
}

/// Remove tracks from a playlist.
#[tauri::command]
pub async fn remove_tracks_from_playlist(
    db: State<'_, Database>,
    playlist_id: Uuid,
    track_ids: Vec<Uuid>,
) -> Result<Playlist, String> {
    let repo = playlist_repo(&db);

    let mut playlist = repo
        .find_by_id(playlist_id)
        .await
        .map_err(|e| format!("Failed to fetch playlist: {e}"))?
        .ok_or_else(|| format!("Playlist not found: {playlist_id}"))?;

    for id in track_ids {
        playlist.remove_track(id);
    }

    repo.update(&playlist).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to remove tracks from playlist");
        format!("Failed to remove tracks from playlist: {e}")
    })?;

    tracing::info!(id = %playlist.id, "Tracks removed from playlist");
    Ok(playlist)
}

/// Reorder tracks within a playlist.
#[tauri::command]
pub async fn reorder_playlist_tracks(
    db: State<'_, Database>,
    playlist_id: Uuid,
    request: ReorderRequest,
) -> Result<Playlist, String> {
    let repo = playlist_repo(&db);

    let mut playlist = repo
        .find_by_id(playlist_id)
        .await
        .map_err(|e| format!("Failed to fetch playlist: {e}"))?
        .ok_or_else(|| format!("Playlist not found: {playlist_id}"))?;

    playlist.reorder(request.track_ids);

    repo.update(&playlist).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to reorder playlist tracks");
        format!("Failed to reorder playlist tracks: {e}")
    })?;

    tracing::info!(id = %playlist.id, "Playlist tracks reordered");
    Ok(playlist)
}

/// Create a smart playlist from criteria.
#[tauri::command]
pub async fn create_smart_playlist(
    db: State<'_, Database>,
    name: String,
    criteria: SmartPlaylistCriteria,
) -> Result<Playlist, String> {
    let repo = playlist_repo(&db);
    let tracks_repo = track_repo(&db);

    let filter = TrackFilter {
        artist: criteria.artist.clone(),
        album: criteria.album.clone(),
        genre: criteria.genre.clone(),
        downloaded_only: criteria.downloaded_only,
        limit: Some(criteria.limit.max(1)),
        sort_desc: criteria.sort_desc,
        sort_by: match criteria.sort_by {
            SmartSortField::Title => Some(TrackSortField::Title),
            SmartSortField::Artist => Some(TrackSortField::Artist),
            SmartSortField::Album => Some(TrackSortField::Album),
            SmartSortField::DateAdded => Some(TrackSortField::DateAdded),
            SmartSortField::LastPlayed => Some(TrackSortField::LastPlayed),
            SmartSortField::PlayCount => Some(TrackSortField::PlayCount),
            SmartSortField::Year => Some(TrackSortField::Year),
            SmartSortField::Random => None,
        },
        ..Default::default()
    };

    let mut tracks = tracks_repo.find_all(filter).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to resolve smart playlist tracks");
        format!("Failed to resolve smart playlist tracks: {e}")
    })?;

    tracks.retain(|t| {
        let year_ok = match (criteria.year_from, criteria.year_to, t.year) {
            (Some(from), Some(to), Some(y)) => y >= from && y <= to,
            (Some(from), None, Some(y)) => y >= from,
            (None, Some(to), Some(y)) => y <= to,
            _ => true,
        };
        let play_ok = match (criteria.min_play_count, criteria.max_play_count) {
            (Some(min), Some(max)) => t.play_count >= min && t.play_count <= max,
            (Some(min), None) => t.play_count >= min,
            (None, Some(max)) => t.play_count <= max,
            _ => true,
        };
        year_ok && play_ok
    });

    if matches!(criteria.sort_by, SmartSortField::Random) {
        use rand::seq::SliceRandom;
        tracks.shuffle(&mut rand::thread_rng());
    }

    let track_ids = tracks
        .into_iter()
        .take(criteria.limit as usize)
        .map(|t| t.id)
        .collect();

    let mut playlist = Playlist::new(name);
    playlist.is_smart = true;
    playlist.smart_criteria = Some(criteria);
    playlist.add_tracks(track_ids);

    repo.insert(&playlist).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to create smart playlist");
        format!("Failed to create smart playlist: {e}")
    })?;

    tracing::info!(id = %playlist.id, name = %playlist.name, "Smart playlist created");
    Ok(playlist)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::AudioFormat;
    use crate::infrastructure::database::repositories::SqliteTrackRepository;

    #[tokio::test]
    async fn test_smart_playlists_builder() {
        let db_path = std::env::temp_dir().join(format!("test_playlists_{}.db", Uuid::new_v4()));
        let db = Database::new(&db_path).unwrap();
        db.run_migrations().unwrap();
        let db_arc = Arc::new(db);
        let tr_repo: Arc<dyn TrackRepository> =
            Arc::new(SqliteTrackRepository::new(db_arc.clone()));

        // Insert tracks
        let mut track1 = Track::new(
            "Song 1".to_string(),
            "/music/1.mp3".to_string(),
            120,
            AudioFormat::Mp3,
        );
        track1.is_favorite = true;
        track1.play_count = 15;
        tr_repo.insert(&track1).await.unwrap();

        let mut track2 = Track::new(
            "Song 2".to_string(),
            "/music/2.mp3".to_string(),
            180,
            AudioFormat::Mp3,
        );
        track2.is_favorite = false;
        track2.play_count = 50;
        tr_repo.insert(&track2).await.unwrap();

        let mut track3 = Track::new(
            "Song 3".to_string(),
            "/music/3.mp3".to_string(),
            200,
            AudioFormat::Mp3,
        );
        track3.is_favorite = true;
        track3.play_count = 0;
        tr_repo.insert(&track3).await.unwrap();

        let smart = build_smart_playlists(&tr_repo).await;
        assert_eq!(smart.len(), 3);

        // Favorites should contain track1 and track3
        let favorites = smart.iter().find(|p| p.name == "Favorites").unwrap();
        assert_eq!(favorites.id, SMART_FAVORITES_UUID);
        assert!(favorites.is_smart);
        assert_eq!(favorites.track_ids.len(), 2);
        assert!(favorites.track_ids.contains(&track1.id));
        assert!(favorites.track_ids.contains(&track3.id));

        // Recently added should contain all 3
        let recent = smart.iter().find(|p| p.name == "Recently Added").unwrap();
        assert_eq!(recent.id, SMART_RECENT_UUID);
        assert_eq!(recent.track_ids.len(), 3);

        // Most played should contain track2 (50) and track1 (15), but not track3 (0)
        let most_played = smart.iter().find(|p| p.name == "Most Played").unwrap();
        assert_eq!(most_played.id, SMART_MOST_PLAYED_UUID);
        assert_eq!(most_played.track_ids.len(), 2);
        assert_eq!(most_played.track_ids[0], track2.id);
        assert_eq!(most_played.track_ids[1], track1.id);

        let _ = std::fs::remove_file(&db_path);
    }
}

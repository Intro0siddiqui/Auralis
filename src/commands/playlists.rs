//! Playlist Commands
//!
//! Tauri command handlers for playlist management.

use crate::domain::models::{
    Playlist, SmartPlaylistCriteria, SmartSortField, Track, TrackFilter, TrackSortField,
};
use crate::domain::repositories::{PlaylistRepository, TrackRepository};
use crate::infrastructure::database::Database;
use crate::templates::render;
use crate::templates::{PlaylistDetailTemplate, PlaylistsTemplate};
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

/// Get all playlists.
#[tauri::command]
pub async fn get_playlists(db: State<'_, Database>) -> Result<Vec<Playlist>, String> {
    let repo = playlist_repo(&db);

    let playlists = repo.find_all().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch playlists");
        format!("Failed to fetch playlists: {e}")
    })?;

    Ok(playlists)
}

/// Get a single playlist with its tracks.
#[tauri::command]
pub async fn get_playlist(
    db: State<'_, Database>,
    id: Uuid,
) -> Result<Option<(Playlist, Vec<Track>)>, String> {
    let repo = playlist_repo(&db);

    let playlist = repo.find_by_id(id).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch playlist");
        format!("Failed to fetch playlist: {e}")
    })?;

    match playlist {
        Some(playlist) => {
            let tracks = tracks_for_playlist(&track_repo(&db), &playlist.track_ids).await;
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

/// Render the playlists index page.
#[tauri::command]
pub async fn render_playlists(db: State<'_, Database>) -> Result<String, String> {
    let repo = playlist_repo(&db);

    let playlists = repo.find_all().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch playlists for render");
        format!("Failed to fetch playlists: {e}")
    })?;

    let tmpl = PlaylistsTemplate {
        playlists: &playlists,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render a single playlist detail page.
#[tauri::command]
pub async fn render_playlist_detail(db: State<'_, Database>, id: Uuid) -> Result<String, String> {
    let repo = playlist_repo(&db);

    let playlist = repo.find_by_id(id).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch playlist for render");
        format!("Failed to fetch playlist: {e}")
    })?;

    let (playlist, tracks) = match playlist {
        Some(playlist) => {
            let tracks = tracks_for_playlist(&track_repo(&db), &playlist.track_ids).await;
            (playlist, tracks)
        }
        None => (Playlist::new("Unknown".to_string()), Vec::new()),
    };

    let tmpl = PlaylistDetailTemplate {
        playlist: &playlist,
        tracks: &tracks,
        show_album: true,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

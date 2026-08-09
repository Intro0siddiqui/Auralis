//! Playlist Commands
//!
//! Tauri command handlers for playlist management.

use crate::domain::models::{Playlist, SmartPlaylistCriteria, Track};
use crate::templates::render;
use crate::templates::{PlaylistDetailTemplate, PlaylistsTemplate};
use serde::{Deserialize, Serialize};
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

/// Get all playlists.
#[tauri::command]
pub async fn get_playlists() -> Result<Vec<Playlist>, String> {
    // TODO: read from PlaylistRepository
    Ok(Vec::new())
}

/// Get a single playlist with its tracks.
#[tauri::command]
pub async fn get_playlist(_id: Uuid) -> Result<Option<(Playlist, Vec<Track>)>, String> {
    // TODO: join playlist + tracks
    Ok(None)
}

/// Create a new playlist.
#[tauri::command]
pub async fn create_playlist(request: CreatePlaylistRequest) -> Result<Playlist, String> {
    // TODO: persist via PlaylistRepository
    let mut playlist = Playlist::new(request.name);
    playlist.description = request.description;
    if let Some(track_ids) = request.track_ids {
        playlist.add_tracks(track_ids);
    }
    Ok(playlist)
}

/// Update playlist metadata.
#[tauri::command]
pub async fn update_playlist(
    _id: Uuid,
    _request: UpdatePlaylistRequest,
) -> Result<Playlist, String> {
    // TODO: load + mutate + persist
    Err("update_playlist not yet implemented".to_string())
}

/// Delete a playlist.
#[tauri::command]
pub async fn delete_playlist(_id: Uuid) -> Result<(), String> {
    // TODO: delete from repository
    Ok(())
}

/// Add tracks to a playlist.
#[tauri::command]
pub async fn add_tracks_to_playlist(
    _playlist_id: Uuid,
    _track_ids: Vec<Uuid>,
) -> Result<Playlist, String> {
    // TODO: load playlist, append tracks, persist
    Err("add_tracks_to_playlist not yet implemented".to_string())
}

/// Remove tracks from a playlist.
#[tauri::command]
pub async fn remove_tracks_from_playlist(
    _playlist_id: Uuid,
    _track_ids: Vec<Uuid>,
) -> Result<Playlist, String> {
    // TODO: load playlist, remove tracks, persist
    Err("remove_tracks_from_playlist not yet implemented".to_string())
}

/// Reorder tracks within a playlist.
#[tauri::command]
pub async fn reorder_playlist_tracks(
    _playlist_id: Uuid,
    _request: ReorderRequest,
) -> Result<Playlist, String> {
    // TODO: load playlist, reorder, persist
    Err("reorder_playlist_tracks not yet implemented".to_string())
}

/// Create a smart playlist from criteria.
#[tauri::command]
pub async fn create_smart_playlist(
    _name: String,
    _criteria: SmartPlaylistCriteria,
) -> Result<Playlist, String> {
    // TODO: persist and resolve tracks
    Err("create_smart_playlist not yet implemented".to_string())
}

/// Render the playlists index page.
#[tauri::command]
pub async fn render_playlists() -> Result<String, String> {
    let playlists: Vec<Playlist> = Vec::new();
    let tmpl = PlaylistsTemplate {
        playlists: &playlists,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render a single playlist detail page.
#[tauri::command]
pub async fn render_playlist_detail(_id: Uuid) -> Result<String, String> {
    let empty_playlist = Playlist::new("Unknown".to_string());
    let tracks: Vec<Track> = Vec::new();
    let tmpl = PlaylistDetailTemplate {
        playlist: &empty_playlist,
        tracks: &tracks,
        show_album: true,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

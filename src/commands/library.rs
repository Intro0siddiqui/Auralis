//! Library Commands
//!
//! Tauri command handlers for the music library domain. These commands
//! expose library operations to the HTMX frontend.

use crate::domain::models::{ScanSummary, Track, TrackFilter, TrackMetadataUpdate};
use crate::templates::render;
use crate::templates::{LibraryTemplate, SearchResultsTemplate, TrackListPartial, TrackRowPartial};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tracks query result wrapper
#[derive(Debug, Serialize, Deserialize)]
pub struct TracksPage {
    pub tracks: Vec<Track>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

/// Search query parameters
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<u32>,
}

/// Get a paginated list of tracks, optionally filtered.
#[tauri::command]
pub async fn get_tracks(filter: Option<TrackFilter>) -> Result<TracksPage, String> {
    let filter = filter.unwrap_or_default();
    // TODO: Wire to TrackRepository once database is fully initialized.
    Ok(TracksPage {
        tracks: Vec::new(),
        total: 0,
        offset: filter.offset.unwrap_or(0) as usize,
        limit: filter.limit.unwrap_or(50) as usize,
    })
}

/// Get a single track by ID.
#[tauri::command]
pub async fn get_track(_id: Uuid) -> Result<Option<Track>, String> {
    // TODO: Wire to TrackRepository.
    Ok(None)
}

/// Update track metadata.
#[tauri::command]
pub async fn update_track_metadata(
    _id: Uuid,
    _update: TrackMetadataUpdate,
) -> Result<Track, String> {
    // TODO: Wire to TrackRepository and persist.
    Err("update_track_metadata not yet implemented".to_string())
}

/// Delete one or more tracks.
#[tauri::command]
pub async fn delete_tracks(_ids: Vec<Uuid>) -> Result<u32, String> {
    // TODO: Wire to TrackRepository and remove files.
    Ok(0)
}

/// Trigger a library scan over the configured paths.
#[tauri::command]
pub async fn scan_library_paths(_paths: Option<Vec<String>>) -> Result<ScanSummary, String> {
    // TODO: Trigger scanner::scan_paths and return summary.
    Ok(ScanSummary {
        tracks_added: 0,
        tracks_updated: 0,
        tracks_removed: 0,
        errors: Vec::new(),
    })
}

/// Search tracks by free-text query (returns HTML fragment for HTMX).
#[tauri::command]
pub async fn search_tracks(query: SearchQuery) -> Result<String, String> {
    let results: Vec<Track> = Vec::new(); // TODO: full-text search via repository
    let tmpl = SearchResultsTemplate {
        tracks: &results,
        query: &query.q,
        show_album: true,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render the full library page as an HTML fragment (HTMX swap).
#[tauri::command]
pub async fn render_library(filter: Option<TrackFilter>) -> Result<String, String> {
    let filter = filter.unwrap_or_default();
    let tracks: Vec<Track> = Vec::new(); // TODO: pull from repository
    let tmpl = LibraryTemplate {
        tracks: &tracks,
        filter: &filter,
        total_count: 0,
        show_album: true,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render a track list as a partial (used for queue/search swaps).
#[tauri::command]
pub async fn render_track_list(_track_ids: Vec<Uuid>, show_album: bool) -> Result<String, String> {
    let tracks: Vec<Track> = Vec::new(); // TODO: lookup
    let tmpl = TrackListPartial {
        tracks: &tracks,
        show_album,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render a single track row.
#[tauri::command]
pub async fn render_track_row(track: Track, show_album: bool) -> Result<String, String> {
    let tmpl = TrackRowPartial {
        track: &track,
        show_album,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

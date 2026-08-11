//! Library Commands
//!
//! Tauri command handlers for the music library domain. These commands
//! expose library operations to the HTMX frontend.

use crate::domain::models::{ScanSummary, Track, TrackFilter, TrackMetadataUpdate};
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::database::Database;
use crate::infrastructure::filesystem::scanner::DirectoryScanner;
use crate::templates::render;
use crate::templates::{LibraryTemplate, SearchResultsTemplate, TrackListPartial, TrackRowPartial};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
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

/// Create a track repository from the database state
fn track_repo(db: &Database) -> Arc<dyn TrackRepository> {
    Arc::new(
        crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
            db.clone(),
        )),
    )
}

/// Get a paginated list of tracks, optionally filtered.
#[tauri::command]
pub async fn get_tracks(
    db: State<'_, Database>,
    filter: Option<TrackFilter>,
) -> Result<TracksPage, String> {
    let filter = filter.unwrap_or_default();
    let repo = track_repo(&db);

    let tracks = repo.find_all(filter.clone()).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch tracks");
        format!("Failed to fetch tracks: {e}")
    })?;

    let total = tracks.len();

    Ok(TracksPage {
        tracks,
        total,
        offset: filter.offset.unwrap_or(0) as usize,
        limit: filter.limit.unwrap_or(50) as usize,
    })
}

/// Get a single track by ID.
#[tauri::command]
pub async fn get_track(db: State<'_, Database>, id: Uuid) -> Result<Option<Track>, String> {
    let repo = track_repo(&db);

    repo.find_by_id(id).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch track");
        format!("Failed to fetch track: {e}")
    })
}

/// Update track metadata.
#[tauri::command]
pub async fn update_track_metadata(
    db: State<'_, Database>,
    id: Uuid,
    update: TrackMetadataUpdate,
) -> Result<Track, String> {
    let repo = track_repo(&db);

    let mut track = repo
        .find_by_id(id)
        .await
        .map_err(|e| format!("Failed to fetch track: {e}"))?
        .ok_or_else(|| format!("Track not found: {id}"))?;

    if let Some(title) = update.title {
        track.title = title;
    }
    if let Some(artist) = update.artist {
        track.artist = Some(artist);
    }
    if let Some(album) = update.album {
        track.album = Some(album);
    }
    if let Some(album_artist) = update.album_artist {
        track.album_artist = Some(album_artist);
    }
    if let Some(genre) = update.genre {
        track.genre = Some(genre);
    }
    if let Some(year) = update.year {
        track.year = Some(year);
    }
    if let Some(track_number) = update.track_number {
        track.track_number = Some(track_number);
    }
    if let Some(disc_number) = update.disc_number {
        track.disc_number = Some(disc_number);
    }

    repo.update(&track).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to update track");
        format!("Failed to update track: {e}")
    })?;

    tracing::info!(id = %id, "Track metadata updated");
    Ok(track)
}

/// Delete one or more tracks.
#[tauri::command]
pub async fn delete_tracks(db: State<'_, Database>, ids: Vec<Uuid>) -> Result<u32, String> {
    let repo = track_repo(&db);
    let count = ids.len() as u32;

    repo.delete_many(ids).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to delete tracks");
        format!("Failed to delete tracks: {e}")
    })?;

    tracing::info!(count = count, "Tracks deleted");
    Ok(count)
}

/// Trigger a library scan over the configured paths.
#[tauri::command]
pub async fn scan_library_paths(
    db: State<'_, Database>,
    paths: Option<Vec<String>>,
) -> Result<ScanSummary, String> {
    let scanner = DirectoryScanner::default_audio();

    let scan_paths: Vec<std::path::PathBuf> = match paths {
        Some(p) => p.into_iter().map(std::path::PathBuf::from).collect(),
        None => {
            let music_dir = dirs::audio_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            vec![music_dir]
        }
    };

    let repo = track_repo(&db);

    scanner
        .scan_library_paths(&scan_paths, repo)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Scan failed");
            format!("Scan failed: {e}")
        })
}

/// Search tracks by free-text query (returns HTML fragment for HTMX).
#[tauri::command]
pub async fn search_tracks(db: State<'_, Database>, query: SearchQuery) -> Result<String, String> {
    let repo = track_repo(&db);

    let results = repo.search(&query.q).await.map_err(|e| {
        tracing::error!(error = %e, "Search failed");
        format!("Search failed: {e}")
    })?;

    let tmpl = SearchResultsTemplate {
        tracks: &results,
        query: &query.q,
        show_album: true,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render the full library page as an HTML fragment (HTMX swap).
#[tauri::command]
pub async fn render_library(
    db: State<'_, Database>,
    filter: Option<TrackFilter>,
) -> Result<String, String> {
    let filter = filter.unwrap_or_default();
    let repo = track_repo(&db);

    let tracks = repo.find_all(filter.clone()).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch tracks for render");
        format!("Failed to fetch tracks: {e}")
    })?;

    let total_count = tracks.len();

    let tmpl = LibraryTemplate {
        tracks: &tracks,
        filter: &filter,
        total_count,
        show_album: true,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render a track list as a partial (used for queue/search swaps).
#[tauri::command]
pub async fn render_track_list(
    db: State<'_, Database>,
    track_ids: Vec<Uuid>,
    show_album: bool,
) -> Result<String, String> {
    let repo = track_repo(&db);

    let mut tracks = Vec::new();
    for id in track_ids {
        match repo.find_by_id(id).await {
            Ok(Some(track)) => tracks.push(track),
            Ok(None) => tracing::warn!(id = %id, "Track not found for render"),
            Err(e) => tracing::error!(id = %id, error = %e, "Failed to fetch track for render"),
        }
    }

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

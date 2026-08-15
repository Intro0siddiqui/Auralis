//! Library Commands
//!
//! Tauri command handlers for the music library domain. These commands
//! expose library operations to the HTMX frontend.

use crate::domain::models::{ScanSummary, Track, TrackFilter, TrackMetadataUpdate};
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::database::Database;
use crate::infrastructure::filesystem::scanner::DirectoryScanner;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use uuid::Uuid;

/// Tracks query result wrapper
#[derive(Debug, Serialize, Deserialize)]
pub struct TracksPage {
    pub tracks: Vec<Track>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
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
    app: tauri::AppHandle,
    db: State<'_, Database>,
    paths: Option<Vec<String>>,
) -> Result<ScanSummary, String> {
    let scanner = DirectoryScanner::default_audio();

    let scan_paths: Vec<std::path::PathBuf> = match paths {
        Some(p) => p.into_iter().map(std::path::PathBuf::from).collect(),
        None => {
            let mut default_paths = Vec::new();
            if let Some(music) = dirs::audio_dir() {
                if music.exists() {
                    default_paths.push(music);
                }
            }
            if let Some(download) = dirs::download_dir() {
                if download.exists() {
                    default_paths.push(download);
                }
            }
            // Android-specific standard storage paths
            for android_path in [
                "/storage/emulated/0/Music",
                "/storage/emulated/0/Download",
                "/storage/emulated/0/Audio",
                "/sdcard/Music",
                "/sdcard/Download",
            ] {
                let p = std::path::PathBuf::from(android_path);
                if p.exists() {
                    default_paths.push(p);
                }
            }

            // Also include internal app downloads directory
            if let Ok(app_dir) = app.path().app_data_dir() {
                let dl_dir = app_dir.join("downloads");
                if dl_dir.exists() {
                    default_paths.push(dl_dir);
                }
            }

            if default_paths.is_empty() {
                default_paths.push(std::path::PathBuf::from("."));
            }
            default_paths
        }
    };

    let repo = track_repo(&db);

    let summary = scanner
        .scan_library_paths(&scan_paths, repo)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Scan failed");
            format!("Scan failed: {e}")
        })?;

    // Notify the frontend that the scan finished so it can refresh.
    let _ = app.emit("library:scan_complete", &summary);

    Ok(summary)
}

/// Import an audio file directly from binary payload (bypasses Android 14/15/16 Scoped Storage restrictions)
#[tauri::command]
pub async fn import_audio_file(
    app: tauri::AppHandle,
    db: State<'_, Database>,
    name: String,
    data: Vec<u8>,
) -> Result<Track, String> {
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    let music_dir = app_dir.join("music");
    std::fs::create_dir_all(&music_dir)
        .map_err(|e| format!("Failed to create music directory: {e}"))?;

    let file_path = music_dir.join(&name);
    std::fs::write(&file_path, &data).map_err(|e| format!("Failed to write audio file: {e}"))?;

    let repo = track_repo(&db);
    let extractor = crate::infrastructure::filesystem::metadata::MetadataExtractor::new();
    let mut track = match extractor.extract(&file_path).await {
        Ok(t) => t,
        Err(_) => {
            // Fallback for minimal metadata when tags are missing
            let ext = std::path::Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp3");
            let format = crate::infrastructure::database::repositories::parse_format(ext);
            Track::new(
                file_path.to_string_lossy().to_string(),
                name.clone(),
                format,
            )
        }
    };

    if track.title.is_empty() {
        track.title = name;
    }

    repo.save(&track)
        .await
        .map_err(|e| format!("Failed to save track to database: {e}"))?;

    let _ = app.emit("library:track_imported", &track);
    Ok(track)
}

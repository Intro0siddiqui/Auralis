//! Library Commands
//!
//! Tauri command handlers for the music library domain. These commands
//! expose library operations to the HTMX frontend.

use crate::domain::models::{ScanSummary, Track, TrackFilter, TrackMetadataUpdate};
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::database::Database;
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
    let repo = track_repo(&db);
    let app_handle = app.clone();
    let app_log_handle = app.clone();

    #[cfg(target_os = "android")]
    {
        let scan_paths: Vec<std::path::PathBuf> = match paths {
            Some(p) => p.into_iter().map(std::path::PathBuf::from).collect(),
            None => {
                let mut default_paths = Vec::new();
                if let Ok(app_dir) = app.path().app_data_dir() {
                    let music_dir = app_dir.join("music");
                    if music_dir.exists() {
                        default_paths.push(music_dir);
                    }
                    let dl_dir = app_dir.join("downloads");
                    if dl_dir.exists() {
                        default_paths.push(dl_dir);
                    }
                }
                default_paths
            }
        };

        let _ = app.emit(
            "library:scan_log",
            format!(
                "🚀 Starting Android sandboxed library scan across {} path(s)",
                scan_paths.len()
            ),
        );
        for p in &scan_paths {
            let _ = app.emit(
                "library:scan_log",
                format!(
                    "📂 Sandboxed path: {} (exists: {})",
                    p.display(),
                    p.exists()
                ),
            );
        }

        let android_scanner = crate::infrastructure::filesystem::AndroidScanner::new();
        let summary = android_scanner
            .scan_sandboxed_dir(
                &scan_paths,
                repo,
                Some(
                    move |progress: crate::infrastructure::filesystem::scanner::ScanProgress| {
                        if !progress.current_file.is_empty() {
                            let _ = app_log_handle.emit(
                                "library:scan_log",
                                format!("🎵 Processing: {}", progress.current_file),
                            );
                        }
                        let _ = app_handle.emit("library:scan_progress", &progress);
                    },
                ),
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Android scan failed");
                let _ = app.emit("library:scan_log", format!("❌ Scan failed: {e}"));
                format!("Scan failed: {e}")
            })?;

        for err in &summary.errors {
            let _ = app.emit("library:scan_log", format!("⚠️ Error: {err}"));
        }

        let _ = app.emit(
            "library:scan_log",
            format!(
                "🎉 Scan finished: +{} tracks added, {} updated, {} removed, {} errors",
                summary.tracks_added,
                summary.tracks_updated,
                summary.tracks_removed,
                summary.errors.len()
            ),
        );

        let _ = app.emit("library:scan_complete", &summary);
        Ok(summary)
    }

    #[cfg(not(target_os = "android"))]
    {
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
                if let Ok(app_dir) = app.path().app_data_dir() {
                    let music_dir = app_dir.join("music");
                    if music_dir.exists() {
                        default_paths.push(music_dir);
                    }
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

        let _ = app.emit(
            "library:scan_log",
            format!(
                "🚀 Starting desktop library scan across {} target paths",
                scan_paths.len()
            ),
        );
        for p in &scan_paths {
            let _ = app.emit(
                "library:scan_log",
                format!(
                    "📂 Candidate path: {} (exists: {})",
                    p.display(),
                    p.exists()
                ),
            );
        }

        let desktop_scanner = crate::infrastructure::filesystem::DesktopScanner::default_audio();
        let summary = desktop_scanner
            .scan_library_paths_with_progress(
                &scan_paths,
                repo,
                Some(
                    move |progress: crate::infrastructure::filesystem::scanner::ScanProgress| {
                        if !progress.current_file.is_empty() {
                            let _ = app_log_handle.emit(
                                "library:scan_log",
                                format!("🎵 Processing: {}", progress.current_file),
                            );
                        }
                        let _ = app_handle.emit("library:scan_progress", &progress);
                    },
                ),
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Desktop scan failed");
                let _ = app.emit("library:scan_log", format!("❌ Scan failed: {e}"));
                format!("Scan failed: {e}")
            })?;

        for err in &summary.errors {
            let _ = app.emit("library:scan_log", format!("⚠️ Warning/Error: {err}"));
        }

        let _ = app.emit(
            "library:scan_log",
            format!(
                "🎉 Scan finished: +{} tracks added, {} updated, {} removed, {} errors",
                summary.tracks_added,
                summary.tracks_updated,
                summary.tracks_removed,
                summary.errors.len()
            ),
        );

        let _ = app.emit("library:scan_complete", &summary);
        Ok(summary)
    }
}

/// Import an audio file directly from binary or base64 payload (bypasses Android 14/15/16 Scoped Storage restrictions)
#[tauri::command]
pub async fn import_audio_file(
    app: tauri::AppHandle,
    db: State<'_, Database>,
    name: String,
    data: Option<Vec<u8>>,
    data_base64: Option<String>,
) -> Result<Track, String> {
    let bytes = match (data, data_base64) {
        (Some(b), _) => b,
        (_, Some(b64)) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| format!("Invalid base64 payload: {e}"))?
        }
        (None, None) => return Err("No audio data provided".to_string()),
    };

    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    let music_dir = app_dir.join("music");

    let repo = track_repo(&db);
    let android_scanner = crate::infrastructure::filesystem::AndroidScanner::new();
    let track = android_scanner
        .ingest_buffer(&name, &bytes, &music_dir, &repo)
        .await
        .map_err(|e| format!("Failed to ingest audio track: {e}"))?;

    let _ = app.emit("library:track_imported", &track);
    Ok(track)
}

/// Set favorite status for a track.
#[tauri::command]
pub async fn set_track_favorite(
    db: State<'_, Database>,
    id: String,
    favorite: bool,
) -> Result<(), String> {
    let repo = track_repo(&db);
    repo.set_favorite(&id, favorite).await.map_err(|e| {
        tracing::error!(error = %e, id = %id, "Failed to set track favorite");
        format!("Failed to set favorite: {e}")
    })?;

    tracing::info!(id = %id, favorite = favorite, "Track favorite status updated");
    Ok(())
}

/// Open native OS folder picker dialog (Windows Explorer / macOS Finder / Linux) and scan selected directory.
#[tauri::command]
pub async fn pick_folder_and_scan(
    app: tauri::AppHandle,
    db: State<'_, Database>,
) -> Result<Option<ScanSummary>, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;

        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog().file().pick_folder(move |folder_path| {
            let _ = tx.send(folder_path);
        });

        let folder_path = rx
            .await
            .map_err(|e| format!("Folder picker channel closed: {e}"))?;

        match folder_path {
            Some(path) => {
                let path_buf = path
                    .into_path()
                    .map_err(|e| format!("Invalid folder path: {e}"))?;
                let path_str = path_buf.to_string_lossy().to_string();
                tracing::info!(path = %path_str, "User selected custom folder to scan");
                let summary = scan_library_paths(app, db, Some(vec![path_str])).await?;
                Ok(Some(summary))
            }
            None => Ok(None),
        }
    }

    #[cfg(not(desktop))]
    {
        let _ = (app, db);
        Err("Native folder dialog is only available on desktop platforms. Please use Android SAF picker.".to_string())
    }
}

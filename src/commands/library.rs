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

    let total = repo.count_filtered(filter.clone()).await.unwrap_or(0) as usize;

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

    // Write updated metadata tags directly to audio file on disk via Lofty
    let path = std::path::Path::new(&track.file_path);
    if path.is_file() {
        let year_u32 = track
            .year
            .and_then(|y| if y > 0 { Some(y as u32) } else { None });
        if let Err(e) = crate::infrastructure::filesystem::metadata::write_metadata(
            path,
            &track.title,
            track.artist.as_deref().unwrap_or(""),
            track.album.as_deref().unwrap_or(""),
            track.genre.as_deref(),
            year_u32,
            track.track_number,
        ) {
            tracing::warn!(
                path = %track.file_path,
                error = %e,
                "Failed to write updated tag metadata to disk file (continuing DB update)"
            );
        } else {
            tracing::info!(path = %track.file_path, "Updated audio file tags on disk via lofty");
        }
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

/// Helper: Emit log messages when starting a library scan.
fn emit_scan_start_logs(
    app: &tauri::AppHandle,
    title: &str,
    path_label: &str,
    paths: &[std::path::PathBuf],
) {
    let _ = app.emit(
        "library:scan_log",
        format!("🚀 Starting {title} across {} target paths", paths.len()),
    );
    for p in paths {
        let _ = app.emit(
            "library:scan_log",
            format!("📂 {path_label}: {} (exists: {})", p.display(), p.exists()),
        );
    }
}

/// Helper: Create progress callback for library scanner.
fn create_scan_progress_callback(
    app: &tauri::AppHandle,
) -> impl FnMut(crate::infrastructure::filesystem::scanner::ScanProgress) + Send + 'static {
    let app_handle = app.clone();
    let app_log_handle = app.clone();
    move |progress: crate::infrastructure::filesystem::scanner::ScanProgress| {
        if !progress.current_file.is_empty() {
            let _ = app_log_handle.emit(
                "library:scan_log",
                format!("🎵 Processing: {}", progress.current_file),
            );
        }
        let _ = app_handle.emit("library:scan_progress", &progress);
    }
}

/// Helper: Emit log messages and event when library scan finishes.
fn emit_scan_completion(app: &tauri::AppHandle, summary: &ScanSummary, error_label: &str) {
    for err in &summary.errors {
        let _ = app.emit("library:scan_log", format!("⚠️ {error_label}: {err}"));
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

    let _ = app.emit("library:scan_complete", summary);
}

/// Trigger a library scan over the configured paths.
#[tauri::command]
pub async fn scan_library_paths(
    app: tauri::AppHandle,
    db: State<'_, Database>,
    paths: Option<Vec<String>>,
) -> Result<ScanSummary, String> {
    let repo = track_repo(&db);

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
                // Note: MediaStore Download/Auralis copy is for Files visibility only;
                // library scan stays sandboxed to avoid duplicate entries (internal + public).
                // `test-android-e2e` verifies the public copy separately via `ls`/`content query`.
                default_paths
            }
        };

        emit_scan_start_logs(
            &app,
            "Android library scan (MediaStore + sandboxed)",
            "Path",
            &scan_paths,
        );

        let android_scanner = crate::infrastructure::filesystem::AndroidScanner::new();
        let summary = android_scanner
            .scan_library_paths_with_progress(
                &scan_paths,
                repo,
                Some(create_scan_progress_callback(&app)),
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Android scan failed");
                let _ = app.emit("library:scan_log", format!("❌ Scan failed: {e}"));
                format!("Scan failed: {e}")
            })?;

        emit_scan_completion(&app, &summary, "Error");
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

        emit_scan_start_logs(&app, "desktop library scan", "Candidate path", &scan_paths);

        let desktop_scanner = crate::infrastructure::filesystem::DesktopScanner::default_audio();
        let summary = desktop_scanner
            .scan_library_paths_with_progress(
                &scan_paths,
                repo,
                Some(create_scan_progress_callback(&app)),
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Desktop scan failed");
                let _ = app.emit("library:scan_log", format!("❌ Scan failed: {e}"));
                format!("Scan failed: {e}")
            })?;

        emit_scan_completion(&app, &summary, "Warning/Error");
        Ok(summary)
    }
}

/// Import an audio file directly from binary or base64 payload (bypasses Android 14/15/16 Scoped Storage restrictions)
#[tauri::command(rename_all = "camelCase")]
pub async fn import_audio_file(
    app: tauri::AppHandle,
    db: State<'_, Database>,
    name: String,
    data: Option<Vec<u8>>,
    data_base64: Option<String>,
) -> Result<Track, String> {
    tracing::info!(name = %name, has_data = data.is_some(), has_b64 = data_base64.is_some(), "Importing audio file");
    let bytes = match (data, data_base64) {
        (Some(b), _) => b,
        (_, Some(b64)) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| {
                    tracing::error!(error = %e, name = %name, "Invalid base64 payload for audio file");
                    format!("Invalid base64 payload for {name}: {e}")
                })?
        }
        (None, None) => {
            tracing::error!(name = %name, "No audio data provided for import");
            return Err(format!("No audio data provided for {name}"));
        }
    };

    let app_dir = app.path().app_data_dir().map_err(|e| {
        tracing::error!(error = %e, "Failed to get app data dir");
        format!("Failed to get app data dir: {e}")
    })?;
    let music_dir = app_dir.join("music");

    let repo = track_repo(&db);
    let android_scanner = crate::infrastructure::filesystem::AndroidScanner::new();
    let track = android_scanner
        .ingest_buffer(&name, &bytes, &music_dir, &repo)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, name = %name, "Failed to ingest audio track");
            format!("Failed to ingest audio track {name}: {e}")
        })?;

    tracing::info!(id = %track.id, title = %track.title, artist = ?track.artist, "Successfully imported audio track");
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
#[tauri::command(rename_all = "snake_case")]
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
        Err("Native folder dialog is only available on desktop platforms.".to_string())
    }
}

/// Open native OS file picker dialog (macOS Finder, Windows Explorer, Linux)
/// and import selected audio files directly into the library.
#[tauri::command(rename_all = "snake_case")]
pub async fn pick_audio_files_and_import(
    app: tauri::AppHandle,
    db: State<'_, Database>,
) -> Result<Option<ScanSummary>, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;

        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .add_filter(
                "Audio Files",
                &["mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma"],
            )
            .pick_files(move |file_paths| {
                let _ = tx.send(file_paths);
            });

        let file_paths = rx
            .await
            .map_err(|e| format!("Audio file picker channel closed: {e}"))?;

        match file_paths {
            Some(paths) if !paths.is_empty() => {
                let app_dir = app
                    .path()
                    .app_data_dir()
                    .map_err(|e| format!("Failed to get app data dir: {e}"))?;
                let music_dir = app_dir.join("music");
                let repo = track_repo(&db);
                let android_scanner = crate::infrastructure::filesystem::AndroidScanner::new();

                let mut summary = ScanSummary {
                    tracks_added: 0,
                    tracks_updated: 0,
                    tracks_removed: 0,
                    errors: Vec::new(),
                };

                let total = paths.len();
                let mut valid_paths = Vec::with_capacity(total);
                for file_path in paths {
                    if let Ok(path_buf) = file_path.into_path() {
                        let path_str = path_buf.to_string_lossy().to_string();
                        valid_paths.push((path_buf, path_str));
                    } else {
                        summary
                            .errors
                            .push("Failed to resolve file path".to_string());
                    }
                }

                let path_strs: Vec<&str> = valid_paths.iter().map(|(_, s)| s.as_str()).collect();
                let existing_tracks = repo.find_by_paths(&path_strs).await.unwrap_or_default();
                let existing_map: std::collections::HashMap<String, Track> = existing_tracks
                    .into_iter()
                    .map(|t| (t.file_path.clone(), t))
                    .collect();

                for (idx, (path_buf, path_str)) in valid_paths.into_iter().enumerate() {
                    let file_name = path_buf
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "audio_track.mp3".to_string());

                    let _ = app.emit(
                        "library:scan_log",
                        format!("📥 Ingesting ({}/{}): {}", idx + 1, total, file_name),
                    );

                    match tokio::fs::read(&path_buf).await {
                        Ok(bytes) => {
                            match android_scanner
                                .ingest_buffer(&file_name, &bytes, &music_dir, &repo)
                                .await
                            {
                                Ok(track) => {
                                    summary.tracks_added += 1;
                                    let _ = app.emit("library:track_imported", &track);
                                }
                                Err(e) => {
                                    summary.errors.push(format!("{file_name}: {e}"));
                                }
                            }
                        }
                        Err(_) => {
                            let format = crate::infrastructure::filesystem::scanner::detect_format(
                                &path_buf,
                            )
                            .unwrap_or(crate::domain::models::AudioFormat::Mp3);
                            let mut track =
                                match crate::infrastructure::filesystem::MetadataExtractor::extract(
                                    &path_buf,
                                ) {
                                    Ok(t) => t,
                                    Err(_) => crate::domain::models::Track::new(
                                        file_name.clone(),
                                        path_str.clone(),
                                        0,
                                        format,
                                    ),
                                };
                            if track.title.trim().is_empty() || track.title == "Unknown" {
                                track.title = file_name.clone();
                            }
                            if let Some(ext) = existing_map.get(&path_str) {
                                track.id = ext.id;
                                let _ = repo.update(&track).await;
                                summary.tracks_updated += 1;
                            } else {
                                let _ = repo.insert(&track).await;
                                summary.tracks_added += 1;
                            }
                            let _ = app.emit("library:track_imported", &track);
                        }
                    }
                }

                let _ = app.emit("library:scan_complete", &summary);
                Ok(Some(summary))
            }
            _ => Ok(None),
        }
    }

    #[cfg(not(desktop))]
    {
        let _ = (app, db);
        Err("Native dialog is disabled on mobile; please use the web file picker.".to_string())
    }
}

/// Maximum size of artwork served through `media_data_url`.
const MEDIA_DATA_URL_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Read a local image file and return it as a `data:` URI so the webview can
/// render cover art even when the Tauri asset protocol is unavailable.
///
/// The requested path is canonicalized and must resolve inside one of the
/// app's managed library roots (the app data dir plus the OS music/download
/// folders used by the scanner); only image extensions are accepted and reads
/// are capped at [`MEDIA_DATA_URL_MAX_BYTES`].
#[tauri::command]
pub async fn media_data_url(app: tauri::AppHandle, path: String) -> Result<String, String> {
    use base64::Engine;
    use std::path::{Path, PathBuf};

    let extension = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    let mime = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        other => {
            tracing::warn!(extension = %other, "Rejected artwork with unsupported image extension");
            return Err(format!(
                "Unsupported image extension '{other}': allowed extensions are jpg, jpeg, png, webp"
            ));
        }
    };

    // Canonicalize to defeat traversal (`..`) tricks; symlinks are resolved to
    // their target so escapes out of the roots cannot pass the check below.
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .map_err(|e| format!("Invalid image path: {e}"))?;

    // Allowlist roots mirror the default scan paths of `scan_library_paths`.
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(app_dir) = app.path().app_data_dir() {
        roots.push(app_dir);
    }
    if let Some(music) = dirs::audio_dir() {
        roots.push(music);
    }
    if let Some(download) = dirs::download_dir() {
        roots.push(download);
    }

    let mut canonical_roots = Vec::with_capacity(roots.len());
    for root in &roots {
        if let Ok(canonical_root) = tokio::fs::canonicalize(root).await {
            canonical_roots.push(canonical_root);
        }
    }

    if canonical_roots.is_empty()
        || !canonical_roots
            .iter()
            .any(|root| canonical.starts_with(root))
    {
        tracing::warn!(path = %path, "Blocked artwork read outside managed library roots");
        return Err("Image path is outside the app's managed library folders".to_string());
    }

    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("Failed to stat image: {e}"))?;
    if !metadata.is_file() {
        return Err("Requested image path is not a regular file".to_string());
    }
    if metadata.len() > MEDIA_DATA_URL_MAX_BYTES {
        return Err(format!(
            "Image exceeds the {} MB limit",
            MEDIA_DATA_URL_MAX_BYTES / (1024 * 1024)
        ));
    }

    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|e| format!("Failed to read image: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

/// Escape HTML special characters for safe insertion into HTML strings.
pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Format duration in seconds as M:SS.
pub(crate) fn format_time(secs: u32) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

/// Render cover artwork image tag with fallback to Lucide icon.
pub(crate) fn render_art_tag(art_path: Option<&str>, alt_text: &str, icon_name: &str) -> String {
    match art_path {
        Some(path) if !path.trim().is_empty() => {
            let safe_alt = html_escape(alt_text);
            let safe_path = html_escape(path);
            let json_path =
                serde_json::to_string(path).unwrap_or_else(|_| format!("\"{}\"", safe_path));
            let safe_json_path = html_escape(&json_path);
            format!(
                r#"<img src="{safe_path}" alt="{safe_alt}" onerror="if(!this.dataset.fb){{this.dataset.fb='1';window.Auralis&amp;&amp;window.Auralis.bridge&amp;&amp;window.Auralis.bridge.embedArt(this,{safe_json_path})}}">"#
            )
        }
        _ => format!(r#"<i data-lucide="{icon_name}"></i>"#),
    }
}

/// Render a single track row HTML.
fn render_track_row_html(
    id: &str,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    duration_secs: u32,
    album_art_path: Option<&str>,
    is_favorite: bool,
) -> String {
    let safe_id = html_escape(id);
    let safe_title = html_escape(title);
    let artist_str = artist.unwrap_or("Unknown Artist");
    let safe_artist = html_escape(artist_str);
    let album_str = album.unwrap_or("Single");
    let safe_album = html_escape(album_str);
    let duration_str = format_time(duration_secs);
    let is_fav_class = if is_favorite { " liked" } else { "" };
    let is_fav_style = if is_favorite {
        r#" style="color: var(--like);""#
    } else {
        ""
    };
    let art_html = render_art_tag(album_art_path, title, "music");

    format!(
        r#"<div class="track-row glass-weak neu-glass" data-track-id="{safe_id}" data-role="play-row" style="cursor: pointer; margin-bottom: var(--space-2); border-radius: var(--radius-md); touch-action: manipulation;">
    <div class="track-row-artwork">
        {art_html}
    </div>
    <div class="track-row-info">
        <div class="track-row-title">{safe_title}</div>
        <div class="track-row-subtitle">{safe_artist} — {safe_album}</div>
    </div>
    <span class="track-row-duration">{duration_str}</span>
    <div class="track-row-actions">
        <button type="button" class="btn btn-ghost btn-icon play-track-btn" 
                title="Play" data-role="play-btn" data-track-id="{safe_id}" 
                onclick="event.stopPropagation(); window._safePlayTrack ? window._safePlayTrack('{safe_id}') : (window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.playTrack('{safe_id}'))" 
                ontouchend="event.stopPropagation(); window._safePlayTrack ? window._safePlayTrack('{safe_id}') : (window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.playTrack('{safe_id}'))">
            <i data-lucide="play"></i>
        </button>
        <button type="button" class="btn btn-ghost btn-icon{is_fav_class}"{is_fav_style} title="Like" onclick="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.toggleTrackFavorite('{safe_id}', this)" ontouchend="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.toggleTrackFavorite('{safe_id}', this)">
            <i data-lucide="heart"></i>
        </button>
        <button type="button" class="btn btn-ghost btn-icon track-menu-btn" 
                title="More options" data-role="menu-btn" data-track-id="{safe_id}" 
                onclick="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.openTrackContextMenu(event, '{safe_id}')" 
                ontouchend="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.openTrackContextMenu(event, '{safe_id}')">
            <i data-lucide="more-vertical"></i>
        </button>
    </div>
</div>"#
    )
}

/// Render HTML grid of albums from database.
#[tauri::command]
pub async fn get_albums_grid_html(db: State<'_, Database>) -> Result<String, String> {
    let repo = crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
        db.inner().clone(),
    ));
    let albums = repo.get_albums_summary().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch albums summary");
        format!("Failed to fetch albums summary: {e}")
    })?;

    if albums.is_empty() {
        return Ok(r#"<div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
    <i data-lucide="disc-3" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
    <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No albums found</h3>
    <p style="color: var(--text-3); font-size: var(--text-sm);">Scan your music library to populate your album catalog.</p>
</div>"#.to_string());
    }

    let mut html = String::new();
    for album in albums {
        let safe_name = html_escape(&album.album);
        let artist_str = album.artist.as_deref().unwrap_or("Unknown Artist");
        let safe_artist = html_escape(artist_str);
        let first_id = album.first_track_id.as_deref().unwrap_or("");
        let safe_first_id = html_escape(first_id);
        let art_html = render_art_tag(album.album_art_path.as_deref(), &album.album, "disc-3");
        let track_count = album.track_count;
        let track_word = if track_count == 1 { "track" } else { "tracks" };

        html.push_str(&format!(
            r#"<div class="card card--glass neu-glass album-card" data-album-name="{safe_name}" data-first-track-id="{safe_first_id}" data-track-id="{safe_first_id}" data-role="play-card" style="cursor: pointer; touch-action: manipulation;">
    <div class="card-artwork">
        {art_html}
    </div>
    <div class="card-body">
        <div class="card-title">{safe_name}</div>
        <div class="card-subtitle">{safe_artist} · {track_count} {track_word}</div>
    </div>
</div>"#
        ));
    }

    Ok(html)
}

/// Render HTML grid of artists from database.
#[tauri::command]
pub async fn get_artists_grid_html(db: State<'_, Database>) -> Result<String, String> {
    let repo = crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
        db.inner().clone(),
    ));
    let artists = repo.get_artists_summary().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch artists summary");
        format!("Failed to fetch artists summary: {e}")
    })?;

    if artists.is_empty() {
        return Ok(r#"<div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
    <i data-lucide="users" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
    <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No artists found</h3>
    <p style="color: var(--text-3); font-size: var(--text-sm);">Scan your music library to discover artists.</p>
</div>"#.to_string());
    }

    let mut html = String::new();
    for artist in artists {
        let safe_name = html_escape(&artist.artist);
        let first_id = artist.first_track_id.as_deref().unwrap_or("");
        let safe_first_id = html_escape(first_id);
        let track_count = artist.track_count;
        let track_word = if track_count == 1 { "track" } else { "tracks" };

        html.push_str(&format!(
            r#"<div class="card card--glass neu-glass artist-card" data-artist-name="{safe_name}" data-first-track-id="{safe_first_id}" data-track-id="{safe_first_id}" data-role="play-card" style="cursor: pointer; touch-action: manipulation;">
    <div class="card-artwork">
        <i data-lucide="user"></i>
    </div>
    <div class="card-body">
        <div class="card-title">{safe_name}</div>
        <div class="card-subtitle">{track_count} {track_word}</div>
    </div>
</div>"#
        ));
    }

    Ok(html)
}

/// Render HTML track rows for library view.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_library_tracks_html(
    db: State<'_, Database>,
    sort_by: Option<String>,
    downloaded_only: Option<bool>,
    artist: Option<String>,
    album: Option<String>,
    search: Option<String>,
) -> Result<String, String> {
    let conn = db.connection().map_err(|e| e.to_string())?;

    let mut sql = "SELECT id, title, artist, album, duration_secs, album_art_path, is_favorite FROM tracks WHERE 1=1".to_string();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if downloaded_only == Some(true) {
        sql.push_str(
            " AND (is_downloaded = 1 OR (file_path IS NOT NULL AND file_path NOT LIKE 'http%'))",
        );
    }

    if let Some(ref a) = artist {
        if !a.trim().is_empty() {
            sql.push_str(" AND artist = ?");
            params.push(Box::new(a.clone()));
        }
    }

    if let Some(ref alb) = album {
        if !alb.trim().is_empty() {
            sql.push_str(" AND album = ?");
            params.push(Box::new(alb.clone()));
        }
    }

    if let Some(ref q) = search {
        let trimmed = q.trim();
        if !trimmed.is_empty() {
            sql.push_str(" AND (title LIKE ? OR artist LIKE ? OR album LIKE ?)");
            let pattern = format!("%{trimmed}%");
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern));
        }
    }

    let sort_field = sort_by.as_deref().unwrap_or("date_added");
    match sort_field {
        "title" => sql.push_str(" ORDER BY title COLLATE NOCASE ASC, artist COLLATE NOCASE ASC"),
        "artist" => sql.push_str(" ORDER BY artist COLLATE NOCASE ASC, album COLLATE NOCASE ASC, track_number ASC, title COLLATE NOCASE ASC"),
        "album" => sql.push_str(" ORDER BY album COLLATE NOCASE ASC, disc_number ASC, track_number ASC, title COLLATE NOCASE ASC"),
        _ => sql.push_str(" ORDER BY date_added DESC, id DESC"),
    }

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare tracks query: {e}"))?;

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)? != 0,
            ))
        })
        .map_err(|e| format!("Failed to query library tracks: {e}"))?;

    let mut html = String::new();
    let mut count = 0;

    for row_res in rows {
        let (id, title, artist, album, duration_secs, art_path, is_fav) =
            row_res.map_err(|e| format!("Row error: {e}"))?;
        count += 1;

        html.push_str(&render_track_row_html(
            &id,
            &title,
            artist.as_deref(),
            album.as_deref(),
            duration_secs,
            art_path.as_deref(),
            is_fav,
        ));
    }

    if count == 0 {
        return Ok(r#"<div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center;">
    <div class="empty-state-icon" style="color: var(--accent); margin-bottom: var(--space-4);">
        <i data-lucide="music" style="width: 48px; height: 48px;"></i>
    </div>
    <h2 class="empty-state-title" style="color: var(--text-1); font-size: var(--text-xl); margin-bottom: var(--space-2);">No tracks found in library</h2>
    <p class="empty-state-description" style="color: var(--text-2); margin-bottom: var(--space-6); max-width: 420px; margin-left: auto; margin-right: auto;">Scan your device storage or download music to start listening.</p>
    <div style="display: flex; gap: var(--space-3); flex-wrap: wrap; justify-content: center;">
        <label class="btn btn-primary neu" for="global-audio-import-input" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="file-plus-2"></i>
            Import Audio
        </label>
        <button type="button" class="btn btn-secondary neu" onclick="window.Auralis.bridge.triggerFolderScan()" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="folder-search"></i>
            Scan Storage
        </button>
    </div>
</div>"#.to_string());
    }

    Ok(html)
}

/// Render HTML shelves for home view.
#[tauri::command]
pub async fn get_home_shelves_html(db: State<'_, Database>) -> Result<String, String> {
    let conn = db.connection().map_err(|e| e.to_string())?;

    let total_tracks: i64 = conn
        .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
        .unwrap_or(0);

    if total_tracks == 0 {
        return Ok(r##"<div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center; margin-top: var(--space-4);">
    <div class="empty-state-icon" style="color: var(--accent); margin-bottom: var(--space-4);">
        <i data-lucide="music" style="width: 48px; height: 48px;"></i>
    </div>
    <h2 class="empty-state-title" style="color: var(--text-1); font-size: var(--text-xl); margin-bottom: var(--space-2);">Your library is empty</h2>
    <p class="empty-state-description" style="color: var(--text-2); margin-bottom: var(--space-6); max-width: 420px; margin-left: auto; margin-right: auto;">Import audio files from your device storage or download music to start playing.</p>
    <div style="display: flex; gap: var(--space-3); flex-wrap: wrap; justify-content: center;">
        <label class="btn btn-primary neu" for="global-audio-import-input" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="file-plus-2"></i>
            Import Audio
        </label>
        <button type="button" class="btn btn-secondary neu" onclick="window.Auralis.bridge.triggerFolderScan()" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="folder-search"></i>
            Scan Storage
        </button>
        <button type="button" class="btn btn-secondary neu" hx-get="/partials/download.html" hx-target="#content" hx-swap="innerHTML" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="download"></i>
            Download Audio
        </button>
    </div>
</div>"##.to_string());
    }

    // Query recently added cards
    let mut stmt = conn
        .prepare("SELECT id, title, artist, album_art_path FROM tracks ORDER BY date_added DESC, id DESC LIMIT 6")
        .map_err(|e| format!("Failed to prepare recent query: {e}"))?;

    let cards = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| format!("Failed to query recent cards: {e}"))?;

    let mut shelf_cards_html = String::new();
    for card_res in cards {
        let (id, title, artist, art_path) = card_res.map_err(|e| format!("Card error: {e}"))?;
        let safe_id = html_escape(&id);
        let safe_title = html_escape(&title);
        let artist_str = artist.unwrap_or_else(|| "Unknown Artist".to_string());
        let safe_artist = html_escape(&artist_str);
        let art_html = render_art_tag(art_path.as_deref(), &title, "disc-3");

        shelf_cards_html.push_str(&format!(
            r#"<div class="card album-card neu-glass" data-track-id="{safe_id}" data-role="play-card" style="cursor: pointer; touch-action: manipulation;">
    <div class="card-artwork">
        {art_html}
    </div>
    <div class="card-body">
        <div class="card-title">{safe_title}</div>
        <div class="card-subtitle">{safe_artist}</div>
    </div>
</div>"#
        ));
    }

    // Query continue listening track rows
    let mut stmt_continue = conn
        .prepare("SELECT id, title, artist, album, duration_secs, album_art_path, is_favorite FROM tracks ORDER BY CASE WHEN last_played IS NOT NULL THEN 0 ELSE 1 END, last_played DESC, date_added DESC LIMIT 6")
        .map_err(|e| format!("Failed to prepare continue query: {e}"))?;

    let track_rows = stmt_continue
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)? != 0,
            ))
        })
        .map_err(|e| format!("Failed to query continue tracks: {e}"))?;

    let mut continue_rows_html = String::new();
    for row_res in track_rows {
        let (id, title, artist, album, duration_secs, art_path, is_fav) =
            row_res.map_err(|e| format!("Row error: {e}"))?;
        continue_rows_html.push_str(&render_track_row_html(
            &id,
            &title,
            artist.as_deref(),
            album.as_deref(),
            duration_secs,
            art_path.as_deref(),
            is_fav,
        ));
    }

    let mut html = String::new();
    html.push_str(
        r##"<section class="section-header">
    <h2 class="section-title">Recently Added</h2>
    <a href="#" class="section-link" hx-get="/partials/library.html" hx-target="#content" hx-swap="innerHTML">See all</a>
</section>

<div class="shelf" id="recently-added-shelf">"##,
    );
    html.push_str(&shelf_cards_html);
    html.push_str(
        r#"</div>

<section class="section-header" style="margin-top: var(--space-6);">
    <h2 class="section-title">Continue Listening</h2>
</section>

<div class="track-list" id="continue-listening-tracks">"#,
    );
    html.push_str(&continue_rows_html);
    html.push_str("</div>");

    Ok(html)
}

/// Render HTML track rows for search view.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_search_results_html(
    db: State<'_, Database>,
    query: Option<String>,
    q: Option<String>,
) -> Result<String, String> {
    let raw_query = query.or(q).unwrap_or_default();
    let trimmed = raw_query.trim();

    if trimmed.is_empty() {
        return Ok(r#"<div class="empty-state glass neu" style="padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
    <div class="empty-state-icon"><i data-lucide="search"></i></div>
    <h2 class="empty-state-title">Search your library</h2>
    <p class="empty-state-description">Enter a query above to find tracks, artists, or albums.</p>
</div>"#.to_string());
    }

    let conn = db.connection().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, title, artist, album, duration_secs, album_art_path, is_favorite 
            FROM tracks 
            WHERE title LIKE ?1 OR artist LIKE ?1 OR album LIKE ?1 
            ORDER BY title COLLATE NOCASE ASC 
            LIMIT 100",
        )
        .map_err(|e| format!("Failed to prepare search query: {e}"))?;

    let pattern = format!("%{trimmed}%");
    let rows = stmt
        .query_map([&pattern], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)? != 0,
            ))
        })
        .map_err(|e| format!("Failed to query search results: {e}"))?;

    let mut html = String::new();
    let mut count = 0;

    for row_res in rows {
        let (id, title, artist, album, duration_secs, art_path, is_fav) =
            row_res.map_err(|e| format!("Row error: {e}"))?;
        count += 1;

        html.push_str(&render_track_row_html(
            &id,
            &title,
            artist.as_deref(),
            album.as_deref(),
            duration_secs,
            art_path.as_deref(),
            is_fav,
        ));
    }

    if count == 0 {
        return Ok(r#"<div class="empty-state glass neu" style="padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
    <div class="empty-state-icon"><i data-lucide="search"></i></div>
    <h2 class="empty-state-title">No matching tracks</h2>
    <p class="empty-state-description">Try searching for a different title, artist, or album.</p>
</div>"#.to_string());
    }

    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(
            html_escape("Rock & Roll <script>'\""),
            "Rock &amp; Roll &lt;script&gt;&#39;&quot;"
        );
    }

    #[test]
    fn test_format_time() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(65), "1:05");
        assert_eq!(format_time(3600), "60:00");
    }

    #[test]
    fn test_render_art_tag() {
        let empty_tag = render_art_tag(None, "Album", "disc-3");
        assert_eq!(empty_tag, r#"<i data-lucide="disc-3"></i>"#);

        let img_tag = render_art_tag(Some("/path/to/art.jpg"), "My Album", "disc-3");
        assert!(img_tag.contains(r#"src="/path/to/art.jpg""#));
        assert!(img_tag.contains(r#"alt="My Album""#));
        assert!(img_tag.contains("embedArt"));
    }

    #[test]
    fn test_render_track_row_html() {
        let html = render_track_row_html(
            "track-1",
            "Midnight City",
            Some("M83"),
            Some("Hurry Up"),
            244,
            Some("https://example.com/art.jpg"),
            true,
        );
        assert!(html.contains(r#"data-track-id="track-1""#));
        assert!(html.contains(r#"data-role="play-row""#));
        assert!(html.contains("Midnight City"));
        assert!(html.contains("M83"));
        assert!(html.contains("4:04"));
        assert!(html.contains("liked"));
        assert!(html.contains("glass-weak"));
    }

    #[test]
    fn test_escape_special_characters_in_row() {
        let html = render_track_row_html(
            "track-2",
            "<script>alert('xss')</script>",
            Some("AC/DC & Friends \"Rock'n'Roll\""),
            Some("Back in Black > White"),
            185,
            None,
            false,
        );
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"));
        assert!(html.contains("AC/DC &amp; Friends &quot;Rock&#39;n&#39;Roll&quot;"));
        assert!(html.contains("Back in Black &gt; White"));
        assert!(html.contains("3:05"));
        assert!(!html.contains("liked"));
    }
}

//! Android Scoped Storage & Media Ingestion Engine
//!
//! On Android 10+ (API 29+) through Android 16 (API 36), Scoped Storage prevents
//! direct POSIX traversal of shared storage paths (such as `/storage/emulated/0/Music`).
//!
//! This module provides:
//! 1. `ingest_buffer`: Ingestion of audio byte buffers streamed from SAF (Storage Access
//!    Framework) or HTML5 file pickers directly into the sandboxed app storage.
//! 2. `scan_sandboxed_dir`: Safe recursive traversal of app-specific internal storage
//!    (`app_data_dir/music`, `app_data_dir/downloads`) which is always accessible without permissions.
//! 3. Shared lofty metadata extraction, format detection, and SQLite persistence.

use crate::domain::models::{AudioFormat, ScanSummary, Track, TrackFilter};
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::filesystem::metadata::MetadataExtractor;
use crate::infrastructure::filesystem::scanner::{
    detect_format, is_audio_file, ScanProgress, ScannerError,
};
use lofty::file::AudioFile;
use lofty::probe::Probe;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Android storage scanner and SAF ingestion engine
pub struct AndroidScanner;

impl Default for AndroidScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidScanner {
    /// Create a new Android storage scanner
    pub fn new() -> Self {
        Self
    }

    /// Check if a path is a supported audio file (case-insensitive)
    pub fn is_audio_file(path: &Path) -> bool {
        is_audio_file(path)
    }

    /// Detect audio format from file path extension
    pub fn detect_format(path: &Path) -> Option<AudioFormat> {
        detect_format(path)
    }

    /// Ingest an in-memory audio byte buffer (streamed via SAF or HTML5 File API)
    /// into the app's sandboxed storage, extract metadata, and persist into SQLite.
    pub async fn ingest_buffer(
        &self,
        name: &str,
        bytes: &[u8],
        target_dir: &Path,
        track_repo: &Arc<dyn TrackRepository>,
    ) -> Result<Track, ScannerError> {
        debug!(name = %name, bytes_len = bytes.len(), target_dir = %target_dir.display(), "Ingesting audio buffer into sandboxed storage");

        // `create_dir_all` and `write` are blocking — offload to the
        // blocking pool so the async runtime is not stalled. We clone
        // `bytes` into an owned Vec for the `spawn_blocking` closure.
        let dir_owned = target_dir.to_path_buf();
        tokio::task::spawn_blocking(move || std::fs::create_dir_all(&dir_owned))
            .await
            .map_err(|e| ScannerError::IoError(format!("Join error: {e}")))?
            .map_err(|e| {
                error!(dir = %target_dir.display(), error = %e, "Failed to create sandboxed destination directory");
                ScannerError::IoError(format!(
                    "Failed to create sandboxed destination directory {}: {e}",
                    target_dir.display()
                ))
            })?;

        // Sanitize `name` to isolate the filename component. This prevents
        // absolute paths (e.g., `/storage/emulated/0/...` or `content://...`)
        // passed from Android SAF/SFS pickers from replacing `target_dir`
        // when calling `target_dir.join(name)`.
        let clean_name = Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name);

        let file_path = target_dir.join(clean_name);
        let file_path_clone = file_path.clone();
        let bytes_owned = bytes.to_vec();
        let bytes_len = bytes_owned.len();
        tokio::task::spawn_blocking(move || std::fs::write(&file_path_clone, &bytes_owned))
            .await
            .map_err(|e| ScannerError::IoError(format!("Join error: {e}")))?
            .map_err(|e| {
                error!(file = %file_path.display(), bytes = bytes_len, error = %e, "Failed to write audio buffer to sandbox");
                ScannerError::IoError(format!(
                    "Failed to write audio buffer to sandbox {}: {e}",
                    file_path.display()
                ))
            })?;

        // Verify audio health and playability (metadata extraction + >0s duration + rodio/symphonia decoder probe)
        let path_for_verify = file_path.clone();
        let verify_res = tokio::task::spawn_blocking(move || {
            crate::infrastructure::filesystem::metadata::verify_audio_health(&path_for_verify)
        })
        .await
        .map_err(|e| ScannerError::MetadataError(format!("Join error: {e}")))?;

        let mut track = match verify_res {
            Ok(t) => {
                debug!(file = %file_path.display(), title = %t.title, "Audio buffer verified and metadata extracted successfully");
                t
            }
            Err(e) => {
                error!(file = %file_path.display(), error = %e, "Corrupted or unplayable audio file rejected; removing from sandbox");
                let file_to_delete = file_path.clone();
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(&file_to_delete))
                    .await;
                return Err(ScannerError::MetadataError(format!(
                    "Cannot import '{clean_name}': File is corrupted or unplayable: {e}"
                )));
            }
        };

        if track.title.trim().is_empty() || track.title == "Unknown" {
            track.title = clean_name.to_string();
        }

        // Record the file's modification time so an incremental re-scan of
        // the sandbox dir can skip this file without re-parsing metadata.
        let meta = tokio::fs::metadata(&file_path).await.ok();
        track.mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        track.file_size = meta.as_ref().map(|m| m.len()).unwrap_or(0);

        let path_str = file_path.to_string_lossy().to_string();
        let existing = track_repo.find_by_path(&path_str).await.map_err(|e| {
            error!(path = %path_str, error = %e, "Database query failed during ingest_buffer");
            ScannerError::RepositoryError(e.to_string())
        })?;

        if let Some(existing_track) = existing {
            let mut updated_track = track;
            updated_track.id = existing_track.id;
            updated_track.date_added = existing_track.date_added;
            updated_track.last_played = existing_track.last_played;
            updated_track.play_count = existing_track.play_count;
            if updated_track.album_art_path.is_none() {
                updated_track.album_art_path = existing_track.album_art_path;
            }
            track_repo
                .update(&updated_track)
                .await
                .map_err(|e| {
                    error!(id = %updated_track.id, error = %e, "Failed to update track in database during buffer ingestion");
                    ScannerError::RepositoryError(e.to_string())
                })?;

            info!(id = %updated_track.id, title = %updated_track.title, "Updated existing audio buffer track in library");
            Ok(updated_track)
        } else {
            track_repo
                .insert(&track)
                .await
                .map_err(|e| {
                    error!(id = %track.id, error = %e, "Failed to insert track into database during buffer ingestion");
                    ScannerError::RepositoryError(e.to_string())
                })?;

            info!(id = %track.id, title = %track.title, "Ingested audio buffer into library");
            Ok(track)
        }
    }

    /// Process single sandboxed audio file and update/insert in repository
    ///
    /// Mirrors `DesktopScanner::process_file` incremental logic (mtime + size)
    /// so re-scans of the sandbox skip unchanged files. All blocking I/O
    /// (`metadata` + `lofty`) is offloaded via `spawn_blocking`.
    async fn process_file(
        &self,
        path: &Path,
        track_repo: &Arc<dyn TrackRepository>,
    ) -> Result<crate::infrastructure::filesystem::scanner::ScanResult, ScannerError> {
        let path_str = path.to_string_lossy().to_string();

        let existing = track_repo
            .find_by_path(&path_str)
            .await
            .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

        // Incremental check — shared with desktop.rs (mtime + size). Stat
        // via spawn_blocking so we don't block the runtime.
        let path_for_stat = path.to_path_buf();
        let inner = tokio::task::spawn_blocking(move || {
            let meta = std::fs::metadata(&path_for_stat)
                .map_err(|e| ScannerError::MetadataError(format!("Failed to stat: {e}")))?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let size = meta.len();
            Ok::<(i64, u64), ScannerError>((mtime, size))
        })
        .await
        .map_err(|e| ScannerError::MetadataError(format!("Join error: {e}")))?;
        let (mtime, size) = inner?;

        if let Some(existing_track) = existing.as_ref() {
            let needs_rescan = existing_track.duration_secs == 0;
            if !needs_rescan && existing_track.mtime == mtime && existing_track.file_size == size {
                debug!(path = %path_str, "File unchanged, skipping");
                return Ok(crate::infrastructure::filesystem::scanner::ScanResult::Skipped);
            }
            if needs_rescan {
                debug!(path = %path_str, "Existing track has 0s duration — forcing re-scan");
            }
        }

        // Blocking lofty extraction — offload.
        let path_for_extract = path.to_path_buf();
        let extract_res = tokio::task::spawn_blocking(move || {
            MetadataExtractor::extract_with_size(&path_for_extract, Some(size))
        })
        .await
        .map_err(|e| ScannerError::MetadataError(format!("Join error: {e}")))?;

        let mut track = match extract_res {
            Ok(t) if t.duration_secs > 0 => t,
            Ok(t) => {
                // Secondary probe: try guessing file type via Probe::open().guess_file_type().read()
                let path_for_probe = path.to_path_buf();
                let secondary_duration = tokio::task::spawn_blocking(move || {
                    let mut probe = match Probe::open(&path_for_probe) {
                        Ok(p) => p,
                        Err(_) => return None,
                    };
                    if probe.file_type().is_none() {
                        match probe.guess_file_type() {
                            Ok(p) => probe = p,
                            Err(_) => return None,
                        }
                    }
                    let tagged = match probe.read() {
                        Ok(tagged) => tagged,
                        Err(_) => return None,
                    };
                    Some(tagged.properties().duration().as_secs() as u32)
                })
                .await
                .unwrap_or(None);
                if secondary_duration.is_none_or(|d| d == 0) {
                    warn!(file = %path_str, "Skipping sandboxed audio file with 0s duration — secondary probe also 0, unplayable");
                    return Ok(
                        crate::infrastructure::filesystem::scanner::ScanResult::SkippedUnplayable,
                    );
                }
                let mut corrected = t;
                corrected.duration_secs = secondary_duration.unwrap_or(0);
                if corrected.duration_secs == 0 {
                    warn!(file = %path_str, "Skipping sandboxed audio file with 0s duration after secondary probe");
                    return Ok(
                        crate::infrastructure::filesystem::scanner::ScanResult::SkippedUnplayable,
                    );
                }
                corrected
            }
            Err(e) => {
                warn!(file = %path_str, error = %e, "Metadata extraction failed; skipping unplayable file");
                return Err(ScannerError::MetadataError(format!(
                    "Failed to extract metadata from {path_str}: {e}"
                )));
            }
        };

        if track.title.trim().is_empty() || track.title == "Unknown" {
            track.title = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
        }
        track.mtime = mtime;
        track.file_size = size;

        if let Some(existing_track) = existing {
            let mut updated_track = track;
            updated_track.id = existing_track.id;
            updated_track.date_added = existing_track.date_added;
            updated_track.last_played = existing_track.last_played;
            updated_track.play_count = existing_track.play_count;
            if updated_track.album_art_path.is_none() {
                updated_track.album_art_path = existing_track.album_art_path;
            }

            track_repo
                .update(&updated_track)
                .await
                .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

            debug!(path = %path_str, id = %updated_track.id, "Track updated");
            Ok(crate::infrastructure::filesystem::scanner::ScanResult::Updated)
        } else {
            track_repo
                .insert(&track)
                .await
                .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

            debug!(path = %path_str, id = %track.id, "Track added");
            Ok(crate::infrastructure::filesystem::scanner::ScanResult::Added)
        }
    }

    /// Recursively scan sandboxed app-internal directories (`app_data_dir/music`, `app_data_dir/downloads`)
    /// where permissions are never required.
    pub async fn scan_sandboxed_dir<F>(
        &self,
        sandboxed_dirs: &[PathBuf],
        track_repo: Arc<dyn TrackRepository>,
        mut progress_callback: Option<F>,
    ) -> Result<ScanSummary, ScannerError>
    where
        F: FnMut(ScanProgress) + Send + 'static,
    {
        info!(
            dir_count = sandboxed_dirs.len(),
            "Starting Android sandboxed library scan"
        );

        let mut summary = ScanSummary {
            tracks_added: 0,
            tracks_updated: 0,
            tracks_removed: 0,
            skipped_unplayable: 0,
            errors: Vec::new(),
        };

        let mut found_files: HashSet<PathBuf> = HashSet::new();
        let mut all_audio_files: Vec<PathBuf> = Vec::new();

        const MAX_DEPTH: usize = 64;
        for root in sandboxed_dirs {
            // Existence check via spawn_blocking to avoid blocking runtime.
            let root_clone = root.clone();
            let exists = tokio::task::spawn_blocking(move || root_clone.exists())
                .await
                .unwrap_or(false);
            if !exists {
                continue;
            }

            let mut dirs_to_visit: Vec<(PathBuf, usize)> = vec![(root.clone(), 0)];
            let mut visited: HashSet<PathBuf> = HashSet::new();
            if let Ok(canonical) = tokio::task::spawn_blocking({
                let p = root.clone();
                move || std::fs::canonicalize(&p)
            })
            .await
            .unwrap_or(Err(std::io::Error::from(std::io::ErrorKind::NotFound)))
            {
                visited.insert(canonical);
            }

            while let Some((current, depth)) = dirs_to_visit.pop() {
                if depth > MAX_DEPTH {
                    warn!(path = %current.display(), depth, "Max depth exceeded; skipping");
                    continue;
                }
                let cur_clone = current.clone();
                let entries_res =
                    tokio::task::spawn_blocking(move || std::fs::read_dir(&cur_clone))
                        .await
                        .map_err(|e| ScannerError::IoError(format!("Join error: {e}")))?;
                let entries = match entries_res {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(path = %current.display(), error = %e, "Failed to read sandboxed directory");
                        continue;
                    }
                };

                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    let file_name = entry.file_name().to_string_lossy().to_string();

                    if file_name.starts_with('.') {
                        continue;
                    }

                    // Symlink loop protection — skip symlinked dirs.
                    let ft = match entry.file_type() {
                        Ok(ft) => ft,
                        Err(_) => continue,
                    };
                    if ft.is_symlink() && entry_path.is_dir() {
                        debug!(path = %entry_path.display(), "Skipping symlinked directory");
                        continue;
                    }

                    if entry_path.is_dir() {
                        let canon = tokio::task::spawn_blocking({
                            let p = entry_path.clone();
                            move || std::fs::canonicalize(&p)
                        })
                        .await
                        .ok()
                        .and_then(|r| r.ok());
                        if let Some(c) = canon {
                            if !visited.insert(c) {
                                debug!(path = %entry_path.display(), "Skipping already-visited directory (symlink loop)");
                                continue;
                            }
                        }
                        dirs_to_visit.push((entry_path, depth + 1));
                    } else if entry_path.is_file()
                        && is_audio_file(&entry_path)
                        && found_files.insert(entry_path.clone())
                    {
                        all_audio_files.push(entry_path);
                    }
                }
            }
        }

        let total_files = all_audio_files.len();

        for (idx, file_path) in all_audio_files.into_iter().enumerate() {
            debug!(path = %file_path.display(), "Processing sandboxed audio file");

            match self.process_file(&file_path, &track_repo).await {
                Ok(crate::infrastructure::filesystem::scanner::ScanResult::Added) => {
                    summary.tracks_added += 1
                }
                Ok(crate::infrastructure::filesystem::scanner::ScanResult::Updated) => {
                    summary.tracks_updated += 1
                }
                Ok(crate::infrastructure::filesystem::scanner::ScanResult::Skipped) => {}
                Ok(crate::infrastructure::filesystem::scanner::ScanResult::SkippedUnplayable) => {
                    warn!(path = %file_path.display(), "Skipped unplayable file with 0s duration");
                    summary.skipped_unplayable += 1;
                }
                Err(e) => {
                    error!(path = %file_path.display(), error = %e, "Failed to process audio file");
                    summary
                        .errors
                        .push(format!("{}: {}", file_path.display(), e));
                }
            }

            let processed = idx + 1;
            let percentage = if total_files > 0 {
                (processed as f32 / total_files as f32) * 100.0
            } else {
                100.0
            };

            if let Some(ref mut cb) = progress_callback {
                cb(ScanProgress {
                    current_file: file_path.to_string_lossy().to_string(),
                    total_files,
                    processed_files: processed,
                    percentage,
                    tracks_added: summary.tracks_added,
                    tracks_updated: summary.tracks_updated,
                    error_count: summary.errors.len(),
                });
            }
        }

        // Clean up stale database records in the sandboxed scope
        let scan_prefixes: Vec<PathBuf> = sandboxed_dirs.to_vec();
        if let Ok(all_tracks) = track_repo.find_all(TrackFilter::default()).await {
            let mut ids_to_remove: Vec<uuid::Uuid> = Vec::new();
            for track in &all_tracks {
                let track_path = PathBuf::from(&track.file_path);
                let in_scope = scan_prefixes.iter().any(|p| track_path.starts_with(p));
                if in_scope && !found_files.contains(&track_path) {
                    ids_to_remove.push(track.id);
                }
            }

            if !ids_to_remove.is_empty() {
                let count = ids_to_remove.len() as u32;
                summary.tracks_removed = count;
                let _ = track_repo.delete_many(ids_to_remove).await;
            }
        }

        info!(
            added = summary.tracks_added,
            updated = summary.tracks_updated,
            removed = summary.tracks_removed,
            skipped_unplayable = summary.skipped_unplayable,
            errors = summary.errors.len(),
            "Android sandboxed scan completed"
        );

        Ok(summary)
    }

    /// Full Android library scan: combines sandboxed directories (`app_data_dir/music`, `app_data_dir/downloads`)
    /// with system-wide MediaStore audio tracks.
    pub async fn scan_library_paths_with_progress<F>(
        &self,
        sandboxed_dirs: &[PathBuf],
        track_repo: Arc<dyn TrackRepository>,
        mut progress_callback: Option<F>,
    ) -> Result<ScanSummary, ScannerError>
    where
        F: FnMut(ScanProgress) + Send + 'static,
    {
        // 1. Scan sandboxed dirs
        let mut summary = self
            .scan_sandboxed_dir(sandboxed_dirs, track_repo.clone(), None::<fn(ScanProgress)>)
            .await?;

        // 2. Query system-wide MediaStore audio
        let media_store_tracks = query_system_mediastore_audio();
        let total_mediastore = media_store_tracks.len();
        info!(
            mediastore_count = total_mediastore,
            "Processing system-wide MediaStore audio tracks"
        );

        for (idx, raw) in media_store_tracks.into_iter().enumerate() {
            let path_buf = PathBuf::from(&raw.path);
            let path_str = raw.path.clone();

            // Skip if path is in sandboxed dirs to avoid double processing
            if sandboxed_dirs.iter().any(|p| path_buf.starts_with(p)) {
                continue;
            }

            // Strict scope check: only process files within standard Music or Download folders
            let path_lower = path_str.to_ascii_lowercase();
            let is_music_or_download = path_lower.contains("/music/")
                || path_lower.contains("/download/")
                || path_lower.contains("/downloads/");
            if !is_music_or_download {
                continue;
            }

            let existing = track_repo
                .find_by_path(&path_str)
                .await
                .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

            // Try to extract rich lofty metadata if file is directly readable
            let path_buf_clone = path_buf.clone();
            let file_readable = tokio::task::spawn_blocking(move || path_buf_clone.exists())
                .await
                .unwrap_or(false);

            let extract_res = if file_readable {
                let p_clone = path_buf.clone();
                tokio::task::spawn_blocking(move || MetadataExtractor::extract(&p_clone))
                    .await
                    .ok()
                    .and_then(|r| r.ok())
            } else {
                None
            };

            let format = detect_format(&path_buf)
                .or_else(|| {
                    raw.mime_type.as_deref().and_then(|m| match m {
                        "audio/mpeg" => Some(AudioFormat::Mp3),
                        "audio/mp4" | "audio/aac" => Some(AudioFormat::M4a),
                        "audio/flac" => Some(AudioFormat::Flac),
                        "audio/ogg" | "audio/opus" => Some(AudioFormat::Ogg),
                        "audio/wav" => Some(AudioFormat::Wav),
                        _ => None,
                    })
                })
                .unwrap_or(AudioFormat::Mp3);

            let fallback_needs_secondary =
                extract_res.as_ref().is_some_and(|t| t.duration_secs == 0);
            let track = if let Some(mut t) = extract_res.filter(|t| t.duration_secs > 0) {
                if t.album_art_path.is_none() && raw.art_uri.is_some() {
                    t.album_art_path = raw.art_uri;
                }
                t
            } else {
                if fallback_needs_secondary {
                    let path_for_probe = path_buf.clone();
                    let secondary_ok = tokio::task::spawn_blocking(move || {
                        let mut probe = match Probe::open(&path_for_probe) {
                            Ok(p) => p,
                            Err(_) => return false,
                        };
                        if probe.file_type().is_none() {
                            match probe.guess_file_type() {
                                Ok(p) => probe = p,
                                Err(_) => return false,
                            }
                        }
                        let tagged = match probe.read() {
                            Ok(tagged) => tagged,
                            Err(_) => return false,
                        };
                        tagged.properties().duration().as_secs() > 0
                    })
                    .await
                    .unwrap_or(false);
                    if !secondary_ok {
                        warn!(path = %path_str, "Skipping MediaStore audio track with 0s duration — secondary probe also 0, unplayable");
                        summary.skipped_unplayable += 1;
                        continue;
                    }
                }
                let duration_secs = (raw.duration_ms / 1000) as u32;
                if duration_secs == 0 {
                    warn!(path = %path_str, "Skipping MediaStore audio track with 0s duration or missing stream");
                    summary.skipped_unplayable += 1;
                    continue;
                }
                let title = if !raw.title.trim().is_empty() {
                    raw.title.clone()
                } else {
                    path_buf
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Unknown".to_string())
                };
                let mut t = Track::new(title, path_str.clone(), duration_secs, format);
                t.artist = raw
                    .artist
                    .filter(|s| !s.trim().is_empty() && s != "<unknown>");
                t.album = raw
                    .album
                    .filter(|s| !s.trim().is_empty() && s != "<unknown>");
                t.track_number = raw.track_number;
                t.year = raw.year;
                t.file_size = raw.size;
                t.album_art_path = raw.art_uri.filter(|s| !s.trim().is_empty());
                t
            };

            if let Some(existing_track) = existing {
                let mut updated_track = track;
                updated_track.id = existing_track.id;
                updated_track.date_added = existing_track.date_added;
                updated_track.last_played = existing_track.last_played;
                updated_track.play_count = existing_track.play_count;
                if updated_track.album_art_path.is_none() {
                    updated_track.album_art_path = existing_track.album_art_path;
                }
                track_repo
                    .update(&updated_track)
                    .await
                    .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;
                summary.tracks_updated += 1;
            } else {
                track_repo
                    .insert(&track)
                    .await
                    .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;
                summary.tracks_added += 1;
            }

            let processed = idx + 1;
            let percentage = if total_mediastore > 0 {
                (processed as f32 / total_mediastore as f32) * 100.0
            } else {
                100.0
            };

            if let Some(ref mut cb) = progress_callback {
                cb(ScanProgress {
                    current_file: path_str,
                    total_files: total_mediastore,
                    processed_files: processed,
                    percentage,
                    tracks_added: summary.tracks_added,
                    tracks_updated: summary.tracks_updated,
                    error_count: summary.errors.len(),
                });
            }
        }

        info!(
            added = summary.tracks_added,
            updated = summary.tracks_updated,
            removed = summary.tracks_removed,
            skipped_unplayable = summary.skipped_unplayable,
            "Android system-wide and sandboxed scan completed"
        );

        Ok(summary)
    }
}

/// Raw track representation returned by MediaStore ContentResolver query
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MediaStoreRawTrack {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub track_number: Option<u32>,
    pub year: Option<i32>,
    pub size: u64,
    pub mime_type: Option<String>,
    pub art_uri: Option<String>,
}

#[cfg(target_os = "android")]
fn cached_vm() -> Option<&'static jni::JavaVM> {
    use std::sync::OnceLock;
    static VM: OnceLock<jni::JavaVM> = OnceLock::new();
    if let Some(vm) = VM.get() {
        return Some(vm);
    }
    let ptr = crate::android_jni::INITIAL_VM.load(std::sync::atomic::Ordering::SeqCst);
    if ptr.is_null() {
        return None;
    }
    if let Ok(vm) = unsafe { jni::JavaVM::from_raw(ptr as *mut jni::sys::JavaVM) } {
        let _ = VM.set(vm);
        return VM.get();
    }
    None
}

#[cfg(target_os = "android")]
fn with_attached_env<T>(
    f: impl FnOnce(&mut jni::JNIEnv<'_>) -> Result<T, String>,
) -> Option<Result<T, String>> {
    let vm = cached_vm()?;
    let mut guard = vm.attach_current_thread().ok()?;
    let res = f(&mut guard);
    if guard.exception_check().unwrap_or(false) {
        let _ = guard.exception_clear();
    }
    match res {
        Ok(v) => Some(Ok(v)),
        Err(e) => {
            if guard.exception_check().unwrap_or(false) {
                let _ = guard.exception_clear();
            }
            Some(Err(e))
        }
    }
}

#[cfg(target_os = "android")]
fn service_context() -> Option<jni::objects::JObject<'static>> {
    let ctx = ndk_context::android_context().context();
    if ctx.is_null() {
        return None;
    }
    Some(unsafe { jni::objects::JObject::from_raw(ctx as jni::sys::jobject) })
}

#[cfg(target_os = "android")]
pub fn query_system_mediastore_audio() -> Vec<MediaStoreRawTrack> {
    let res = with_attached_env(|env| -> Result<String, String> {
        let ctx = service_context().ok_or_else(|| "no android context".to_string())?;
        let scanner_class = env
            .find_class("com/auralis/v2/MediaStoreScanner")
            .map_err(|e| format!("find MediaStoreScanner class: {e}"))?;
        let result = env
            .call_static_method(
                scanner_class,
                "queryAllAudio",
                "(Landroid/content/Context;)Ljava/lang/String;",
                &[jni::objects::JValue::Object(&ctx)],
            )
            .map_err(|e| format!("call queryAllAudio: {e}"))?
            .l()
            .map_err(|e| format!("return l: {e}"))?;
        if result.is_null() {
            return Ok("[]".to_string());
        }
        let j_str: jni::objects::JString<'_> = jni::objects::JString::from(result);
        let s: String = env
            .get_string(&j_str)
            .map(|st| st.into())
            .unwrap_or_default();
        Ok(s)
    });

    let json_str = match res {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            warn!(error = %e, "Failed to query system MediaStore audio via JNI");
            return Vec::new();
        }
        None => {
            warn!("JNI environment unavailable for MediaStore query");
            return Vec::new();
        }
    };

    if json_str.is_empty() {
        return Vec::new();
    }

    serde_json::from_str(&json_str).unwrap_or_default()
}

#[cfg(not(target_os = "android"))]
pub fn query_system_mediastore_audio() -> Vec<MediaStoreRawTrack> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::TrackFilter;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct TestTrackRepo {
        tracks: Mutex<Vec<Track>>,
    }

    impl TestTrackRepo {
        fn new() -> Self {
            Self {
                tracks: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl TrackRepository for TestTrackRepo {
        async fn find_all(
            &self,
            _filter: TrackFilter,
        ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.tracks.lock().unwrap().clone())
        }

        async fn count_filtered(
            &self,
            _filter: TrackFilter,
        ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.tracks.lock().unwrap().len() as u64)
        }

        async fn find_by_id(
            &self,
            id: uuid::Uuid,
        ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self
                .tracks
                .lock()
                .unwrap()
                .iter()
                .find(|t| t.id == id)
                .cloned())
        }

        async fn find_by_ids(
            &self,
            ids: &[uuid::Uuid],
        ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
            let list = self.tracks.lock().unwrap();
            let track_map: std::collections::HashMap<uuid::Uuid, Track> =
                list.iter().map(|t| (t.id, t.clone())).collect();
            let mut result = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(track) = track_map.get(id) {
                    result.push(track.clone());
                }
            }
            Ok(result)
        }

        async fn find_by_path(
            &self,
            path: &str,
        ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self
                .tracks
                .lock()
                .unwrap()
                .iter()
                .find(|t| t.file_path == path)
                .cloned())
        }

        async fn find_by_paths(
            &self,
            paths: &[&str],
        ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
            let list = self.tracks.lock().unwrap();
            let track_map: std::collections::HashMap<String, Track> = list
                .iter()
                .map(|t| (t.file_path.clone(), t.clone()))
                .collect();
            let mut result = Vec::with_capacity(paths.len());
            for path in paths {
                if let Some(track) = track_map.get(*path) {
                    result.push(track.clone());
                }
            }
            Ok(result)
        }

        async fn find_by_artist(
            &self,
            artist: &str,
        ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self
                .tracks
                .lock()
                .unwrap()
                .iter()
                .filter(|t| t.artist.as_deref() == Some(artist))
                .cloned()
                .collect())
        }

        async fn find_by_album(
            &self,
            album: &str,
        ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self
                .tracks
                .lock()
                .unwrap()
                .iter()
                .filter(|t| t.album.as_deref() == Some(album))
                .cloned()
                .collect())
        }

        async fn search(
            &self,
            _query: &str,
        ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.tracks.lock().unwrap().clone())
        }

        async fn insert(
            &self,
            track: &Track,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.tracks.lock().unwrap().push(track.clone());
            Ok(())
        }

        async fn update(
            &self,
            track: &Track,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let mut list = self.tracks.lock().unwrap();
            if let Some(pos) = list.iter().position(|t| t.id == track.id) {
                list[pos] = track.clone();
            }
            Ok(())
        }

        async fn delete(
            &self,
            id: uuid::Uuid,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.tracks.lock().unwrap().retain(|t| t.id != id);
            Ok(())
        }

        async fn delete_many(
            &self,
            ids: Vec<uuid::Uuid>,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let id_set: HashSet<uuid::Uuid> = ids.into_iter().collect();
            self.tracks
                .lock()
                .unwrap()
                .retain(|t| !id_set.contains(&t.id));
            Ok(())
        }

        async fn count(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.tracks.lock().unwrap().len() as u64)
        }

        async fn total_duration(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
            let total = self
                .tracks
                .lock()
                .unwrap()
                .iter()
                .map(|t| t.duration_secs as u64)
                .sum();
            Ok(total)
        }

        async fn recent(
            &self,
            limit: u32,
        ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
            let list = self.tracks.lock().unwrap();
            Ok(list.iter().take(limit as usize).cloned().collect())
        }

        async fn most_played(
            &self,
            limit: u32,
        ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
            let list = self.tracks.lock().unwrap();
            Ok(list.iter().take(limit as usize).cloned().collect())
        }

        async fn set_favorite(
            &self,
            _id: &str,
            _is_favorite: bool,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Ok(())
        }

        async fn get_albums_summary(
            &self,
        ) -> Result<
            Vec<crate::domain::models::AlbumSummary>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            Ok(Vec::new())
        }

        async fn get_artists_summary(
            &self,
        ) -> Result<
            Vec<crate::domain::models::ArtistSummary>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            Ok(Vec::new())
        }
    }

    fn generate_valid_wav() -> Vec<u8> {
        let sample_rate: u32 = 8000;
        let num_samples: u32 = 8000;
        let mut data = Vec::with_capacity(44 + num_samples as usize);
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + num_samples).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes()); // PCM
        data.extend_from_slice(&1u16.to_le_bytes()); // Mono
        data.extend_from_slice(&sample_rate.to_le_bytes());
        data.extend_from_slice(&sample_rate.to_le_bytes()); // Byte rate
        data.extend_from_slice(&1u16.to_le_bytes()); // Block align
        data.extend_from_slice(&8u16.to_le_bytes()); // Bits per sample
        data.extend_from_slice(b"data");
        data.extend_from_slice(&num_samples.to_le_bytes());
        data.resize(44 + num_samples as usize, 0x80);
        data
    }

    #[tokio::test]
    async fn test_ingest_buffer_corrupt_rejection_and_cleanup() {
        let temp_dir =
            std::env::temp_dir().join(format!("auralis_android_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let scanner = AndroidScanner::new();
        let repo: Arc<dyn TrackRepository> = Arc::new(TestTrackRepo::new());

        let corrupt_name = "damaged_track.mp3";
        let corrupt_bytes = b"CORRUPTED_BINARY_HEADER_NOT_AUDIO";

        let res = scanner
            .ingest_buffer(corrupt_name, corrupt_bytes, &temp_dir, &repo)
            .await;
        assert!(res.is_err(), "Corrupt audio buffer must be rejected");

        let err_msg = res.unwrap_err().to_string();
        assert!(
            err_msg.contains("Cannot import 'damaged_track.mp3'"),
            "Error message should mention clean filename: {err_msg}"
        );

        // Assert that the file was deleted from disk and not left in sandbox
        let expected_file_path = temp_dir.join(corrupt_name);
        assert!(
            !expected_file_path.exists(),
            "Corrupted sandbox file must be automatically deleted"
        );

        // Assert that repository has 0 tracks
        let count = repo.count().await.unwrap();
        assert_eq!(
            count, 0,
            "No dummy track should be inserted into repo on failure"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_ingest_buffer_valid_wav_success() {
        let temp_dir =
            std::env::temp_dir().join(format!("auralis_android_valid_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let scanner = AndroidScanner::new();
        let repo: Arc<dyn TrackRepository> = Arc::new(TestTrackRepo::new());

        let valid_name = "test_song.wav";
        let valid_bytes = generate_valid_wav();

        let track = scanner
            .ingest_buffer(valid_name, &valid_bytes, &temp_dir, &repo)
            .await
            .expect("Valid audio buffer must be ingested successfully");

        assert!(
            track.duration_secs > 0,
            "Ingested track duration must be > 0"
        );
        assert_eq!(track.title, "test_song.wav");

        let expected_file_path = temp_dir.join(valid_name);
        assert!(
            expected_file_path.exists(),
            "Valid file must be retained on disk"
        );

        let count = repo.count().await.unwrap();
        assert_eq!(count, 1, "Track must be inserted in repo");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

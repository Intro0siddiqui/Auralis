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

        std::fs::create_dir_all(target_dir).map_err(|e| {
            error!(dir = %target_dir.display(), error = %e, "Failed to create sandboxed destination directory");
            ScannerError::IoError(format!(
                "Failed to create sandboxed destination directory {}: {e}",
                target_dir.display()
            ))
        })?;

        let file_path = target_dir.join(name);
        std::fs::write(&file_path, bytes).map_err(|e| {
            error!(file = %file_path.display(), bytes = bytes.len(), error = %e, "Failed to write audio buffer to sandbox");
            ScannerError::IoError(format!(
                "Failed to write audio buffer to sandbox {}: {e}",
                file_path.display()
            ))
        })?;

        // Extract metadata using lofty; fallback gracefully if metadata is missing/corrupt
        let mut track = match MetadataExtractor::extract(&file_path) {
            Ok(t) => {
                debug!(file = %file_path.display(), title = %t.title, "Metadata extracted successfully from audio buffer");
                t
            }
            Err(e) => {
                warn!(
                    file = %file_path.display(),
                    error = %e,
                    "Metadata extraction failed for buffer payload; using fallback"
                );
                let format = detect_format(&file_path).unwrap_or(AudioFormat::Mp3);
                Track::new(
                    name.to_string(),
                    file_path.to_string_lossy().to_string(),
                    0,
                    format,
                )
            }
        };

        if track.title.trim().is_empty() || track.title == "Unknown" {
            track.title = name.to_string();
        }

        let path_str = file_path.to_string_lossy().to_string();
        let existing = track_repo
            .find_by_path(&path_str)
            .await
            .map_err(|e| {
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

        let mut track = match MetadataExtractor::extract(path) {
            Ok(t) => t,
            Err(e) => {
                warn!(file = %path_str, error = %e, "Metadata extraction failed; using fallback");
                let format = detect_format(path).unwrap_or(AudioFormat::Mp3);
                let file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Unknown".to_string());
                Track::new(file_name, path_str.clone(), 0, format)
            }
        };

        if track.title.trim().is_empty() || track.title == "Unknown" {
            track.title = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
        }

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
            errors: Vec::new(),
        };

        let mut found_files: HashSet<PathBuf> = HashSet::new();
        let mut all_audio_files: Vec<PathBuf> = Vec::new();

        for root in sandboxed_dirs {
            if !root.exists() {
                continue;
            }

            let mut dirs_to_visit = vec![root.clone()];
            while let Some(current) = dirs_to_visit.pop() {
                let entries = match std::fs::read_dir(&current) {
                    Ok(entries) => entries,
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

                    if entry_path.is_dir() {
                        dirs_to_visit.push(entry_path);
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
            errors = summary.errors.len(),
            "Android sandboxed scan completed"
        );

        Ok(summary)
    }
}

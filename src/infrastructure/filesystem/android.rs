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
        std::fs::create_dir_all(target_dir).map_err(|e| {
            ScannerError::IoError(format!(
                "Failed to create sandboxed destination directory: {e}"
            ))
        })?;

        let file_path = target_dir.join(name);
        std::fs::write(&file_path, bytes).map_err(|e| {
            ScannerError::IoError(format!("Failed to write audio buffer to sandbox: {e}"))
        })?;

        // Extract metadata using lofty; fallback gracefully if metadata is missing/corrupt
        let mut track = match MetadataExtractor::extract(&file_path) {
            Ok(t) => t,
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

        track_repo
            .insert(&track)
            .await
            .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

        info!(id = %track.id, title = %track.title, "Ingested audio buffer into library");
        Ok(track)
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
                    } else if entry_path.is_file() && is_audio_file(&entry_path) {
                        found_files.insert(entry_path.clone());
                        all_audio_files.push(entry_path);
                    }
                }
            }
        }

        let total_files = all_audio_files.len();

        for (idx, file_path) in all_audio_files.into_iter().enumerate() {
            debug!(path = %file_path.display(), "Processing sandboxed audio file");

            match MetadataExtractor::extract(&file_path) {
                Ok(mut track) => {
                    let file_name = file_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Unknown".to_string());

                    if track.title.trim().is_empty() || track.title == "Unknown" {
                        track.title = file_name;
                    }

                    match track_repo.insert(&track).await {
                        Ok(()) => summary.tracks_added += 1,
                        Err(e) => {
                            error!(path = %file_path.display(), error = %e, "Failed to save track to DB");
                            summary
                                .errors
                                .push(format!("{}: {}", file_path.display(), e));
                        }
                    }
                }
                Err(e) => {
                    error!(path = %file_path.display(), error = %e, "Failed to extract metadata");
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

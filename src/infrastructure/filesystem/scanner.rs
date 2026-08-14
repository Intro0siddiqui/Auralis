//! Directory Scanner
//!
//! Scans directories for music files.

use crate::domain::models::{AudioFormat, ScanSummary, TrackFilter};
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::filesystem::metadata::MetadataExtractor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Scans directories for music files
pub struct DirectoryScanner {
    #[allow(dead_code)]
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
}

impl DirectoryScanner {
    /// Create a new directory scanner
    pub fn new(include_patterns: Vec<String>, exclude_patterns: Vec<String>) -> Self {
        Self {
            include_patterns,
            exclude_patterns,
        }
    }

    /// Create a scanner with default audio file patterns
    pub fn default_audio() -> Self {
        Self::new(
            vec![
                "**/*.mp3".to_string(),
                "**/*.flac".to_string(),
                "**/*.wav".to_string(),
                "**/*.aac".to_string(),
                "**/*.ogg".to_string(),
                "**/*.m4a".to_string(),
            ],
            // NOTE: The `".*"` / `".*/"` exclude patterns only filter Unix-style
            // dotfiles (files/dirs whose name begins with `.`). Windows marks
            // hidden files via filesystem attributes instead, so this pattern
            // will NOT exclude them. This is a deliberate Unix-centric
            // assumption; filtering Windows hidden files would require checking
            // file attributes via the `windows` crate.
            vec![".*".to_string(), ".*/".to_string()],
        )
    }

    /// Scan a directory for music files recursively
    pub async fn scan(&self, path: &Path) -> Result<Vec<PathBuf>, ScannerError> {
        info!(path = %path.display(), "Scanning directory");

        if !path.exists() {
            warn!(path = %path.display(), "Directory does not exist");
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        let mut dirs_to_visit = vec![path.to_path_buf()];

        while let Some(current_dir) = dirs_to_visit.pop() {
            if let Ok(entries) = std::fs::read_dir(&current_dir) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    let file_name = entry.file_name().to_string_lossy().to_string();

                    // Skip hidden files/folders (starting with dot)
                    if file_name.starts_with('.') {
                        continue;
                    }

                    if entry_path.is_dir() {
                        dirs_to_visit.push(entry_path);
                    } else if entry_path.is_file() {
                        if let Some(ext) = entry_path.extension().and_then(|e| e.to_str()) {
                            let ext_lower = ext.to_lowercase();
                            if matches!(
                                ext_lower.as_str(),
                                "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a"
                            ) {
                                if !self.is_excluded(&entry_path) {
                                    files.push(entry_path);
                                }
                            }
                        }
                    }
                }
            }
        }

        info!(path = %path.display(), found = files.len(), "Scan completed");
        Ok(files)
    }

    /// Scan multiple directories
    pub async fn scan_paths(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut all_files = Vec::new();

        for path in paths {
            match self.scan(path).await {
                Ok(files) => all_files.extend(files),
                Err(e) => {
                    error!(path = %path.display(), error = %e, "Failed to scan path");
                }
            }
        }

        all_files
    }

    /// Scan library paths and insert tracks into the repository.
    ///
    /// After scanning, removes any database tracks whose file paths fall
    /// within the scanned directories but no longer exist on disk.
    pub async fn scan_library_paths(
        &self,
        paths: &[PathBuf],
        track_repo: Arc<dyn TrackRepository>,
    ) -> Result<ScanSummary, ScannerError> {
        info!(path_count = paths.len(), "Starting library scan");

        let mut summary = ScanSummary {
            tracks_added: 0,
            tracks_updated: 0,
            tracks_removed: 0,
            errors: Vec::new(),
        };

        // Collect the set of files found on disk for later comparison.
        let mut found_files: std::collections::HashSet<std::path::PathBuf> =
            std::collections::HashSet::new();

        for path in paths {
            info!(path = %path.display(), "Scanning path");

            if !path.exists() {
                warn!(path = %path.display(), "Path does not exist, skipping");
                summary
                    .errors
                    .push(format!("Path does not exist: {}", path.display()));
                continue;
            }

            let audio_files = self.scan(path).await?;
            info!(path = %path.display(), count = audio_files.len(), "Found audio files");

            for file_path in &audio_files {
                found_files.insert(file_path.clone());
            }

            for file_path in audio_files {
                debug!(path = %file_path.display(), "Processing file");

                match self.process_file(&file_path, &track_repo).await {
                    Ok(ScanResult::Added) => summary.tracks_added += 1,
                    Ok(ScanResult::Updated) => summary.tracks_updated += 1,
                    Ok(ScanResult::Skipped) => {}
                    Err(e) => {
                        error!(path = %file_path.display(), error = %e, "Failed to process file");
                        summary
                            .errors
                            .push(format!("{}: {}", file_path.display(), e));
                    }
                }
            }
        }

        // Detect and remove tracks no longer present in the scanned directories.
        let scan_prefixes: Vec<std::path::PathBuf> = paths.to_vec();
        let all_tracks = track_repo
            .find_all(TrackFilter::default())
            .await
            .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

        let mut ids_to_remove: Vec<uuid::Uuid> = Vec::new();
        for track in &all_tracks {
            let track_path = std::path::PathBuf::from(&track.file_path);
            // Only consider tracks that live inside one of the scanned directories.
            let in_scan_scope = scan_prefixes.iter().any(|p| track_path.starts_with(p));
            if in_scan_scope && !found_files.contains(&track_path) {
                ids_to_remove.push(track.id);
            }
        }

        if !ids_to_remove.is_empty() {
            let count = ids_to_remove.len() as u32;
            summary.tracks_removed = count;
            if let Err(e) = track_repo.delete_many(ids_to_remove).await {
                error!(count = count, error = %e, "Failed to remove stale tracks");
                summary
                    .errors
                    .push(format!("Failed to remove {count} stale tracks: {e}"));
                summary.tracks_removed = 0;
            } else {
                info!(count = count, "Removed stale tracks");
            }
        }

        info!(
            added = summary.tracks_added,
            updated = summary.tracks_updated,
            removed = summary.tracks_removed,
            errors = summary.errors.len(),
            "Library scan completed"
        );

        Ok(summary)
    }

    /// Process a single file: extract metadata and insert/update in repository
    async fn process_file(
        &self,
        path: &Path,
        track_repo: &Arc<dyn TrackRepository>,
    ) -> Result<ScanResult, ScannerError> {
        let path_str = path.to_string_lossy().to_string();

        // Check if track already exists
        let existing = track_repo
            .find_by_path(&path_str)
            .await
            .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

        // Extract metadata
        let track = MetadataExtractor::extract(path).map_err(|e| {
            ScannerError::MetadataError(format!("Failed to extract metadata from {path_str}: {e}"))
        })?;

        if let Some(existing_track) = existing {
            // Update existing track with new metadata but preserve play stats
            let mut updated_track = track;
            updated_track.id = existing_track.id;
            updated_track.date_added = existing_track.date_added;
            updated_track.last_played = existing_track.last_played;
            updated_track.play_count = existing_track.play_count;

            track_repo
                .update(&updated_track)
                .await
                .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

            debug!(path = %path_str, id = %updated_track.id, "Track updated");
            Ok(ScanResult::Updated)
        } else {
            track_repo
                .insert(&track)
                .await
                .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

            debug!(path = %path_str, id = %track.id, "Track added");
            Ok(ScanResult::Added)
        }
    }

    /// Check if a path should be excluded
    ///
    /// NOTE: This is Unix-centric — it only matches names beginning with `.`
    /// (dotfiles). Windows hidden files are marked via filesystem attributes
    /// and are not excluded by these patterns. See `default_audio`.
    fn is_excluded(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in &self.exclude_patterns {
            if pattern.ends_with('/') {
                // Directory pattern
                let dir_pattern = &pattern[..pattern.len() - 1];
                if let Some(parent) = path.parent() {
                    if parent.to_string_lossy().contains(dir_pattern) {
                        return true;
                    }
                }
            } else if path_str.contains(pattern) {
                return true;
            }
        }

        false
    }

    /// Detect audio format from file extension
    pub fn detect_format(path: &Path) -> Option<AudioFormat> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(AudioFormat::from_extension)
    }

    /// Get file size
    pub fn get_file_size(path: &Path) -> Result<u64, ScannerError> {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| ScannerError::IoError(e.to_string()))
    }
}

/// Result of scanning a single file
#[derive(Debug)]
#[allow(dead_code)]
enum ScanResult {
    Added,
    Updated,
    Skipped,
}

/// Scanner-related errors
#[derive(Debug, thiserror::Error)]
pub enum ScannerError {
    #[error("IO error: {0}")]
    IoError(String),

    #[error("Glob error: {0}")]
    GlobError(String),

    #[error("Metadata error: {0}")]
    MetadataError(String),

    #[error("Repository error: {0}")]
    RepositoryError(String),
}

//! Directory Scanner
//!
//! Scans directories for music files.

use crate::domain::models::{AudioFormat, ScanSummary};
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::filesystem::metadata::MetadataExtractor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Scans directories for music files
pub struct DirectoryScanner {
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
                "**/*.opus".to_string(),
                "**/*.m4a".to_string(),
            ],
            vec![".*".to_string(), ".*/".to_string()],
        )
    }

    /// Scan a directory for music files
    pub async fn scan(&self, path: &Path) -> Result<Vec<PathBuf>, ScannerError> {
        info!(path = %path.display(), "Scanning directory");

        let mut files = Vec::new();

        for pattern in &self.include_patterns {
            let full_pattern = path.join(pattern);
            debug!(pattern = %full_pattern.display(), "Scanning with pattern");

            match glob::glob(full_pattern.to_str().unwrap_or("")) {
                Ok(entries) => {
                    for entry in entries.filter_map(|e| e.ok()) {
                        if entry.is_file() && !self.is_excluded(&entry) {
                            files.push(entry);
                        }
                    }
                }
                Err(e) => {
                    warn!(pattern = %pattern, error = %e, "Invalid glob pattern");
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

    /// Scan library paths and insert tracks into the repository
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

        info!(
            added = summary.tracks_added,
            updated = summary.tracks_updated,
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

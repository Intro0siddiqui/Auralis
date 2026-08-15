//! Android Filesystem and Scoped Storage Scanner
//!
//! Handles sandboxed SAF storage ingestion, app internal storage (`app_data_dir/music`),
//! streaming buffer decoding, and lofty tag extraction.

use crate::domain::models::{AudioFormat, ScanSummary, Track};
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::filesystem::desktop::DesktopScanner;
use crate::infrastructure::filesystem::metadata::MetadataExtractor;
use crate::infrastructure::filesystem::scanner::{is_audio_file, ScanProgress, ScannerError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

/// Android scanner specialized for Android Scoped Storage and SAF ingestion
pub struct AndroidScanner {
    scanner: DesktopScanner,
}

impl Default for AndroidScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidScanner {
    /// Create a new Android scanner
    pub fn new() -> Self {
        Self {
            scanner: DesktopScanner::default_audio(),
        }
    }

    /// Standard Android storage paths to inspect
    pub fn standard_android_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for p in [
            "/storage/emulated/0/Music",
            "/storage/emulated/0/Download",
            "/storage/emulated/0/Audio",
            "/sdcard/Music",
            "/sdcard/Download",
        ] {
            let path = PathBuf::from(p);
            if path.exists() {
                paths.push(path);
            }
        }
        paths
    }

    /// Ingest an in-memory audio byte buffer directly into the library (bypassing SAF restrictions)
    pub async fn ingest_buffer(
        &self,
        name: &str,
        bytes: &[u8],
        target_dir: &Path,
        track_repo: &Arc<dyn TrackRepository>,
    ) -> Result<Track, ScannerError> {
        std::fs::create_dir_all(target_dir).map_err(|e| {
            ScannerError::IoError(format!("Failed to create destination directory: {e}"))
        })?;

        let file_path = target_dir.join(name);
        std::fs::write(&file_path, bytes).map_err(|e| {
            ScannerError::IoError(format!("Failed to write audio buffer to file: {e}"))
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
                let format = DesktopScanner::detect_format(&file_path).unwrap_or(AudioFormat::Mp3);
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

    /// Scan scoped storage paths with progress updates
    pub async fn scan_library_paths_with_progress<F>(
        &self,
        paths: &[PathBuf],
        track_repo: Arc<dyn TrackRepository>,
        progress_callback: Option<F>,
    ) -> Result<ScanSummary, ScannerError>
    where
        F: FnMut(ScanProgress) + Send + 'static,
    {
        self.scanner
            .scan_library_paths_with_progress(paths, track_repo, progress_callback)
            .await
    }

    /// Scan a single path or directory
    pub async fn scan(&self, path: &Path) -> Result<Vec<PathBuf>, ScannerError> {
        self.scanner.scan(path).await
    }

    /// Check if a path is an audio file
    pub fn is_audio_file(path: &Path) -> bool {
        is_audio_file(path)
    }
}

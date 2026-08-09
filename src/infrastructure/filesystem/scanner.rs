//! Directory Scanner
//!
//! Scans directories for music files.

use crate::domain::models::AudioFormat;
use glob::glob;
use std::path::{Path, PathBuf};
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

    /// Scan a directory for music files
    pub async fn scan(&self, path: &Path) -> Result<Vec<PathBuf>, ScannerError> {
        info!(path = %path.display(), "Scanning directory");

        let mut files = Vec::new();

        for pattern in &self.include_patterns {
            let full_pattern = path.join(pattern);
            debug!(pattern = %full_pattern.display(), "Scanning with pattern");

            match glob(full_pattern.to_str().unwrap_or("")) {
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
            .and_then(|ext| AudioFormat::from_extension(ext))
    }

    /// Get file size
    pub fn get_file_size(path: &Path) -> Result<u64, ScannerError> {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| ScannerError::IoError(e.to_string()))
    }
}

/// Scanner-related errors
#[derive(Debug, thiserror::Error)]
pub enum ScannerError {
    #[error("IO error: {0}")]
    IoError(String),

    #[error("Glob error: {0}")]
    GlobError(String),
}

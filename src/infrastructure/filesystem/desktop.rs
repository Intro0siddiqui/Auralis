//! Desktop Filesystem Scanner
//!
//! Handles POSIX and Win32 recursive filesystem walking, case-insensitive
//! audio format filtering, and live progress reporting.

use crate::domain::models::{AudioFormat, ScanSummary, TrackFilter};
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::filesystem::metadata::MetadataExtractor;
use crate::infrastructure::filesystem::scanner::{
    is_audio_file, ScanProgress, ScanResult, ScannerError,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Desktop scanner for POSIX/Win32 local filesystems
pub struct DesktopScanner {
    #[allow(dead_code)]
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
}

impl DesktopScanner {
    /// Create a new desktop directory scanner
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
            vec![".*".to_string(), ".*/".to_string()],
        )
    }

    /// Check if a path is a supported audio file (case-insensitive)
    pub fn is_audio_file(path: &Path) -> bool {
        is_audio_file(path)
    }

    /// Recursively scan a directory or file path for audio files
    ///
    /// Uses `tokio::task::spawn_blocking` for `read_dir` so the async runtime
    /// is not blocked, and protects against symlink loops via a visited
    /// canonical-dir set + depth limit + symlink skip.
    pub async fn scan(&self, path: &Path) -> Result<Vec<PathBuf>, ScannerError> {
        info!(path = %path.display(), "Scanning desktop filesystem path");

        // Existence check offloaded to blocking pool as well.
        let path_owned = path.to_path_buf();
        let exists = tokio::task::spawn_blocking(move || path_owned.exists())
            .await
            .map_err(|e| ScannerError::IoError(format!("Join error: {e}")))?;
        if !exists {
            warn!(path = %path.display(), "Directory does not exist");
            return Ok(Vec::new());
        }

        if path.is_file() {
            if Self::is_audio_file(path) && !self.is_excluded(path) {
                return Ok(vec![path.to_path_buf()]);
            }
            return Ok(Vec::new());
        }

        const MAX_DEPTH: usize = 64;
        let mut files = Vec::new();
        let mut dirs_to_visit: Vec<(PathBuf, usize)> = vec![(path.to_path_buf(), 0)];
        let mut visited: HashSet<PathBuf> = HashSet::new();
        // Seed visited with canonical root to avoid revisiting via symlink.
        if let Ok(canonical) = tokio::task::spawn_blocking({
            let p = path.to_path_buf();
            move || std::fs::canonicalize(&p)
        })
        .await
        .unwrap_or(Err(std::io::Error::from(std::io::ErrorKind::NotFound)))
        {
            visited.insert(canonical);
        }

        while let Some((current_dir, depth)) = dirs_to_visit.pop() {
            if depth > MAX_DEPTH {
                warn!(path = %current_dir.display(), depth, "Max depth exceeded; skipping");
                continue;
            }
            // `read_dir` is blocking — offload to the blocking pool.
            let dir_clone = current_dir.clone();
            let entries_res = tokio::task::spawn_blocking(move || std::fs::read_dir(&dir_clone))
                .await
                .map_err(|e| ScannerError::IoError(format!("Join error: {e}")))?;
            let entries = match entries_res {
                Ok(entries) => entries,
                Err(e) => {
                    warn!(path = %current_dir.display(), error = %e, "Cannot read directory");
                    if current_dir == path {
                        return Err(ScannerError::IoError(format!(
                            "Access denied or error reading {}: {}",
                            current_dir.display(),
                            e
                        )));
                    }
                    continue;
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) => {
                        warn!(path = %current_dir.display(), error = %e, "Error reading entry; skipping");
                        continue;
                    }
                };

                let entry_path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();

                // Skip hidden files/folders (starting with dot)
                if file_name.starts_with('.') {
                    continue;
                }

                // Symlink protection: skip symlinked dirs entirely to avoid
                // loops (e.g., `ln -s ..` or circular mounts). We check
                // `symlink_metadata` without following, then decide.
                let ft = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if ft.is_symlink() {
                    // Resolve symlink target and check if it points to a dir —
                    // if so, skip to avoid loops. Files are handled via the
                    // is_file branch below after following? We intentionally
                    // skip symlinked dirs only; symlinked files are allowed
                    // if they are audio files but we still validate canonical.
                    if entry_path.is_dir() {
                        debug!(path = %entry_path.display(), "Skipping symlinked directory");
                        continue;
                    }
                }

                if entry_path.is_dir() {
                    // Canonicalize and check visited set to prevent loops via
                    // hard links / bind mounts that are not symlinks.
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
                    && Self::is_audio_file(&entry_path)
                    && !self.is_excluded(&entry_path)
                {
                    files.push(entry_path);
                }
            }
        }

        info!(path = %path.display(), found = files.len(), "Scan completed");
        Ok(files)
    }

    /// Scan multiple paths and aggregate results
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

    /// Scan library paths without a progress callback
    pub async fn scan_library_paths(
        &self,
        paths: &[PathBuf],
        track_repo: Arc<dyn TrackRepository>,
    ) -> Result<ScanSummary, ScannerError> {
        self.scan_library_paths_with_progress(paths, track_repo, None::<fn(ScanProgress)>)
            .await
    }

    /// Scan library paths with real-time progress callbacks and error resilience
    pub async fn scan_library_paths_with_progress<F>(
        &self,
        paths: &[PathBuf],
        track_repo: Arc<dyn TrackRepository>,
        mut progress_callback: Option<F>,
    ) -> Result<ScanSummary, ScannerError>
    where
        F: FnMut(ScanProgress) + Send + 'static,
    {
        info!(path_count = paths.len(), "Starting desktop library scan");

        let mut summary = ScanSummary {
            tracks_added: 0,
            tracks_updated: 0,
            tracks_removed: 0,
            errors: Vec::new(),
        };

        let mut found_files: HashSet<PathBuf> = HashSet::new();
        let mut all_audio_files: Vec<PathBuf> = Vec::new();

        for path in paths {
            if !path.exists() {
                warn!(path = %path.display(), "Path does not exist, skipping");
                summary
                    .errors
                    .push(format!("Path does not exist: {}", path.display()));
                continue;
            }

            match self.scan(path).await {
                Ok(audio_files) => {
                    for file_path in audio_files {
                        if found_files.insert(file_path.clone()) {
                            all_audio_files.push(file_path);
                        }
                    }
                }
                Err(e) => {
                    error!(path = %path.display(), error = %e, "Failed to scan path");
                    summary
                        .errors
                        .push(format!("Failed to scan {}: {}", path.display(), e));
                }
            }
        }

        let total_files = all_audio_files.len();

        if let Some(ref mut cb) = progress_callback {
            cb(ScanProgress {
                current_file: String::new(),
                total_files,
                processed_files: 0,
                percentage: if total_files == 0 { 100.0 } else { 0.0 },
                tracks_added: 0,
                tracks_updated: 0,
                error_count: summary.errors.len(),
            });
        }

        for (idx, file_path) in all_audio_files.into_iter().enumerate() {
            debug!(path = %file_path.display(), "Processing file");

            match self.process_file(&file_path, &track_repo).await {
                Ok(ScanResult::Added) => summary.tracks_added += 1,
                Ok(ScanResult::Updated) => summary.tracks_updated += 1,
                Ok(ScanResult::Skipped) => {}
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

        // Clean up stale database records that no longer exist on disk
        let scan_prefixes: Vec<PathBuf> = paths.to_vec();
        match track_repo.find_all(TrackFilter::default()).await {
            Ok(all_tracks) => {
                let mut ids_to_remove: Vec<uuid::Uuid> = Vec::new();
                for track in &all_tracks {
                    let track_path = PathBuf::from(&track.file_path);
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
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to query repository for stale tracks");
                summary
                    .errors
                    .push(format!("Failed to check stale tracks: {e}"));
            }
        }

        info!(
            added = summary.tracks_added,
            updated = summary.tracks_updated,
            removed = summary.tracks_removed,
            errors = summary.errors.len(),
            "Desktop library scan completed"
        );

        Ok(summary)
    }

    /// Process single audio file and update repository
    ///
    /// `std::fs::metadata` and `lofty` parsing are blocking — both are
    /// offloaded via `tokio::task::spawn_blocking` so the async executor is
    /// not stalled. Shares the same mtime+size incremental check as
    /// `AndroidScanner::process_file` (see `file_mtime_size` helper).
    async fn process_file(
        &self,
        path: &Path,
        track_repo: &Arc<dyn TrackRepository>,
    ) -> Result<ScanResult, ScannerError> {
        let path_str = path.to_string_lossy().to_string();

        let existing = track_repo
            .find_by_path(&path_str)
            .await
            .map_err(|e| ScannerError::RepositoryError(e.to_string()))?;

        // Incremental scan: stat via spawn_blocking (shared logic with
        // android.rs — see `file_mtime_size` in scanner.rs). If mtime and
        // size match the indexed row, skip the expensive lofty re-parse.
        let path_for_stat = path.to_path_buf();
        let path_str_for_stat = path_str.clone();
        let inner = tokio::task::spawn_blocking(move || {
            let meta = std::fs::metadata(&path_for_stat).map_err(|e| {
                ScannerError::MetadataError(format!("Failed to stat {path_str_for_stat}: {e}"))
            })?;
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
            if existing_track.mtime == mtime && existing_track.file_size == size {
                debug!(path = %path_str, "File unchanged, skipping");
                return Ok(ScanResult::Skipped);
            }
        }

        // Lofty extraction is blocking (file I/O + header decode) — offload.
        let path_for_extract = path.to_path_buf();
        let path_str_for_extract = path_str.clone();
        let inner_track = tokio::task::spawn_blocking(move || {
            MetadataExtractor::extract(&path_for_extract).map_err(|e| {
                ScannerError::MetadataError(format!(
                    "Failed to extract metadata from {path_str_for_extract}: {e}"
                ))
            })
        })
        .await
        .map_err(|e| ScannerError::MetadataError(format!("Join error: {e}")))?;
        let mut track = inner_track?;
        track.mtime = mtime;

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

    /// Check if path matches exclusion patterns
    fn is_excluded(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in &self.exclude_patterns {
            if let Ok(glob_pattern) = glob::Pattern::new(pattern) {
                if glob_pattern.matches(&path_str) {
                    return true;
                }
                if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
                    if glob_pattern.matches(file_name) {
                        return true;
                    }
                }
            }
            if pattern.ends_with('/') {
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

    /// Detect format from path extension
    pub fn detect_format(path: &Path) -> Option<AudioFormat> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(AudioFormat::from_extension)
    }

    /// Get file size in bytes
    pub fn get_file_size(path: &Path) -> Result<u64, ScannerError> {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| ScannerError::IoError(e.to_string()))
    }
}

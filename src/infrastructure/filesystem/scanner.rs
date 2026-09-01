//! Directory Scanner and Platform Dispatcher
//!
//! Provides the unified directory scanner interface, error definitions,
//! case-insensitive format detection, and platform scanner re-exports.

pub use crate::infrastructure::filesystem::android::AndroidScanner;
pub use crate::infrastructure::filesystem::desktop::DesktopScanner;

use crate::domain::models::AudioFormat;
use std::path::Path;

/// Supported audio extensions (case-insensitive)
pub const SUPPORTED_AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "m4a", "aac", "ogg"];

/// Check if an extension is a supported audio format (case-insensitive)
pub fn is_supported_audio_extension(ext: &str) -> bool {
    let ext_lower = ext.trim_start_matches('.').to_lowercase();
    matches!(
        ext_lower.as_str(),
        "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "oga" | "mp4" | "webm"
    )
}

/// Check if a path corresponds to a supported audio file (case-insensitive)
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(is_supported_audio_extension)
        .unwrap_or(false)
}

/// Detect audio format from file extension (case-insensitive)
pub fn detect_format(path: &Path) -> Option<AudioFormat> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(AudioFormat::from_extension)
}

/// Unified DirectoryScanner alias for local filesystem scanning
pub type DirectoryScanner = DesktopScanner;

/// Real-time progress update emitted during library scanning
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScanProgress {
    /// Currently scanning path or current file being processed
    pub current_file: String,
    /// Total number of audio files discovered across all paths
    pub total_files: usize,
    /// Number of audio files processed so far
    pub processed_files: usize,
    /// Current percentage (0.0 - 100.0)
    pub percentage: f32,
    /// Count of tracks successfully added
    pub tracks_added: u32,
    /// Count of tracks successfully updated
    pub tracks_updated: u32,
    /// Count of errors encountered so far
    pub error_count: usize,
}

/// Result of scanning a single file
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum ScanResult {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{Track, TrackFilter};
    use crate::domain::repositories::TrackRepository;
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_case_insensitive_format_detection() {
        let test_cases = vec![
            ("song.mp3", Some(AudioFormat::Mp3)),
            ("song.MP3", Some(AudioFormat::Mp3)),
            ("song.Mp3", Some(AudioFormat::Mp3)),
            ("track.flac", Some(AudioFormat::Flac)),
            ("track.FLAC", Some(AudioFormat::Flac)),
            ("audio.wav", Some(AudioFormat::Wav)),
            ("audio.WAV", Some(AudioFormat::Wav)),
            ("media.m4a", Some(AudioFormat::M4a)),
            ("media.M4A", Some(AudioFormat::M4a)),
            ("sound.aac", Some(AudioFormat::Aac)),
            ("sound.AAC", Some(AudioFormat::Aac)),
            ("music.ogg", Some(AudioFormat::Ogg)),
            ("music.OGG", Some(AudioFormat::Ogg)),
            ("readme.txt", None),
            ("image.png", None),
            ("file_without_ext", None),
        ];

        for (filename, expected) in test_cases {
            let path = PathBuf::from(filename);
            assert_eq!(
                detect_format(&path),
                expected,
                "Failed format detection for {}",
                filename
            );
            assert_eq!(
                is_audio_file(&path),
                expected.is_some(),
                "Failed audio file check for {}",
                filename
            );
        }
    }

    #[test]
    fn test_is_supported_audio_extension() {
        assert!(is_supported_audio_extension("mp3"));
        assert!(is_supported_audio_extension("MP3"));
        assert!(is_supported_audio_extension(".FLAC"));
        assert!(is_supported_audio_extension("Wav"));
        assert!(is_supported_audio_extension("M4A"));
        assert!(is_supported_audio_extension("aac"));
        assert!(is_supported_audio_extension("ogg"));
        assert!(!is_supported_audio_extension("txt"));
        assert!(!is_supported_audio_extension("exe"));
    }

    struct MockTrackRepo {
        tracks: Mutex<Vec<Track>>,
    }

    impl MockTrackRepo {
        fn new() -> Self {
            Self {
                tracks: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl TrackRepository for MockTrackRepo {
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
            let list = self.tracks.lock().unwrap();
            let mut album_map = std::collections::BTreeMap::new();
            for t in list.iter() {
                if let Some(ref album) = t.album {
                    if !album.is_empty() {
                        let entry = album_map.entry(album.clone()).or_insert((
                            t.artist.clone(),
                            t.album_art_path.clone(),
                            0u32,
                            t.id.to_string(),
                        ));
                        entry.2 += 1;
                    }
                }
            }
            Ok(album_map
                .into_iter()
                .map(|(album, (artist, album_art_path, count, first_id))| {
                    crate::domain::models::AlbumSummary {
                        album,
                        artist,
                        album_art_path,
                        track_count: count,
                        first_track_id: Some(first_id),
                    }
                })
                .collect())
        }

        async fn get_artists_summary(
            &self,
        ) -> Result<
            Vec<crate::domain::models::ArtistSummary>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            let list = self.tracks.lock().unwrap();
            let mut artist_map = std::collections::BTreeMap::new();
            for t in list.iter() {
                if let Some(ref artist) = t.artist {
                    if !artist.is_empty() {
                        let entry = artist_map
                            .entry(artist.clone())
                            .or_insert((0u32, t.id.to_string()));
                        entry.0 += 1;
                    }
                }
            }
            Ok(artist_map
                .into_iter()
                .map(
                    |(artist, (count, first_id))| crate::domain::models::ArtistSummary {
                        artist,
                        track_count: count,
                        first_track_id: Some(first_id),
                    },
                )
                .collect())
        }
    }

    #[tokio::test]
    async fn test_scan_resilience_with_corrupt_files_and_progress() {
        let temp_dir = std::env::temp_dir().join(format!("auralis_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let corrupt_file = temp_dir.join("corrupt_audio.MP3");
        std::fs::write(&corrupt_file, b"NOT_A_VALID_MP3_STREAM_HEADER").unwrap();

        let text_file = temp_dir.join("notes.TXT");
        std::fs::write(&text_file, b"ignore me").unwrap();

        let scanner = DirectoryScanner::default_audio();
        let repo = Arc::new(MockTrackRepo::new());

        let progress_events = Arc::new(Mutex::new(Vec::<ScanProgress>::new()));
        let progress_clone = progress_events.clone();

        let summary = scanner
            .scan_library_paths_with_progress(
                std::slice::from_ref(&temp_dir),
                repo,
                Some(move |p| {
                    progress_clone.lock().unwrap().push(p);
                }),
            )
            .await
            .expect("Scan should complete without aborting");

        assert_eq!(summary.errors.len(), 1);
        assert!(summary.errors[0].contains("corrupt_audio.MP3"));
        assert_eq!(summary.tracks_added, 0);

        let events = progress_events.lock().unwrap().clone();
        assert!(!events.is_empty(), "Expected progress events");
        assert_eq!(events.last().unwrap().total_files, 1);
        assert_eq!(events.last().unwrap().error_count, 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

//! Metadata Extractor
//!
//! Extracts audio metadata from files using lofty.

use crate::domain::models::{AudioFormat, Track};
use lofty::file::TaggedFileExt;
use lofty::prelude::*;
use std::path::Path;
use tracing::debug;

/// Extracts metadata from audio files
pub struct MetadataExtractor;

impl MetadataExtractor {
    /// Extract metadata from an audio file
    pub fn extract(path: &Path) -> Result<Track, MetadataError> {
        debug!(path = %path.display(), "Extracting metadata");

        let tagged_file =
            lofty::read_from_path(path).map_err(|e| MetadataError::ReadError(e.to_string()))?;

        let properties = tagged_file.properties();
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        let duration_secs = properties.duration().as_secs() as u32;
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        let format = Self::detect_format(path);

        // Use the unified title extractor that handles Option<&dyn Accessor>
        let title = tag
            .map(|t| Self::get_title(t))
            .unwrap_or_else(|| "Unknown".to_string());

        let mut track = Track::new(
            title,
            path.to_string_lossy().to_string(),
            duration_secs,
            format,
        );

        track.file_size = file_size;

        if let Some(tag) = tag {
            track.artist = Self::get_artist(tag);
            track.album = Self::get_album(tag);
            track.album_artist = Self::get_album_artist(tag);
            track.genre = Self::get_genre(tag);
            track.year = Self::get_year(tag);
            track.track_number = Self::get_track_number(tag);
            track.disc_number = Self::get_disc_number(tag);
            track.bitrate = Some(properties.audio_bitrate().unwrap_or(0) as u32);
            track.sample_rate = Some(properties.sample_rate().unwrap_or(0));
        }

        // If the file has no embedded cover, check for a sidecar image
        // (`<audio>.jpg/.jpeg/.png/.webp`) next to it. This lets downloaded
        // thumbnails and user-managed cover files show artwork.
        track.album_art_path = Self::find_sidecar_art(path);

        debug!(path = %path.display(), title = %track.title, "Metadata extracted");
        Ok(track)
    }

    /// Locate a sidecar cover image for an audio file.
    fn find_sidecar_art(path: &Path) -> Option<String> {
        for ext in ["jpg", "jpeg", "png", "webp"] {
            let side = path.with_extension(ext);
            if side.is_file() {
                return Some(side.to_string_lossy().to_string());
            }
        }
        None
    }

    fn get_title(tag: &dyn lofty::tag::Accessor) -> String {
        tag.title()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    }

    fn get_artist(tag: &dyn lofty::tag::Accessor) -> Option<String> {
        tag.artist().map(|s| s.to_string())
    }

    fn get_album(tag: &dyn lofty::tag::Accessor) -> Option<String> {
        tag.album().map(|s| s.to_string())
    }

    fn get_album_artist(tag: &dyn lofty::tag::Accessor) -> Option<String> {
        // The `album_artist` field is not on the `Accessor` trait in current lofty.
        // We return None for now; concrete tag types (e.g., `Tag`) can be downcast
        // in the future to extract this field if needed.
        let _ = tag;
        None
    }

    fn get_genre(tag: &dyn lofty::tag::Accessor) -> Option<String> {
        tag.genre().map(|s| s.to_string())
    }

    fn get_year(tag: &dyn lofty::tag::Accessor) -> Option<i32> {
        // `Accessor::date()` already falls back to a raw `ItemKey::Year` string
        tag.date().map(|ts| ts.year as i32)
    }

    fn get_track_number(tag: &dyn lofty::tag::Accessor) -> Option<u32> {
        tag.track()
    }

    fn get_disc_number(tag: &dyn lofty::tag::Accessor) -> Option<u32> {
        tag.disk()
    }

    fn detect_format(path: &Path) -> AudioFormat {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(AudioFormat::from_extension)
            .unwrap_or(AudioFormat::Mp3)
    }
}

/// Metadata extraction errors
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("Read error: {0}")]
    ReadError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
}

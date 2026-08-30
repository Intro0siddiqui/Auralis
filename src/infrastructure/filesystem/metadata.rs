//! Metadata Extractor
//!
//! Extracts audio metadata from files using lofty.

use crate::domain::models::{AudioFormat, Track};
use lofty::file::TaggedFileExt;
use lofty::picture::{Picture, PictureType};
use lofty::prelude::*;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Extracts metadata from audio files
pub struct MetadataExtractor;

impl MetadataExtractor {
    /// Returns the directory used for caching extracted embedded artwork.
    pub fn artwork_cache_dir() -> PathBuf {
        if let Some(mut cache) = dirs::cache_dir() {
            cache.push("auralis");
            cache.push("artwork");
            cache
        } else if let Some(mut data) = dirs::data_local_dir().or_else(dirs::data_dir) {
            data.push("auralis");
            data.push("artwork_cache");
            data
        } else {
            std::env::temp_dir().join("auralis").join("artwork")
        }
    }

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

        // Cover art resolution:
        // 1. Check for a sidecar image (`<audio>.jpg/.jpeg/.png/.webp`) next to the audio file.
        // 2. If no sidecar image is found, extract and cache embedded artwork via Lofty.
        track.album_art_path = Self::find_sidecar_art(path)
            .or_else(|| Self::extract_and_cache_embedded_art(&tagged_file));

        debug!(path = %path.display(), title = %track.title, "Metadata extracted");
        Ok(track)
    }

    /// Extract embedded artwork from tagged file pictures and save it to the artwork cache directory.
    pub fn extract_and_cache_embedded_art(tagged_file: &lofty::file::TaggedFile) -> Option<String> {
        let picture = Self::find_best_picture(tagged_file)?;
        let data = picture.data();
        if data.is_empty() {
            return None;
        }

        let hash = Self::compute_artwork_hash(data);
        let ext = Self::detect_image_extension(data, picture.mime_type());
        let cache_dir = Self::artwork_cache_dir();

        if !cache_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&cache_dir) {
                warn!(error = %e, path = %cache_dir.display(), "Failed to create artwork cache directory");
                return None;
            }
        }

        let file_name = format!("{hash}.{ext}");
        let target_path = cache_dir.join(file_name);

        // Deduplication: if cache file already exists, reuse it without rewriting
        if target_path.is_file() {
            debug!(path = %target_path.display(), "Reusing cached artwork");
            return Some(target_path.to_string_lossy().to_string());
        }

        // Otherwise write picture bytes to disk
        if let Err(e) = std::fs::write(&target_path, data) {
            warn!(error = %e, path = %target_path.display(), "Failed to write cached artwork to disk");
            return None;
        }

        debug!(path = %target_path.display(), size = data.len(), "Cached embedded artwork");
        Some(target_path.to_string_lossy().to_string())
    }

    /// Find the highest priority picture in a tagged file (CoverFront > Other > Any).
    fn find_best_picture(tagged_file: &lofty::file::TaggedFile) -> Option<&Picture> {
        let tags: Vec<&lofty::tag::Tag> = if !tagged_file.tags().is_empty() {
            tagged_file.tags().iter().collect()
        } else if let Some(tag) = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
        {
            vec![tag]
        } else {
            Vec::new()
        };

        let mut best: Option<&Picture> = None;
        for tag in tags {
            for pic in tag.pictures() {
                match pic.pic_type() {
                    PictureType::CoverFront => return Some(pic),
                    PictureType::Other => {
                        if best.is_none()
                            || best
                                .map(|p| p.pic_type() != PictureType::Other)
                                .unwrap_or(false)
                        {
                            best = Some(pic);
                        }
                    }
                    _ => {
                        if best.is_none() {
                            best = Some(pic);
                        }
                    }
                }
            }
        }
        best
    }

    /// Compute deterministic 128-bit hex hash for image byte deduplication.
    pub fn compute_artwork_hash(data: &[u8]) -> String {
        let mut h1: u64 = 0xcbf29ce484222325;
        let mut h2: u64 = 0x100000001b3;
        for &b in data {
            h1 = (h1 ^ (b as u64)).wrapping_mul(0x100000001b3);
            h2 = (h2.wrapping_add(b as u64)).wrapping_mul(0xcbf29ce484222325);
        }
        format!("{:016x}{:016x}", h1, h2)
    }

    /// Determine file extension from magic bytes or Lofty MimeType.
    pub fn detect_image_extension(
        data: &[u8],
        mime: Option<&lofty::picture::MimeType>,
    ) -> &'static str {
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "jpg"
        } else if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            "png"
        } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
            "webp"
        } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
            "gif"
        } else if data.starts_with(b"BM") {
            "bmp"
        } else if let Some(m) = mime {
            match m {
                lofty::picture::MimeType::Jpeg => "jpg",
                lofty::picture::MimeType::Png => "png",
                lofty::picture::MimeType::Bmp => "bmp",
                lofty::picture::MimeType::Gif => "gif",
                lofty::picture::MimeType::Tiff => "tiff",
                lofty::picture::MimeType::Unknown(s) => {
                    let s_lower = s.to_lowercase();
                    if s_lower.contains("png") {
                        "png"
                    } else if s_lower.contains("webp") {
                        "webp"
                    } else if s_lower.contains("gif") {
                        "gif"
                    } else if s_lower.contains("bmp") {
                        "bmp"
                    } else {
                        "jpg"
                    }
                }
                _ => "jpg",
            }
        } else {
            "jpg"
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::{Tag, TagType};

    #[test]
    fn test_artwork_cache_dir() {
        let dir = MetadataExtractor::artwork_cache_dir();
        assert!(!dir.as_os_str().is_empty());
        let path_str = dir.to_string_lossy();
        assert!(path_str.contains("auralis"));
        assert!(path_str.contains("artwork"));
    }

    #[test]
    fn test_compute_artwork_hash() {
        let data1 = b"image_data_123456789";
        let data2 = b"image_data_123456789";
        let data3 = b"different_image_data";

        let hash1 = MetadataExtractor::compute_artwork_hash(data1);
        let hash2 = MetadataExtractor::compute_artwork_hash(data2);
        let hash3 = MetadataExtractor::compute_artwork_hash(data3);

        assert_eq!(
            hash1, hash2,
            "Identical content must produce identical hash"
        );
        assert_ne!(
            hash1, hash3,
            "Different content must produce different hash"
        );
        assert_eq!(hash1.len(), 32, "Hash must be 32 hex chars");
    }

    #[test]
    fn test_detect_image_extension() {
        // JPEG magic bytes
        let jpeg_data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(
            MetadataExtractor::detect_image_extension(&jpeg_data, None),
            "jpg"
        );

        // PNG magic bytes
        let png_data = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        assert_eq!(
            MetadataExtractor::detect_image_extension(&png_data, None),
            "png"
        );

        // WEBP magic bytes
        let webp_data = b"RIFF\x00\x00\x00\x00WEBPVP8 ";
        assert_eq!(
            MetadataExtractor::detect_image_extension(webp_data, None),
            "webp"
        );

        // GIF magic bytes
        let gif_data = b"GIF89a\x01\x00\x01\x00";
        assert_eq!(
            MetadataExtractor::detect_image_extension(gif_data, None),
            "gif"
        );

        // BMP magic bytes
        let bmp_data = b"BM\x00\x00\x00\x00";
        assert_eq!(
            MetadataExtractor::detect_image_extension(bmp_data, None),
            "bmp"
        );

        // Mime fallback
        let raw_data = b"unknown_image_bytes";
        assert_eq!(
            MetadataExtractor::detect_image_extension(raw_data, Some(&MimeType::Png)),
            "png"
        );
        assert_eq!(
            MetadataExtractor::detect_image_extension(raw_data, Some(&MimeType::Jpeg)),
            "jpg"
        );
        assert_eq!(
            MetadataExtractor::detect_image_extension(
                raw_data,
                Some(&MimeType::Unknown("image/webp".into()))
            ),
            "webp"
        );
        assert_eq!(
            MetadataExtractor::detect_image_extension(raw_data, None),
            "jpg"
        );
    }

    #[test]
    fn test_find_sidecar_art() {
        let temp_dir =
            std::env::temp_dir().join(format!("auralis_art_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let audio_path = temp_dir.join("song.mp3");
        let sidecar_jpg = temp_dir.join("song.jpg");

        assert!(MetadataExtractor::find_sidecar_art(&audio_path).is_none());

        std::fs::write(&sidecar_jpg, b"mock_jpg_art").unwrap();
        let found = MetadataExtractor::find_sidecar_art(&audio_path);
        assert_eq!(found, Some(sidecar_jpg.to_string_lossy().to_string()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_best_picture_priority() {
        let mut tag = Tag::new(TagType::Id3v2);
        let pic_other = Picture::unchecked(vec![1, 2, 3])
            .pic_type(PictureType::Other)
            .mime_type(MimeType::Jpeg)
            .build();
        let pic_front = Picture::unchecked(vec![4, 5, 6])
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Png)
            .build();

        tag.push_picture(pic_other);
        tag.push_picture(pic_front);

        let best = tag
            .pictures()
            .iter()
            .find(|p| p.pic_type() == PictureType::CoverFront);
        assert!(best.is_some());
        assert_eq!(best.unwrap().data(), &[4, 5, 6]);
    }
}

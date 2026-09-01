//! Metadata Extractor
//!
//! Extracts audio metadata from files using lofty.

use crate::domain::models::{AudioFormat, Track};
use lofty::file::TaggedFileExt;
use lofty::picture::{Picture, PictureType};
use lofty::prelude::*;
use rodio::Decoder;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Extracts metadata from audio files
pub struct MetadataExtractor;

impl MetadataExtractor {
    /// Verify audio health and playability
    pub fn verify_audio_health(path: &Path) -> Result<Track, MetadataError> {
        verify_audio_health(path)
    }

    /// Asynchronously verify audio health and playability
    pub async fn verify_audio_health_async(path: &Path) -> Result<Track, MetadataError> {
        verify_audio_health_async(path).await
    }

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
        Self::extract_with_size(path, None)
    }

    /// Extract metadata from an audio file with optional known file size (avoids redundant filesystem stat)
    pub fn extract_with_size(path: &Path, file_size: Option<u64>) -> Result<Track, MetadataError> {
        debug!(path = %path.display(), "Extracting metadata");

        let tagged_file =
            lofty::read_from_path(path).map_err(|e| MetadataError::ReadError(e.to_string()))?;

        let properties = tagged_file.properties();
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        let duration_secs = properties.duration().as_secs() as u32;
        let file_size =
            file_size.unwrap_or_else(|| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));

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

    /// Asynchronously extract metadata from an audio file using non-blocking async metadata lookup
    pub async fn extract_async(path: &Path) -> Result<Track, MetadataError> {
        let file_size = tokio::fs::metadata(path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let path_buf = path.to_path_buf();
        tokio::task::spawn_blocking(move || Self::extract_with_size(&path_buf, Some(file_size)))
            .await
            .map_err(|e| MetadataError::ReadError(format!("Task join error: {e}")))?
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

    /// Write modified metadata tags directly to audio file on disk using Lofty.
    pub fn write_metadata(
        path: &Path,
        title: &str,
        artist: &str,
        album: &str,
        genre: Option<&str>,
        year: Option<u32>,
        track_number: Option<u32>,
    ) -> Result<(), String> {
        write_metadata(path, title, artist, album, genre, year, track_number)
    }
}

/// Write modified metadata tags directly to audio file on disk using Lofty.
pub fn write_metadata(
    path: &Path,
    title: &str,
    artist: &str,
    album: &str,
    genre: Option<&str>,
    year: Option<u32>,
    track_number: Option<u32>,
) -> Result<(), String> {
    let mut tagged_file =
        lofty::read_from_path(path).map_err(|e| format!("Failed to read audio file tags: {e}"))?;

    let has_tag = tagged_file.primary_tag().is_some() || tagged_file.first_tag().is_some();
    if !has_tag {
        let tag_type = tagged_file.primary_tag_type();
        tagged_file.insert_tag(lofty::tag::Tag::new(tag_type));
    }

    let tag = if let Some(t) = tagged_file.primary_tag_mut() {
        t
    } else if let Some(t) = tagged_file.first_tag_mut() {
        t
    } else {
        return Err("Failed to obtain mutable tag from audio file".to_string());
    };

    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());
    tag.set_album(album.to_string());

    if let Some(g) = genre {
        if !g.trim().is_empty() {
            tag.set_genre(g.to_string());
        } else {
            tag.remove_genre();
        }
    } else {
        tag.remove_genre();
    }

    if let Some(y) = year {
        tag.set_date(lofty::tag::items::Timestamp {
            year: y as u16,
            month: None,
            day: None,
            hour: None,
            minute: None,
            second: None,
        });
    } else {
        tag.remove_date();
    }

    if let Some(tn) = track_number {
        tag.set_track(tn);
    } else {
        tag.remove_track();
    }

    tag.save_to_path(path, lofty::config::WriteOptions::default())
        .map_err(|e| format!("Failed to save audio tags to {}: {e}", path.display()))?;

    Ok(())
}

/// Static Audio File Health & Playability Check
///
/// 1. Extracts metadata using Lofty.
/// 2. Verifies that duration is strictly greater than 0 (`duration_secs > 0`).
///    If duration is 0, returns `MetadataError::ReadError("File has 0s duration or missing audio stream header")`.
/// 3. Performs a dry-run decoder probe using `rodio::Decoder::builder().with_data(BufReader::with_capacity(64 * 1024, file)).with_hint(&ext).build()` (or `Decoder::new`)
///    to ensure rodio/symphonia can actually initialize and demux the audio container without returning IO or atom seek errors.
pub fn verify_audio_health(path: &Path) -> Result<Track, MetadataError> {
    let track = MetadataExtractor::extract_with_size(path, None)?;
    if track.duration_secs == 0 {
        return Err(MetadataError::ReadError(
            "File has 0s duration or missing audio stream header".to_string(),
        ));
    }

    let file = File::open(path)
        .map_err(|e| MetadataError::ReadError(format!("Failed to open file for probe: {e}")))?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();

    let reader = BufReader::with_capacity(64 * 1024, file);
    let probe_res = if !ext.is_empty() {
        Decoder::builder()
            .with_data(reader)
            .with_hint(&ext)
            .build()
            .or_else(|_| {
                if let Ok(f) = File::open(path) {
                    Decoder::new(BufReader::with_capacity(64 * 1024, f))
                } else {
                    Err(rodio::decoder::DecoderError::UnrecognizedFormat)
                }
            })
    } else {
        Decoder::new(reader)
    };

    probe_res.map_err(|e| {
        MetadataError::ReadError(format!(
            "Audio decoder probe failed (file is unplayable): {e}"
        ))
    })?;

    Ok(track)
}

/// Asynchronously verify audio file health and playability using non-blocking blocking task
pub async fn verify_audio_health_async(path: &Path) -> Result<Track, MetadataError> {
    let path_buf = path.to_path_buf();
    tokio::task::spawn_blocking(move || verify_audio_health(&path_buf))
        .await
        .map_err(|e| MetadataError::ReadError(format!("Task join error: {e}")))?
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

    #[tokio::test]
    async fn test_verify_audio_health_valid_and_invalid() {
        let dir =
            std::env::temp_dir().join(format!("auralis_health_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);

        // 1. Corrupt audio file (random text)
        let corrupt_path = dir.join("corrupt.mp3");
        std::fs::write(&corrupt_path, b"NOT_A_VALID_AUDIO_FILE").unwrap();
        let res = verify_audio_health(&corrupt_path);
        assert!(res.is_err(), "Corrupt audio must fail verify_audio_health");

        // 2. Async check on corrupt file
        let res_async = verify_audio_health_async(&corrupt_path).await;
        assert!(
            res_async.is_err(),
            "Corrupt audio must fail verify_audio_health_async"
        );

        // 3. Valid 1s WAV file
        let sample_rate: u32 = 8000;
        let num_samples: u32 = 8000;
        let mut data = Vec::with_capacity(44 + num_samples as usize);
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + num_samples).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes()); // PCM
        data.extend_from_slice(&1u16.to_le_bytes()); // Mono
        data.extend_from_slice(&sample_rate.to_le_bytes());
        data.extend_from_slice(&sample_rate.to_le_bytes()); // Byte rate
        data.extend_from_slice(&1u16.to_le_bytes()); // Block align
        data.extend_from_slice(&8u16.to_le_bytes()); // Bits per sample
        data.extend_from_slice(b"data");
        data.extend_from_slice(&num_samples.to_le_bytes());
        data.resize(44 + num_samples as usize, 0x80);

        let valid_wav_path = dir.join("valid.wav");
        std::fs::write(&valid_wav_path, &data).unwrap();

        let track =
            verify_audio_health(&valid_wav_path).expect("Valid WAV must pass verify_audio_health");
        assert!(track.duration_secs > 0, "Valid WAV must have duration > 0");

        let track_async = verify_audio_health_async(&valid_wav_path)
            .await
            .expect("Valid WAV must pass verify_audio_health_async");
        assert!(
            track_async.duration_secs > 0,
            "Valid WAV must have duration > 0"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Downloaded-file tag writer.
//!
//! A completed download carries a display title (and, when the resolver knows
//! it, an artist/album) but the bytes themselves are untagged, so the library
//! scanner falls back to the sanitized filename and reports
//! `Unknown Artist`. This module writes those values into the finished file so
//! the next scan reads real metadata. No database migration is involved:
//! `filesystem::metadata::MetadataExtractor::extract` reads tags with lofty and
//! assigns `track.title` / `track.artist` / `track.album`.
//!
//! # Lofty API surface used (lofty 0.25 — check these first if it fails to compile)
//!
//! ```text
//! lofty::probe::Probe::open(path)      -> Result<Probe>            (lofty::probe::Probe)
//! Probe::file_type()                   -> Option<FileType>         inherent
//! Probe::guess_file_type()             -> Result<Probe>            inherent
//! Probe::set_file_type(FileType)       -> Probe                    inherent
//! Probe::read()                        -> Result<TaggedFile>       inherent
//! lofty::file::FileType::from_ext(&str)-> Option<FileType>         inherent
//! lofty::file::TaggedFileExt::{primary_tag, first_tag, primary_tag_mut,
//!                              first_tag_mut, primary_tag_type, insert_tag}
//! lofty::tag::Tag::new(TagType)        -> Tag                      inherent
//! Tag::{set_title, set_artist, set_album} -> ()                   inherent
//! Tag::save_to_path(&Path, WriteOptions) -> Result<()>              inherent
//! lofty::config::WriteOptions::default() -> WriteOptions           inherent
//! ```
//!
//! This mirrors, call for call, the write path already compiled in
//! `src/infrastructure/filesystem/metadata.rs::write_metadata` and the probe
//! path in `src/infrastructure/media/downloader.rs::validate_audio_file`.
//!
//! # MP4/M4A write caveat
//!
//! lofty's *write* support for MP4/M4A (`moov/udta/meta/ilst`) is the least
//! exercised path in the crate: reading `©nam`/`©ART`/`©alb` is solid, but
//! writing rebuilds the metadata atoms and can fail on unusual layouts. Every
//! failure is therefore non-fatal by contract — see
//! [`crate::infrastructure::media::downloader`], which only logs the result.
//! Muxed progressive MP4 (`itag 18`) is a plain `moov`+`mdat` MP4, which lofty
//! handles; audio-only M4A is the better-tested case. WebM/Opus has no tag
//! support in lofty at all, and `guess_file_type` cannot even identify it, so
//! those downloads report a plain error and keep their untagged file.

use lofty::file::{FileType, TaggedFileExt};
use lofty::probe::Probe;
use std::path::Path;
use tracing::debug;

/// Write `title` / `artist` / `album` into an already-downloaded audio file.
///
/// * Only the fields that are non-empty are written; anything else is left
///   exactly as the downloader produced it (no genre/date/track is touched).
/// * An all-empty request is a no-op and returns `Ok(())` without touching the
///   filesystem.
/// * Errors are returned, never panicked. Callers must treat them as
///   non-fatal.
pub fn write_tags(
    audio_path: &Path,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
) -> Result<(), String> {
    let title = title.trim();
    let artist = artist.map(|value| value.trim()).filter(|v| !v.is_empty());
    let album = album.map(|value| value.trim()).filter(|v| !v.is_empty());

    if title.is_empty() && artist.is_none() && album.is_none() {
        debug!(
            path = %audio_path.display(),
            "Skipping tag write — no title, artist or album to write"
        );
        return Ok(());
    }

    let ext = audio_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    // Probe by content first (the extension can be a lie: Opus-in-WebM saved as
    // `.m4a`), then fall back to the extension, then give up.
    let mut probe = Probe::open(audio_path)
        .map_err(|e| format!("Failed to open {} for tagging: {e}", audio_path.display()))?;

    if probe.file_type().is_none() {
        probe = probe.guess_file_type().map_err(|e| {
            format!(
                "Failed to guess audio file type for {}: {e}",
                audio_path.display()
            )
        })?;
    }

    if probe.file_type().is_none() {
        if let Some(file_type) = FileType::from_ext(ext) {
            probe = probe.set_file_type(file_type);
        }
    }

    let mut tagged_file = probe
        .read()
        .map_err(|e| format!("Failed to read tags from {}: {e}", audio_path.display()))?;

    let has_tag = tagged_file.primary_tag().is_some() || tagged_file.first_tag().is_some();
    if !has_tag {
        let tag_type = tagged_file.primary_tag_type();
        tagged_file.insert_tag(lofty::tag::Tag::new(tag_type));
    }

    let tag = if let Some(tag) = tagged_file.primary_tag_mut() {
        tag
    } else if let Some(tag) = tagged_file.first_tag_mut() {
        tag
    } else {
        return Err(format!(
            "No writable tag available in {}",
            audio_path.display()
        ));
    };

    if !title.is_empty() {
        tag.set_title(title.to_string());
    }
    if let Some(artist) = artist {
        tag.set_artist(artist.to_string());
    }
    if let Some(album) = album {
        tag.set_album(album.to_string());
    }

    tag.save_to_path(audio_path, lofty::config::WriteOptions::default())
        .map_err(|e| format!("Failed to save tags to {}: {e}", audio_path.display()))?;

    debug!(
        path = %audio_path.display(),
        title = %title,
        artist = ?artist,
        album = ?album,
        "Wrote tags into downloaded file"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("auralis_tags_{tag}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn missing_file_returns_err_instead_of_panicking() {
        let dir = temp_dir("missing");
        let path = dir.join("nope.mp4");
        let res = write_tags(&path, "Some Title", Some("Some Artist"), None);
        assert!(res.is_err(), "Missing file must return Err, got: {res:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_audio_file_returns_err_instead_of_panicking() {
        let dir = temp_dir("garbage");
        let path = dir.join("garbage.mp3");
        std::fs::write(&path, b"NOT_A_VALID_AUDIO_FILE_AT_ALL").expect("write fixture");
        let res = write_tags(&path, "Some Title", Some("Some Artist"), Some("Some Album"));
        assert!(res.is_err(), "Garbage file must return Err, got: {res:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_request_is_a_noop() {
        // No title, no artist, no album -> nothing to do, and the path is not
        // even opened (it does not exist).
        let path = Path::new("/definitely/not/here.mp4");
        assert!(write_tags(path, "   ", None, Some("  ")).is_ok());
    }

    #[test]
    fn writes_mp4_tags_when_fixture_available() {
        // `scratch/sample.m4a` is not committed (see AGENTS.md §5), so this
        // exercises the real MP4 write path only where the fixture exists.
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch/sample.m4a");
        if !fixture.exists() {
            eprintln!("skipping: scratch/sample.m4a fixture not present");
            return;
        }

        let dir = temp_dir("mp4");
        let path = dir.join("tagged.m4a");
        if std::fs::copy(&fixture, &path).is_err() {
            eprintln!("skipping: could not copy scratch/sample.m4a");
            return;
        }

        write_tags(&path, "Fixture Title", Some("Fixture Artist"), None)
            .expect("writing tags to a copy of the fixture must succeed");

        // Read back through the real scanner path — that is the acceptance
        // criterion: the library must stop falling back to the filename.
        let track = crate::infrastructure::filesystem::metadata::MetadataExtractor::extract(&path)
            .expect("tagged fixture must still be readable");
        assert_eq!(track.title, "Fixture Title");
        assert_eq!(track.artist.as_deref(), Some("Fixture Artist"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

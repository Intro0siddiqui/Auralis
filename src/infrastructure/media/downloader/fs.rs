use std::path::{Path, PathBuf};
use tracing::debug;
#[cfg(any(target_os = "android", test))]
use tracing::warn;
use uuid::Uuid;

use super::DownloaderError;

pub(crate) const ALLOWED_EXTS: &[&str] = &[
    "mp3", "m4a", "aac", "flac", "ogg", "opus", "wav", "webm", "mp4", "mov", "oga",
];

/// Extension of a staging file — which is also what makes it a reservation.
pub(crate) const STAGING_EXT: &str = "part";

/// Maximum length of a filename stem in bytes (255 NAME_MAX − 8 max ext − 1 dot).
pub(crate) const MAX_FILENAME_LEN: usize = 246;
#[allow(dead_code)]
pub(crate) const MAX_STEM_BYTES: usize = MAX_FILENAME_LEN;

/// How many output names one download may try before giving up.
///
/// Eight is not a tuning parameter: the fallback names are all distinct, so this
/// only bounds a pathological case (every candidate already on disk) that would
/// otherwise spin. Failing is the right answer there, because the alternative
/// is sharing a path with a job that already holds it.
pub(crate) const RESERVE_ATTEMPTS: usize = 8;

/// Remove a staging `.part` file, logging any error at debug level.
pub(crate) async fn cleanup_staging_file(path: &Path) {
    if let Err(e) = tokio::fs::remove_file(path).await {
        debug!(path = %path.display(), error = %e, "Failed to remove staging file during cleanup");
    } else {
        debug!(path = %path.display(), "Cleaned up staging file");
    }
}

/// Whitelist and sanitize an extension string. Returns a safe extension from
/// the allow-list; falls back to the trusted `fallback` (AudioFormat) or "mp3".
pub(crate) fn sanitize_ext(raw: &str, fallback: &str) -> String {
    let t = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    // must be purely alphanumeric and on allow-list — any slash, dot, or
    // control char causes fallback (prevents traversal like "../../etc")
    let is_clean = !t.is_empty() && t.len() <= 8 && t.chars().all(|c| c.is_ascii_alphanumeric());
    if is_clean && ALLOWED_EXTS.contains(&t.as_str()) {
        return t;
    }
    let fb = fallback.trim().trim_start_matches('.').to_ascii_lowercase();
    let fb_clean: String = fb.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if ALLOWED_EXTS.contains(&fb_clean.as_str()) {
        fb_clean
    } else {
        "mp3".to_string()
    }
}

/// Replace filesystem-unsafe characters so titles produce valid filenames.
/// Strips path separators, control chars, "..", reserved Windows names, and
/// limits length to fit within the filesystem's 255-byte NAME_MAX. Never
/// returns empty or "." / "..".
pub(crate) fn sanitize_filename(name: &str) -> String {
    // Replace control chars and map path separators/unsafe chars to '_'
    let filtered: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned: String = filtered
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut trimmed = cleaned.trim().trim_matches('.').to_string();
    // Collapse any remaining ".." to avoid traversal
    while trimmed.contains("..") {
        trimmed = trimmed.replace("..", "_");
    }
    // Remove any lingering path separators (already mapped to _ but be safe)
    trimmed = trimmed.replace(['/', '\\'], "_");
    // Collapse consecutive underscores
    while trimmed.contains("__") {
        trimmed = trimmed.replace("__", "_");
    }
    trimmed = trimmed
        .trim_matches(|c| c == '.' || c == '_' || c == ' ')
        .to_string();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return "audio_track".to_string();
    }
    // Windows reserved device names
    let lower = trimmed.to_ascii_lowercase();
    const RESERVED: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if RESERVED.contains(&lower.as_str()) {
        return format!("{}_{}", trimmed, "track");
    }
    // Limit stem to 246 bytes (255 NAME_MAX − 8 max ext − 1 dot).
    // Char-based limit allowed CJK titles (~600 bytes) to exceed the OS limit.
    if trimmed.len() > MAX_FILENAME_LEN {
        let mut end = MAX_FILENAME_LEN;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        trimmed = trimmed[..end].to_string();
        trimmed = trimmed.trim_end_matches(['.', '_', ' ']).to_string();
        if trimmed.is_empty() {
            return "audio_track".to_string();
        }
    }
    trimmed
}

/// One path, one owner: the sidecar name is derived from the output name, which
/// is what the reservation makes exclusive, so a job that owns `<name>.mp4`
/// owns `<name>.jpg` by the same token.
pub(crate) fn sidecar_path(audio_path: &Path) -> PathBuf {
    audio_path.with_extension("jpg")
}

/// One job's exclusive claim on an output name.
///
/// The claim **is** the staging file. Nothing else is created, nothing else is
/// registered, and there is no release to forget: the marker disappears when
/// the staging file is renamed to its final name at commit time, or when a
/// failure path removes it — which is exactly the window the claim is meant to
/// cover.
pub(crate) struct OutputReservation {
    /// Where the verified bytes will live.
    pub(crate) output_path: PathBuf,
    /// The claim itself. Present from reservation until the rename or cleanup.
    pub(crate) staging_path: PathBuf,
    /// The name the *public* copy should carry.
    ///
    /// Always the clean `<title>.<ext>`, even when the internal path needed a
    /// dedup suffix. The suffix exists only to keep two internal files apart;
    /// the owner looking at `Download/Auralis/` has not downloaded anything
    /// twice, so showing them a UUID is a lie about their own library.
    pub(crate) public_name: String,
}

/// Filenames to try, in order, for one download's output.
///
/// The first is the clean `<title>.<ext>`; the rest carry a short job-id suffix.
/// The suffix is what the fallback *looks* like — exclusivity comes from
/// [`reserve_output`] — but it is derived from the job id, so two concurrent
/// jobs of the same title never even propose the same fallback name.
pub(crate) fn candidate_file_names(stem: &str, ext: &str, id: Uuid) -> Vec<String> {
    let short: String = id.to_string().chars().take(8).collect();
    (0..RESERVE_ATTEMPTS)
        .map(|attempt| match attempt {
            0 => format!("{stem}.{ext}"),
            1 => format!("{stem}_{short}.{ext}"),
            n => format!("{stem}_{short}_{n}.{ext}"),
        })
        .collect()
}

/// Take an exclusive claim on an output name for `id`.
///
/// # Why an exclusive create and not a `stat`
///
/// The name used to be chosen with `if path.exists() { add a uuid }`, and there
/// was an `await` between that test and the first create. Two jobs with the
/// same title both observed "free" and were handed the *same* `output_path`;
/// one of them then deleted the other's finished file on the way out, and two
/// that both succeeded overwrote each other. `create_new` is the only
/// filesystem operation that answers "is this name free?" and *takes* it in a
/// single step, so the answer and the claim cannot be separated by a
/// scheduling point.
///
/// The claim is deliberately a staging file and not a zero-byte file at the
/// destination. Occupying the destination would put an empty
/// `Song.mp4` in the library folder for the length of the download, and — worse
/// — `rename` fails on Windows when the target exists, so every Windows
/// download would be pushed onto the non-atomic copy fallback.
///
/// A destination that already exists is *rejected*, never claimed: a job must
/// not adopt or overwrite a file from an earlier run.
pub(crate) async fn reserve_output(
    output_dir: &Path,
    staging_dir: &Path,
    stem: &str,
    ext: &str,
    id: Uuid,
) -> Result<OutputReservation, DownloaderError> {
    for file_name in candidate_file_names(stem, ext, id) {
        let output_path = output_dir.join(&file_name);
        if output_path.exists() {
            debug!(
                download_id = %id,
                candidate = %file_name,
                "Output name already used by an earlier download"
            );
            continue;
        }
        let staging_path = staging_dir.join(format!("{file_name}.{STAGING_EXT}"));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging_path)
            .await
        {
            Ok(claim) => {
                // Closed immediately: the file's *existence* is the claim, and
                // an open handle would only stop the commit from renaming over
                // it on Windows.
                drop(claim);
                // A cover-art sidecar whose audio is gone is a leftover from an
                // earlier run of the same title. The claim is exclusive, so
                // nothing else can be using it, and inheriting it would attach
                // the wrong artwork to this download — or, when this download
                // has no thumbnail of its own, leave the stale one sitting next
                // to the new file for good.
                let stale_cover = sidecar_path(&output_path);
                if let Err(e) = tokio::fs::remove_file(&stale_cover).await {
                    debug!(
                        path = %stale_cover.display(),
                        error = %e,
                        "No stale cover-art sidecar to clear"
                    );
                }
                return Ok(OutputReservation {
                    output_path,
                    staging_path,
                    public_name: format!("{stem}.{ext}"),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                debug!(
                    download_id = %id,
                    candidate = %file_name,
                    "Output name already claimed by a live download"
                );
                continue;
            }
            Err(e) => {
                return Err(DownloaderError::IoError(e));
            }
        }
    }
    // Built here rather than hoisted above the loop: hoisting it and moving it
    // into the successful arm would be a move out of a loop body, which the
    // borrow checker rejects outright.
    Err(DownloaderError::InvalidState(format!(
        "could not claim an unused filename for {stem}.{ext} in {}",
        output_dir.display()
    )))
}

/// Delete the bytes this job owns — and only those.
///
/// The output path is removed only while the claim is still held, i.e. only
/// while the staging file this job created is still on disk. Because the claim
/// is what makes the name exclusive, a file sitting at the output path at that
/// moment can only be this job's own partial work. Once the claim is gone the
/// file is *committed*, and removing it would delete a finished track.
///
/// The claim is read before the staging file is touched, because removing the
/// staging file is what releases it.
pub(crate) async fn discard_owned_paths(staging: &Path, output: &Path) {
    let holds_claim = tokio::fs::metadata(staging)
        .await
        .map(|meta| meta.is_file())
        .unwrap_or(false);
    cleanup_staging_file(staging).await;
    if !holds_claim {
        debug!(
            output = %output.display(),
            "Keeping the output file — this job's claim is already released, so it is committed"
        );
        return;
    }
    if let Err(e) = tokio::fs::remove_file(output).await {
        debug!(path = %output.display(), error = %e, "No partial output file to remove");
    }
    let cover = sidecar_path(output);
    if let Err(e) = tokio::fs::remove_file(&cover).await {
        debug!(path = %cover.display(), error = %e, "No cover-art sidecar to remove");
    }
}

/// Empty the staging directory at construction.
///
/// A staging file *is* a claim (see [`reserve_output`]), so a crash part way
/// through a download would otherwise hold that name until the user manually
/// cleared the directory. The same directory holds the transient public-name
/// link, which a crash can strand too. Both are per-process scratch, and both
/// are gone by the time anything can be mid-download.
///
/// Wholesale, and only here: the `Downloader` is constructed once per process,
/// before any job exists, so nothing reachable from this function can be live.
/// A job killed mid-run — never swept, only released early — leaks its claim
/// until the next start, which is the safe direction to be wrong in.
pub(crate) fn sweep_stale_staging(staging_dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(staging_dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            debug!(path = %staging_dir.display(), error = %e, "Could not clear the staging directory");
        }
        return;
    }
    if let Err(e) = std::fs::create_dir_all(staging_dir) {
        debug!(path = %staging_dir.display(), error = %e, "Could not recreate the staging directory");
    }
}

/// Present `internal` under `public_name` without copying a single byte of it.
///
/// `publish_to_downloads` builds the MediaStore display name from the file name
/// of the path it is handed, so the only lever this side owns is the path. A
/// hard link says "this file, under that name" with no resolution involved
/// anywhere and no second copy of the bytes; the caller removes the link as
/// soon as the copy exists.
///
/// The link goes in `link_dir` (the staging directory), **not** beside the
/// finished file. Beside it is exactly where the clean name is *already taken* —
/// that is the only reason the internal name needed a suffix in the first place —
/// so a link placed there would fail with `EEXIST` in precisely the case it
/// exists for. The staging directory is this job's own scratch space, and
/// nothing scans it.
///
/// Returns `None` when the internal name already is the public name (nothing to
/// do) and when no link could be made. The second case is a lost nicety, not a
/// failure: the caller publishes the internal path and the public copy keeps the
/// suffix, which is what it did before.
///
/// Two live jobs of the same title both want the same link name. Only one gets
/// it; the other publishes from its internal path. That is a race on a
/// presentation detail with no correctness consequence — no shared bytes, no
/// deletion, no lost file — so it is left unarbitrated rather than paid for with
/// a second round of locking.
///
/// The link is briefly visible to the library scanner, which is why the caller's
/// post-commit flag keeps `download:completed` (and the scan it triggers) behind
/// it.
#[cfg(any(target_os = "android", test))]
pub(crate) fn link_under_public_name(
    internal: &Path,
    link_dir: &Path,
    public_name: &str,
) -> Option<PathBuf> {
    let existing = internal
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    if existing == public_name {
        return None;
    }
    let link = link_dir.join(public_name);
    match std::fs::hard_link(internal, &link) {
        Ok(()) => Some(link),
        Err(e) => {
            // Kept short on purpose: rustfmt cannot break a string literal, and a
            // `warn!` line it cannot fit is known to drop the statements around
            // it out of the `cargo fmt --check` gate.
            warn!(
                internal = %internal.display(),
                public_name = %public_name,
                error = %e,
                "No link under the clean name — the public copy will keep the suffix"
            );
            None
        }
    }
}

use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

use lofty::file::{AudioFile, FileType};
use lofty::probe::Probe;

use crate::domain::models::AudioFormat;
use crate::infrastructure::media::forensics::{ContainerFacts, ContentFacts, Verdict};

/// A decoded length or a container sample table that covers less than this
/// fraction of the expected track counts as short. Deliberately the same
/// threshold `completeness.rs` uses, so the decoder's gate and the
/// container/content cross-check cannot disagree about what "short" means. The
/// *audible* check below deliberately does not use it — see
/// [`MIN_AUDIBLE_RATIO`].
pub(crate) const MIN_COVERAGE_RATIO: f64 = 0.9;

/// The audible fraction below which an otherwise complete file is still refused.
///
/// `audible_secs` is the position of the last sample above the silence
/// threshold, not a length. It therefore cannot be *required*: a track whose
/// last 10 % is a legitimate fade-out is complete, and demanding audible audio
/// for 90 % of it sent real downloads through four pointless range-top-up
/// rounds. It can still **veto**, because a server-side window keeps serving
/// container and silence long after it stops serving audio — the one shape where
/// the byte count is complete, the sample table describes the full track, and
/// the file is still not the track.
///
/// Both device measurements of that window sit at roughly a third of the track
/// (99 s of 287 s; 75 s of 216 s), so half clears them with room to spare while
/// leaving any plausible silent tail alone. This is an **inference** from those
/// two samples, not a measured constant: the length check above is the
/// load-bearing one, and this only has to tell a fade-out from a window.
pub(crate) const MIN_AUDIBLE_RATIO: f64 = 0.5;

// The reject reasons are `const`s rather than literals inline in `acceptance`
// for two reasons. They are long enough that an unbreakable literal would push
// its line past `max_width`, and rustfmt gives up on an item it cannot fit —
// which would silently take the whole gate out of the `cargo fmt --check` gate.
// And a test can assert against the very same value production returns, so a
// reworded message cannot quietly stop being the one the test checks.
pub(crate) const REASON_MISSING_BYTES: &str =
    "the sample table references bytes the file does not contain";
pub(crate) const REASON_SHORT_TABLE: &str =
    "the container itself describes a shorter track than the \
                                   resolver expected, so the server sent a windowed object \
                                   and reported it as complete";
pub(crate) const REASON_SILENT_TAIL: &str =
    "the bytes and the decoded length are complete but audio \
                                 stops well before the end, which is the signature of a \
                                 server-side window rather than of a whole file";
pub(crate) const REASON_UNPARSABLE: &str =
    "the container could not be parsed and the decoder agrees \
                                  it is short";

/// Try the Opus/WebM fallback (Symphonia probe with EBML sniffing) for files
/// lofty/rodio cannot handle — e.g. Opus-in-WebM mislabeled as `.m4a`
/// (`https://d.uguu.se/jXSTGTDj.m4a`: EBML `1A 45 DF A3`, `google/video-file`,
/// lofty guesses `Mpeg` and fails, rodio reports "format not recognized").
/// Returns `Some(duration)` when the Symphonia probe yields a usable duration
/// within the ±5s expected-duration tolerance.
pub(crate) fn try_opus_fallback(path: &Path, expected_duration_secs: Option<u32>) -> Option<u32> {
    let meta = crate::infrastructure::media::opus::extract_opus_metadata(path).ok()?;
    if meta.duration_secs == 0 {
        return None;
    }
    if let Some(expected) = expected_duration_secs {
        if expected > 0 && (meta.duration_secs as i64 - expected as i64).abs() > 5 {
            return None;
        }
    }
    Some(meta.duration_secs)
}

/// Check whether a file starts with the EBML header (`1A 45 DF A3`)
/// identifying a WebM/Matroska container (usually Opus audio from YouTube).
pub(crate) fn is_ebml_container(path: &Path) -> bool {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut header = [0u8; 4];
    file.read_exact(&mut header).is_ok() && header == [0x1a, 0x45, 0xdf, 0xa3]
}

/// Validate downloaded audio file integrity using lofty.
/// Checks that the file is non-empty, contains valid audio headers/properties,
/// and that decoded duration matches expected duration within ±5s tolerance (if expected is known).
pub fn validate_audio_file(
    path: &Path,
    expected_duration_secs: Option<u32>,
    ext: &str,
    format: AudioFormat,
) -> Result<u32, String> {
    if !path.exists() {
        return Err(format!("Staging file does not exist: {}", path.display()));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read metadata for {}: {e}", path.display()))?;
    let file_size = metadata.len();
    if file_size == 0 {
        return Err("Staging file is empty (0 bytes)".to_string());
    }

    // Fast path: EBML/WebM container (Opus audio mislabeled as .m4a/.mp3, …).
    // lofty misdetects these bytes as `Mpeg` and rodio cannot decode Opus at
    // all, so consult the Symphonia Opus probe first — mirroring
    // `player.rs::create_decoder` + `metadata.rs::extract_with_size`.
    if is_ebml_container(path) {
        if let Some(dur) = try_opus_fallback(path, expected_duration_secs) {
            return Ok(dur);
        }
    }

    let mut probe =
        Probe::open(path).map_err(|e| format!("Failed to open file for lofty probe: {e}"))?;

    if probe.file_type().is_none() {
        probe = probe
            .guess_file_type()
            .map_err(|e| format!("Failed to guess audio file type: {e}"))?;
    }

    if probe.file_type().is_none() {
        if let Some(ft) = FileType::from_ext(ext) {
            probe = probe.set_file_type(ft);
        } else if let Some(ft) = FileType::from_ext(format.extension()) {
            probe = probe.set_file_type(ft);
        }
    }

    let tagged_file = match probe.read() {
        Ok(tf) => tf,
        Err(e) => {
            // lofty cannot parse WebM/Opus (no WebM FileType; EBML bytes guess
            // as Mpeg) — fall back to the Symphonia Opus probe before failing.
            if let Some(dur) = try_opus_fallback(path, expected_duration_secs) {
                return Ok(dur);
            }
            return Err(format!("Corrupt or unreadable audio headers: {e}"));
        }
    };

    let duration_secs = tagged_file.properties().duration().as_secs() as u32;
    if duration_secs == 0 {
        if expected_duration_secs.is_some() {
            return Err(
                "Decoded duration is 0s — file has unreadable atom index tables or is truncated"
                    .to_string(),
            );
        }
        // No expected duration — validate file size and audio properties
        let props = tagged_file.properties();
        let sample_rate = props.sample_rate().unwrap_or(0);
        let channels = props.channels().unwrap_or(0);
        if file_size <= 10_240 || sample_rate == 0 || channels == 0 {
            return Err(format!(
                "Decoded duration is 0s — file has unreadable atom index tables or is truncated (size={} bytes, sample_rate={}, channels={})",
                file_size, sample_rate, channels
            ));
        }
        return Err(
            "Decoded duration is 0s — file has unreadable atom index tables or is truncated"
                .to_string(),
        );
    }

    if let Some(expected) = expected_duration_secs {
        if expected > 0 {
            let diff = (duration_secs as i64 - expected as i64).abs();
            if diff > 5 {
                return Err(format!(
                    "Decoded duration ({}s) differs by > 5s from expected duration ({}s, diff={}s)",
                    duration_secs, expected, diff
                ));
            }
        }
    }

    // Dry-run decoder probe using rodio::Decoder with 64 KB BufReader
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open file for decoder probe: {e}"))?;
    let probe_res = if !ext.is_empty() {
        if let Ok(cloned_file) = file.try_clone() {
            let reader = BufReader::with_capacity(64 * 1024, cloned_file);
            match rodio::Decoder::builder()
                .with_data(reader)
                .with_hint(ext)
                .build()
            {
                Ok(decoder) => Ok(decoder),
                Err(_) => {
                    let mut f = file;
                    let _ = f.seek(SeekFrom::Start(0));
                    rodio::Decoder::new(BufReader::with_capacity(64 * 1024, f))
                }
            }
        } else {
            let mut f = file;
            let _ = f.seek(SeekFrom::Start(0));
            rodio::Decoder::new(BufReader::with_capacity(64 * 1024, f))
        }
    } else {
        let mut f = file;
        let _ = f.seek(SeekFrom::Start(0));
        rodio::Decoder::new(BufReader::with_capacity(64 * 1024, f))
    };

    if let Err(e) = probe_res {
        // rodio has no Opus decoder (WebM/Opus always fails here) — accept the
        // file when the native OpusSource probe decodes its metadata.
        if let Some(dur) = try_opus_fallback(path, expected_duration_secs) {
            return Ok(dur);
        }
        return Err(format!("Decoder probe failed for {}: {e}", path.display()));
    }

    Ok(duration_secs)
}

/// Asynchronously validate downloaded audio file integrity offloaded to tokio::task::spawn_blocking.
///
/// Prevents synchronous file I/O, lofty probe, and rodio decoder probing from blocking
/// Tokio executor worker threads.
pub async fn validate_audio_file_async(
    path: &Path,
    expected_duration_secs: Option<u32>,
    ext: &str,
    format: AudioFormat,
) -> Result<u32, String> {
    let path_buf = path.to_path_buf();
    let ext_owned = ext.to_string();
    tokio::task::spawn_blocking(move || {
        validate_audio_file(&path_buf, expected_duration_secs, &ext_owned, format)
    })
    .await
    .map_err(|e| format!("Task join error for validate_audio_file: {e}"))?
}

// ---------------------------------------------------------------------------
// Completeness cross-check (container structure + decoded content)
// ---------------------------------------------------------------------------

/// Everything the completeness gate needs from one on-disk file.
///
/// The three inspections are bundled because they answer a single question —
/// *is this file whole?* — and because every one of them is a full read or a
/// full decode of the file.
pub(crate) struct ForensicReport {
    pub(crate) container: ContainerFacts,
    pub(crate) content: ContentFacts,
    /// `completeness::verify_decoded_duration`'s verdict.
    ///
    /// Kept only for the unparseable-container case, where no structural
    /// evidence exists and the decoder's own number is all that is left. It is
    /// the least trustworthy of the three, so it never gets the last word
    /// anywhere else.
    pub(crate) decoded: Result<Option<u64>, String>,
}

/// What the cross-check concluded about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Acceptance {
    /// Proven whole: keep it.
    Accept,
    /// Not proven whole. `reason` names the single signal that decided, so the
    /// failure shown to the user is attributable rather than vague.
    Reject(&'static str),
}

/// Does this measured length cover the expected track?
///
/// `expected_secs == 0.0` means no expectation was available, and a
/// measurement cannot prove shortness against nothing, so it does not veto.
///
/// What an **absent** measurement means is deliberately not decided here:
/// `forensics::ContentFacts::measured_secs` returns `None` exactly when no
/// decoder could be built, and its own documentation says a caller "must fall
/// back to its other evidence" — so the decision belongs to `acceptance`, which
/// has the other evidence in hand.
pub(crate) fn covers(value: f64, expected_secs: f64) -> bool {
    if expected_secs <= 0.0 {
        return true;
    }
    value >= expected_secs * MIN_COVERAGE_RATIO
}

/// Cross-check the container and the decoded content of one file.
///
/// The **order** is the whole point, and it is what NEW-04 turned over:
///
/// 1. Bytes, from the sample table. Exact, needs no decoding, and
///    `Verdict::Truncated` is a hard no.
/// 2. Container coverage — does the table describe the full track?
/// 3. NOT the decoded length. A short decode is non-evidence, not failure:
///    measured on `yF9nmg_jHNs` (2026-09-29, dev box) — container `Complete`,
///    table 216.3 s, every byte present — the decoder walked 25.2 % of the
///    samples (`measured=54.4s`) and the gate refused a perfect file. The
///    transfer was always whole; the decoder stopped early. Byte accounting
///    (step 1) and table coverage (step 2) prove what a decode cannot
///    disprove, so a short `measured_secs` abstains here unconditionally.
/// 4. Audible position — consulted last, only as a **veto**, and only when
///    the decode actually walked the whole track (`measured_secs` covers).
///    Then a short audible position means observed trailing silence, the
///    signature of a served window. When measured is short too, audible is
///    just where the decode ended and adds no information, so the veto
///    abstains with it.
///
/// Step 4 has to stay in that gated form. A server-side window is the shape
/// where steps 1-2 pass and the decoded stream is the right length — because
/// what arrived after the cutoff is silence. Removing the veto would silently
/// delete the only check that catches it; applying it to a short decode
/// would refuse every file whose decoder stops early, which is the bug this
/// ordering exists to prevent.
pub(crate) fn acceptance(report: &ForensicReport, expected_secs: f64) -> Acceptance {
    match report.container.verdict.as_ref() {
        // The sample table references bytes the file does not have. No amount
        // of decoded audio makes that whole.
        Some(Verdict::Truncated { .. }) => return Acceptance::Reject(REASON_MISSING_BYTES),
        Some(Verdict::Complete) => {}
        // `Verdict::Unknown` and an absent verdict mean the same thing: the box
        // walker read nothing — a file over `forensics::MAX_INSPECT_BYTES`, or a
        // container it does not parse. `forensics` documents that as "keep your
        // previous behaviour", and the previous behaviour is the decoder's
        // verdict, so that is what decides here.
        _ => {
            return match &report.decoded {
                Ok(_) => Acceptance::Accept,
                Err(_) => Acceptance::Reject(REASON_UNPARSABLE),
            }
        }
    }

    // The table is `Some` whenever the verdict is — `inspect_container` only
    // reports one from a structure whose duration it read — so an absent table
    // here is not a measurement question, it is an absence of evidence, and an
    // absence of evidence does not pass a completeness gate.
    if !report
        .container
        .table_secs
        .is_some_and(|table| covers(table, expected_secs))
    {
        return Acceptance::Reject(REASON_SHORT_TABLE);
    }

    // A short decode is non-evidence (see the step-3 note above), so there
    // is no length rejection here at all: byte accounting and table coverage
    // already decided. What remains is the audible veto, gated on the decode
    // having walked the whole track — only then is a short audible position
    // observed silence rather than the place the decoder stopped.
    //
    // `measured_secs() == None` means the sample stream could not be walked
    // at all, and `forensics` is explicit that this "never means empty": the
    // veto abstains with it, and the container's proof stands.
    let decoded_covers = report
        .content
        .measured_secs()
        .is_some_and(|measured| covers(measured, expected_secs));
    if decoded_covers
        && report
            .content
            .audible_secs
            .is_some_and(|audible| audible < expected_secs * MIN_AUDIBLE_RATIO)
    {
        return Acceptance::Reject(REASON_SILENT_TAIL);
    }

    Acceptance::Accept
}

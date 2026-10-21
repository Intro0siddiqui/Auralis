use super::*;

/// The `ip=` binding tests are gone, and so is the belief they encoded.
///
/// They existed to protect §4.7.16's conclusion — that a googlevideo URL is
/// refused unless the request egresses from the address in `ip=`. That
/// conclusion is falsified. `examples/fetch_probe.rs`, run on the device
/// against a freshly resolved URL, gets HTTP 206 from the OS-chosen address,
/// from the `ip=` address, and from **a different local address on the same
/// device**. Row three is the one that matters and there is no row that
/// contradicts it.
///
/// The 403 that produced the belief was **signature validation**: `ip` is listed
/// in `sparams`, so editing it invalidates `sig`/`lsig`. @audit's research in
/// `issue.md` said so before the pin was written.
///
/// Kept as a note rather than deleted silently, because the inverse test is the
/// one worth having: if a future change reintroduces address pinning, this
/// comment is the reason not to, and `fetch_probe --bind` is how to re-check it.
///
/// The client must build, since every download depends on it and a builder
/// failure takes the whole path down rather than one transfer.
#[test]
fn stream_client_builds() {
    assert!(build_stream_client("auralis-test/1.0").is_ok());
}

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ---------------------------------------------------------------------
// parse_content_range_total
// ---------------------------------------------------------------------

#[test]
fn test_parse_content_range_total_valid() {
    assert_eq!(parse_content_range_total("bytes 0-99/1234"), Some(1234));
    assert_eq!(parse_content_range_total("bytes 100-199/5000"), Some(5000));
    assert_eq!(parse_content_range_total("bytes 0-0/1"), Some(1));
    assert_eq!(
        parse_content_range_total("bytes 500-600/18446744073709551615"),
        Some(18446744073709551615)
    );
}

#[test]
fn test_parse_content_range_total_unsatisfied() {
    // Unsatisfied range: bytes */total
    assert_eq!(parse_content_range_total("bytes */1234"), Some(1234));
    assert_eq!(parse_content_range_total("bytes */0"), Some(0));
}

#[test]
fn test_parse_content_range_total_invalid() {
    // Star total
    assert_eq!(parse_content_range_total("bytes 0-99/*"), None);
    assert_eq!(parse_content_range_total("bytes */*"), None);

    // Empty string
    assert_eq!(parse_content_range_total(""), None);

    // No slash
    assert_eq!(parse_content_range_total("bytes 0-99"), None);

    // Non-numeric total
    assert_eq!(parse_content_range_total("bytes 0-99/abc"), None);

    // Garbage
    assert_eq!(parse_content_range_total("garbage"), None);
    assert_eq!(parse_content_range_total("/"), None);
    assert_eq!(parse_content_range_total("bytes /"), None);
}

// ---------------------------------------------------------------------
// Fixtures for the completeness cross-check
// ---------------------------------------------------------------------

/// A temp file that removes itself, so a failing assertion cannot leave a
/// stray directory behind for the next run to trip over.
struct TempFile {
    dir: PathBuf,
    path: PathBuf,
}

impl TempFile {
    fn new(tag: &str, bytes: &[u8]) -> TempFile {
        let dir = std::env::temp_dir().join(format!(
            "auralis_dl_{tag}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("probe.m4a");
        std::fs::write(&path, bytes).expect("write probe file");
        TempFile { dir, path }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A `ContentFacts` shaped the way `forensics::inspect_content` builds it:
/// the decoder's claim, plus the sample stream that was really iterated.
///
/// Deliberately built with the same arithmetic as production, including
/// deriving `audible_secs` from the audible sample count, so a test cannot
/// assert a relationship the real code would not hold.
fn content_facts(sample_rate: u32, total_samples: u64, audible_samples: u64) -> ContentFacts {
    ContentFacts {
        decoded_secs: Some(total_samples / u64::from(sample_rate.max(1))),
        audible_secs: Some(audible_samples as f64 / f64::from(sample_rate.max(1))),
        sample_rate,
        total_samples,
        audible_samples,
    }
}

/// An `MP4` whose single audio track declares a 4-minute sample table, and
/// `media_bytes` of the media data that table points at.
///
/// This is the only shape that can drive `Verdict::Complete` /
/// `Verdict::Truncated` from a test: `forensics::inspect_container` answers
/// `None` for anything it cannot walk as MP4, and the cross-check routes
/// that case to the decoder instead — which would leave the container half
/// of the gate untested.
fn synthetic_mp4(media_bytes: usize) -> Vec<u8> {
    fn box_of(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + 8);
        out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    const CHUNK_OFFSET: u32 = 1000;
    const SAMPLE_BYTES: u32 = 4096;
    const SAMPLES: u32 = 2;
    // 4 minutes of audio at a 48 kHz timescale, i.e. a table that only a
    // complete file can satisfy.
    const TABLE_UNITS: u32 = 48_000 * 240;

    // stts: one run of `SAMPLES` samples of `TABLE_UNITS / SAMPLES` each.
    let mut stts = vec![0u8, 0, 0, 0];
    stts.extend_from_slice(&1u32.to_be_bytes());
    stts.extend_from_slice(&SAMPLES.to_be_bytes());
    stts.extend_from_slice(&(TABLE_UNITS / SAMPLES).to_be_bytes());

    // stsc: from chunk 1, `SAMPLES` samples per chunk.
    let mut stsc = vec![0u8, 0, 0, 0];
    stsc.extend_from_slice(&1u32.to_be_bytes());
    stsc.extend_from_slice(&1u32.to_be_bytes());
    stsc.extend_from_slice(&SAMPLES.to_be_bytes());
    stsc.extend_from_slice(&1u32.to_be_bytes());

    // stsz: a uniform sample size, so no per-sample table is needed.
    let mut stsz = vec![0u8, 0, 0, 0];
    stsz.extend_from_slice(&SAMPLE_BYTES.to_be_bytes());
    stsz.extend_from_slice(&SAMPLES.to_be_bytes());

    // stco: one chunk, at CHUNK_OFFSET.
    let mut stco = vec![0u8, 0, 0, 0];
    stco.extend_from_slice(&1u32.to_be_bytes());
    stco.extend_from_slice(&CHUNK_OFFSET.to_be_bytes());

    let stbl = box_of(
        b"stbl",
        &[
            box_of(b"stts", &stts),
            box_of(b"stsc", &stsc),
            box_of(b"stsz", &stsz),
            box_of(b"stco", &stco),
        ]
        .concat(),
    );

    // mdhd: timescale 48000, duration TABLE_UNITS.
    let mut mdhd = vec![0u8, 0, 0, 0];
    mdhd.extend_from_slice(&0u32.to_be_bytes());
    mdhd.extend_from_slice(&0u32.to_be_bytes());
    mdhd.extend_from_slice(&48_000u32.to_be_bytes());
    mdhd.extend_from_slice(&TABLE_UNITS.to_be_bytes());
    mdhd.extend_from_slice(&0u16.to_be_bytes());

    let minf = box_of(b"minf", &stbl);
    let mdia = box_of(b"mdia", &[box_of(b"mdhd", &mdhd), minf].concat());

    // hdlr: handler_type "soun" marks this as the audio track.
    let mut hdlr = vec![0u8, 0, 0, 0];
    hdlr.extend_from_slice(&0u32.to_be_bytes());
    hdlr.extend_from_slice(b"soun");

    let trak = box_of(b"trak", &[box_of(b"hdlr", &hdlr), mdia].concat());
    let moov = box_of(b"moov", &trak);

    let mut file = moov;
    while file.len() < CHUNK_OFFSET as usize {
        file.push(0);
    }
    file.resize(CHUNK_OFFSET as usize + media_bytes, 0u8);
    file
}

/// A report describing a container that is complete and whose table claims
/// `table_secs`, plus the given decoded content.
fn complete_container_report(table_secs: f64, content: ContentFacts) -> ForensicReport {
    ForensicReport {
        container: ContainerFacts {
            container: "mp4-stbl",
            audio_track_found: true,
            table_secs: Some(table_secs),
            declared_secs: Some(table_secs),
            verdict: Some(Verdict::Complete),
            ..Default::default()
        },
        content,
        decoded: Ok(None),
    }
}

// ---------------------------------------------------------------------
// NEW-04 — the gate must compare length before audible position
// ---------------------------------------------------------------------

#[test]
fn a_complete_track_that_ends_in_silence_is_accepted() {
    // The defect NEW-04 fixes. A 240 s track whose last stretch is silence:
    // every advertised byte arrived, the sample table covers the track, and
    // the decoded stream is the full 240 s. The old rule read
    // `near_expected(content.audible_secs)` — *audible audio for 90 % of the
    // track* — so anything past a 24 s outro was "unproven", cost four
    // pointless range-top-up rounds, and was ultimately refused.
    //
    // The fixtures below all sit well inside what the old rule rejected, so
    // each one is a case the old gate demonstrably refused and the new one
    // demonstrably keeps.
    for audible in [144.0_f64, 132.0] {
        let report = complete_container_report(
            240.0,
            content_facts(48_000, 240 * 48_000, (audible * 48_000.0) as u64),
        );
        assert!(
            audible < 240.0 * 0.9,
            "fixture with {audible}s of audible audio must be one the old 90% \
                 audible rule refused, or it proves nothing"
        );
        assert_eq!(
            acceptance(&report, 240.0),
            Acceptance::Accept,
            "a legitimate silent tail is not a truncation: {}",
            report.content.summary()
        );
    }

    // The exact old threshold, pinned because it is the boundary the old
    // rule was written around and the new one no longer consults.
    let boundary =
        complete_container_report(240.0, content_facts(48_000, 240 * 48_000, 216 * 48_000));
    assert_eq!(acceptance(&boundary, 240.0), Acceptance::Accept);
}

#[test]
fn a_complete_container_overrules_a_short_decode() {
    // Deliberate inversion of the pre-§4.7.13 belief this test used to
    // pin (it asserted `Reject(REASON_SHORT_AUDIO)` on this exact shape).
    // The 2026-09-27 device download, `yF9nmg_jHNs`: all advertised bytes
    // received, container `table=216.3s`, `decoded=75s measured=54.4s`.
    // Measured on the dev box 2026-09-29, the transfer was always whole —
    // the decoder walked 25.2 % of the samples and stopped. A short
    // decode is non-evidence, so the container's proof stands.
    let report = complete_container_report(216.3, content_facts(48_000, 54 * 48_000, 54 * 48_000));
    assert_eq!(
        acceptance(&report, 216.0),
        Acceptance::Accept,
        "a complete container overrules a short decode: {}",
        report.content.summary()
    );
}

#[test]
fn a_windowed_object_with_complete_bytes_is_still_rejected() {
    // The shape only the audible signal can catch, and the one that would
    // have been lost if acceptance had simply moved to `measured_secs`:
    // every advertised byte arrived, the sample table describes the whole
    // track, the decoded stream is the *right length* — because what came
    // after the cutoff was silence.
    //
    // 99 s of 287 s is the measured v2.6.44 device ratio, so this is not a
    // shape that was invented to make the test pass.
    let audible_secs = 99.0_f64;
    let report = complete_container_report(
        287.0,
        content_facts(44_100, 287 * 44_100, (audible_secs * 44_100.0) as u64),
    );
    let verdict = acceptance(&report, 287.0);
    assert_eq!(
        verdict,
        Acceptance::Reject(REASON_SILENT_TAIL),
        "a windowed object must keep being refused even when every byte arrived: {}",
        report.content.summary()
    );
}

#[test]
fn a_short_decode_is_non_evidence_when_the_container_is_complete() {
    // The two signals must not be collapsed into one. A short decode is
    // non-evidence once the container has proven the file whole, so the
    // audible veto only fires when the decode walked the full track and
    // found early silence.
    let expected = 240.0;

    // Short decoded length, and audible is *fine* (the whole thing is
    // loud). Accepted — the container proved the file; the decoder
    // stopping early disproves nothing.
    let short_but_loud =
        complete_container_report(240.0, content_facts(48_000, 60 * 48_000, 60 * 48_000));
    assert_eq!(
        acceptance(&short_but_loud, expected),
        Acceptance::Accept,
        "a loud but short decode is non-evidence: {}",
        short_but_loud.content.summary()
    );

    // Short decoded length, and audible ends where the decode ended.
    // Accepted — audible == end-of-decode adds no information beyond
    // "the decoder stopped"; it is not observed trailing silence.
    let short_and_quiet =
        complete_container_report(240.0, content_facts(48_000, 60 * 48_000, 48 * 48_000));
    assert_eq!(
        acceptance(&short_and_quiet, expected),
        Acceptance::Accept,
        "audible-at-decode-end is not trailing silence: {}",
        short_and_quiet.content.summary()
    );

    // Long enough, and the audible tail is 10 % of the track: accepted, so
    // the veto does not fire on a fade-out.
    let long_fading =
        complete_container_report(240.0, content_facts(48_000, 240 * 48_000, 216 * 48_000));
    assert_eq!(acceptance(&long_fading, expected), Acceptance::Accept);

    // Long enough, and audio stops at 20 % of the track: vetoed.
    let long_but_windowed =
        complete_container_report(240.0, content_facts(48_000, 240 * 48_000, 48 * 48_000));
    assert!(
        matches!(
            acceptance(&long_but_windowed, expected),
            Acceptance::Reject(_)
        ),
        "a 20 % audible tail is not a fade-out: {}",
        long_but_windowed.content.summary()
    );
}

#[test]
fn a_short_container_table_is_refused_before_anything_else() {
    // The server sent a window and called it the whole object: the sample
    // table itself describes a short track. Nothing about the decoded audio
    // can make that acceptable.
    let report = complete_container_report(75.0, content_facts(48_000, 75 * 48_000, 75 * 48_000));
    let verdict = acceptance(&report, 216.0);
    assert_eq!(verdict, Acceptance::Reject(REASON_SHORT_TABLE),);
}

#[test]
fn missing_bytes_the_sample_table_references_are_refused() {
    // The one signal that is exact and needs no decoding.
    let report = ForensicReport {
        container: ContainerFacts {
            container: "mp4-stbl",
            audio_track_found: true,
            table_secs: Some(240.0),
            missing_bytes: Some(4096),
            verdict: Some(Verdict::Truncated {
                missing_bytes: 4096,
            }),
            ..Default::default()
        },
        content: content_facts(48_000, 240 * 48_000, 240 * 48_000),
        decoded: Ok(None),
    };
    assert_eq!(
        acceptance(&report, 240.0),
        Acceptance::Reject(REASON_MISSING_BYTES),
    );
}

#[test]
fn an_unparseable_container_falls_back_to_the_decoder_verbatim() {
    // `forensics::inspect_container` returns no verdict for a WebM, an
    // oversized file, or any layout it cannot walk, and documents that as
    // "keep your previous behaviour". The previous behaviour is the
    // decoder's verdict — so both directions are pinned, because silently
    // turning the fallback into a blanket accept (or a blanket reject)
    // would change what an Opus download is allowed to do.
    let undecodable = ForensicReport {
        container: ContainerFacts {
            container: "unknown",
            ..Default::default()
        },
        content: ContentFacts::default(),
        decoded: Ok(None),
    };
    assert_eq!(
        acceptance(&undecodable, 240.0),
        Acceptance::Accept,
        "no structural evidence and a decoder that is happy: previous behaviour kept"
    );

    let decoder_agrees_short = ForensicReport {
        decoded: Err("Truncated download: only 75s of 240s".to_string()),
        ..undecodable
    };
    assert_eq!(
        acceptance(&decoder_agrees_short, 240.0),
        Acceptance::Reject(REASON_UNPARSABLE),
    );
}

#[test]
fn an_unmeasured_decoded_length_does_not_veto_a_complete_container() {
    // `ContentFacts::measured_secs()` is `None` exactly when no decoder could
    // be built. `forensics` is explicit that `None` "never means empty" and
    // that a caller must fall back to its other evidence — which is the
    // container's byte accounting, and that is exact.
    let report = complete_container_report(240.0, ContentFacts::default());
    assert_eq!(
        report.content.measured_secs(),
        None,
        "the fixture really is unmeasured"
    );
    assert_eq!(
        acceptance(&report, 240.0),
        Acceptance::Accept,
        "nothing was measured, so the length signal abstains: {}",
        report.content.summary()
    );

    // A *present but short* measurement is the same non-evidence: the
    // container proved the file, the decoder stopped early.
    let measured_short = complete_container_report(240.0, content_facts(48_000, 12_000, 12_000));
    assert_eq!(
        acceptance(&measured_short, 240.0),
        Acceptance::Accept,
        "a short decode does not veto a complete container: {}",
        measured_short.content.summary()
    );
}

// ---------------------------------------------------------------------
// NEW-02 — the facts must describe the file as it is *now*
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_verdict_must_be_re_derived_after_an_append() {
    // NEW-02 in its exact shape: the container is consulted once, `top_up`
    // appends bytes, and the pre-append facts then judge the post-append
    // file. Here the only difference between the two states is the tail the
    // sample table still points at, and it is the difference between
    // refusing and saving.
    let short = synthetic_mp4(4096);
    let probe = TempFile::new("recompute", &short);
    let before = gather_forensics(&probe.path, "m4a", None).await;
    assert!(
        matches!(before.container.verdict, Some(Verdict::Truncated { .. })),
        "the fixture must start short: {}",
        before.container.summary()
    );
    assert!(
        matches!(acceptance(&before, 0.0), Acceptance::Reject(_)),
        "a file missing bytes its sample table references is not whole"
    );

    // The top-up: append exactly the bytes the sample table still points at.
    // The count is read out of the report rather than hard-coded, so this
    // does not depend on how `forensics` computes the media-data end — only
    // on the fact that it reports one, and reports the same one twice.
    let missing = before
        .container
        .audio_data_end
        .expect("a truncated sample table must say where its media data ends")
        .saturating_sub(before.container.size_bytes) as usize;
    assert!(missing > 0, "the fixture must have bytes outstanding");

    // A read-resize-write rather than `append` + `write_all`: the only thing
    // that matters here is the resulting file length, and this cannot leave a
    // handle open across the second gather.
    let mut with_tail = std::fs::read(&probe.path).expect("read the short file");
    with_tail.resize(with_tail.len() + missing, 0u8);
    std::fs::write(&probe.path, &with_tail).expect("append the missing media bytes");
    assert_eq!(
        with_tail.len() as u64,
        before.container.audio_data_end.expect("audio_data_end"),
        "the file must now reach the end the sample table points at"
    );

    let after = gather_forensics(&probe.path, "m4a", None).await;
    assert_eq!(
        after.container.verdict,
        Some(Verdict::Complete),
        "the appended file must read as complete: {}",
        after.container.summary()
    );
    assert_eq!(
        acceptance(&after, 0.0),
        Acceptance::Accept,
        "the same decision, applied to the re-derived facts, must accept: {}",
        after.container.summary()
    );

    // And the point of the whole exercise: the *stale* report still says
    // reject. Any implementation that hoisted the facts out of the top-up
    // loop would reach that answer and never recover, even after four rounds
    // that completed the file.
    assert!(
        matches!(acceptance(&before, 0.0), Acceptance::Reject(_)),
        "reusing the pre-append facts would refuse a file the append finished"
    );
    assert_ne!(
        before.container.size_bytes, after.container.size_bytes,
        "a re-derived report must be about the new file, not the old one"
    );
}

#[tokio::test]
async fn gather_forensics_reads_the_file_rather_than_caching_it() {
    // The mechanism NEW-02 leans on, tested on its own: two gathers of the
    // same path must differ once the file changes. A gatherer that memoised,
    // or that snapshotted the size once, would pass the cross-check test
    // above by accident.
    let probe = TempFile::new("fresh", b"PROBE-BODY");
    let first = gather_forensics(&probe.path, "m4a", None).await;
    assert_eq!(first.container.size_bytes, 10);

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&probe.path)
        .expect("open for append");
    file.write_all(b"-AND-MORE").expect("append");
    drop(file);

    let second = gather_forensics(&probe.path, "m4a", None).await;
    assert_eq!(
        second.container.size_bytes, 19,
        "the second gather must see the appended bytes"
    );
}

// ---------------------------------------------------------------------
// NEW-03 — the inspections must not run on the runtime thread
// ---------------------------------------------------------------------

#[test]
fn the_three_inspections_exist_in_exactly_one_place_and_it_is_blocking() {
    // "Off the Tokio runtime" is a property of *where the call is written*,
    // and there is no runtime signal a unit test can assert on directly: a
    // blocking call on a worker thread still returns exactly the right
    // answer. So this pins the shape the property depends on.
    //
    // It exists because the fix is one deleted `spawn_blocking` away from
    // regressing, and because `inspect_container` reads up to 192 MB into
    // memory — a regression here is a multi-second stall of every other task
    // on the runtime, with no error and no failing test anywhere else.
    // `include_str!`, not `concat!`. `concat!` produced a *path*, and every
    // search below ran against those 52 characters: `test_module` was always
    // `None`, so the slice was the path itself and each `matches(...).count()`
    // was 0. The pre-existing NEW-03 test had this shape and could not pass
    // — the marker assertion below is what it tripped over, which is the one
    // place the mistake was visible.
    const SOURCE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/infrastructure/media/downloader/mod.rs"
    ));

    // The test module holds the very strings it searches for, so counting
    // them in the whole file would count this test. Only the code above the
    // test module is production.
    let test_module = SOURCE.find("\n#[cfg(test)]\nmod tests;");
    let production = match test_module {
        Some(at) => &SOURCE[..at],
        None => SOURCE,
    };
    assert!(
        test_module.is_some(),
        "the test module marker must be findable, or this test is scanning itself"
    );

    for (name, needle) in [
        ("inspect_container", "inspect_container("),
        ("inspect_content", "inspect_content("),
        ("verify_decoded_duration", "verify_decoded_duration("),
    ] {
        let occurrences = production.matches(needle).count();
        assert_eq!(
            occurrences, 1,
            "{name} must be called from exactly one place, the blocking hop, \
                 but production code calls it {occurrences} times — an inline \
                 call has crept back in"
        );
    }

    // The one place must be a `spawn_blocking` closure, and it must be
    // `gather_forensics`, which is what `run_stream` awaits.
    let hop_start = production
        .find("async fn gather_forensics(")
        .expect("gather_forensics must exist");
    let hop_end = hop_start
        + production[hop_start..]
            .find("\n/// What one completed HTTP response")
            .expect("gather_forensics must be followed by the next item");
    let hop = &production[hop_start..hop_end];
    assert!(
        hop.contains("spawn_blocking"),
        "the single call site must be inside a spawn_blocking hop"
    );
    // `run_stream` must reach the forensics through the hop, and it must do
    // so at the *post-append* site too — that second call is NEW-02, and
    // counting them keeps the two fixes from being unlinked later.
    //
    // The slice is `run_stream`'s own body, from its signature to the next
    // method's doc comment. It cannot stop at the hop: `gather_forensics` is
    // defined far *above* `run_stream`, and the two call sites sit deep
    // inside a function that is itself several hundred lines long.
    let stream_start = production
        .find("    async fn run_stream(")
        .expect("run_stream must exist");
    let stream_end = stream_start
        + production[stream_start..]
            .find("\n    /// Fetch a thumbnail/cover URL")
            .expect("run_stream must be followed by save_thumbnail");
    let stream = &production[stream_start..stream_end];
    assert_eq!(
        stream
            .matches("gather_forensics(staging, ext, dur).await")
            .count(),
        2,
        "run_stream must call the blocking hop once for the initial facts and \
             once more after each append"
    );
}

// ---------------------------------------------------------------------
// DL-05 — a zero advertised length must terminate the job
// ---------------------------------------------------------------------

#[test]
fn a_zero_advertised_length_is_never_a_completed_response() {
    // The exact state that used to loop forever: with `Some(0)`, the
    // "all bytes received" test needs `total > 0` and the "bytes still
    // outstanding" test needs `total > current`, so a zero-byte clean EOF
    // satisfied neither and the loop asked again with nothing changed.
    assert_eq!(
        classify_response(0, false, Some(0), 0),
        StreamStep::NoProgress,
        "a zero-length object owes nothing and delivered nothing: that is not progress"
    );
    assert_eq!(
        classify_response(0, false, Some(0), 12_345),
        StreamStep::NoProgress,
        "the same holds once bytes are held, because 0 > 12345 is false"
    );
}

#[test]
fn every_response_that_delivered_nothing_consumes_retry_budget() {
    assert_eq!(
        classify_response(4096, false, Some(10_000), 4096),
        StreamStep::Progress,
        "bytes arrived: the budget resets"
    );
    assert_eq!(
        classify_response(4096, true, Some(10_000), 4096),
        StreamStep::Progress,
        "bytes that did arrive are progress even if the read then failed, \
             because the next request resumes from the real on-disk length"
    );
    assert_eq!(
        classify_response(0, true, None, 4096),
        StreamStep::Interrupted,
        "a stalled read is an error"
    );
    assert_eq!(
        classify_response(0, false, Some(10_000), 4096),
        StreamStep::Short { advertised: 10_000 },
        "a clean early end with bytes provably outstanding"
    );
    // The two cases that used to fall through every branch and be treated
    // as a finished stream.
    assert_eq!(
        classify_response(0, false, None, 4096),
        StreamStep::NoProgress,
        "an unknown length plus silence is not the end of the object"
    );
    assert_eq!(
        classify_response(0, false, Some(4096), 4096),
        StreamStep::NoProgress,
        "a length already satisfied plus silence is not the end of the object"
    );
}

// ---------------------------------------------------------------------
// DL-01 — one job, one output name
// ---------------------------------------------------------------------

/// An output dir plus its staging dir, cleaned up with the test.
struct ReservationDir {
    dir: PathBuf,
    output: PathBuf,
    staging: PathBuf,
}

impl ReservationDir {
    fn new(tag: &str) -> ReservationDir {
        let dir = std::env::temp_dir().join(format!("auralis_own_{tag}_{}", Uuid::new_v4()));
        let output = dir.join("downloads");
        let staging = output.join(".tmp");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        ReservationDir {
            dir,
            output,
            staging,
        }
    }
}

impl Drop for ReservationDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn two_downloads_of_one_title_are_never_given_the_same_output() {
    // The defect: the name used to be chosen with `if path.exists()`, and
    // two jobs with the same title both saw "free" and were handed the same
    // path — so one of them deleted the other's finished file, and two that
    // both succeeded overwrote each other.
    let area = ReservationDir::new("same_title");
    let first = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("first download must be able to claim the clean name");
    let second = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("second download must fall through to a dedup name");

    assert_ne!(
        first.output_path, second.output_path,
        "two live jobs must not share an output path"
    );
    assert_ne!(
        first.staging_path, second.staging_path,
        "and therefore not share the claim that makes it exclusive"
    );
    assert!(
        first.staging_path.exists() && second.staging_path.exists(),
        "both claims are held until their own job releases them"
    );
}

#[tokio::test]
async fn a_claim_is_exclusive_even_for_the_same_job_id() {
    // Belt and braces against the list, not the filesystem: the fallback
    // names are derived from the job id, so one job asking twice proposes
    // the same candidate twice and the exclusive create has to be what stops
    // it. A "fix" that only deduplicated the candidate list would pass the
    // test above and fail this one.
    let area = ReservationDir::new("same_id");
    let id = Uuid::new_v4();
    let first = reserve_output(&area.output, &area.staging, "Song", "m4a", id)
        .await
        .expect("first claim");
    let second = reserve_output(&area.output, &area.staging, "Song", "m4a", id)
        .await
        .expect("second claim must not reuse the name the first holds");
    assert_ne!(first.output_path, second.output_path);
}

#[tokio::test]
async fn an_existing_file_is_never_claimed_or_overwritten() {
    // A file from an earlier run must be neither adopted nor clobbered: the
    // old code only ever *renamed over* it when the dedup name was also
    // taken, which is the other half of the same data loss.
    let area = ReservationDir::new("existing");
    let existing = area.output.join("Song.m4a");
    std::fs::write(&existing, b"not ours").expect("seed existing file");

    let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("a dedup name is available");
    assert_ne!(
        claimed.output_path, existing,
        "a name already holding a file cannot be claimed"
    );
    assert_eq!(
        std::fs::read(&existing).expect("existing file still readable"),
        b"not ours",
        "and the file an earlier download wrote is untouched"
    );
}

#[tokio::test]
async fn the_public_name_stays_clean_however_the_internal_one_did_not() {
    // Defect 3: the dedup suffix exists so two *internal* files can coexist.
    // Showing it to the owner in `Download/Auralis/` tells them they
    // downloaded the same track twice when they did not.
    let area = ReservationDir::new("public_name");
    std::fs::write(area.output.join("Song.m4a"), b"earlier").expect("seed earlier download");

    let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("claim a dedup name");
    let internal = claimed
        .output_path
        .file_name()
        .expect("a file name")
        .to_string_lossy()
        .to_string();
    assert_ne!(
        internal, "Song.m4a",
        "precondition: the internal name is suffixed"
    );
    assert_eq!(
        claimed.public_name, "Song.m4a",
        "the public name is the clean title, not the internal one"
    );
}

#[tokio::test]
async fn a_stale_cover_sidecar_is_cleared_rather_than_inherited() {
    // The sidecar is a separate file derived from the same name, so it
    // carries its own hazard: an earlier download's artwork can outlive its
    // audio, and then a download with no thumbnail of its own leaves the old
    // cover art sitting next to the new file, attached to the wrong track.
    let area = ReservationDir::new("stale_cover");
    let cover = area.output.join("Song.jpg");
    std::fs::write(&cover, b"stale artwork").expect("seed stale sidecar");

    let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("claim the clean name");
    let name = claimed
        .output_path
        .file_name()
        .expect("a file name")
        .to_string_lossy();
    assert_eq!(name, "Song.m4a");
    assert!(
        !cover.exists(),
        "a sidecar whose audio is gone must not survive into the new download"
    );
}

#[tokio::test]
async fn a_committed_file_outlives_its_own_jobs_failure() {
    // `discard_owned_paths` is what a failing job and a cancelling one both
    // call, and it is the only thing allowed to delete an output path. The
    // rename consumes the claim, so after a commit it must decline to touch
    // the file: that is the completed download, and the "just clean up the
    // output path" instinct is what deleted it.
    let area = ReservationDir::new("committed");
    let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("claim");
    std::fs::write(&claimed.staging_path, b"verified bytes").expect("write staging bytes");
    // The commit.
    std::fs::rename(&claimed.staging_path, &claimed.output_path).expect("commit");

    discard_owned_paths(&claimed.staging_path, &claimed.output_path).await;

    assert!(
        claimed.output_path.exists(),
        "a job that has already committed must not delete its own finished file"
    );
    assert!(
        !claimed.staging_path.exists(),
        "the claim is spent either way"
    );
}

#[tokio::test]
async fn a_partial_file_is_removed_by_the_job_that_owns_it() {
    // The other half, so the guard above cannot be satisfied by simply never
    // deleting anything: while the claim is held the output is this job's
    // own partial work, and a failure must clear it.
    let area = ReservationDir::new("partial");
    let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("claim");
    std::fs::write(&claimed.staging_path, b"half a track").expect("write staging bytes");
    std::fs::write(&claimed.output_path, b"leftover").expect("write stray output");
    std::fs::write(sidecar_path(&claimed.output_path), b"art").expect("write stray cover");

    discard_owned_paths(&claimed.staging_path, &claimed.output_path).await;

    assert!(!claimed.output_path.exists(), "partial output removed");
    assert!(
        !sidecar_path(&claimed.output_path).exists(),
        "and its cover sidecar with it"
    );
    assert!(!claimed.staging_path.exists(), "the claim is released");
}

#[test]
fn a_claim_left_by_a_dead_process_is_swept_at_startup() {
    // A staging file *is* a claim, so a crash part way through a download
    // would hold that name until the user cleared the directory by hand — and
    // every later download of that title would be pushed onto a dedup
    // suffix for no reason.
    let area = ReservationDir::new("sweep");
    std::fs::write(area.staging.join("Song.m4a.part"), b"interrupted").expect("seed claim");
    std::fs::write(area.staging.join("Song.m4a"), b"stranded link").expect("seed link");
    std::fs::write(area.output.join("Song.m4a"), b"committed").expect("seed a real file");

    sweep_stale_staging(&area.staging);

    assert!(
        area.staging
            .read_dir()
            .expect("read staging")
            .next()
            .is_none(),
        "the staging directory is scratch and must be empty at startup"
    );
    assert!(
        area.output.join("Song.m4a").exists(),
        "and a committed file is not scratch: the sweep must not reach it"
    );
}

#[test]
fn the_dedup_fallback_names_are_distinct_from_each_other() {
    // The reservation is what makes the choice safe, but a list that offered
    // the same name twice would make every collision look like a bug in the
    // log — and the third attempt onwards is not a shape any test of
    // behaviour would reach.
    let id = Uuid::new_v4();
    let names = candidate_file_names("Song", "m4a", id);
    assert_eq!(names.len(), RESERVE_ATTEMPTS);
    assert_eq!(names[0], "Song.m4a", "the clean name is always tried first");
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "every fallback must be distinct");
}

// ---------------------------------------------------------------------
// DL-02 — the commit boundary
// ---------------------------------------------------------------------

#[test]
fn the_interrupt_table_refuses_everything_after_verification() {
    // The whole table, because the interesting cells are the ones a
    // hand-written test would leave out. `Committing` is the new row: a
    // cancel there used to delete a finished file, and a pause there used to
    // report `Paused` over a job whose staging file had just been renamed
    // away.
    let all = [
        DownloadStatus::Queued,
        DownloadStatus::Downloading,
        DownloadStatus::Paused,
        DownloadStatus::Committing,
        DownloadStatus::Completed,
        DownloadStatus::Failed,
        DownloadStatus::Cancelled,
    ];
    for status in all {
        let pause = classify_interrupt(Interrupt::Pause, status);
        let cancel = classify_interrupt(Interrupt::Cancel, status);
        // One arm per state rather than grouped or-patterns: a grouped arm
        // whose body needs a block is a formatting decision the two rustfmt
        // versions in this project disagree about, and the table is the
        // thing under test.
        let expected = match status {
            DownloadStatus::Queued => (RequestOutcome::Apply, RequestOutcome::Apply),
            DownloadStatus::Downloading => (RequestOutcome::Apply, RequestOutcome::Apply),
            DownloadStatus::Paused => (RequestOutcome::AlreadySettled, RequestOutcome::Apply),
            DownloadStatus::Committing => (RequestOutcome::Refuse, RequestOutcome::Refuse),
            DownloadStatus::Completed => (RequestOutcome::Refuse, RequestOutcome::Refuse),
            DownloadStatus::Failed => (RequestOutcome::Refuse, RequestOutcome::Refuse),
            DownloadStatus::Cancelled => (RequestOutcome::Refuse, RequestOutcome::AlreadySettled),
        };
        assert_eq!(
            (pause, cancel),
            expected,
            "pause/cancel of a {status} download"
        );
    }
}

#[test]
fn a_refusal_names_the_state_and_the_consequence() {
    let error = interrupt_error(Interrupt::Cancel, DownloadStatus::Committing);
    let message = error.to_string();
    assert!(message.contains("cancel"), "says which request: {message}");
    assert!(
        message.contains("committing"),
        "says which state: {message}"
    );
    assert!(
        message.contains("complete") && message.contains("resume"),
        "and says what the user stands to lose, which is the part that was \
             missing: {message}"
    );
}

/// A downloader with one job already registered in `status`, holding a claim
/// on an output name, and no live task.
async fn downloader_with_job_in(
    dir: &Path,
    status: DownloadStatus,
) -> (Downloader, Uuid, OutputReservation) {
    let staging = dir.join(".tmp");
    std::fs::create_dir_all(&staging).expect("create staging dir");
    let downloader = Downloader::new(dir.to_path_buf());
    let id = Uuid::new_v4();
    let claimed = reserve_output(dir, &staging, "Song", "m4a", id)
        .await
        .expect("claim the output name");
    std::fs::write(&claimed.output_path, b"a finished track").expect("write the output");
    let job = DownloadJob {
        stream_url: "https://example.com/song.m4a".to_string(),
        title: "Song".to_string(),
        artist: None,
        album: None,
        output_path: claimed.output_path.clone(),
        staging_path: claimed.staging_path.clone(),
        public_name: claimed.public_name.clone(),
        thumbnail: None,
        headers: None,
        expected_duration_secs: None,
        total_bytes: None,
        format: AudioFormat::M4a,
        ext: "m4a".to_string(),
    };
    let mut progress =
        DownloadProgress::with_id(id, job.stream_url.clone(), job.title.clone(), job.format);
    progress.status = status;
    downloader.jobs.write().await.insert(id, job);
    downloader
        .active_downloads
        .write()
        .await
        .insert(id, progress);
    (downloader, id, claimed)
}

#[tokio::test]
async fn a_committing_job_refuses_to_pause_or_cancel_and_keeps_its_file() {
    // The behavioural half of DL-02, on the real `pause`/`cancel` rather than
    // on the table they consult: a refusal that still deleted the file would
    // satisfy a table-only test.
    let dir = std::env::temp_dir().join(format!("auralis_commit_{}", Uuid::new_v4()));
    let (downloader, id, claimed) = downloader_with_job_in(&dir, DownloadStatus::Committing).await;

    let pause = downloader.pause(id).await;
    let cancel = downloader.cancel(id).await;

    assert!(
        matches!(pause, Err(DownloaderError::InvalidState(_))),
        "pause during the commit must be refused, got {pause:?}"
    );
    assert!(
        matches!(cancel, Err(DownloaderError::InvalidState(_))),
        "cancel during the commit must be refused, got {cancel:?}"
    );
    assert!(
        claimed.output_path.exists(),
        "and a refused request must not have deleted the file"
    );
    assert_eq!(
        downloader.read_status(id).await.expect("still tracked"),
        DownloadStatus::Committing,
        "a refused request must not have moved the job either"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_committed_job_is_still_untouchable_and_reports_its_state() {
    // The guard DL-02 was built on, pinned so a later edit cannot quietly
    // narrow it to `Committing` only.
    let dir = std::env::temp_dir().join(format!("auralis_done_{}", Uuid::new_v4()));
    let (downloader, id, claimed) = downloader_with_job_in(&dir, DownloadStatus::Completed).await;

    assert!(matches!(
        downloader.cancel(id).await,
        Err(DownloaderError::InvalidState(_))
    ));
    assert!(matches!(
        downloader.pause(id).await,
        Err(DownloaderError::InvalidState(_))
    ));
    assert!(claimed.output_path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_cancelled_job_still_deletes_its_own_partial_bytes() {
    // The other direction: `Committing` must not turn cancel into a no-op.
    let dir = std::env::temp_dir().join(format!("auralis_cancel_{}", Uuid::new_v4()));
    let (downloader, id, claimed) = downloader_with_job_in(&dir, DownloadStatus::Paused).await;

    downloader
        .cancel(id)
        .await
        .expect("a paused job may be cancelled");

    assert!(
        !claimed.output_path.exists(),
        "an uncommitted job's own bytes are removed on cancel"
    );
    assert_eq!(
        downloader.read_status(id).await.expect("still tracked"),
        DownloadStatus::Cancelled
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_commit_boundary_is_ordered_and_held() {
    // The ordering DL-02 is about cannot be observed from outside without a
    // real 4-minute download and a CD-R, and a test that asserted a
    // reimplementation of it would pass no matter what the shipped code did —
    // which is how `pot-for-TV` shipped a live 403. So this reads the real
    // source and checks the order the file is in.
    // `include_str!`, not `concat!`. `concat!` produced a *path*, and every
    // search below ran against those 52 characters: `test_module` was always
    // `None`, so the slice was the path itself and each `matches(...).count()`
    // was 0. The pre-existing NEW-03 test had this shape and could not pass
    // — the marker assertion below is what it tripped over, which is the one
    // place the mistake was visible.
    const SOURCE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/infrastructure/media/downloader/mod.rs"
    ));
    let test_module = SOURCE.find("\n#[cfg(test)]\nmod tests;");
    let production = match test_module {
        Some(at) => &SOURCE[..at],
        None => SOURCE,
    };
    let start = production
        .find("    async fn run_stream(")
        .expect("run_stream must exist");
    let body = &production[start..];

    let at = |needle: &str| {
        body.find(needle)
            .unwrap_or_else(|| panic!("{needle} must appear in run_stream"))
    };
    let gate = at("let _commit = commit_gate.lock().await;");
    let committing = at("state.begin_commit();");
    let rename = at("tokio::fs::rename(&job.staging_path, &job.output_path)");
    let completed = at("state.complete(job.output_path.to_string_lossy().to_string());");
    let released = at("drop(_commit);");
    let cover = at("Self::save_thumbnail(&client, thumb, &job.output_path).await;");
    let publish = at("Self::publish_public_copy(id, job, &active).await;");
    let settled = at("state.finish_post_commit();");

    assert!(
        gate < committing && committing < rename && rename < completed,
        "the gate is taken, then Committing, then the rename, then Completed"
    );
    assert!(
        completed < released && released < cover && cover < publish && publish < settled,
        "Completed is terminal immediately after the rename, and everything \
             that can improve the file happens after it"
    );
}

#[test]
fn an_interrupt_is_decided_before_the_task_is_aborted() {
    // The table above decides whether a request may be *obeyed*; it cannot
    // decide whether the task is killed first. That ordering is the other
    // half of DL-02 and it is not observable from the state table, because
    // a request that reads `Downloading` and *then* aborts has already done
    // the damage by the time the table gets a say — which is exactly what
    // `cancel` used to do, killing the task mid-rename and only then
    // discovering the job was `Completed` and declining to delete anything.
    //
    // So the order is pinned in the source. A test that reimplemented the
    // order would pass whatever the shipped code did, which is how a
    // regression here once shipped as a live defect.
    // `include_str!`, not `concat!`. `concat!` produced a *path*, and every
    // search below ran against those 52 characters: `test_module` was always
    // `None`, so the slice was the path itself and each `matches(...).count()`
    // was 0. The pre-existing NEW-03 test had this shape and could not pass
    // — the marker assertion below is what it tripped over, which is the one
    // place the mistake was visible.
    const SOURCE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/infrastructure/media/downloader/mod.rs"
    ));
    let test_module = SOURCE.find("\n#[cfg(test)]\nmod tests;");
    let production = match test_module {
        Some(at) => &SOURCE[..at],
        None => SOURCE,
    };

    let pause = span_of(
        production,
        "    pub async fn pause(",
        "    /// Resume a paused download",
        "pause",
    );
    let cancel = span_of(
        production,
        "    pub async fn cancel(",
        "    /// Prune finished, failed, and cancelled download records",
        "cancel",
    );

    for (name, body) in [("pause", pause), ("cancel", cancel)] {
        let at = |needle: &str| {
            body.find(needle)
                .unwrap_or_else(|| panic!("{name} must contain {needle:?}"))
        };
        let gate = at("self.commit_gate.lock().await");
        let status = at("self.read_status(id).await?");
        let verdict = at("classify_interrupt");
        let abort = at("handle.abort()");
        assert!(
            gate < status && status < verdict && verdict < abort,
            "{name}: the gate is taken, the state read, the verdict taken, \
                 and only then is the task aborted"
        );
    }

    // The rest of the ordering. `cancel` deletes, so the deletion has to come
    // after the abort has been awaited, and `pause` must never delete at all
    // — a pause that removed bytes would turn "stop here" into "lose this".
    // And neither may touch the staging file before the aborted task has
    // actually stopped, which is the race the code has always guarded.
    let cancel_delete = cancel
        .find("discard_owned_paths")
        .expect("cancel must clean up through discard_owned_paths");
    assert!(
        cancel.find("handle.abort()").expect("cancel aborts") < cancel_delete,
        "cancel: the files are touched only after the task is stopped"
    );
    assert!(
        !pause.contains("discard_owned_paths") && !pause.contains("remove_file"),
        "pause: stopping a download must never delete anything"
    );
    let awaited = pause
        .find("let _ = handle.await;")
        .expect("pause must await the aborted task");
    assert!(
        awaited < pause.find("set_len").expect("pause truncates what it owns"),
        "pause: the staging file is truncated only once the writer has stopped"
    );
}

/// The text of one `fn`/`async fn` in the production slice, by its signature
/// and whatever doc comment follows it.
fn span_of<'a>(production: &'a str, start: &str, end: &str, what: &str) -> &'a str {
    let from = production
        .find(start)
        .unwrap_or_else(|| panic!("{what} must exist in the production source"));
    let to = production
        .find(end)
        .unwrap_or_else(|| panic!("{what} must be followed by {end:?}"));
    assert!(to > from, "{what} must precede {end:?}");
    &production[from..to]
}

// ---------------------------------------------------------------------
// Defect 3 — the public name
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_deduped_file_can_be_published_under_its_clean_name() {
    // `publish_to_downloads` derives the MediaStore display name from the
    // path it is given, so the only lever this side owns is the path. A hard
    // link is what carries the clean name without a second copy of the
    // bytes, and without anything having to resolve at read time.
    //
    // The link goes in the staging directory precisely because the clean
    // name is *taken* next to the file: that is the only reason the internal
    // name was deduped. Placing it beside the file fails with EEXIST in the
    // one case it exists for — which is what the first version of this test
    // did, and the failure it produced is the reason the link is here.
    let area = ReservationDir::new("public_link");
    std::fs::write(area.output.join("Song.m4a"), b"earlier").expect("seed earlier download");
    let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("claim a dedup name");
    std::fs::write(&claimed.output_path, b"the real bytes").expect("write the finished file");

    let link = link_under_public_name(&claimed.output_path, &area.staging, &claimed.public_name)
        .expect("a link must be available for a deduped name");

    assert_eq!(
        link.file_name().expect("a file name").to_string_lossy(),
        "Song.m4a",
        "the path handed to the publisher is named after the clean title"
    );
    assert_eq!(
        std::fs::read(&link).expect("link readable"),
        b"the real bytes",
        "and it carries the finished file's bytes, not a copy of its own"
    );
    assert!(
        claimed.output_path.exists(),
        "and the internal file is untouched by the presentation"
    );
}

#[tokio::test]
async fn no_link_is_made_when_the_names_already_agree() {
    // The overwhelmingly common case. Returning a link here would put a
    // second copy of every ordinary download in the library folder for the
    // duration of the publish.
    //
    // The finished file has to exist before this is meaningful: a link
    // cannot be made from a path that is not there, so a test that asserted
    // `None` on a merely-*reserved* name would pass with the guard deleted —
    // which is exactly what the first version of it did, and mutation
    // testing found it.
    let area = ReservationDir::new("public_link_same");
    let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
        .await
        .expect("claim the clean name");
    std::fs::write(&claimed.output_path, b"the real bytes").expect("write the finished file");
    let name = claimed
        .output_path
        .file_name()
        .expect("a file name")
        .to_string_lossy();
    assert_eq!(name, "Song.m4a");
    assert!(
        link_under_public_name(&claimed.output_path, &area.staging, &claimed.public_name).is_none(),
        "nothing to present when the internal name is already the public one"
    );
}

/// A loopback origin that answers every request with one fixed response, and
/// counts how many requests it served.
///
/// The count is what makes the zero-length test honest: it distinguishes
/// "the downloader stopped" from "the server gave up", so a downloader that
/// asked ten thousand times and then happened to fail would still fail.
struct CannedServer {
    url: String,
    served: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl CannedServer {
    fn start(response: &'static str) -> CannedServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local_addr").port();
        listener.set_nonblocking(true).expect("non-blocking");
        let served = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let accept = {
            let served = Arc::clone(&served);
            let running = Arc::clone(&running);
            std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    match listener.accept() {
                        // `serve_one` takes the stream by value, so the
                        // binding is never mutated. CI's clippy
                        // (`-D warnings`) rejects the `mut`.
                        Ok((stream, _)) => {
                            let served = Arc::clone(&served);
                            let running = Arc::clone(&running);
                            std::thread::spawn(move || {
                                serve_one(stream, response, &served, &running);
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        CannedServer {
            url: format!("http://127.0.0.1:{port}/videoplayback?itag=140&clen=0"),
            served,
            running,
            accept: Some(accept),
        }
    }
}

impl Drop for CannedServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
    }
}

/// Answer requests on one connection until the peer goes away. Read byte by
/// byte and reply per request head, because a request with a body would
/// otherwise be answered twice.
fn serve_one(
    mut stream: std::net::TcpStream,
    response: &'static str,
    served: &AtomicUsize,
    running: &AtomicBool,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while running.load(Ordering::Relaxed) {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        head.push(byte[0]);
        if head.len() >= 4 && head[head.len() - 4..] == *b"\r\n\r\n" {
            served.fetch_add(1, Ordering::Relaxed);
            head.clear();
            if stream.write_all(response.as_bytes()).is_err() {
                return;
            }
            let _ = stream.flush();
        }
        if head.len() > 16 * 1024 {
            return;
        }
    }
}

#[tokio::test]
async fn a_zero_advertised_length_fails_instead_of_retrying_forever() {
    // The 2026-09-27 shape: the edge reports a `Content-Length: 0` object and
    // an empty body, and the resolver's `total_bytes` is `Some(0)`. Every
    // branch of the read loop's state machine said "keep going", so the job
    // spun until the process was killed. It must end, with a reason.
    const EMPTY: &str = "HTTP/1.1 200 OK\r\nContent-Type: audio/mp4\r\nContent-Length: 0\r\n\r\n";
    let server = CannedServer::start(EMPTY);

    let dir = std::env::temp_dir().join(format!("auralis_dl_zero_{}", Uuid::new_v4()));
    let downloader = Downloader::new(dir.clone());
    let id = downloader
        .download(StreamDownload {
            stream_url: server.url.clone(),
            title: "Zero Length".to_string(),
            artist: None,
            album: None,
            platform: "direct".to_string(),
            format: AudioFormat::M4a,
            ext: "m4a".to_string(),
            total_bytes: Some(0),
            thumbnail: None,
            headers: None,
            expected_duration_secs: None,
        })
        .await
        .expect("a download request should be accepted");

    let progress = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(state) = downloader.get_progress(id).await {
                if matches!(
                    state.status,
                    DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled
                ) {
                    return state;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a zero-length object must not leave the download spinning");

    assert_eq!(
        progress.status,
        DownloadStatus::Failed,
        "expected a failure, got {status:?}: {error:?}",
        status = progress.status,
        error = progress.error
    );
    let error = progress.error.unwrap_or_default();
    assert!(
        error.contains("zero-length object"),
        "the failure must name the cause: {error}"
    );
    // `DownloadProgress::output_path` is a `String`, not a `PathBuf`, so the
    // on-disk check needs an explicit conversion. Calling `.exists()` on it
    // straight is a compile error, not a lint — this is the E0599 that kept
    // v2.6.59 and v2.6.60 red.
    let saved_to = downloader
        .get_progress(id)
        .await
        .expect("progress record")
        .output_path
        .expect("an output path");
    assert!(
        !std::path::Path::new(&saved_to).exists(),
        "nothing may be saved for an empty object, but {saved_to} exists"
    );

    let served = server.served.load(Ordering::Relaxed);
    assert!(
        served <= 1,
        "the job must be refused before or on the first request, \
             but the origin served {served}"
    );

    if let Some(handle) = downloader.tasks.write().await.remove(&id) {
        handle.abort();
        let _ = handle.await;
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn test_download_progress_id_matches_job_id() {
    let dir = std::env::temp_dir().join(format!("auralis_dl_id_test_{}", Uuid::new_v4()));
    let downloader = Downloader::new(dir.clone());
    let request = StreamDownload {
        stream_url: "http://127.0.0.1:1/test.wav".to_string(),
        title: "Identity Test".to_string(),
        artist: None,
        album: None,
        platform: "direct".to_string(),
        format: AudioFormat::Wav,
        ext: "wav".to_string(),
        total_bytes: None,
        thumbnail: None,
        headers: None,
        expected_duration_secs: None,
    };

    let job_id = downloader
        .download(request)
        .await
        .expect("test download should be accepted");
    let progress = downloader
        .get_progress(job_id)
        .await
        .expect("started download should have progress");

    assert_eq!(
        progress.id, job_id,
        "serialized progress ID must match the downloader job ID"
    );

    if let Some(handle) = downloader.tasks.write().await.remove(&job_id) {
        handle.abort();
        let _ = handle.await;
    }
    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_resume_rejects_non_paused_download() {
    let dir = std::env::temp_dir().join(format!("auralis_dl_state_test_{}", Uuid::new_v4()));
    let downloader = Downloader::new(dir.clone());
    let id = Uuid::new_v4();
    let job = DownloadJob {
        stream_url: "https://example.com/audio.mp3".to_string(),
        title: "Audio".to_string(),
        artist: None,
        album: None,
        output_path: dir.join("audio.mp3"),
        staging_path: dir.join(".tmp").join("audio.mp3.part"),
        public_name: "audio.mp3".to_string(),
        thumbnail: None,
        headers: None,
        expected_duration_secs: None,
        total_bytes: None,
        format: AudioFormat::Mp3,
        ext: "mp3".to_string(),
    };
    let mut progress = DownloadProgress::with_id(
        id,
        "https://example.com/audio.mp3".to_string(),
        "Audio".to_string(),
        AudioFormat::Mp3,
    );
    progress.status = DownloadStatus::Downloading;
    downloader.jobs.write().await.insert(id, job);
    downloader
        .active_downloads
        .write()
        .await
        .insert(id, progress);

    let result = downloader.resume(id).await;

    assert!(matches!(result, Err(DownloaderError::InvalidState(_))));
    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_downloader_cleanup_and_prune() {
    let dir = std::env::temp_dir().join(format!("auralis_dl_test_{}", Uuid::new_v4()));
    let downloader = Downloader::new(dir.clone());

    let id1 = Uuid::new_v4();
    let mut p1 = DownloadProgress::new(
        "https://example.com/1".into(),
        "Track 1".into(),
        AudioFormat::Mp3,
    );
    p1.status = DownloadStatus::Completed;
    p1.updated_at = Utc::now() - chrono::Duration::minutes(15); // > 10 min old

    let id2 = Uuid::new_v4();
    let mut p2 = DownloadProgress::new(
        "https://example.com/2".into(),
        "Track 2".into(),
        AudioFormat::Mp3,
    );
    p2.status = DownloadStatus::Failed;
    p2.updated_at = Utc::now() - chrono::Duration::seconds(30); // recent

    let id3 = Uuid::new_v4();
    let mut p3 = DownloadProgress::new(
        "https://example.com/3".into(),
        "Track 3".into(),
        AudioFormat::Mp3,
    );
    p3.status = DownloadStatus::Downloading; // in progress
    p3.updated_at = Utc::now() - chrono::Duration::minutes(20);

    {
        let mut active = downloader.active_downloads.write().await;
        active.insert(id1, p1);
        active.insert(id2, p2);
        active.insert(id3, p3);
    }

    // Run default cleanup (10 min threshold, 50 cap)
    downloader.cleanup().await;

    {
        let active = downloader.active_downloads.read().await;
        assert!(
            !active.contains_key(&id1),
            "Old completed download should be pruned"
        );
        assert!(
            active.contains_key(&id2),
            "Recent failed download should be retained"
        );
        assert!(
            active.contains_key(&id3),
            "Active downloading track should not be pruned"
        );
    }

    // Test max_retained limit
    for i in 0..10 {
        let id = Uuid::new_v4();
        let mut p = DownloadProgress::new(
            format!("https://example.com/{i}"),
            format!("Track {i}"),
            AudioFormat::Mp3,
        );
        p.status = DownloadStatus::Completed;
        p.updated_at = Utc::now() - chrono::Duration::seconds(i as i64);
        downloader.active_downloads.write().await.insert(id, p);
    }

    // Prune with max_retained = 3
    downloader
        .prune_finished(Duration::from_secs(3600), 3)
        .await;

    {
        let active = downloader.active_downloads.read().await;
        let completed_count = active
            .values()
            .filter(|p| p.status == DownloadStatus::Completed)
            .count();
        assert_eq!(
            completed_count, 3,
            "Should retain exactly 3 completed records"
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_sanitize_filename() {
    assert_eq!(sanitize_filename("Valid Name"), "Valid Name");
    assert_eq!(sanitize_filename("Slash/In/Name"), "Slash_In_Name");
    assert_eq!(sanitize_filename("Back\\Slash"), "Back_Slash");
    assert_eq!(sanitize_filename("../../etc/passwd"), "etc_passwd");
    assert_eq!(sanitize_filename("AUX"), "AUX_track");
    assert_eq!(sanitize_filename("COM1"), "COM1_track");
}

#[test]
fn test_sanitize_filename_unicode_boundary() {
    // The cap is 246 BYTES, not characters: 255 NAME_MAX − 8 for the longest
    // extension we allow − 1 for the dot. A char-based limit let a 200-char
    // CJK title reach ~600 bytes and fail the OS with ENAMETOOLONG, so
    // `sanitize_filename` now truncates on a char boundary at a byte budget.
    //
    // This test asserted 200 *characters* and had been failing since that
    // change landed — which is one of the reasons build-linux stayed red.
    // 246 / 2 bytes per 'é' is exactly 123 characters, so assert the byte
    // budget and the boundary, and let the character count follow from it
    // rather than being a second number that can drift.
    let name = "é".repeat(201);
    let sanitized = sanitize_filename(&name);

    assert!(
        sanitized.len() <= 246,
        "must fit the byte budget, got {} bytes",
        sanitized.len()
    );
    assert_eq!(
        sanitized.chars().count(),
        123,
        "246 bytes of 2-byte 'é' is 123 characters"
    );
    assert!(!sanitized.is_empty());

    // The point of walking back to a boundary: the result must still be valid
    // UTF-8 and must not end in a replacement or partial scalar. This is the
    // case the old `String::truncate(200)` PANICKED on.
    assert!(
        std::str::from_utf8(sanitized.as_bytes()).is_ok(),
        "a truncated multibyte name must stay valid UTF-8"
    );

    // A 3-byte scalar must land on a boundary too, not merely not panic.
    let cjk = "字".repeat(200);
    let cut = sanitize_filename(&cjk);
    assert!(
        cut.len() <= 246,
        "3-byte scalars must also respect the budget"
    );
    assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
}

#[test]
fn test_sanitize_ext() {
    assert_eq!(sanitize_ext("mp3", "mp3"), "mp3");
    assert_eq!(sanitize_ext("m4a", "m4a"), "m4a");
    assert_eq!(sanitize_ext("..exe", "mp3"), "mp3");
    assert_eq!(sanitize_ext("unknown", "flac"), "flac");
}

#[test]
fn test_validate_audio_file_nonexistent() {
    let path = PathBuf::from("/nonexistent/path/audio.mp3");
    let res = validate_audio_file(&path, Some(100), "mp3", AudioFormat::Mp3);
    assert!(res.is_err());
}

#[test]
fn test_validate_audio_file_empty() {
    let dir = std::env::temp_dir().join(format!("auralis_test_{}", Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    let empty_path = dir.join("empty.mp3");
    std::fs::write(&empty_path, b"").unwrap();

    let res = validate_audio_file(&empty_path, None, "mp3", AudioFormat::Mp3);
    assert!(res.is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_validate_audio_file_valid_wav() {
    let dir = std::env::temp_dir().join(format!("auralis_test_{}", Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    let wav_path = dir.join("test.part");

    // 1s 8000Hz 8-bit mono WAV = 8044 bytes
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

    std::fs::write(&wav_path, &data).unwrap();

    // Valid with expected duration 1s
    let res = validate_audio_file(&wav_path, Some(1), "wav", AudioFormat::Wav);
    assert!(
        res.is_ok(),
        "Expected valid WAV to pass validation: {:?}",
        res
    );
    assert_eq!(res.unwrap(), 1);

    // Fails if expected duration is 60s (diff > 5s)
    let res_fail = validate_audio_file(&wav_path, Some(60), "wav", AudioFormat::Wav);
    assert!(res_fail.is_err(), "Expected duration mismatch to fail");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_validate_audio_file_webm_opus_mislabeled_m4a() {
    // Regression: Opus-in-WebM mislabeled as `.m4a`
    // (EBML `1A 45 DF A3`, e.g. https://d.uguu.se/jXSTGTDj.m4a which is
    // byte-identical to scratch/sample.m4a). lofty guesses `Mpeg` and
    // rodio reports "format not recognized", so validation must fall back
    // to the Symphonia Opus probe instead of rejecting the download.
    let sample_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch/sample.m4a");
    if !sample_path.exists() {
        eprintln!("scratch/sample.m4a not found, skipping test");
        return;
    }
    assert!(is_ebml_container(&sample_path));
    let res = validate_audio_file(&sample_path, None, "m4a", AudioFormat::M4a);
    assert!(
        res.is_ok(),
        "Expected WebM/Opus mislabeled as .m4a to pass validation: {:?}",
        res
    );
    assert!(res.unwrap() > 0, "Duration should be non-zero");
}

#[tokio::test]
async fn test_validate_audio_file_async_performance_baseline() {
    let dir = std::env::temp_dir().join(format!("auralis_perf_test_{}", Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    let wav_path = dir.join("test.part");

    // 1s 8000Hz 8-bit mono WAV = 8044 bytes
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

    std::fs::write(&wav_path, &data).unwrap();

    let start_sync = std::time::Instant::now();
    let sync_res = validate_audio_file(&wav_path, Some(1), "wav", AudioFormat::Wav);
    let sync_elapsed = start_sync.elapsed();
    assert!(sync_res.is_ok());

    let start_async = std::time::Instant::now();
    let async_res = validate_audio_file_async(&wav_path, Some(1), "wav", AudioFormat::Wav).await;
    let async_elapsed = start_async.elapsed();
    assert!(async_res.is_ok());

    println!(
        "validate_audio_file sync time: {:?}, async spawn_blocking time: {:?}",
        sync_elapsed, async_elapsed
    );

    let _ = std::fs::remove_dir_all(dir);
}

use super::*;
use crate::domain::models::AudioFormat;
use crate::domain::repositories::TrackRepository;
use crate::infrastructure::database::repositories::SqliteTrackRepository;

/// A report as `resume` builds it, for tests that only care about the
/// fields they set.
fn sample_report(track: &Track, strategy: ResumeStrategy) -> ResumeReport {
    let mut report = ResumeReport::new(
        Some(track),
        strategy,
        PRE_PAUSED,
        "ok:1.00MB".to_string(),
        42.5,
    );
    report.result = RESULT_OK;
    report
}

fn sample_track() -> Track {
    Track::new(
        "Rësume <Test>".to_string(),
        "/library/resume.mp3".to_string(),
        180,
        AudioFormat::Mp3,
    )
}

// ---------------------------------------------------------------------
// Strategy selection
// ---------------------------------------------------------------------

/// The mitigation only fires where it is meant to: a paused sink that
/// still holds a playable source. `sink_snapshot()` answers
/// `(false, false)` for exactly that state.
#[test]
fn paused_sink_with_a_readable_file_is_replayed_on_a_fresh_sink() {
    assert_eq!(
        choose_resume_strategy(false, true, true),
        ResumeStrategy::FreshSinkReplay
    );
}

#[test]
fn playing_sink_is_never_replayed() {
    assert_eq!(
        choose_resume_strategy(true, true, true),
        ResumeStrategy::AlreadyPlaying
    );
    // Whatever the file looks like, a running sink is left alone.
    assert_eq!(
        choose_resume_strategy(true, true, false),
        ResumeStrategy::AlreadyPlaying
    );
}

/// No sink and a drained sink both answer `is_empty == true` from outside
/// `AudioPlayer`; both must go through `player.resume()` so its
/// `StateError` still reaches the frontend, which is what triggers the
/// replay the log then correlates.
#[test]
fn sink_without_a_live_source_keeps_the_old_unpause_route() {
    assert_eq!(
        choose_resume_strategy(false, false, true),
        ResumeStrategy::Unpause
    );
}

/// A file that cannot be stat'ed is withheld from the mitigation rather
/// than treated as fatal: `start_sink` tears the paused sink down *before*
/// it opens the file, so a bogus stat must not cost the listener a sink
/// that was still resumable. (On Android a MediaStore copy can still
/// resolve a file that will not stat, which is the same reason.)
#[test]
fn unreadable_file_falls_back_to_the_old_unpause_route() {
    assert_eq!(
        choose_resume_strategy(false, true, false),
        ResumeStrategy::Unpause
    );
}

// ---------------------------------------------------------------------
// Probe verdicts
// ---------------------------------------------------------------------

#[test]
fn probe_verdicts_separate_the_failure_shapes() {
    // The window produced no samples at all: the watcher task is starved.
    assert_eq!(classify_probe(0, 0, 0, false), VERDICT_NO_OBSERVATION);
    // Never playing, source still queued: the unpause itself did nothing.
    assert_eq!(classify_probe(6, 0, 0, false), VERDICT_NEVER_PLAYING);
    // Never playing and the source was already drained.
    assert_eq!(classify_probe(6, 0, 6, false), VERDICT_DRAINED_IMMEDIATE);
    // A single drained sample among several is still "drained": the
    // alternative shape of this check is "every sample was empty", which
    // would call a sink that died halfway through healthy.
    assert_eq!(classify_probe(6, 0, 1, false), VERDICT_DRAINED_IMMEDIATE);
    // Played, then consumed inside the window.
    assert_eq!(classify_probe(6, 3, 2, true), VERDICT_PLAYED_THEN_DRAINED);
    // A single drained sample after playing still counts as played-then-
    // drained; the "confirmed" verdict must not paper over it.
    assert_eq!(classify_probe(2, 1, 1, true), VERDICT_PLAYED_THEN_DRAINED);
    // Playing throughout: silence here is downstream of our state.
    assert_eq!(classify_probe(6, 6, 0, false), VERDICT_CONFIRMED);
}

#[test]
fn only_uninteresting_verdicts_stay_quiet() {
    assert!(!verdict_is_failure(VERDICT_CONFIRMED));
    assert!(!verdict_is_failure(VERDICT_NOT_PROBED));
    assert!(!verdict_is_failure(VERDICT_SUPERSEDED));
    for bad in [
        VERDICT_DRAINED_IMMEDIATE,
        VERDICT_PLAYED_THEN_DRAINED,
        VERDICT_NEVER_PLAYING,
        VERDICT_PAUSED_BY_COMMAND,
        VERDICT_NO_OBSERVATION,
        VERDICT_FAILED,
    ] {
        assert!(verdict_is_failure(bad), "{bad} must be reported");
    }
}

// ---------------------------------------------------------------------
// Pause attribution
//
// The device line that motivated all of this
// (`never_playing (polls=7 progress=0 empty=0)`) cannot mean "rodio never
// started the fresh sink", because `start_sink` never calls `pause()` and a
// new rodio `Player` is born unpaused. It means "a live sink is sitting
// paused", and these tests are what stops the log saying otherwise.
// ---------------------------------------------------------------------

/// The exact shape from the device run, with the pause that produced it
/// accounted for. Before the epoch existed this reported
/// `never_playing`, which sent three releases of forensics looking at rodio.
#[test]
fn a_pause_inside_the_window_is_reported_as_a_cancelled_resume() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    // The resume armed with nothing paused, then a `pause` command landed.
    arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay), 0);
    let sample = PauseSample {
        epoch: 1,
        at: Some(Instant::now()),
    };
    let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
    let report = observe_resume_probe(past_window, false, false, sample).expect("closes");
    assert_eq!(
        report.probe, VERDICT_PAUSED_BY_COMMAND,
        "a live paused sink with a pause on record is a cancelled resume, not a rodio failure"
    );
    // The original evidence is still on the line — the attribution
    // narrows the reading, it does not replace the measurement.
    assert!(report.probe_detail.contains("polls=1 progress=0 empty=0"));
    assert!(report.probe_detail.contains("paused_cmd=+"));
    assert!(verdict_is_failure(report.probe));
    reset_resume_log();
}

/// The counter is sampled at arm time, so a pause that happened *before*
/// this resume is invisible to it. Without this, every resume after the
/// owner's first pause would be blamed on a command that had nothing to do
/// with it — the attribution would be as wrong as the verdict it replaced.
#[test]
fn a_pause_before_the_resume_is_not_charged_to_it() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    // Epoch 7 at arm time: seven pauses have already been served.
    arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay), 7);
    let sample = PauseSample {
        epoch: 7,
        at: Some(Instant::now()),
    };
    let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
    let report = observe_resume_probe(past_window, false, false, sample).expect("closes");
    assert_eq!(
        report.probe, VERDICT_NEVER_PLAYING,
        "an unchanged epoch is the only case where a paused sink is our fault"
    );
    assert!(
        !report.probe_detail.contains("paused_cmd"),
        "detail was {}",
        report.probe_detail
    );
    reset_resume_log();
}

/// A resume that really did play is not rewritten by a later pause: audio
/// ran, and whatever stopped it afterwards is a separate fact. Rewriting
/// this would make the new verdict *less* honest than the old one.
#[test]
fn a_confirmed_resume_survives_a_later_pause_untouched() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay), 0);
    assert!(observe_resume_probe(Instant::now(), true, false, PauseSample::default()).is_none());
    let sample = PauseSample {
        epoch: 3,
        at: Some(Instant::now()),
    };
    let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
    let report = observe_resume_probe(past_window, true, false, sample).expect("closes");
    assert_eq!(report.probe, VERDICT_CONFIRMED);
    assert!(
        report.probe_detail.contains("progress=2"),
        "detail was {}",
        report.probe_detail
    );
    reset_resume_log();
}

/// A superseded probe was never judged against a sink, so a pause landing
/// while it waited must not invent a verdict for it — the newer resume owns
/// the sink and the newer report.
#[test]
fn a_superseded_probe_is_not_rewritten_by_a_pause() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    arm_resume_probe(sample_report(&track, ResumeStrategy::Unpause), 0);
    arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay), 0);
    let snapshot = resume_log_snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].probe, VERDICT_SUPERSEDED);
    assert!(
        !snapshot[0].probe_detail.contains("paused_cmd"),
        "detail was {}",
        snapshot[0].probe_detail
    );
    reset_resume_log();
}

/// The offset is the only thing in the log that separates the two actors the
/// counter cannot tell apart: a user tapping the button again a few hundred
/// milliseconds in, versus a focus or MediaSession pause nobody asked for.
#[test]
fn the_pause_offset_is_measured_from_arming() {
    let armed = Instant::now();
    let sample = PauseSample {
        epoch: 1,
        at: Some(armed + Duration::from_millis(310)),
    };
    assert_eq!(
        attribute_pause(armed, 0, sample),
        Some(Duration::from_millis(310)),
        "a pause 310ms in is a tap; the report has to say so in ms"
    );
    // No epoch movement: nothing was served, whatever the timestamp says.
    assert_eq!(attribute_pause(armed, 4, sample), None);
    // A pause with no recorded instant still counts. Degrading to `None`
    // here would silently restore the old "blame rodio" verdict, which is
    // the exact failure this mechanism exists to prevent.
    assert_eq!(
        attribute_pause(armed, 0, PauseSample { epoch: 1, at: None }),
        Some(Duration::ZERO)
    );
}

/// `drained_immediately` is re-attributed too. A paused source never ends,
/// so a pause and a drain in the same window means the drain was a
/// *consequence* of the pause and naming the pause first is the more
/// specific claim. The `empty=N` count stays on the line either way, so the
/// drain is still visible to whoever reads the next line.
#[test]
fn attribution_rewrites_every_verdict_that_means_the_sink_was_silent() {
    assert_eq!(
        apply_pause_attribution(VERDICT_NEVER_PLAYING, Some(Duration::from_millis(5))),
        VERDICT_PAUSED_BY_COMMAND
    );
    assert_eq!(
        apply_pause_attribution(VERDICT_DRAINED_IMMEDIATE, Some(Duration::from_millis(5))),
        VERDICT_PAUSED_BY_COMMAND
    );
    // And with no pause on record, nothing is rewritten.
    assert_eq!(
        apply_pause_attribution(VERDICT_NEVER_PLAYING, None),
        VERDICT_NEVER_PLAYING
    );
    assert_eq!(
        apply_pause_attribution(VERDICT_PLAYED_THEN_DRAINED, Some(Duration::from_millis(5))),
        VERDICT_PLAYED_THEN_DRAINED,
        "audio started: a later pause is a separate fact, not this resume's story"
    );
    assert_eq!(
        apply_pause_attribution(VERDICT_CONFIRMED, Some(Duration::from_millis(5))),
        VERDICT_CONFIRMED
    );
}

/// A release build writes `tracing` to stdout and the owner has no logcat,
/// so this toast is the only user-visible account of a cancelled resume. It
/// has to name the cancellation rather than send the owner to the log.
#[test]
fn a_cancelled_resume_toast_says_what_cancelled_it() {
    let track = sample_track();
    let mut report = sample_report(&track, ResumeStrategy::FreshSinkReplay);
    report.probe = VERDICT_PAUSED_BY_COMMAND;
    report.probe_detail = "polls=7 progress=0 empty=0 paused_cmd=+310ms".to_string();
    let toast = resume_failure_toast(&report);
    assert!(toast.contains("cancelled"), "toast was {toast}");
    assert!(toast.contains("+310ms"), "toast was {toast}");
    assert!(toast.contains("pause command"), "toast was {toast}");

    // The generic wording is still correct for every other verdict.
    report.probe = VERDICT_NEVER_PLAYING;
    let generic = resume_failure_toast(&report);
    assert!(generic.contains("produced no audio"), "toast was {generic}");
    assert!(!generic.contains("cancelled"), "toast was {generic}");
}

// ---------------------------------------------------------------------
// Log bookkeeping
// ---------------------------------------------------------------------

#[test]
fn the_log_keeps_the_last_twenty_attempts() {
    let mut reports: Vec<ResumeReport> = Vec::new();
    let track = sample_track();
    for _ in 0..(RESUME_LOG_CAPACITY + 7) {
        push_resume_report(&mut reports, sample_report(&track, ResumeStrategy::Unpause));
    }
    assert_eq!(reports.len(), RESUME_LOG_CAPACITY);
}

#[tokio::test]
async fn preflight_reads_the_file_the_replay_would_decode() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("auralis_resume_preflight_{}.mp3", Uuid::new_v4()));
    std::fs::write(&path, b"not really audio, but it exists").unwrap();

    let mut track = sample_track();
    track.file_path = path.to_string_lossy().to_string();
    let ok = preflight_audio_file(Some(&track)).await;
    assert!(ok.readable, "an existing non-empty file is readable");
    assert!(ok.note.starts_with("ok:"), "note was {:?}", ok.note);

    track.file_path = dir
        .join(format!("auralis_resume_absent_{}.mp3", Uuid::new_v4()))
        .to_string_lossy()
        .to_string();
    let missing = preflight_audio_file(Some(&track)).await;
    assert!(!missing.readable);
    assert!(
        missing.note.starts_with("stat-failed:"),
        "note was {:?}",
        missing.note
    );

    let none = preflight_audio_file(None).await;
    assert!(!none.readable);
    assert_eq!(none.note, "no-track");

    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------
// Probe state machine (global — serialised)
// ---------------------------------------------------------------------

#[test]
fn a_probe_closes_after_the_window_and_files_its_verdict() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay), 0);
    assert!(resume_probe_pending());

    // Inside the window: counted, not judged.
    assert!(observe_resume_probe(Instant::now(), true, false, PauseSample::default()).is_none());
    assert!(resume_probe_pending());

    let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
    let report = observe_resume_probe(past_window, true, false, PauseSample::default())
        .expect("the window must close on this tick");
    assert_eq!(report.probe, VERDICT_CONFIRMED);
    assert_eq!(report.probe_detail, "polls=2 progress=2 empty=0");
    assert!(!resume_probe_pending());

    let snapshot = resume_log_snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].probe, VERDICT_CONFIRMED);
    reset_resume_log();
}

#[test]
fn a_probe_never_played_is_reported_as_such() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay), 0);
    let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
    let report =
        observe_resume_probe(past_window, false, false, PauseSample::default()).expect("closes");
    assert_eq!(report.probe, VERDICT_NEVER_PLAYING);
    assert_eq!(report.probe_detail, "polls=1 progress=0 empty=0");
    reset_resume_log();
}

#[test]
fn a_probe_that_drains_after_playing_is_not_reported_as_healthy() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay), 0);
    let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
    // First tick plays, second tick is drained: the real "audio started and
    // was consumed" shape.
    assert!(observe_resume_probe(Instant::now(), true, false, PauseSample::default()).is_none());
    let report =
        observe_resume_probe(past_window, false, true, PauseSample::default()).expect("closes");
    assert_eq!(report.probe, VERDICT_PLAYED_THEN_DRAINED);
    assert_eq!(report.probe_detail, "polls=2 progress=1 empty=1");
    reset_resume_log();
}

#[test]
fn a_second_resume_supersedes_the_first_instead_of_losing_it() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    arm_resume_probe(sample_report(&track, ResumeStrategy::Unpause), 0);
    assert!(observe_resume_probe(Instant::now(), false, false, PauseSample::default()).is_none());
    arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay), 0);
    let snapshot = resume_log_snapshot();
    assert_eq!(snapshot.len(), 1, "the superseded probe is still on record");
    assert_eq!(snapshot[0].probe, VERDICT_SUPERSEDED);
    assert!(snapshot[0].probe_detail.contains("replaced after 1 poll"));
    reset_resume_log();
}

#[test]
fn the_replay_outcome_is_stamped_onto_the_rejected_resume() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    let mut report = sample_report(&track, ResumeStrategy::Unpause);
    report.result = RESULT_ERROR;
    report.probe = VERDICT_FAILED;
    report.detail = "State error: nothing to resume".to_string();
    file_resume_report_opening_replay(report);

    // A different track's play must not rewrite this report.
    note_replay_outcome(&Uuid::new_v4(), Ok(()));
    assert!(resume_log_snapshot()[0].replay.is_empty());

    note_replay_outcome(
        &track.id,
        Err("File error: /library/resume.mp3".to_string()),
    );
    let snapshot = resume_log_snapshot();
    assert_eq!(
        snapshot[0].replay, "failed:File error: /library/resume.mp3",
        "one line has to say the replay failed too, and why"
    );
    reset_resume_log();
}

#[test]
fn a_successful_resume_is_never_stamped_with_a_replay() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    // Opened for replay but the report says the resume itself succeeded —
    // the state cannot happen in production, and must not be forged.
    file_resume_report_opening_replay(sample_report(&track, ResumeStrategy::Unpause));
    note_replay_outcome(&track.id, Ok(()));
    assert!(resume_log_snapshot()[0].replay.is_empty());
    reset_resume_log();
}

// ---------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------

#[test]
fn a_resume_line_carries_the_facts_the_next_run_is_judged_on() {
    let track = sample_track();
    let mut report = sample_report(&track, ResumeStrategy::FreshSinkReplay);
    report.probe = VERDICT_NEVER_PLAYING;
    report.probe_detail = "polls=6 progress=0 empty=0".to_string();
    report.replay = "failed:File error: /library/resume.mp3".to_string();
    let line = format_resume_report(&report);
    for needle in [
        "ok",
        "strategy=fresh_sink_replay",
        "pre=paused",
        "file=ok:1.00MB",
        "pos=42.5s restored=n/a",
        "probe=never_playing",
        "progress=0",
        "replay=failed:File error: /library/resume.mp3",
        "Rësume <Test>",
    ] {
        assert!(line.contains(needle), "{needle} missing from {line}");
    }
}

#[test]
fn no_resume_attempts_means_no_log_block() {
    assert_eq!(render_resume_log_html(&[]), "");
}

#[test]
fn the_log_block_escapes_the_report_and_copies_the_whole_archive() {
    let track = sample_track();
    let mut reports = Vec::new();
    for i in 0..(RESUME_LOG_VISIBLE as u32 + 2) {
        let mut report = sample_report(&track, ResumeStrategy::Unpause);
        // A per-report marker, so "shown" and "copied" can be told apart.
        report.file_note = format!("ok:{i}.00MB");
        if i == 0 {
            report.probe = VERDICT_FAILED;
            report.detail = "<script>alert(1)</script>".to_string();
        }
        reports.push(report);
    }
    let html = render_resume_log_html(&reports);
    assert!(
        !html.contains("<script>"),
        "a title or detail must not inject markup"
    );
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("data-action=\"copy-resume-log\""));
    assert!(html.contains("navigator.clipboard"));
    assert!(html.contains("this.dataset.report"));
    // Reports arrive newest first, so index 0 is the newest of the 7.
    assert!(
        html.contains(&format!("newest {RESUME_LOG_VISIBLE} of 7")),
        "the title must say how much is hidden: {html}"
    );
    // Shown lines are in the visible window: newest (i=0) plus the next
    // RESUME_LOG_VISIBLE-1.
    let newest = reports[0].file_note.clone();
    assert_eq!(
        html.matches(&newest).count(),
        2,
        "the newest line is both rendered and copied"
    );
    let oldest_shown = reports[RESUME_LOG_VISIBLE - 1].file_note.clone();
    assert_eq!(html.matches(&oldest_shown).count(), 2);
    // Everything older is copied but not rendered: exactly one occurrence,
    // inside the copy payload.
    let hidden = reports[RESUME_LOG_VISIBLE].file_note.clone();
    assert_eq!(
        html.matches(&hidden).count(),
        1,
        "a hidden line must still be copied, not dropped"
    );
}

#[test]
fn the_queue_panel_shows_the_log_when_there_is_something_to_show() {
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let track = sample_track();
    let mut report = sample_report(&track, ResumeStrategy::FreshSinkReplay);
    report.probe = VERDICT_DRAINED_IMMEDIATE;
    file_resume_report(report);
    // `std::slice::from_ref`, not `&[track.clone()]`: the latter is what
    // `clippy::clone_on_copy` fires on, and CI's lint job runs
    // `-D warnings`, so it is a build failure rather than a suggestion.
    let html = render_queue_html(Some(&track), std::slice::from_ref(&track));
    assert!(html.contains("Resume log"), "the panel must carry the log");
    assert!(html.contains("drained_immediately"));
    reset_resume_log();
}

#[tokio::test]
async fn test_set_queue_performance_baseline() {
    let db_path = std::env::temp_dir().join(format!(
        "test_playback_set_queue_baseline_{}.db",
        Uuid::new_v4()
    ));
    let db = Database::new(&db_path).unwrap();
    db.run_migrations().unwrap();
    let db_arc = Arc::new(db);
    let tr_repo: Arc<dyn TrackRepository> = Arc::new(SqliteTrackRepository::new(db_arc.clone()));

    let mut track_ids = Vec::with_capacity(500);
    for i in 0..500 {
        let track = Track::new(
            format!("Song {}", i),
            format!("/music/song_{}.mp3", i),
            180,
            AudioFormat::Mp3,
        );
        tr_repo.insert(&track).await.unwrap();
        track_ids.push(track.id);
    }

    // Measure sequential lookup (N+1 queries - baseline)
    let start_seq = std::time::Instant::now();
    let mut seq_tracks = Vec::with_capacity(track_ids.len());
    for tid in &track_ids {
        match lookup_track(*tid, &db_arc).await {
            Ok(t) => seq_tracks.push(t),
            Err(e) => warn!(%tid, error=%e, "set_queue: skip missing track"),
        }
    }
    let elapsed_seq = start_seq.elapsed();

    // Measure batch lookup (IN clause - optimized)
    let start_batch = std::time::Instant::now();
    let batch_tracks = tr_repo.find_by_ids(&track_ids).await.unwrap();
    let elapsed_batch = start_batch.elapsed();

    assert_eq!(seq_tracks.len(), 500);
    assert_eq!(batch_tracks.len(), 500);
    for (i, t) in batch_tracks.iter().enumerate() {
        assert_eq!(t.id, track_ids[i]);
    }

    eprintln!("[BENCHMARK set_queue (500 tracks)]");
    eprintln!("  Baseline (N+1 queries)    : {:?}", elapsed_seq);
    eprintln!("  Optimized (batch IN query): {:?}", elapsed_batch);
    let speedup = elapsed_seq.as_secs_f64() / elapsed_batch.as_secs_f64();
    eprintln!("  Speedup: {:.2}x", speedup);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_set_queue_correctness_and_duplicates() {
    let db_path = std::env::temp_dir().join(format!(
        "test_playback_set_queue_correctness_{}.db",
        Uuid::new_v4()
    ));
    let db = Database::new(&db_path).unwrap();
    db.run_migrations().unwrap();
    let db_arc = Arc::new(db);
    let tr_repo: Arc<dyn TrackRepository> = Arc::new(SqliteTrackRepository::new(db_arc.clone()));

    let t1 = Track::new(
        "Song A".to_string(),
        "/music/a.mp3".to_string(),
        120,
        AudioFormat::Mp3,
    );
    let t2 = Track::new(
        "Song B".to_string(),
        "/music/b.mp3".to_string(),
        180,
        AudioFormat::Mp3,
    );
    tr_repo.insert(&t1).await.unwrap();
    tr_repo.insert(&t2).await.unwrap();

    let missing_id = Uuid::new_v4();
    let input_ids = vec![t2.id, t1.id, t2.id, missing_id, t1.id];

    let loaded = tr_repo.find_by_ids(&input_ids).await.unwrap();
    assert_eq!(loaded.len(), 4);
    assert_eq!(loaded[0].id, t2.id);
    assert_eq!(loaded[1].id, t1.id);
    assert_eq!(loaded[2].id, t2.id);
    assert_eq!(loaded[3].id, t1.id);

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_render_queue_html_empty() {
    // Serialised against the resume-log tests: the panel now carries the
    // log, and the log is process-global.
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let html = render_queue_html(None, &[]);
    assert!(html.contains("Playback Queue (0)"));
    assert!(html.contains("No tracks in queue"));
    assert!(!html.contains("Now Playing"));
    assert!(!html.contains("Clear Queue"));
    assert!(
        !html.contains("Resume log"),
        "no resume attempted, so no heading for one"
    );
}

#[test]
fn test_render_queue_html_with_tracks_and_escape() {
    // Same reason as `test_render_queue_html_empty`.
    let _guard = RESUME_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    reset_resume_log();
    let mut t_cur = Track::new(
        "Rock <&> Roll \"Live\" '26".to_string(),
        "/music/live.mp3".to_string(),
        205, // 3:25
        AudioFormat::Mp3,
    );
    t_cur.artist = Some("AC/<DC> & Friends".to_string());

    let mut t_next = Track::new(
        "Thunderstruck <script>alert(1)</script>".to_string(),
        "/music/thunder.mp3".to_string(),
        65, // 1:05
        AudioFormat::Mp3,
    );
    t_next.artist = Some("Artist & Co".to_string());

    let html = render_queue_html(Some(&t_cur), &[t_next.clone()]);
    assert!(html.contains("Playback Queue (1)"));
    assert!(html.contains("Next Up (1)"));
    assert!(html.contains("Now Playing"));
    assert!(html.contains("Rock &lt;&amp;&gt; Roll &quot;Live&quot; &#39;26"));
    assert!(html.contains("AC&#x2F;&lt;DC&gt; &amp; Friends"));
    assert!(html.contains("3:25"));
    assert!(html.contains("1:05"));
    assert!(html.contains("Thunderstruck &lt;script&gt;alert(1)&lt;&#x2F;script&gt;"));
    assert!(html.contains("Artist &amp; Co"));
    assert!(html.contains(&format!(
        "window.Auralis.bridge.removeFromQueue('{}')",
        t_next.id
    )));
    assert!(html.contains("window.Auralis.bridge.clearQueue()"));
}

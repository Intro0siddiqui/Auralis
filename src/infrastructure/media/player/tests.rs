use super::*;
use crate::domain::models::AudioFormat;

/// Publish the state a successful `play_track` leaves behind: track 0 of a
/// two-track queue is current, with a 4:26 length.
async fn seed_previous_state(player: &AudioPlayer) -> Track {
    let current = Track::new(
        "Previous".to_string(),
        "/library/previous.mp3".to_string(),
        266,
        AudioFormat::Mp3,
    );
    let queued = Track::new(
        "Queued".to_string(),
        "/library/queued.mp3".to_string(),
        180,
        AudioFormat::Mp3,
    );
    player.set_queue(vec![current.clone(), queued]).await;
    player.set_current_index(Some(0)).await;
    *player.current_track.write().await = Some(current.clone());
    *player.track_duration.write().await = Duration::from_secs(266);
    current
}

#[test]
fn reconcile_never_shortens_a_known_duration() {
    // The real case: container says 4:26, rodio says 1:32.
    let db = Duration::from_secs(266);
    let decoded = Duration::from_secs(92);
    assert_eq!(reconcile_duration(db, Some(decoded), None), db);
}

#[test]
fn reconcile_uses_the_decoder_when_the_library_has_nothing() {
    assert_eq!(
        reconcile_duration(Duration::ZERO, Some(Duration::from_secs(92)), None),
        Duration::from_secs(92)
    );
}

#[test]
fn reconcile_raises_the_duration_when_the_decoder_claims_more() {
    // A library row with a placeholder duration must still be corrected.
    assert_eq!(
        reconcile_duration(Duration::from_secs(30), Some(Duration::from_secs(92)), None),
        Duration::from_secs(92)
    );
}

#[test]
fn reconcile_trusts_the_container_downward_against_a_stale_library() {
    // `yF9nmg_jHNs` shape with a stale-long library row: the table says
    // 216.34 s, the decoder says 75 s, the library still claims 300 s.
    // `max()` would keep the wrong 300; the container corrects it down.
    assert_eq!(
        reconcile_duration(
            Duration::from_secs(300),
            Some(Duration::from_secs(75)),
            Some(Duration::from_secs_f64(216.34)),
        ),
        Duration::from_secs_f64(216.34)
    );
}

#[test]
fn reconcile_trusts_the_container_upward_from_an_unscanned_library() {
    // Same file, library value missing: the container corrects up past
    // what the decoder managed, and seeks are capped at the true length.
    assert_eq!(
        reconcile_duration(
            Duration::ZERO,
            Some(Duration::from_secs(75)),
            Some(Duration::from_secs_f64(216.34)),
        ),
        Duration::from_secs_f64(216.34)
    );
}

#[test]
fn reconcile_ignores_a_zero_container_reading() {
    // A zero table is absence of evidence, not a zero-length track.
    assert_eq!(
        reconcile_duration(
            Duration::from_secs(216),
            Some(Duration::from_secs(75)),
            Some(Duration::ZERO),
        ),
        Duration::from_secs(216)
    );
}

#[test]
fn reconcile_container_agrees_with_a_healthy_library() {
    // The common case for the fixture file: lofty stored 216 s, rodio
    // says 75 s, the table says 216.34 s. Answer 216 s either way.
    let with_container = reconcile_duration(
        Duration::from_secs(216),
        Some(Duration::from_secs(75)),
        Some(Duration::from_secs_f64(216.34)),
    );
    let without_container = reconcile_duration(
        Duration::from_secs(216),
        Some(Duration::from_secs(75)),
        None,
    );
    assert_eq!(with_container, Duration::from_secs_f64(216.34));
    assert_eq!(without_container, Duration::from_secs(216));
}

#[tokio::test]
async fn container_table_secs_abstains_on_an_unparseable_file() {
    // The arbitration input must fail safe: garbage in, `None` out, and
    // the caller keeps the reconciled answer — never a worse one.
    let path =
        std::env::temp_dir().join(format!("auralis_no_container_{}.bin", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"not an mp4 at all").expect("seed garbage file");
    let table = container_table_secs(path.to_str().expect("temp path is UTF-8")).await;
    assert_eq!(table, None);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn reconcile_keeps_the_library_value_without_a_decoder_opinion() {
    assert_eq!(
        reconcile_duration(Duration::from_secs(266), None, None),
        Duration::from_secs(266)
    );
}

// ---------------------------------------------------------------------
// Pause attribution
//
// rodio's pause flag is a bare `AtomicBool` on the `Player` with no cause
// attached, so "the sink is paused" and "a pause command was served" are
// the same statement to anything reading it. These are the two fields that
// pull them apart, and they are the reason a `never_playing (empty=0)`
// verdict stopped meaning "rodio refused to start the fresh sink".
// ---------------------------------------------------------------------

/// The baseline the fresh-sink replay arms its probe with: a player nobody
/// has paused. If this drifts, every resume would inherit a stale epoch
/// and the attribution would be a no-op.
#[tokio::test]
async fn a_fresh_player_has_served_no_pause() {
    let player = AudioPlayer::new().unwrap();
    assert_eq!(player.pause_epoch(), 0);
    assert_eq!(player.last_pause_at(), None);
}

/// `pause()` is recorded even when there is no sink to pause. That is the
/// point: the frontend's `PlayerController.pause()` fires
/// `invoke('pause')` without asking the backend anything first, so a pause
/// issued against a sink that is about to be replaced still has to leave a
/// trace — otherwise the resume that replaces it cannot tell it was
/// cancelled.
#[tokio::test]
async fn pause_is_recorded_even_with_no_sink_to_pause() {
    let player = AudioPlayer::new().unwrap();
    player.pause().await.expect("pause must not fail");
    assert_eq!(player.pause_epoch(), 1);
    assert!(
        player.last_pause_at().is_some(),
        "the instant is what turns 'a pause happened' into 'a pause happened 300ms in'"
    );

    player.pause().await.expect("pause must not fail");
    assert_eq!(
        player.pause_epoch(),
        2,
        "the counter has to be monotonic: a resume snapshots it and compares"
    );
}

/// The flag has to be cleared by a seek that did not rebuild, or a resume's
/// position restore would be reported as `restored=rebuilt` on the strength
/// of an earlier, unrelated seek. Headless-safe: a `seek` with no sink
/// errors before touching anything, which is exactly the case that has to
/// leave the flag clear.
#[tokio::test]
async fn a_failed_seek_does_not_claim_to_have_rebuilt_the_sink() {
    let player = AudioPlayer::new().unwrap();
    assert!(!player.take_seek_rebuilt_sink());
    // A rebuild recorded by some earlier seek, deliberately left unconsumed:
    // without it this test passes whether or not `seek` clears the flag,
    // because the flag starts out false either way.
    player.last_seek_rebuilt_sink.store(true, Ordering::SeqCst);
    player
        .seek(Duration::from_secs(7))
        .await
        .expect_err("there is no sink to seek");
    assert!(
        !player.take_seek_rebuilt_sink(),
        "a seek that never reached a sink must not leave a rebuild on record"
    );
}

/// Read-and-clear: two consecutive callers must not both be told the same
/// seek rebuilt the sink.
#[tokio::test]
async fn the_seek_rebuild_flag_is_consumed_by_reading_it() {
    let player = AudioPlayer::new().unwrap();
    player.last_seek_rebuilt_sink.store(true, Ordering::SeqCst);
    assert!(player.take_seek_rebuilt_sink());
    assert!(
        !player.take_seek_rebuilt_sink(),
        "the second reader must get its own answer"
    );
}

/// Headless-safe: `AudioPlayer::new()` opens no device (the output stream is
/// lazy), so a fresh player has `sink == None`. `resume()` must not report
/// success there, or the frontend waits forever for audio that never starts.
#[tokio::test]
async fn resume_without_a_sink_reports_an_error() {
    let player = AudioPlayer::new().unwrap();
    let err = player
        .resume()
        .await
        .expect_err("resume must not report success with no sink");
    assert!(
        matches!(&err, PlayerError::StateError(_)),
        "expected a StateError, got {err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains("nothing to resume"),
        "unexpected error message: {message}"
    );
    assert!(!player.is_playing().await);
}

/// `stop()` takes the sink out of the player but leaves `current_track`
/// set — the exact state that used to turn every later resume into a silent
/// no-op that reported success ("cannot play anything until restart").
#[tokio::test]
async fn resume_after_stop_reports_an_error() {
    let player = AudioPlayer::new().unwrap();
    player.stop().await.unwrap();
    let err = player
        .resume()
        .await
        .expect_err("resume must not report success after stop");
    assert!(
        matches!(&err, PlayerError::StateError(_)),
        "expected a StateError, got {err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains("nothing to resume"),
        "unexpected error message: {message}"
    );
    assert!(!player.is_playing().await);
}

/// Headless-safe: a missing file is rejected by `File::open`, long before the
/// output device is touched, so the whole "failed start" contract is
/// exercisable with no audio hardware.
///
/// This is the regression being pinned. `play_track` used to publish
/// `current_track` and `track_duration` *before* doing the fallible work, so a
/// failure returned `Err` while the player kept claiming the track that never
/// started — the player bar pointed at a song that was not playing, and
/// "next" walked on from a bogus position.
#[tokio::test]
async fn play_track_that_cannot_start_keeps_the_previous_track() {
    let player = AudioPlayer::new().unwrap();
    let previous = seed_previous_state(&player).await;
    let duration_before = player.duration().await;

    let incoming = Track::new(
        "Never Plays".to_string(),
        "/nonexistent/auralis/never-plays.mp3".to_string(),
        92,
        AudioFormat::Mp3,
    );
    let err = player
        .play_track(incoming)
        .await
        .expect_err("a missing file must not report success");
    assert!(
        matches!(&err, PlayerError::FileError(_)),
        "expected a FileError, got {err:?}"
    );

    let current = player
        .get_current_track()
        .await
        .expect("a failed start must not clear the current track");
    assert_eq!(
        current.id, previous.id,
        "current_track moved to a track that never started"
    );
    assert_eq!(current.title, previous.title);
    assert_eq!(
        current.duration_secs, previous.duration_secs,
        "the current track's own length was rewritten by a failed start"
    );
    assert_eq!(
        player.duration().await,
        duration_before,
        "track_duration was overwritten by a failed start"
    );
    assert_eq!(
        player.get_current_index().await,
        Some(0),
        "the queue index must not move on a failed start"
    );
    let queue = player.get_queue().await;
    assert_eq!(queue[0].id, previous.id);
    assert_eq!(
        queue[0].duration_secs, 266,
        "the queue entry was re-stamped by a failed start"
    );
    assert!(
        !player.is_playing().await,
        "a failed start must not leave a sink behind"
    );
}

/// The same guarantee at the *second* rejection point: the file is there, so
/// `File::open` succeeds, and `create_decoder` is what refuses it. Pins that
/// the rollback is structural (nothing is written until playback exists) and
/// not just an early `return` on the open.
#[tokio::test]
async fn play_track_on_an_undecodable_file_keeps_the_previous_track() {
    let dir = std::env::temp_dir().join(format!("auralis_test_play_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    let corrupt = dir.join("corrupt.m4a");
    std::fs::write(&corrupt, b"NOT_A_REAL_AUDIO_FILE").unwrap();

    let player = AudioPlayer::new().unwrap();
    let previous = seed_previous_state(&player).await;
    let duration_before = player.duration().await;

    let incoming = Track::new(
        "Corrupt".to_string(),
        corrupt.to_str().unwrap_or_default().to_string(),
        120,
        AudioFormat::M4a,
    );
    let err = player
        .play_track(incoming)
        .await
        .expect_err("an undecodable file must not report success");
    assert!(
        matches!(&err, PlayerError::DecodeError(_)),
        "expected a DecodeError, got {err:?}"
    );

    assert_eq!(
        player.get_current_track().await.map(|t| t.id),
        Some(previous.id),
        "current_track moved to a track that never started"
    );
    assert_eq!(player.duration().await, duration_before);
    assert_eq!(player.get_queue().await[0].duration_secs, 266);
    assert!(!player.is_playing().await);

    let _ = std::fs::remove_dir_all(dir);
}

/// The commit half, driven directly.
///
/// A real `start_sink` needs a rodio output device, so the *successful* start
/// path cannot be exercised headlessly and this test does not claim to: it
/// proves that the commit publishes every field `play_track` owns, in one
/// place, from an `EstablishedStart` and nothing else.
#[tokio::test]
async fn commit_start_publishes_the_established_track() {
    let player = AudioPlayer::new().unwrap();
    let incoming = Track::new(
        "Incoming".to_string(),
        "/library/incoming.mp3".to_string(),
        30,
        AudioFormat::Mp3,
    );
    player.set_queue(vec![incoming.clone()]).await;
    player.set_current_index(Some(0)).await;

    // A payload `start_sink` would have produced for a 30s library row whose
    // decoder claimed 92s: `reconcile_duration` keeps the larger value and the
    // commit re-stamps every published copy with it.
    let start = EstablishedStart {
        duration: Duration::from_secs(92),
        decoded: Some(Duration::from_secs(92)),
    };
    player.commit_start(Some(incoming.clone()), &start).await;

    assert_eq!(player.duration().await, Duration::from_secs(92));
    let current = player
        .get_current_track()
        .await
        .expect("the commit must publish the track");
    assert_eq!(current.id, incoming.id);
    assert_eq!(current.duration_secs, 92);
    let queue = player.get_queue().await;
    assert_eq!(
        queue[0].duration_secs, 92,
        "the queue entry at current_index must be re-stamped with the established length"
    );
}

/// With no decoder opinion there is nothing new to say about the length, so
/// the commit must not reach outside the track it was given. The queue entry
/// at `current_index` is a *copy* that can be some other track entirely — a
/// bare `play(path)` has no queue identity at all — and restamping it here
/// would be a write nobody asked for.
///
/// This test seeds the index as the previous track's rather than the
/// incoming one's, which is now a caller bug rather than a `next` /
/// `previous` state; it is kept because the guard it pins is exactly the
/// protection a bare-path commit needs. The ordering that keeps `next` /
/// `previous` out of this situation is pinned separately, by
/// `next_moves_the_queue_index_before_starting_the_track` and its two
/// siblings.
#[tokio::test]
async fn commit_start_without_a_decoder_opinion_leaves_the_queue_entry_alone() {
    let player = AudioPlayer::new().unwrap();
    let previous = seed_previous_state(&player).await;
    let incoming = Track::new(
        "Incoming".to_string(),
        "/library/incoming.mp3".to_string(),
        42,
        AudioFormat::Mp3,
    );

    let start = EstablishedStart {
        duration: Duration::from_secs(42),
        decoded: None,
    };
    player.commit_start(Some(incoming.clone()), &start).await;

    assert_eq!(player.duration().await, Duration::from_secs(42));
    let current = player
        .get_current_track()
        .await
        .expect("the commit must publish the track");
    assert_eq!(current.id, incoming.id);
    assert_eq!(
        current.duration_secs, 42,
        "with no decoder opinion the published track keeps the library's own length"
    );
    let queue = player.get_queue().await;
    assert_eq!(
        queue[0].id, previous.id,
        "the queue itself must be untouched"
    );
    assert_eq!(
        queue[0].duration_secs, 266,
        "the still-current queue entry must keep its own length"
    );
}

/// `play(path)` has no `Track` to publish, so its commit may only refresh the
/// length of what is already current — it must not invent an identity, and it
/// must not clear one either.
#[tokio::test]
async fn commit_start_without_a_track_refreshes_only_the_duration() {
    let player = AudioPlayer::new().unwrap();
    let previous = seed_previous_state(&player).await;

    let start = EstablishedStart {
        duration: Duration::from_secs(300),
        decoded: Some(Duration::from_secs(300)),
    };
    player.commit_start(None, &start).await;

    assert_eq!(player.duration().await, Duration::from_secs(300));
    let current = player
        .get_current_track()
        .await
        .expect("a bare-path commit must not clear the current track");
    assert_eq!(current.id, previous.id);
    assert_eq!(current.duration_secs, 300);
}

/// Attach a recorder for the queue index in effect at each start attempt, and
/// hand back the log.
///
/// This is the only seam that can show which entry a transition's commit
/// would re-stamp: a *successful* start needs a rodio output device, and a
/// failed one publishes nothing at all, so neither the committed state nor
/// the queue tells you what the index was while the start was in flight.
fn record_start_indices(player: &AudioPlayer) -> Arc<std::sync::Mutex<Vec<Option<usize>>>> {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    *player.start_observer.lock().expect("observer mutex") = Some(Arc::new(move |index| {
        recorder.lock().expect("recorder mutex").push(index);
    }));
    seen
}

/// The forward transition, and the regression this pins: `next` used to call
/// `play_track` first and move the index afterwards, so during the commit the
/// index still named the *outgoing* track. Its queue entry was re-stamped with
/// the incoming track's length and the track that ended up playing never
/// received its own repair — on every ordinary transition, auto-advance
/// included.
///
/// The start is made to fail (the queued track's path does not exist) so this
/// runs with no audio hardware; the observer reports the index at the moment
/// the start was attempted, which is the value the commit would have read.
/// The rollback is asserted in the same breath, because a failed start that
/// left the index forward would highlight one entry while the player bar
/// still showed another.
#[tokio::test]
async fn next_moves_the_queue_index_before_starting_the_track() {
    let player = AudioPlayer::new().unwrap();
    seed_previous_state(&player).await;
    let seen = record_start_indices(&player);

    let err = player
        .next_internal(false)
        .await
        .expect_err("the queued track's file does not exist, so the start must fail");
    assert!(
        matches!(&err, PlayerError::FileError(_)),
        "expected a FileError, got {err:?}"
    );

    assert_eq!(
        *seen.lock().unwrap(),
        vec![Some(1)],
        "next must move the queue index to the incoming track before starting it, \
             otherwise the commit re-stamps the outgoing track's entry"
    );
    assert_eq!(
        player.get_current_index().await,
        Some(0),
        "a failed start must leave the index on the track the player still describes"
    );
}

/// The same ordering where `next` wraps off the end of the queue to index 0.
/// Covered separately because the outgoing track is then the *last* entry: the
/// old ordering stamped the tail of the queue while the track actually
/// playing was the head, which the forward case cannot show.
#[tokio::test]
async fn next_wrapping_to_the_start_moves_the_index_before_starting_the_track() {
    let player = AudioPlayer::new().unwrap();
    seed_previous_state(&player).await;
    player.set_current_index(Some(1)).await;
    player.set_repeat_mode(RepeatMode::All).await;
    let seen = record_start_indices(&player);

    let err = player
        .next_internal(false)
        .await
        .expect_err("the wrapped-to track's file does not exist, so the start must fail");
    assert!(
        matches!(&err, PlayerError::FileError(_)),
        "expected a FileError, got {err:?}"
    );

    assert_eq!(
        *seen.lock().unwrap(),
        vec![Some(0)],
        "a wrapped next must point the index at the wrapped-to track before starting it"
    );
    assert_eq!(
        player.get_current_index().await,
        Some(1),
        "a failed start must leave the index on the track the player still describes"
    );
}

/// `previous` carried the identical bug and needs the identical ordering; its
/// rollback matters as much, since going back to an unplayable track would
/// otherwise leave the highlight on that track while the player bar showed
/// the one before it.
#[tokio::test]
async fn previous_moves_the_queue_index_before_starting_the_track() {
    let player = AudioPlayer::new().unwrap();
    seed_previous_state(&player).await;
    player.set_current_index(Some(1)).await;
    let seen = record_start_indices(&player);

    let err = player
        .previous()
        .await
        .expect_err("the previous track's file does not exist, so the start must fail");
    assert!(
        matches!(&err, PlayerError::FileError(_)),
        "expected a FileError, got {err:?}"
    );

    assert_eq!(
        *seen.lock().unwrap(),
        vec![Some(0)],
        "previous must move the queue index to the incoming track before starting it, \
             otherwise the commit re-stamps the outgoing track's entry"
    );
    assert_eq!(
        player.get_current_index().await,
        Some(1),
        "a failed start must leave the index on the track the player still describes"
    );
}

/// `previous` wrapping from the head of the queue to the tail under
/// `RepeatAll`, so the target index is neither the current one nor the
/// neighbouring one. Same ordering, same rollback, and the only remaining
/// combination of (method × wrap-around) that the three tests above leave
/// unpinned.
#[tokio::test]
async fn previous_wrapping_to_the_end_moves_the_index_before_starting_the_track() {
    let player = AudioPlayer::new().unwrap();
    seed_previous_state(&player).await;
    player.set_repeat_mode(RepeatMode::All).await;
    let seen = record_start_indices(&player);

    let err = player
        .previous()
        .await
        .expect_err("the wrapped-to track's file does not exist, so the start must fail");
    assert!(
        matches!(&err, PlayerError::FileError(_)),
        "expected a FileError, got {err:?}"
    );

    assert_eq!(
        *seen.lock().unwrap(),
        vec![Some(1)],
        "a wrapped previous must point the index at the wrapped-to track before starting it"
    );
    assert_eq!(
        player.get_current_index().await,
        Some(0),
        "a failed start must leave the index on the track the player still describes"
    );
}

#[tokio::test]
async fn test_next_shuffle_index_single_track() {
    let player = AudioPlayer::new().unwrap();
    assert_eq!(
        player.next_shuffle_index(1, Some(0), RepeatMode::Off).await,
        None
    );
    assert_eq!(
        player.next_shuffle_index(1, None, RepeatMode::Off).await,
        Some(0)
    );
    assert_eq!(
        player.next_shuffle_index(1, Some(0), RepeatMode::All).await,
        Some(0)
    );
}

#[tokio::test]
async fn test_next_shuffle_index_repeat_all() {
    let player = AudioPlayer::new().unwrap();
    let idx = player.next_shuffle_index(3, Some(0), RepeatMode::All).await;
    assert!(idx.is_some());
    assert_ne!(idx, Some(0));
    assert!(idx.unwrap() < 3);
}

#[tokio::test]
async fn test_next_shuffle_index_repeat_off_exhaustion() {
    let player = AudioPlayer::new().unwrap();
    {
        let mut hist = player.shuffle_history.write().await;
        hist.push(0);
        hist.push(1);
        hist.push(2);
    }
    assert_eq!(
        player.next_shuffle_index(3, Some(2), RepeatMode::Off).await,
        None
    );
}

#[tokio::test]
async fn test_record_shuffle_history() {
    let player = AudioPlayer::new().unwrap();
    player.record_shuffle_history(Some(0), 1).await;
    {
        let hist = player.shuffle_history.read().await;
        assert_eq!(*hist, vec![0, 1]);
    }

    // Recording the same next index should not duplicate it at the end
    player.record_shuffle_history(Some(1), 1).await;
    {
        let hist = player.shuffle_history.read().await;
        assert_eq!(*hist, vec![0, 1]);
    }

    // Recording a new index
    player.record_shuffle_history(Some(1), 2).await;
    {
        let hist = player.shuffle_history.read().await;
        assert_eq!(*hist, vec![0, 1, 2]);
    }
}

#[test]
fn test_create_decoder_extension_hint_and_fallback() {
    let dir = std::env::temp_dir().join(format!("auralis_test_decoder_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);

    // Generate 1s 8000Hz 8-bit mono WAV fixture
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

    let wav_path = dir.join("test_track.wav");
    std::fs::write(&wav_path, &data).unwrap();

    // 1. Test creation with explicit extension hint (.wav)
    let file_wav = File::open(&wav_path).unwrap();
    let dec_wav = create_decoder(file_wav, wav_path.to_str().unwrap());
    assert!(
        dec_wav.is_ok(),
        "Expected create_decoder to succeed with .wav extension hint"
    );

    // 2. Test fallback when extension hint is unknown/custom (.customext)
    let custom_path = dir.join("test_track.customext");
    std::fs::write(&custom_path, &data).unwrap();
    let file_custom = File::open(&custom_path).unwrap();
    let dec_custom = create_decoder(file_custom, custom_path.to_str().unwrap());
    assert!(
        dec_custom.is_ok(),
        "Expected create_decoder to succeed via fallback Decoder::new"
    );

    // 3. Test failure on corrupt audio content
    let corrupt_path = dir.join("corrupt.m4a");
    std::fs::write(&corrupt_path, b"NOT_A_REAL_AUDIO_FILE").unwrap();
    let file_corrupt = File::open(&corrupt_path).unwrap();
    let dec_corrupt = create_decoder(file_corrupt, corrupt_path.to_str().unwrap());
    assert!(
        dec_corrupt.is_err(),
        "Expected corrupt file to fail decoding"
    );

    // 4. Test real WebM Opus file (scratch/sample.m4a)
    let sample_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch/sample.m4a");
    if sample_path.exists() {
        let file_sample = File::open(&sample_path).unwrap();
        let dec_sample = create_decoder(file_sample, sample_path.to_str().unwrap());
        assert!(
            dec_sample.is_ok(),
            "Expected create_decoder to succeed on WebM Opus file"
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

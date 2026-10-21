//! Playback Commands
//!
//! Tauri command handlers for the audio playback domain.

use crate::commands::library::{format_time, html_escape, render_art_tag};
use crate::domain::models::{NowPlaying, RepeatMode, Track};
use crate::infrastructure::database::repositories::{parse_datetime, parse_format};
use crate::infrastructure::database::Database;
use crate::infrastructure::media::background_service;
use crate::infrastructure::media::AudioPlayer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Playback queue state
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaybackQueue {
    pub tracks: Vec<Track>,
    pub current_index: Option<usize>,
}

/// Seek request
#[derive(Debug, Serialize, Deserialize)]
pub struct SeekRequest {
    pub position_secs: u32,
}

/// Serialized `playback:progress` payload: current position and total
/// duration in seconds (fractional), matching the frontend listener at
/// `ui/js/player.js` (`data.position` / `data.duration`).
#[derive(Debug, Clone, Serialize)]
pub struct PlaybackProgress {
    pub position: f64,
    pub duration: f64,
}

/// Polling interval of the playback watcher while playing.
///
/// 250 ms is the only cadence the frontend needs — the progress bar is
/// event-driven and snaps optimistically on seek, so finer ticks would only
/// burn battery on Android with no visible benefit.
const WATCHER_INTERVAL: Duration = Duration::from_millis(250);

/// Polling interval while paused or idle.
///
/// The watcher has nothing to report then (`state_changed` events already
/// carry the frozen position), so it just sleeps — keeping the app at ~1
/// wakeup per 2 s instead of 4 per second when the user isn't listening.
const IDLE_INTERVAL: Duration = Duration::from_secs(2);

/// Epsilon for `position >= duration - epsilon` comparison when duration is
/// known. Covers scheduler jitter and the 250 ms watcher cadence.
const TRACK_END_EPSILON: Duration = Duration::from_millis(350);

/// Fallback guard for unknown-duration tracks (e.g., streams). Short enough
/// to allow < 1.5 s clips to advance, long enough to avoid the
/// just-appended empty transient.
const TRACK_END_MIN_GUARD: Duration = Duration::from_millis(300);

/// Spawn a background task that:
///
/// - emits `playback:progress` every [`WATCHER_INTERVAL`] **while playing**
///   (paused/idle sessions slow to [`IDLE_INTERVAL`] and emit nothing — the
///   pause/seek commands already report the frozen position),
/// - judges any resume armed by [`resume`] (see [`observe_resume_probe`]),
///   keeping the fast cadence for the length of that window, and
/// - auto-advances the queue (honoring repeat/shuffle) when the current
///   track finishes, emitting `playback:track_changed` + `playback:state_changed`.
pub fn spawn_playback_watcher(app: AppHandle, player: Arc<AudioPlayer>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(WATCHER_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut was_playing = false;
        loop {
            interval.tick().await;

            // Single atomic snapshot avoids TOCTOU between `is_playing` and
            // `is_sink_empty` (two separate `RwLock` reads could interleave
            // with a `play()` that swaps the sink).
            let (is_playing, is_empty) = player.sink_snapshot().await;

            if is_playing {
                let progress = PlaybackProgress {
                    position: player.current_position().await.as_secs_f64(),
                    duration: player.duration().await.as_secs_f64(),
                };
                let _ = app.emit("playback:progress", &progress);
            }

            // A resume in flight is being judged against this very tick: a sink
            // that is not playing emits no progress, so the cadence below is
            // what decides how much evidence the report gets. Closing the
            // window returns the finished report, if this was the last poll.
            //
            // The pause bookkeeping rides along on the same tick, because a
            // `pause` command that lands mid-window is the difference between
            // "rodio never started the fresh sink" and "something paused the
            // sink this resume had just built" — a distinction rodio itself
            // cannot express (see `attribute_pause`).
            if let Some(report) = observe_resume_probe(
                Instant::now(),
                is_playing,
                is_empty,
                PauseSample::of(&player),
            ) {
                warn!(
                    verdict = %report.probe,
                    detail = %report.probe_detail,
                    track = %report.title,
                    "Resume judged: the sink did not stay playable"
                );
                if verdict_is_failure(report.probe) {
                    // Release builds have no logcat, so this is the only place
                    // the owner finds out that a resume was accepted and then
                    // produced nothing. The full report is in the queue panel.
                    let msg = resume_failure_toast(&report);
                    let _ = app.emit("playback:error", &msg);
                }
            }

            // Track-end detection: prefer duration-vs-position when known
            // (handles < 1.5 s tracks that never exceed the old 1500 ms
            // guard), fall back to a short MIN_GUARD for unknown-duration
            // streams. This also covers the case where `duration` is zero
            // because metadata was missing.
            let track_just_ended = was_playing && is_empty && !player.is_recently_seeking() && {
                let dur = player.duration().await;
                let elapsed_opt = player.play_started_elapsed().await;
                if !dur.is_zero() {
                    let pos = player.current_position().await;
                    // Require elapsed_opt.is_some() to avoid false trigger after stop()
                    // zeroes position (0 >= 0 for dur <= 350ms would otherwise be true).
                    (elapsed_opt.is_some() && pos >= dur.saturating_sub(TRACK_END_EPSILON))
                        || (dur <= Duration::from_millis(1500)
                            && elapsed_opt.is_some_and(|e| {
                                e + TRACK_END_EPSILON >= dur
                                    // For short tracks, also accept any
                                    // elapsed beyond max(dur - epsilon, MIN_GUARD)
                                    // so a 800 ms clip can still advance.
                                    || e
                                        >= dur
                                            .saturating_sub(TRACK_END_EPSILON)
                                            .max(TRACK_END_MIN_GUARD)
                            }))
                } else {
                    // Unknown duration: require at least MIN_GUARD to filter
                    // the empty-transient after append, but allow sub-1500 ms
                    // clips to advance.
                    elapsed_opt.is_some_and(|e| e > TRACK_END_MIN_GUARD)
                }
            };

            // Truncated / buffer-underrun detection: sink went empty mid-track far from duration
            let truncated_stop = was_playing
                && !is_playing
                && is_empty
                && !track_just_ended
                && !player.is_recently_seeking()
                && {
                    let dur = player.duration().await;
                    let pos = player.current_position().await;
                    let elapsed = player
                        .play_started_elapsed()
                        .await
                        .unwrap_or(Duration::ZERO);
                    !dur.is_zero()
                        && pos + Duration::from_secs(5) < dur
                        && elapsed + Duration::from_secs(5) < dur
                };
            if truncated_stop {
                let dur = player.duration().await.as_secs();
                let pos = player.current_position().await.as_secs();
                warn!(
                    pos,
                    dur, "Playback stopped mid-track — likely truncated file or buffer underrun"
                );
                let msg = format!("Playback stopped at {pos}s of {dur}s — file may be truncated. Try re-downloading.");
                let _ = app.emit("playback:error", &msg);
                emit_state_changed(&app, &player).await;
            }

            if track_just_ended {
                info!("Current track ended; advancing playback");
                match player.next_for_auto_advance().await {
                    Ok(Some(track)) => {
                        emit_track_changed(&app, &player).await;
                        emit_state_changed(&app, &player).await;
                        background_service::push_now_playing(&player).await;
                        debug!(track_id = %track.id, "Auto-advanced to next track");
                    }
                    Ok(None) => {
                        emit_state_changed(&app, &player).await;
                        background_service::stop_service();
                        info!("Queue exhausted; playback stopped");
                    }
                    Err(e) => {
                        warn!(error = %e, "Auto-advance failed");
                        let msg = e.to_string();
                        let _ = app.emit("playback:error", &msg);
                        emit_state_changed(&app, &player).await;
                    }
                }
            }

            was_playing = is_playing;

            // Adaptive cadence: poll fast only while audio is actually
            // playing; slow to a heartbeat otherwise to save battery. A
            // pending resume probe counts as "playing": its whole purpose is to
            // sample a sink that may well *not* be playing, and at the idle
            // cadence a 1.5 s window would hold a single sample.
            let next = if is_playing || resume_probe_pending() {
                WATCHER_INTERVAL
            } else {
                IDLE_INTERVAL
            };
            interval.reset_after(next);
        }
    });
}

/// Start playback of a track (optionally via a queue index).
#[tauri::command]
pub async fn play(
    track_id: Uuid,
    queue_index: Option<usize>,
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    db: State<'_, Database>,
) -> Result<NowPlaying, String> {
    info!(%track_id, ?queue_index, "Play command received");

    // Look up track from database
    let track = lookup_track(track_id, &db)
        .await
        .map_err(|e| format!("Failed to look up track: {e}"))?;

    // If queue index is provided, set it — but remember what it was.
    //
    // It has to be set *before* `play_track`, because the commit step mirrors a
    // decoder-repaired duration onto the queue entry at `current_index`, and that
    // must be the entry for the track we are about to play.
    //
    // If playback then fails, `play_track` now leaves `current_track` describing
    // the *previous* track (it commits nothing until rodio has accepted a
    // source). Leaving the index pointing at the track that failed to start
    // would then highlight one entry in the queue while the player bar shows
    // another, so put the index back.
    let previous_index = player.get_current_index().await;
    if let Some(idx) = queue_index {
        player.set_current_index(Some(idx)).await;
    }

    let initial_dur = track.duration_secs;

    // Play the track — log full context so the UI can show exactly why it failed
    if let Err(e) = player.play_track(track.clone()).await {
        player.set_current_index(previous_index).await;
        warn!(%track_id, file_path=%track.file_path, title=%track.title, error=%e, "Playback failed — file missing or undecodable");
        // If this is the replay the frontend started after a rejected resume,
        // record the outcome on that resume's report: "resume failed, and the
        // replay failed too, because the file is gone" is the single most
        // useful line in the log and is otherwise split across two commands.
        note_replay_outcome(&track_id, Err(e.to_string()));
        return Err(format!(
            "Playback error [{} — {}]: {}",
            track.title, track.file_path, e
        ));
    }
    note_replay_outcome(&track_id, Ok(()));

    // If duration was auto-repaired during playback, persist to DB and reflect in NowPlaying
    let track = player.get_current_track().await.unwrap_or(track);
    if track.duration_secs != initial_dur {
        use crate::domain::repositories::TrackRepository;
        let repo = Arc::new(
            crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
                db.inner().clone(),
            )),
        );
        if let Err(e) = repo.update(&track).await {
            warn!(id = %track.id, error = %e, "Failed to persist auto-repaired duration in DB");
        } else {
            info!(id = %track.id, old_dur = initial_dur, new_dur = track.duration_secs, "Persisted auto-repaired duration in DB");
        }
    }

    // Build NowPlaying response
    let now_playing = NowPlaying {
        track,
        position_secs: 0,
        is_playing: player.is_playing().await,
        volume: player.get_volume().await,
        repeat_mode: player.get_repeat_mode().await,
        shuffle_enabled: player.get_shuffle().await,
    };

    emit_track_changed(&app, &player).await;
    emit_state_changed(&app, &player).await;
    background_service::push_now_playing(&player).await;
    background_service::request_notification_permission();

    debug!(%track_id, "Playback started");
    Ok(now_playing)
}

/// Request notification permissions (Android 13+ / API 33+).
#[tauri::command]
pub async fn request_notification_permission() -> Result<(), String> {
    background_service::request_notification_permission();
    Ok(())
}

/// Pause current playback.
#[tauri::command]
pub async fn pause(app: AppHandle, player: State<'_, AudioPlayer>) -> Result<(), String> {
    info!("Pause command received");

    player
        .pause()
        .await
        .map_err(|e| format!("Pause error: {e}"))?;

    emit_state_changed(&app, &player).await;
    background_service::push_now_playing(&player).await;

    debug!("Playback paused");
    Ok(())
}

/// Resume paused playback.
///
/// Three things happen here, and the second is the point of the rewrite:
///
/// 1. the pre-state is snapshotted and a strategy chosen (see
///    [`choose_resume_strategy`]) — the paused→playing transition is where this
///    bug lives, so which path ran has to be on the record;
/// 2. a *paused sink is never un-paused*. A fresh sink is built instead
///    ([`ResumeStrategy::FreshSinkReplay`]). See the section banner above for
///    what that is worth believing;
/// 3. a report is filed so the next device run is decisive. Release builds
///    write `tracing` to stdout and the owner has no logcat, so the report is
///    also rendered into the queue panel ("Resume log" + a copy button) and a
///    bad verdict raises a `playback:error` toast.
///
/// **Only one resume runs at a time** ([`RESUME_INFLIGHT`]). That is not a
/// throughput decision: two of them overlapping interleave inside
/// `play_track`, and each rebuild of the sink is three separate `.await`s —
/// `stop()`, the install, then the position restore. A second resume landing
/// in that window takes the sink out from under the first, and the first's
/// `seek` then finds no sink at all and reports `No active playback` for a
/// resume that was working. The device log shows exactly the situation this
/// guards — five resume attempts inside two seconds, four of them replacing
/// the one before — and it is also what makes `pre=paused` mean what the log
/// says it means: the state when *this* resume started, not when it happened
/// to be scheduled.
#[tauri::command]
pub async fn resume(app: AppHandle, player: State<'_, AudioPlayer>) -> Result<(), String> {
    info!("Resume command received");

    let _serialised = RESUME_INFLIGHT.lock().await;

    // One snapshot, before anything is touched: `is_playing` is
    // `!is_paused && !empty`, so `(false, false)` is exactly the paused-with-
    // live-source state the mitigation targets, and `(false, true)` is "no
    // usable source" (no sink at all, or a drained one — the two are not
    // distinguishable from outside `AudioPlayer`, so the report does not
    // pretend otherwise).
    let (is_playing, is_empty) = player.sink_snapshot().await;
    let track = player.get_current_track().await;
    let preflight = preflight_audio_file(track.as_ref()).await;
    let strategy = choose_resume_strategy(is_playing, !is_empty, preflight.readable);
    let pos_before = player.current_position().await;
    let mut report = ResumeReport::new(
        track.as_ref(),
        strategy,
        pre_state_str(is_playing, is_empty),
        preflight.note,
        pos_before.as_secs_f64(),
    );

    info!(
        strategy = %strategy.as_str(),
        pre = %report.pre_state,
        file = %report.file_note,
        "Resume strategy chosen"
    );

    let outcome = match strategy {
        ResumeStrategy::FreshSinkReplay => {
            replay_on_fresh_sink(&player, track.as_ref(), pos_before, &mut report).await
        }
        // `AlreadyPlaying` still goes through `player.resume()`: it no-ops on a
        // playing sink, and if the anchor were ever missing it re-arms it,
        // which is the pre-existing behaviour.
        ResumeStrategy::AlreadyPlaying | ResumeStrategy::Unpause => {
            player.resume().await.map_err(|e| e.to_string())
        }
    };

    match outcome {
        Ok(()) => {
            report.result = RESULT_OK;
            if strategy == ResumeStrategy::AlreadyPlaying {
                // Nothing was resumed, so there is nothing to watch (the report
                // already carries `not_probed`).
                file_resume_report(report);
            } else {
                // The watcher fills in the verdict once the window closes. The
                // pause epoch is sampled *after* the rebuild, so a `pause` that
                // cancels this attempt is inside the probe's window and gets
                // attributed to it (see `attribute_pause`).
                arm_resume_probe(report, player.pause_epoch());
            }
            emit_state_changed(&app, &player).await;
            background_service::push_now_playing(&player).await;
            debug!(strategy = %strategy.as_str(), "Playback resumed");
            Ok(())
        }
        Err(e) => {
            report.result = RESULT_ERROR;
            report.probe = VERDICT_FAILED;
            report.detail = e.to_string();
            // The frontend answers every rejected resume by replaying the track
            // through `play`; that call stamps its outcome onto this report
            // (see `note_replay_outcome`) so one log line says whether the
            // replay worked and, if not, why.
            file_resume_report_opening_replay(report);
            warn!(
                strategy = %strategy.as_str(),
                pre = %pre_state_str(is_playing, is_empty),
                error = %e,
                "Resume failed — see the queue panel's Resume log"
            );
            Err(format!("Resume error: {e}"))
        }
    }
}

/// Stop playback and clear the queue.
#[tauri::command]
pub async fn stop(app: AppHandle, player: State<'_, AudioPlayer>) -> Result<(), String> {
    info!("Stop command received");

    player
        .stop()
        .await
        .map_err(|e| format!("Stop error: {e}"))?;

    emit_state_changed(&app, &player).await;
    background_service::stop_service();

    debug!("Playback stopped");
    Ok(())
}

/// Skip to next track in queue.
#[tauri::command]
pub async fn next_track(
    app: AppHandle,
    player: State<'_, AudioPlayer>,
) -> Result<Option<NowPlaying>, String> {
    info!("Next track command received");

    let track = player
        .next()
        .await
        .map_err(|e| format!("Next track error: {e}"))?;

    match track {
        Some(t) => {
            emit_track_changed(&app, &player).await;
            emit_state_changed(&app, &player).await;
            background_service::push_now_playing(&player).await;
            let now_playing = build_now_playing(&player, &t).await;
            debug!(%t.id, "Now playing next track");
            Ok(Some(now_playing))
        }
        None => {
            emit_state_changed(&app, &player).await;
            background_service::stop_service();
            info!("Reached end of queue");
            Ok(None)
        }
    }
}

/// Go back to previous track.
#[tauri::command]
pub async fn previous_track(
    app: AppHandle,
    player: State<'_, AudioPlayer>,
) -> Result<Option<NowPlaying>, String> {
    info!("Previous track command received");

    let track = player
        .previous()
        .await
        .map_err(|e| format!("Previous track error: {e}"))?;

    match track {
        Some(t) => {
            emit_track_changed(&app, &player).await;
            emit_state_changed(&app, &player).await;
            background_service::push_now_playing(&player).await;
            let now_playing = build_now_playing(&player, &t).await;
            debug!(%t.id, "Now playing previous track");
            Ok(Some(now_playing))
        }
        None => {
            info!("At beginning of queue");
            Ok(None)
        }
    }
}

/// Seek to a position within the current track.
#[tauri::command]
pub async fn seek(
    request: SeekRequest,
    app: AppHandle,
    player: State<'_, AudioPlayer>,
) -> Result<(), String> {
    info!(
        position_secs = request.position_secs,
        "Seek command received"
    );

    let position = Duration::from_secs(request.position_secs as u64);

    player
        .seek(position)
        .await
        .map_err(|e| format!("Seek error: {e}"))?;

    emit_state_changed(&app, &player).await;
    background_service::push_now_playing(&player).await;

    debug!(position_secs = request.position_secs, "Seek completed");
    Ok(())
}

/// Set the output volume (0.0..=1.0).
#[tauri::command]
pub async fn set_volume(volume: f32, player: State<'_, AudioPlayer>) -> Result<(), String> {
    if !(0.0..=1.0).contains(&volume) {
        return Err("Volume must be between 0.0 and 1.0".to_string());
    }

    info!(volume, "Set volume command received");

    player
        .set_volume(volume)
        .await
        .map_err(|e| format!("Set volume error: {e}"))?;

    debug!(volume, "Volume set");
    Ok(())
}

/// Set the repeat mode.
#[tauri::command]
pub async fn set_repeat_mode(
    mode: RepeatMode,
    player: State<'_, AudioPlayer>,
) -> Result<(), String> {
    info!(?mode, "Set repeat mode command received");

    player.set_repeat_mode(mode).await;

    debug!(?mode, "Repeat mode set");
    Ok(())
}

/// Toggle shuffle mode.
#[tauri::command]
pub async fn set_shuffle(enabled: bool, player: State<'_, AudioPlayer>) -> Result<(), String> {
    info!(enabled, "Set shuffle command received");

    player.set_shuffle(enabled).await;

    debug!(enabled, "Shuffle mode set");
    Ok(())
}

/// Get current now-playing state.
#[tauri::command]
pub async fn get_now_playing(player: State<'_, AudioPlayer>) -> Result<Option<NowPlaying>, String> {
    debug!("Get now-playing command received");

    match player.get_current_track().await {
        Some(track) => {
            let now_playing = build_now_playing(&player, &track).await;
            Ok(Some(now_playing))
        }
        None => Ok(None),
    }
}

/// Get the current playback queue.
#[tauri::command]
pub async fn get_queue(player: State<'_, AudioPlayer>) -> Result<PlaybackQueue, String> {
    debug!("Get queue command received");

    let tracks = player.get_queue().await;
    let current_index = player.get_current_index().await;

    Ok(PlaybackQueue {
        tracks,
        current_index,
    })
}

/// Append a track to the queue.
#[tauri::command]
pub async fn add_to_queue(
    track_id: Uuid,
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    db: State<'_, Database>,
) -> Result<PlaybackQueue, String> {
    info!(%track_id, "Add to queue command received");

    let track = lookup_track(track_id, &db)
        .await
        .map_err(|e| format!("Failed to look up track: {e}"))?;

    player.add_to_queue(track).await;

    let queue = build_queue(&player).await;
    emit_queue_updated(&app, &queue).await;

    debug!(%track_id, "Track added to queue");
    Ok(queue)
}

/// Insert a track right after the currently playing track in the queue.
#[tauri::command(rename_all = "camelCase")]
pub async fn play_next(
    track_id: Uuid,
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    db: State<'_, Database>,
) -> Result<PlaybackQueue, String> {
    info!(%track_id, "Play next command received");

    let track = lookup_track(track_id, &db)
        .await
        .map_err(|e| format!("Failed to look up track: {e}"))?;

    player.play_next(track).await;

    let queue = build_queue(&player).await;
    emit_queue_updated(&app, &queue).await;

    debug!(%track_id, "Track inserted as next in queue");
    Ok(queue)
}

/// Remove the track at the given queue index.
#[tauri::command]
pub async fn remove_from_queue(
    index: usize,
    app: AppHandle,
    player: State<'_, AudioPlayer>,
) -> Result<PlaybackQueue, String> {
    info!(index, "Remove from queue command received");

    player
        .remove_from_queue(index)
        .await
        .map_err(|e| format!("Remove from queue error: {e}"))?;

    let queue = build_queue(&player).await;
    emit_queue_updated(&app, &queue).await;

    debug!(index, "Track removed from queue");
    Ok(queue)
}

/// Clear the playback queue.
#[tauri::command]
pub async fn clear_queue(
    app: AppHandle,
    player: State<'_, AudioPlayer>,
) -> Result<PlaybackQueue, String> {
    info!("Clear queue command received");

    player.clear_queue().await;

    let queue = build_queue(&player).await;
    emit_queue_updated(&app, &queue).await;

    Ok(queue)
}

/// Replace the playback queue wholesale (used to make Next/Prev context-aware
/// when playing from Library/Home/Playlist). Sets queue = tracks, current_index
/// = index of current track (or 0 if id not found). Called by JS before `play`.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_queue(
    track_ids: Vec<Uuid>,
    current_id: Option<Uuid>,
    app: AppHandle,
    player: State<'_, AudioPlayer>,
    db: State<'_, Database>,
) -> Result<PlaybackQueue, String> {
    info!(
        count = track_ids.len(),
        ?current_id,
        "Set queue command received"
    );
    use crate::domain::repositories::TrackRepository;
    let repo = crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
        db.inner().clone(),
    ));
    let tracks = repo
        .find_by_ids(&track_ids)
        .await
        .map_err(|e| format!("set_queue: failed to query tracks: {e}"))?;

    if tracks.is_empty() {
        return Err("set_queue: no valid tracks".into());
    }
    let idx = current_id
        .and_then(|cid| tracks.iter().position(|t| t.id == cid))
        .or(Some(0));
    player.set_queue(tracks.clone()).await;
    player.set_current_index(idx).await;
    let queue = PlaybackQueue {
        tracks,
        current_index: idx,
    };
    emit_queue_updated(&app, &queue).await;
    Ok(queue)
}

/// Pre-rendered queue HTML for the queue panel drawer.
#[tauri::command]
pub async fn get_queue_html(
    player: State<'_, AudioPlayer>,
    db: State<'_, Database>,
) -> Result<String, String> {
    let current_track = player.get_current_track().await;
    let queue_tracks = player.get_queue().await;

    use crate::domain::repositories::TrackRepository;
    let repo = crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
        db.inner().clone(),
    ));

    let mut ids_to_fetch = Vec::new();
    if let Some(ref cur) = current_track {
        ids_to_fetch.push(cur.id);
    }
    for t in &queue_tracks {
        ids_to_fetch.push(t.id);
    }

    let fetched = if !ids_to_fetch.is_empty() {
        repo.find_by_ids(&ids_to_fetch).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let track_map: std::collections::HashMap<Uuid, Track> =
        fetched.into_iter().map(|t| (t.id, t)).collect();

    let resolved_current = current_track.map(|cur| track_map.get(&cur.id).cloned().unwrap_or(cur));

    let resolved_queue: Vec<Track> = queue_tracks
        .into_iter()
        .map(|t| track_map.get(&t.id).cloned().unwrap_or(t))
        .collect();

    Ok(render_queue_html(
        resolved_current.as_ref(),
        &resolved_queue,
    ))
}

/// Render pre-rendered queue HTML from current track and queue tracks slice.
pub(crate) fn render_queue_html(current_track: Option<&Track>, queue_tracks: &[Track]) -> String {
    let queue_count = queue_tracks.len();

    let mut html = String::with_capacity(2048);
    html.push_str(
        r#"<div style="padding: var(--space-4); height: 100%; display: flex; flex-direction: column;">"#,
    );

    // Header
    html.push_str(
        r#"<div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: var(--space-4);"><div style="display: flex; align-items: center; gap: var(--space-2);"><h3 style="font-size: var(--text-lg); font-weight: var(--font-semibold); color: var(--text-1); margin: 0;">Playback Queue ("#,
    );
    html.push_str(&queue_count.to_string());
    html.push_str(
        r#")</h3></div><div style="display: flex; align-items: center; gap: var(--space-2);"><button type="button" class="btn btn-ghost btn-icon btn-sm" style="padding: var(--space-1);" onclick="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.player &amp;&amp; (window.Auralis.player.toggleFullScreenQueue &amp;&amp; window.Auralis.player.toggleFullScreenQueue(false), window.Auralis.player.toggleQueue &amp;&amp; window.Auralis.player.toggleQueue(false))" title="Close"><i data-lucide="x"></i></button></div></div>"#,
    );

    html.push_str(r#"<div class="queue-content queue-body" style="flex: 1; overflow-y: auto;">"#);

    // Now Playing card
    if let Some(track) = current_track {
        let safe_title = html_escape(&track.title);
        let artist_str = track.artist.as_deref().unwrap_or("Unknown Artist");
        let safe_artist = html_escape(artist_str);
        let dur_str = format_time(track.duration_secs);
        let art_html = render_art_tag(track.album_art_path.as_deref(), &track.title, "disc-3");

        html.push_str(&format!(
            r#"<div style="font-size: var(--text-xs); color: var(--text-3); text-transform: uppercase; margin-bottom: var(--space-2); font-weight: var(--font-semibold);">Now Playing</div><div class="track-row neu-glass" style="margin-bottom: var(--space-4); border-radius: var(--radius-md); padding: var(--space-2) var(--space-3); display: flex; align-items: center; gap: var(--space-3);"><div class="track-row-artwork" style="width: 40px; height: 40px; border-radius: var(--radius-sm); overflow: hidden; display: flex; align-items: center; justify-content: center; background: var(--glass-base); flex-shrink: 0;">{art_html}</div><div class="track-row-info" style="flex: 1; min-width: 0;"><div class="track-row-title" style="color: var(--accent); font-weight: var(--font-semibold); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">{safe_title}</div><div class="track-row-subtitle" style="white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">{safe_artist}</div></div><span class="track-row-duration" style="font-size: var(--text-xs); color: var(--text-3); flex-shrink: 0;">{dur_str}</span></div>"#
        ));
    }

    // Upcoming list header
    html.push_str(&format!(
        r#"<div style="font-size: var(--text-xs); color: var(--text-3); text-transform: uppercase; margin-bottom: var(--space-2); font-weight: var(--font-semibold);">Next Up ({queue_count})</div>"#
    ));

    if queue_tracks.is_empty() {
        html.push_str(
            r#"<div class="empty-state glass neu" style="padding: var(--space-4); text-align: center; border-radius: var(--radius-md);"><p style="color: var(--text-3); font-size: var(--text-xs); margin: 0;">No tracks in queue</p></div>"#,
        );
    } else {
        for track in queue_tracks {
            let safe_id = html_escape(&track.id.to_string());
            let safe_title = html_escape(&track.title);
            let artist_str = track.artist.as_deref().unwrap_or("Unknown Artist");
            let safe_artist = html_escape(artist_str);
            let dur_str = format_time(track.duration_secs);
            let art_html = render_art_tag(track.album_art_path.as_deref(), &track.title, "music");

            html.push_str(&format!(
                r#"<div class="queue-track-row glass-weak neu-glass" data-track-id="{safe_id}" style="margin-bottom: var(--space-2); border-radius: var(--radius-md); padding: var(--space-2) var(--space-3); display: flex; justify-content: space-between; align-items: center; cursor: pointer; touch-action: manipulation;" onclick="window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.playTrack('{safe_id}')"><div class="track-row-artwork" style="width: 36px; height: 36px; border-radius: var(--radius-sm); overflow: hidden; display: flex; align-items: center; justify-content: center; background: var(--glass-base); flex-shrink: 0; margin-right: var(--space-3);">{art_html}</div><div class="track-row-info" style="flex: 1; min-width: 0; overflow: hidden;"><div class="track-row-title" style="white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: var(--text-sm);">{safe_title}</div><div class="track-row-subtitle" style="white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: var(--text-xs); color: var(--text-3);">{safe_artist}</div></div><span class="track-row-duration" style="font-size: var(--text-xs); color: var(--text-3); margin-left: var(--space-2); margin-right: var(--space-2); flex-shrink: 0;">{dur_str}</span><button type="button" class="btn btn-ghost btn-icon btn-sm" onclick="event.stopPropagation(); window.Auralis.bridge.removeFromQueue('{safe_id}')" ontouchend="event.stopPropagation(); window.Auralis.bridge.removeFromQueue('{safe_id}')" title="Remove from queue" style="flex-shrink: 0;"><i data-lucide="trash-2"></i></button></div>"#
            ));
        }
    }

    html.push_str("</div>");

    // Resume diagnostics, newest first. This is the only surface the owner can
    // read without a debugger: release builds send `tracing` to stdout and
    // there is no logcat, and the frontend (which owns the download-side
    // "Copy report" button) is not this change's file. `get_queue_html` is the
    // one panel the backend renders, and it is re-fetched every time the queue
    // is opened, so the report is always the current one.
    html.push_str(&render_resume_log_html(&resume_log_snapshot()));

    if queue_count > 0 {
        html.push_str(
            r#"<div style="padding-top: var(--space-3); border-top: 1px solid var(--glass-border); display: flex; gap: var(--space-2);"><button type="button" class="btn btn-secondary btn-sm neu" style="width: 100%; justify-content: center;" onclick="window.Auralis.bridge.clearQueue()"><i data-lucide="trash-2"></i> Clear Queue</button></div>"#,
        );
    }

    html.push_str("</div>");

    html
}

// ============================================================================
// Helper functions
// ============================================================================

async fn lookup_track(track_id: Uuid, db: &Database) -> Result<Track, Box<dyn std::error::Error>> {
    let conn = db
        .connection()
        .map_err(|e| format!("Database connection error: {e}"))?;

    let mut stmt = conn
        .prepare("SELECT * FROM tracks WHERE id = ?")
        .map_err(|e| format!("Query preparation error: {e}"))?;

    let track = stmt
        .query_row([track_id.to_string()], |row| {
            Ok(crate::domain::models::Track {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                album_artist: row.get(4)?,
                genre: row.get(5)?,
                year: row.get(6)?,
                track_number: row.get(7)?,
                disc_number: row.get(8)?,
                duration_secs: row.get(9)?,
                file_path: row.get(10)?,
                file_size: row.get::<_, i64>(11)? as u64,
                format: parse_format(&row.get::<_, String>(12)?),
                bitrate: row.get(13)?,
                sample_rate: row.get(14)?,
                album_art_path: row.get(15)?,
                date_added: parse_datetime(&row.get::<_, String>(16)?),
                last_played: row
                    .get::<_, Option<String>>(17)?
                    .as_deref()
                    .map(parse_datetime),
                play_count: row.get(18)?,
                is_downloaded: row.get::<_, i32>(19)? != 0,
                source_url: row.get(20)?,
                is_favorite: row
                    .get::<_, Option<i32>>(21)
                    .unwrap_or_default()
                    .unwrap_or(0)
                    != 0,
                mtime: row
                    .get::<_, Option<i64>>(22)
                    .unwrap_or_default()
                    .unwrap_or(0),
            })
        })
        .map_err(|e| format!("Track lookup error: {e}"))?;

    Ok(track)
}

async fn build_now_playing(player: &AudioPlayer, track: &Track) -> NowPlaying {
    NowPlaying {
        track: track.clone(),
        position_secs: player.current_position().await.as_secs() as u32,
        is_playing: player.is_playing().await,
        volume: player.get_volume().await,
        repeat_mode: player.get_repeat_mode().await,
        shuffle_enabled: player.get_shuffle().await,
    }
}

/// Build the current queue snapshot for emission to the frontend.
async fn build_queue(player: &AudioPlayer) -> PlaybackQueue {
    PlaybackQueue {
        tracks: player.get_queue().await,
        current_index: player.get_current_index().await,
    }
}

/// Notify the frontend of a playback-state change (play/pause/stop/seek).
pub(crate) async fn emit_state_changed(app: &AppHandle, player: &AudioPlayer) {
    if let Some(track) = player.get_current_track().await {
        let now_playing = build_now_playing(player, &track).await;
        let _ = app.emit("playback:state_changed", &now_playing);
    }
}

/// Notify the frontend that the currently playing track changed.
pub(crate) async fn emit_track_changed(app: &AppHandle, player: &AudioPlayer) {
    if let Some(track) = player.get_current_track().await {
        let _ = app.emit("playback:track_changed", &track);
    }
}

/// Notify the frontend that the playback queue changed.
async fn emit_queue_updated(app: &AppHandle, queue: &PlaybackQueue) {
    let _ = app.emit("playback:queue_updated", queue);
}

pub mod diagnostics;
pub use diagnostics::*;

#[cfg(test)]
mod tests;

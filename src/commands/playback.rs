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
use std::sync::{Arc, Mutex, MutexGuard};
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
            if let Some(report) = observe_resume_probe(Instant::now(), is_playing, is_empty) {
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
            let track_just_ended = was_playing && !is_playing && is_empty && {
                let dur = player.duration().await;
                let elapsed_opt = player.play_started_elapsed().await;
                if !dur.is_zero() {
                    let pos = player.current_position().await;
                    // Require elapsed_opt.is_some() to avoid false trigger after stop()
                    // zeroes position (0 >= 0 for dur <= 350ms would otherwise be true).
                    (elapsed_opt.is_some() && pos >= dur.saturating_sub(TRACK_END_EPSILON))
                        || elapsed_opt.is_some_and(|e| {
                            e + TRACK_END_EPSILON >= dur
                                    // For short tracks, also accept any
                                    // elapsed beyond max(dur - epsilon, MIN_GUARD)
                                    // so a 800 ms clip can still advance.
                                    || e
                                        >= dur
                                            .saturating_sub(TRACK_END_EPSILON)
                                            .max(TRACK_END_MIN_GUARD)
                        })
                } else {
                    // Unknown duration: require at least MIN_GUARD to filter
                    // the empty-transient after append, but allow sub-1500 ms
                    // clips to advance.
                    elapsed_opt.is_some_and(|e| e > TRACK_END_MIN_GUARD)
                }
            };

            // Truncated / buffer-underrun detection: sink went empty mid-track far from duration
            let truncated_stop = was_playing && !is_playing && is_empty && !track_just_ended && {
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
#[tauri::command]
pub async fn resume(app: AppHandle, player: State<'_, AudioPlayer>) -> Result<(), String> {
    info!("Resume command received");

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
                // The watcher fills in the verdict once the window closes.
                arm_resume_probe(report);
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

// ============================================================================
// Pause / resume: mitigation + diagnostics
// ============================================================================
//
// Symptom, live since v2.6.16 and the owner's top complaint after downloads:
// pause, resume, and nothing plays — sometimes with the bar still claiming to
// be playing.
//
// Already ruled out, and deliberately not re-litigated here: the 250 ms watcher,
// the frontend's 700 ms proof-of-life watch, `start_sink`, auto-advance via
// `sink.empty()`, and sink replacement. Nothing in our own code runs at 1 ms.
//
// THE MITIGATION is a candidate, not a diagnosed fix. The only asymmetry
// anyone could find in rodio's paused -> playing transition is inside
// `Pausable`: `set_paused(false)` clears `paused_channels` but leaves
// `remaining_paused_samples` set, so the first samples after an unpause can
// still be silence. That predicts a short silent lead-in, not permanent
// silence, so it may not be the whole story — and no comment here should be
// read as a root-cause claim until the owner's exact observed symptom is in
// hand. What the fresh-sink path *does* buy is a decisive next run: a new sink
// cannot inherit state from the paused one, so if the replay plays and the
// unpause does not, that transition is the differentiator.
//
// THE DIAGNOSTICS exist because the failure is silent in both directions. A
// successful `resume` is indistinguishable from a resume that produced no
// audio, and `tracing` goes to stdout, which a release Android build gives the
// owner no way to read. So every attempt files a report, the watcher judges it
// against its own ticks, and the report is rendered into the queue panel with a
// copy button — the same shape as the download row's "Copy report".
//
// The five shapes the log is built to separate:
//
// | verdict                | meaning                                              |
// |------------------------|------------------------------------------------------|
// | `playing_confirmed`    | the sink reported playing; silence here is downstream of our state |
// | `drained_immediately`  | nothing ever played and the source was already drained |
// | `played_then_drained`  | audio started and was consumed before the window closed |
// | `never_playing`        | the sink has a live source but rodio never left `is_paused` |
// | `no_observation`       | the watcher did not sample inside the window (task starved) |
//
// `playing_confirmed` versus `never_playing` is the pair that matters: the
// first clears this code and the driver/`Pausable` transition, the second
// means the unpause itself did nothing. `progress=N` in the detail line is the
// number of `playback:progress` events the run really emitted, so "no audio"
// can be told apart from "no events reached the frontend" without guessing.

/// How many resume attempts the log keeps. Matches the download side's
/// `window.__auralisClientReports` (last 20), so the evidence for a failure
/// sits in the same place whichever failure is being chased.
const RESUME_LOG_CAPACITY: usize = 20;

/// How many of those the queue panel shows. The copy button carries all of
/// them; on-screen text stays short enough to read on a phone.
const RESUME_LOG_VISIBLE: usize = 5;

/// How long the watcher samples a resumed sink before judging it.
///
/// Long enough for several [`WATCHER_INTERVAL`] ticks — "never playing" should
/// be several independent samples, not one — and short enough that the log
/// line still describes the moment the user pressed play.
const RESUME_PROBE_WINDOW: Duration = Duration::from_millis(1500);

/// How long a rejected resume waits for the frontend's replay to be reported
/// back onto its report. The frontend replays within milliseconds of the
/// rejection arriving.
const RESUME_REPLAY_WINDOW: Duration = Duration::from_secs(10);

/// Paused positions shorter than this are not restored after a fresh-sink
/// replay: the seek would cost more than the fraction of a second it saves.
const POSITION_RESTORE_FLOOR: Duration = Duration::from_millis(500);

/// The sink is running and kept running: the resume worked as far as our state
/// is concerned.
const VERDICT_CONFIRMED: &str = "playing_confirmed";
/// No sample ever saw it playing, and the source was already drained.
const VERDICT_DRAINED_IMMEDIATE: &str = "drained_immediately";
/// It played and was consumed inside the window.
const VERDICT_PLAYED_THEN_DRAINED: &str = "played_then_drained";
/// A live source is queued, but rodio never left the paused state.
const VERDICT_NEVER_PLAYING: &str = "never_playing";
/// The watcher sampled nothing: the task is starved, which is its own bug.
const VERDICT_NO_OBSERVATION: &str = "no_observation";
/// A newer resume replaced this one before its window closed.
const VERDICT_SUPERSEDED: &str = "superseded";
/// Nothing was resumed (already playing), so there was nothing to watch.
const VERDICT_NOT_PROBED: &str = "not_probed";
/// The command itself failed; there is no sink to judge.
const VERDICT_FAILED: &str = "failed";

/// `result` values. `pending` is only ever a transient initial value: a report
/// is filed with its result already set, which is why
/// [`note_replay_outcome`] matches on `error` rather than on "not ok".
const RESULT_PENDING: &str = "pending";
const RESULT_OK: &str = "ok";
const RESULT_ERROR: &str = "error";

/// Pre-state labels, from the one atomic `sink_snapshot()`.
const PRE_PLAYING: &str = "playing";
const PRE_PAUSED: &str = "paused";
/// No sink at all, or a drained one. The two are indistinguishable from
/// outside `AudioPlayer` (`is_sink_empty()` answers `true` for both), so the
/// label does not pretend to know which.
const PRE_NO_SOURCE: &str = "no_source";

/// The `onclick` for the log's copy button.
///
/// A const rather than an inline literal so rustfmt is never asked to break it:
/// rustfmt abandons an item containing a line it cannot wrap, and that is how
/// real misformatting hides from `cargo fmt --check`. Single quotes only — the
/// string is interpolated into a double-quoted HTML attribute.
const RESUME_LOG_COPY_JS: &str =
    "if(navigator.clipboard){var b=window.Auralis&&window.Auralis.bridge;\
navigator.clipboard.writeText(this.dataset.report)\
.then(function(){b&&b.showToast&&b.showToast('Resume log copied','success');})\
.catch(function(){b&&b.showToast&&b.showToast('Copy failed','error');});}";

const RESUME_LOG_OPEN: &str = "<div class=\"resume-log\" style=\"margin-top: var(--space-3); \
border-top: 1px solid var(--glass-border); padding-top: var(--space-3);\">";

const RESUME_LOG_TITLE_OPEN: &str =
    "<div style=\"font-size: var(--text-xs); color: var(--text-3); \
text-transform: uppercase; letter-spacing: 0.05em; margin-bottom: var(--space-2); \
font-weight: var(--font-semibold);\">";

const RESUME_LOG_TEXT_OPEN: &str =
    "<pre style=\"margin: 0 0 var(--space-2); font-size: var(--text-xs); \
color: var(--text-2); white-space: pre-wrap; word-break: break-word; \
line-height: 1.4;\">";

const RESUME_LOG_BUTTON: &str = "<button type=\"button\" class=\"btn btn-secondary btn-sm neu\" \
data-action=\"copy-resume-log\" data-report=\"{REPORT}\" onclick=\"{JS}\">Copy resume log</button>";

const RESUME_LOG_CLOSE: &str = "</div>";

/// Which route a resume takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResumeStrategy {
    /// rodio already reports the sink as playing.
    AlreadyPlaying,
    /// Un-pause the existing sink.
    Unpause,
    /// Build a new sink from the current track and restore the position.
    FreshSinkReplay,
}

impl ResumeStrategy {
    fn as_str(self) -> &'static str {
        match self {
            ResumeStrategy::AlreadyPlaying => "none",
            ResumeStrategy::Unpause => "unpause",
            ResumeStrategy::FreshSinkReplay => "fresh_sink_replay",
        }
    }
}

/// One `stat` of the file a resume would have to decode: what the log shows,
/// and whether a fresh decode is worth attempting.
struct FilePreflight {
    note: String,
    readable: bool,
}

/// Pick the route for one resume.
///
/// `has_live_source` is `!is_empty` from the atomic snapshot: false covers both
/// "no sink" and "drained", and in both cases `AudioPlayer::resume` is the right
/// caller because it is what reports the `StateError` the frontend already
/// knows how to answer with a replay.
///
/// `file_ready` only ever *withholds* the mitigation. A file that cannot be
/// stat'ed is not proof of an unplayable file — on Android a MediaStore copy can
/// still resolve it (`cached_copy_for_path`) — so the worst a wrong `false` does
/// is fall back to the old behaviour, which is exactly the safe direction.
fn choose_resume_strategy(
    is_playing: bool,
    has_live_source: bool,
    file_ready: bool,
) -> ResumeStrategy {
    if is_playing {
        return ResumeStrategy::AlreadyPlaying;
    }
    if !has_live_source {
        return ResumeStrategy::Unpause;
    }
    if file_ready {
        return ResumeStrategy::FreshSinkReplay;
    }
    ResumeStrategy::Unpause
}

fn pre_state_str(is_playing: bool, is_empty: bool) -> &'static str {
    if is_playing {
        PRE_PLAYING
    } else if is_empty {
        PRE_NO_SOURCE
    } else {
        PRE_PAUSED
    }
}

/// What one resume attempt did, in enough detail to tell the failure shapes
/// apart. Rendered verbatim into the queue panel, so every field earns its
/// place.
#[derive(Debug, Clone)]
struct ResumeReport {
    at: String,
    track_id: String,
    track_id_short: String,
    title: String,
    strategy: &'static str,
    pre_state: &'static str,
    result: &'static str,
    /// The error text, when there is one.
    detail: String,
    file_note: String,
    pos_before_secs: f64,
    /// `n/a`, `ok`, or `failed: …` — whether the paused position survived the
    /// replay.
    pos_restored: String,
    /// Filled in later by the frontend's replay (`note_replay_outcome`).
    replay: String,
    probe: &'static str,
    probe_detail: String,
}

impl ResumeReport {
    fn new(
        track: Option<&Track>,
        strategy: ResumeStrategy,
        pre_state: &'static str,
        file_note: String,
        pos_before_secs: f64,
    ) -> Self {
        let track_id = track.map(|t| t.id.to_string()).unwrap_or_default();
        let track_id_short: String = track_id.chars().take(8).collect();
        let title = track
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "(no track)".to_string());
        Self {
            at: now_iso(),
            track_id,
            track_id_short,
            title,
            strategy: strategy.as_str(),
            pre_state,
            result: RESULT_PENDING,
            detail: String::new(),
            file_note,
            pos_before_secs,
            pos_restored: "n/a".to_string(),
            replay: String::new(),
            probe: VERDICT_NOT_PROBED,
            probe_detail: String::new(),
        }
    }
}

/// A resume in flight, waiting for the watcher to judge it.
struct ResumeProbe {
    armed_at: Instant,
    report: ResumeReport,
    polls: u32,
    playing_polls: u32,
    empty_polls: u32,
    /// An empty sample *after* a playing one: audio started and was consumed,
    /// which is a different failure from never starting.
    drained_after_playing: bool,
}

struct ResumeLog {
    pending: Option<ResumeProbe>,
    /// Oldest first, capped at [`RESUME_LOG_CAPACITY`]. A `VecDeque` would be
    /// the natural shape, but `VecDeque::new` is not a `const fn` and this is
    /// built in a `static`; at 20 entries the `remove(0)` costs nothing.
    reports: Vec<ResumeReport>,
    /// The newest report came from a rejected resume and is still waiting for
    /// the frontend's replay to be reported back onto it.
    awaiting_replay: bool,
    awaiting_since: Option<Instant>,
}

static RESUME_LOG: Mutex<ResumeLog> = Mutex::new(ResumeLog {
    pending: None,
    reports: Vec::new(),
    awaiting_replay: false,
    awaiting_since: None,
});

/// Poison-tolerant lock: a panic in one resume attempt must not blind every
/// later one, and the state here is plain data with no invariant a panic could
/// half-apply. Same recovery as `AudioPlayer::output_stream_sync`.
fn resume_log() -> MutexGuard<'static, ResumeLog> {
    RESUME_LOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_iso() -> String {
    // UTC, matching the download reports' `new Date().toISOString()`.
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

async fn preflight_audio_file(track: Option<&Track>) -> FilePreflight {
    let path = match track {
        Some(t) => t.file_path.as_str(),
        None => {
            return FilePreflight {
                note: "no-track".to_string(),
                readable: false,
            }
        }
    };
    match tokio::fs::metadata(path).await {
        Ok(md) if md.is_file() && md.len() > 0 => {
            let mib = md.len() as f64 / (1024.0 * 1024.0);
            FilePreflight {
                note: format!("ok:{mib:.2}MB"),
                readable: true,
            }
        }
        Ok(md) => {
            let note = if md.is_file() {
                "empty-file"
            } else {
                "not-a-file"
            };
            FilePreflight {
                note: note.to_string(),
                readable: false,
            }
        }
        Err(e) => FilePreflight {
            note: format!("stat-failed:{e}"),
            readable: false,
        },
    }
}

/// Restart the current track on a brand-new sink instead of un-pausing the old
/// one, then put the listener back where they were.
///
/// The position has to be read by the caller *before* this runs:
/// `play_track` → `start_sink` → `mark_playing` zeroes the accumulated position,
/// and `start_sink` also tears the paused sink down before it opens the file —
/// so a file that cannot be opened costs the listener their paused sink. That is
/// why [`choose_resume_strategy`] refuses to take this path without a readable
/// file, and why the position restore is best-effort rather than required: a
/// failed seek still leaves audio playing from the start, which beats silence.
async fn replay_on_fresh_sink(
    player: &AudioPlayer,
    track: Option<&Track>,
    pos_before: Duration,
    report: &mut ResumeReport,
) -> Result<(), String> {
    let track = match track {
        Some(t) => t.clone(),
        None => {
            return Err("nothing to resume: no current track is loaded".to_string());
        }
    };
    info!(
        track_id = %track.id,
        file_path = %track.file_path,
        pos_before = pos_before.as_secs_f64(),
        "Rebuilding the sink for resume instead of un-pausing it"
    );
    if let Err(e) = player.play_track(track).await {
        return Err(e.to_string());
    }
    if pos_before < POSITION_RESTORE_FLOOR {
        report.pos_restored = "n/a".to_string();
        return Ok(());
    }
    match player.seek(pos_before).await {
        Ok(()) => report.pos_restored = "ok".to_string(),
        Err(e) => {
            // Not fatal: playback is running, from the wrong offset. The log
            // says so, because "resume restarted my track" is otherwise
            // indistinguishable from a bug.
            report.pos_restored = format!("failed:{e}");
            warn!(error = %e, "Resume replay could not restore the paused position");
        }
    }
    Ok(())
}

/// Judge a finished probe window. Pure, so the whole table above is testable.
fn classify_probe(
    polls: u32,
    playing: u32,
    empty: u32,
    drained_after_playing: bool,
) -> &'static str {
    if polls == 0 {
        return VERDICT_NO_OBSERVATION;
    }
    if playing == 0 {
        return if empty > 0 {
            VERDICT_DRAINED_IMMEDIATE
        } else {
            VERDICT_NEVER_PLAYING
        };
    }
    if drained_after_playing {
        return VERDICT_PLAYED_THEN_DRAINED;
    }
    VERDICT_CONFIRMED
}

/// Whether a verdict is worth a toast. A confirmed resume, a resume that was
/// never attempted, and one replaced by a later attempt are all quiet: the first
/// two need no news, and the third is about to be reported by the probe that
/// replaced it.
fn verdict_is_failure(verdict: &str) -> bool {
    !matches!(
        verdict,
        VERDICT_CONFIRMED | VERDICT_NOT_PROBED | VERDICT_SUPERSEDED
    )
}

fn push_resume_report(reports: &mut Vec<ResumeReport>, report: ResumeReport) {
    reports.push(report);
    while reports.len() > RESUME_LOG_CAPACITY {
        reports.remove(0);
    }
}

fn file_resume_report(report: ResumeReport) {
    let mut log = resume_log();
    log.awaiting_replay = false;
    push_resume_report(&mut log.reports, report);
}

fn file_resume_report_opening_replay(report: ResumeReport) {
    let mut log = resume_log();
    log.awaiting_replay = true;
    log.awaiting_since = Some(Instant::now());
    push_resume_report(&mut log.reports, report);
}

fn arm_resume_probe(report: ResumeReport) {
    let mut log = resume_log();
    if let Some(stale) = log.pending.take() {
        // Two resumes inside one window: the newer attempt is the one the user
        // cares about, but the older must not silently vanish from the log.
        let mut superseded = stale.report;
        superseded.probe = VERDICT_SUPERSEDED;
        superseded.probe_detail = format!("replaced after {} poll(s)", stale.polls);
        push_resume_report(&mut log.reports, superseded);
    }
    log.pending = Some(ResumeProbe {
        armed_at: Instant::now(),
        report,
        polls: 0,
        playing_polls: 0,
        empty_polls: 0,
        drained_after_playing: false,
    });
}

fn resume_probe_pending() -> bool {
    resume_log().pending.is_some()
}

/// Feed one watcher tick to the pending probe. Returns the finished report on
/// the tick that closes the window.
///
/// `playing` is also the number of `playback:progress` events the run emitted
/// for this attempt, because the watcher emits that event on exactly this
/// condition — which is what makes "no audio" and "no events reached the
/// frontend" separable from here at all.
fn observe_resume_probe(now: Instant, is_playing: bool, is_empty: bool) -> Option<ResumeReport> {
    let mut log = resume_log();
    let probe = log.pending.as_mut()?;
    probe.polls = probe.polls.saturating_add(1);
    if is_playing {
        probe.playing_polls = probe.playing_polls.saturating_add(1);
    }
    if is_empty {
        probe.empty_polls = probe.empty_polls.saturating_add(1);
        if probe.playing_polls > 0 {
            probe.drained_after_playing = true;
        }
    }
    if now.saturating_duration_since(probe.armed_at) < RESUME_PROBE_WINDOW {
        return None;
    }
    let probe = log.pending.take()?;
    let mut report = probe.report;
    report.probe = classify_probe(
        probe.polls,
        probe.playing_polls,
        probe.empty_polls,
        probe.drained_after_playing,
    );
    report.probe_detail = format!(
        "polls={} progress={} empty={}",
        probe.polls, probe.playing_polls, probe.empty_polls
    );
    log.awaiting_replay = false;
    push_resume_report(&mut log.reports, report.clone());
    Some(report)
}

/// Stamp the outcome of the replay the frontend runs after a rejected resume
/// onto that resume's report.
///
/// A no-op unless the newest report is a rejected resume for this same track
/// that is still inside [`RESUME_REPLAY_WINDOW`], so an unrelated play cannot
/// rewrite history.
fn note_replay_outcome(track_id: &Uuid, outcome: Result<(), String>) {
    let now = Instant::now();
    let mut log = resume_log();
    if !log.awaiting_replay {
        return;
    }
    let fresh = log
        .awaiting_since
        .is_some_and(|t| now.saturating_duration_since(t) <= RESUME_REPLAY_WINDOW);
    if !fresh {
        log.awaiting_replay = false;
        return;
    }
    let id = track_id.to_string();
    let applies = log
        .reports
        .last()
        .is_some_and(|r| r.result == RESULT_ERROR && r.track_id == id);
    if !applies {
        return;
    }
    if let Some(report) = log.reports.last_mut() {
        report.replay = match outcome {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("failed:{e}"),
        };
    }
    log.awaiting_replay = false;
}

/// The log, newest first.
fn resume_log_snapshot() -> Vec<ResumeReport> {
    let log = resume_log();
    log.reports.iter().rev().cloned().collect()
}

/// Test-only reset for the process-wide [`RESUME_LOG`].
///
/// The state is global because the watcher and the commands are separate
/// tasks; the tests below serialise on [`RESUME_LOG_TEST_LOCK`] instead of
/// pretending it is per-test.
#[cfg(test)]
fn reset_resume_log() {
    let mut log = resume_log();
    log.pending = None;
    log.reports.clear();
    log.awaiting_replay = false;
    log.awaiting_since = None;
}

/// Serialises the tests that touch [`RESUME_LOG`].
#[cfg(test)]
static RESUME_LOG_TEST_LOCK: Mutex<()> = Mutex::new(());

/// One log line. Field order is the debugging order: what ran, what the state
/// was, what the file looked like, what the probe concluded, then identity.
fn format_resume_report(report: &ResumeReport) -> String {
    let mut line = String::with_capacity(200);
    line.push_str(&report.at);
    line.push(' ');
    line.push_str(report.result);
    line.push_str(&format!(" strategy={}", report.strategy));
    line.push_str(&format!(" pre={}", report.pre_state));
    line.push_str(&format!(" file={}", report.file_note));
    line.push_str(&format!(
        " pos={:.1}s restored={}",
        report.pos_before_secs, report.pos_restored
    ));
    line.push_str(&format!(" probe={}", report.probe));
    if !report.probe_detail.is_empty() {
        line.push_str(&format!(" ({})", report.probe_detail));
    }
    if !report.replay.is_empty() {
        line.push_str(&format!(" replay={}", report.replay));
    }
    if !report.detail.is_empty() {
        line.push_str(&format!(" err={}", report.detail));
    }
    line.push_str(&format!(
        " track=\"{}\" id={}",
        report.title, report.track_id_short
    ));
    line
}

/// The toast text for a resume that was accepted and then produced nothing.
fn resume_failure_toast(report: &ResumeReport) -> String {
    format!(
        "Resume produced no audio ({}). Details: queue panel > Resume log.",
        report.probe
    )
}

/// Render the log into the queue panel. `reports` is newest first.
///
/// Empty in, empty out: with no resume attempted yet this must not put a
/// heading on the panel.
fn render_resume_log_html(reports: &[ResumeReport]) -> String {
    if reports.is_empty() {
        return String::new();
    }
    let mut html = String::with_capacity(2048);
    html.push_str(RESUME_LOG_OPEN);
    let shown = reports.len().min(RESUME_LOG_VISIBLE);
    html.push_str(RESUME_LOG_TITLE_OPEN);
    html.push_str(&format!("Resume log (newest {shown} of {})", reports.len()));
    html.push_str("</div>");
    html.push_str(RESUME_LOG_TEXT_OPEN);
    for report in reports.iter().take(RESUME_LOG_VISIBLE) {
        html.push_str(&html_escape(&format_resume_report(report)));
        html.push('\n');
    }
    html.push_str("</pre>");
    // The whole log, not just the visible lines: the point of the button is to
    // get the evidence off the device.
    let full: Vec<String> = reports.iter().map(format_resume_report).collect();
    let button = RESUME_LOG_BUTTON
        .replace("{REPORT}", &html_escape(&full.join("\n")))
        .replace("{JS}", &html_escape(RESUME_LOG_COPY_JS));
    html.push_str(&button);
    html.push_str(RESUME_LOG_CLOSE);
    html
}

#[cfg(test)]
mod tests {
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
            VERDICT_NO_OBSERVATION,
            VERDICT_FAILED,
        ] {
            assert!(verdict_is_failure(bad), "{bad} must be reported");
        }
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
        arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay));
        assert!(resume_probe_pending());

        // Inside the window: counted, not judged.
        assert!(observe_resume_probe(Instant::now(), true, false).is_none());
        assert!(resume_probe_pending());

        let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
        let report = observe_resume_probe(past_window, true, false)
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
        arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay));
        let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
        let report = observe_resume_probe(past_window, false, false).expect("closes");
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
        arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay));
        let past_window = Instant::now() + RESUME_PROBE_WINDOW + Duration::from_millis(1);
        // First tick plays, second tick is drained: the real "audio started and
        // was consumed" shape.
        assert!(observe_resume_probe(Instant::now(), true, false).is_none());
        let report = observe_resume_probe(past_window, false, true).expect("closes");
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
        arm_resume_probe(sample_report(&track, ResumeStrategy::Unpause));
        assert!(observe_resume_probe(Instant::now(), false, false).is_none());
        arm_resume_probe(sample_report(&track, ResumeStrategy::FreshSinkReplay));
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
        let tr_repo: Arc<dyn TrackRepository> =
            Arc::new(SqliteTrackRepository::new(db_arc.clone()));

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
        let tr_repo: Arc<dyn TrackRepository> =
            Arc::new(SqliteTrackRepository::new(db_arc.clone()));

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
}

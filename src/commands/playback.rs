//! Playback Commands
//!
//! Tauri command handlers for the audio playback domain.

use crate::domain::models::{NowPlaying, RepeatMode, Track};
use crate::infrastructure::database::repositories::{parse_datetime, parse_format};
use crate::infrastructure::database::Database;
use crate::infrastructure::media::background_service;
use crate::infrastructure::media::AudioPlayer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
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
///   pause/seek commands already report the frozen position), and
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
            was_playing = is_playing;

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

            // Adaptive cadence: poll fast only while audio is actually
            // playing; slow to a heartbeat otherwise to save battery.
            let next = if is_playing {
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

    // If queue index is provided, set it
    if let Some(idx) = queue_index {
        player.set_current_index(Some(idx)).await;
    }

    let initial_dur = track.duration_secs;

    // Play the track — log full context so the UI can show exactly why it failed
    if let Err(e) = player.play_track(track.clone()).await {
        warn!(%track_id, file_path=%track.file_path, title=%track.title, error=%e, "Playback failed — file missing or undecodable");
        return Err(format!(
            "Playback error [{} — {}]: {}",
            track.title, track.file_path, e
        ));
    }

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

    debug!(%track_id, "Playback started");
    Ok(now_playing)
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
#[tauri::command]
pub async fn resume(app: AppHandle, player: State<'_, AudioPlayer>) -> Result<(), String> {
    info!("Resume command received");

    player
        .resume()
        .await
        .map_err(|e| format!("Resume error: {e}"))?;

    emit_state_changed(&app, &player).await;
    background_service::push_now_playing(&player).await;

    debug!("Playback resumed");
    Ok(())
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
    let mut tracks = Vec::with_capacity(track_ids.len());
    for tid in &track_ids {
        match lookup_track(*tid, &db).await {
            Ok(t) => tracks.push(t),
            Err(e) => warn!(%tid, error=%e, "set_queue: skip missing track"),
        }
    }
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

// ============================================================================
// Helper functions
// ============================================================================

async fn lookup_track(
    track_id: Uuid,
    db: &State<'_, Database>,
) -> Result<Track, Box<dyn std::error::Error>> {
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

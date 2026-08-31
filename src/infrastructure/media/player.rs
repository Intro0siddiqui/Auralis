//! Audio Player
//!
//! Audio playback using rodio (0.22+).

use rodio::{mixer::Mixer, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::domain::models::{RepeatMode, Track};
use rand::seq::{IndexedRandom, SliceRandom};
use rand::RngExt;

/// Holds the lazily-opened audio output stream.
struct OutputStreamHolder {
    // We keep the MixerDeviceSink alive here. Dropping it stops the audio output.
    stream: Option<MixerDeviceSink>,
}

// SAFETY: `MixerDeviceSink` wraps an `Arc` to the cpal/ALSA/CoreAudio output
// stream. rodio's sink is `Send` but not `Sync`; we wrap it in a
// `std::sync::Mutex` and document the following invariants:
//
// 1. Every access to the inner `MixerDeviceSink` is through a short,
//    synchronous `Mutex::lock()` critical section that never holds the guard
//    across an `.await` point (see `output_stream_sync`). This prevents the
//    non-`Sync` interior from being shared concurrently.
// 2. The sink is opened lazily on first `play()` and then never moved between
//    threads except via the `Arc<Mutex<_>>` — the underlying OS handle is
//    thread-safe for the operations we perform (`mixer().clone()` only).
// 3. `OutputStreamHolder` is only `Send + Sync` because `Mutex<T>` is `Sync`
//    when `T: Send`; the `MixerDeviceSink` itself is `Send`.
//
// If rodio ever makes `MixerDeviceSink: !Send`, this impl must be removed and
// audio I/O confined to a dedicated thread via a channel.
unsafe impl Send for OutputStreamHolder {}
unsafe impl Sync for OutputStreamHolder {}

/// Audio player using rodio
#[derive(Clone)]
pub struct AudioPlayer {
    output: Arc<std::sync::Mutex<OutputStreamHolder>>,
    sink: Arc<RwLock<Option<Player>>>,
    volume: Arc<RwLock<f32>>,
    current_track: Arc<RwLock<Option<Track>>>,
    queue: Arc<RwLock<Vec<Track>>>,
    current_index: Arc<RwLock<Option<usize>>>,
    repeat_mode: Arc<RwLock<RepeatMode>>,
    shuffle_enabled: Arc<RwLock<bool>>,
    /// History stack for shuffle playback (visited indices in order, last is current).
    /// Used to implement exhaust-on-RepeatOff and previous-with-history.
    shuffle_history: Arc<RwLock<Vec<usize>>>,
    /// Playback time accumulated before the current play session.
    played: Arc<RwLock<Duration>>,
    /// `Some` while the sink is actively playing.
    play_anchor: Arc<RwLock<Option<Instant>>>,
    /// When the current playback session started.
    play_started_at: Arc<RwLock<Option<Instant>>>,
    track_duration: Arc<RwLock<Duration>>,
}

// SAFETY: `AudioPlayer` is a bag of `Arc<RwLock<_>>` / `Arc<Mutex<_>>`
// around `Send` primitives (`Duration`, `Track`, `Player`, `bool`, etc.)
// and the `OutputStreamHolder` above, which is itself documented as
// `Send + Sync` under the invariants noted there. All interior state is
// behind `Arc` + synchronization primitives, so sharing `&AudioPlayer`
// across threads (as Tauri's `State` requires) is sound. No `&mut self`
// aliasing is exposed.
unsafe impl Send for AudioPlayer {}
unsafe impl Sync for AudioPlayer {}

impl AudioPlayer {
    pub fn new() -> Result<Self, PlayerError> {
        info!("Initializing audio player (output stream opened lazily)");
        Ok(Self {
            output: Arc::new(std::sync::Mutex::new(OutputStreamHolder { stream: None })),
            sink: Arc::new(RwLock::new(None)),
            volume: Arc::new(RwLock::new(0.8)),
            current_track: Arc::new(RwLock::new(None)),
            queue: Arc::new(RwLock::new(Vec::new())),
            current_index: Arc::new(RwLock::new(None)),
            repeat_mode: Arc::new(RwLock::new(RepeatMode::Off)),
            shuffle_enabled: Arc::new(RwLock::new(false)),
            shuffle_history: Arc::new(RwLock::new(Vec::new())),
            played: Arc::new(RwLock::new(Duration::ZERO)),
            play_anchor: Arc::new(RwLock::new(None)),
            play_started_at: Arc::new(RwLock::new(None)),
            track_duration: Arc::new(RwLock::new(Duration::ZERO)),
        })
    }

    fn output_stream_sync(&self) -> Result<Mixer, PlayerError> {
        let mut guard = self.output.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(device_sink) = guard.stream.as_ref() {
            // Clone the Mixer reference so it can be sent across async boundaries
            return Ok(device_sink.mixer().clone());
        }
        let device_sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| PlayerError::InitError(e.to_string()))?;
        let mixer = device_sink.mixer().clone();
        guard.stream = Some(device_sink);
        Ok(mixer)
    }

    async fn output_stream_handle(&self) -> Result<Mixer, PlayerError> {
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_DELAY: Duration = Duration::from_millis(500);
        for attempt in 1..=MAX_ATTEMPTS {
            match self.output_stream_sync() {
                Ok(mixer) => return Ok(mixer),
                Err(e) if attempt == MAX_ATTEMPTS => return Err(e),
                Err(e) => {
                    warn!(attempt, error = %e, "Audio output unavailable; retrying");
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
        Err(PlayerError::InitError("audio output unavailable".into()))
    }

    pub async fn play(&self, path: &str) -> Result<(), PlayerError> {
        info!(path = %path, "Starting playback");
        self.stop().await?;

        let vol = *self.volume.read().await;
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                // Android scoped-storage fallback: try to resolve a MediaStore
                // Download/Auralis entry via ContentResolver into a cache copy.
                #[cfg(target_os = "android")]
                {
                    if let Some(cached) =
                        crate::infrastructure::media::android_downloads::cached_copy_for_path(path)
                    {
                        File::open(&cached).map_err(|ce| {
                            PlayerError::FileError(format!("{path}: {e} (cached {cached:?}: {ce})"))
                        })?
                    } else {
                        return Err(PlayerError::FileError(format!("{path}: {e}")));
                    }
                }
                #[cfg(not(target_os = "android"))]
                {
                    return Err(PlayerError::FileError(format!("{path}: {e}")));
                }
            }
        };
        let source = Decoder::new(BufReader::new(file))
            .map_err(|e| PlayerError::DecodeError(format!("{path}: {e}")))?;
        // Auto-repair duration: compare actual decoded stream duration with expected database duration
        if let Some(dec_dur) = source.total_duration() {
            let db_dur = *self.track_duration.read().await;
            let dec_secs = dec_dur.as_secs();
            let db_secs = db_dur.as_secs();

            let reconciled = if !db_dur.is_zero() {
                let diff = (dec_secs as i64 - db_secs as i64).abs();
                if diff > 5 {
                    warn!(
                        path = %path,
                        db_secs = db_secs,
                        dec_secs = dec_secs,
                        diff_secs = diff,
                        "Severe duration mismatch (>5s) between DB metadata and decoder; auto-repairing track duration"
                    );
                    dec_dur
                } else if diff > 2 {
                    warn!(path = %path, db_secs = db_secs, dec_secs = dec_secs, "Minor duration mismatch DB vs decoder, using max");
                    db_dur.max(dec_dur)
                } else {
                    db_dur
                }
            } else {
                dec_dur
            };

            *self.track_duration.write().await = reconciled;

            let mut curr = self.current_track.write().await;
            if let Some(ref mut t) = *curr {
                t.duration_secs = reconciled.as_secs() as u32;
            }

            if let Some(idx) = *self.current_index.read().await {
                let mut q = self.queue.write().await;
                if let Some(t) = q.get_mut(idx) {
                    t.duration_secs = reconciled.as_secs() as u32;
                }
            }
        }

        let mixer = self.output_stream_handle().await?;
        let player = Player::connect_new(&mixer);

        player.set_volume(vol);
        player.append(source);

        *self.sink.write().await = Some(player);
        self.mark_playing().await;

        debug!(path = %path, "Playback started");
        Ok(())
    }

    pub async fn play_track(&self, track: Track) -> Result<(), PlayerError> {
        info!(track_id = %track.id, title = %track.title, "Starting track playback");
        *self.track_duration.write().await = Duration::from_secs(track.duration_secs as u64);
        *self.current_track.write().await = Some(track.clone());
        self.play(&track.file_path).await
    }

    pub async fn pause(&self) -> Result<(), PlayerError> {
        debug!("Pausing playback");
        if let Some(s) = self.sink.read().await.as_ref() {
            s.pause();
        }
        let elapsed = self
            .play_anchor
            .read()
            .await
            .map(|a| a.elapsed())
            .unwrap_or_default();
        if !elapsed.is_zero() {
            *self.played.write().await += elapsed;
        }
        *self.play_anchor.write().await = None;
        Ok(())
    }

    pub async fn resume(&self) -> Result<(), PlayerError> {
        debug!("Resuming playback");
        let sink_guard = self.sink.read().await;
        if let Some(s) = sink_guard.as_ref() {
            // Guard against double resume: if already playing (anchor Some && !is_paused), no-op.
            let anchor_some = self.play_anchor.read().await.is_some();
            if anchor_some && !s.is_paused() {
                return Ok(());
            }
            s.play();
            // Discard elapsed while paused — do not fold stale anchor into `played`.
            *self.play_anchor.write().await = Some(Instant::now());
        }
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), PlayerError> {
        debug!("Stopping playback");
        if let Some(s) = self.sink.write().await.take() {
            s.stop();
        }
        *self.played.write().await = Duration::ZERO;
        *self.play_anchor.write().await = None;
        *self.play_started_at.write().await = None;
        Ok(())
    }

    /// Seek using rodio's native `try_seek` with fallback for unseekable MP3 bitstreams.
    pub async fn seek(&self, position: Duration) -> Result<(), PlayerError> {
        debug!(?position, "Seeking to position");

        let total = *self.track_duration.read().await;
        if !total.is_zero() && position > total {
            return Err(PlayerError::StateError(
                "Seek position exceeds track duration".into(),
            ));
        }

        let current_track = self.current_track.read().await.clone();

        let seek_res = {
            let sink_guard = self.sink.read().await;
            if let Some(player) = sink_guard.as_ref() {
                let was_paused = player.is_paused();
                let res = player.try_seek(position);
                (res, was_paused)
            } else {
                return Err(PlayerError::StateError("No active playback".into()));
            }
        };

        let (res, was_paused) = seek_res;

        if let Err(e) = res {
            warn!(
                ?position,
                error = %e,
                "Native player seek failed (e.g. unseekable VBR MP3); attempting fallback seek"
            );
            if let Some(track) = current_track {
                if let Err(fallback_err) = self.seek_fallback(&track, position, was_paused).await {
                    warn!(
                        error = %fallback_err,
                        "Fallback seek failed; preserving playback state"
                    );
                    return Err(PlayerError::StateError(format!(
                        "Seek failed: {e} (fallback error: {fallback_err})"
                    )));
                }
                info!(?position, "Fallback seek completed successfully");
                return Ok(());
            } else {
                return Err(PlayerError::StateError(format!("Seek failed: {e}")));
            }
        }

        *self.played.write().await = position;
        if was_paused {
            *self.play_anchor.write().await = None;
        } else {
            *self.play_anchor.write().await = Some(Instant::now());
        }

        info!(?position, "Seek completed");
        Ok(())
    }

    /// Fallback seeking when Symphonia cannot perform frame-accurate timestamp seeks directly on the active decoder.
    /// Re-opens the track, creates a fresh decoder, attempts `skip_duration` or `try_seek`, and replaces the active player.
    async fn seek_fallback(
        &self,
        track: &Track,
        position: Duration,
        was_paused: bool,
    ) -> Result<(), PlayerError> {
        let file_path = &track.file_path;
        let file = match File::open(file_path) {
            Ok(f) => f,
            Err(e) => {
                #[cfg(target_os = "android")]
                {
                    if let Some(cached) =
                        crate::infrastructure::media::android_downloads::cached_copy_for_path(
                            file_path,
                        )
                    {
                        File::open(&cached).map_err(|ce| {
                            PlayerError::FileError(format!(
                                "{file_path}: {e} (cached {cached:?}: {ce})"
                            ))
                        })?
                    } else {
                        return Err(PlayerError::FileError(format!("{file_path}: {e}")));
                    }
                }
                #[cfg(not(target_os = "android"))]
                {
                    return Err(PlayerError::FileError(format!("{file_path}: {e}")));
                }
            }
        };

        let source = Decoder::new(BufReader::new(file))
            .map_err(|e| PlayerError::DecodeError(format!("{file_path}: {e}")))?;

        let skipped_source = source.skip_duration(position);

        let vol = *self.volume.read().await;
        let mixer = self.output_stream_handle().await?;
        let player = Player::connect_new(&mixer);
        player.set_volume(vol);
        player.append(skipped_source);

        if was_paused {
            player.pause();
        }

        if let Some(old_sink) = self.sink.write().await.replace(player) {
            old_sink.stop();
        }

        *self.played.write().await = position;
        if was_paused {
            *self.play_anchor.write().await = None;
        } else {
            *self.play_anchor.write().await = Some(Instant::now());
        }

        Ok(())
    }

    pub async fn set_volume(&self, volume: f32) -> Result<(), PlayerError> {
        let volume = volume.clamp(0.0, 1.0);
        *self.volume.write().await = volume;
        if let Some(s) = self.sink.read().await.as_ref() {
            s.set_volume(volume);
        }
        Ok(())
    }

    pub async fn next(&self) -> Result<Option<Track>, PlayerError> {
        self.next_internal(false).await
    }

    /// Auto-advance variant used by the playback watcher on natural track end.
    /// Honors `RepeatOne` by repeating the current track; manual `next()` always advances.
    pub async fn next_for_auto_advance(&self) -> Result<Option<Track>, PlayerError> {
        self.next_internal(true).await
    }

    /// Calculates the next track index when shuffle mode is enabled.
    async fn next_shuffle_index(
        &self,
        queue_len: usize,
        current_idx: Option<usize>,
        repeat: RepeatMode,
    ) -> Option<usize> {
        if queue_len == 1 {
            return match (current_idx, repeat) {
                (Some(_), RepeatMode::Off) => None,
                _ => Some(0),
            };
        }

        if repeat == RepeatMode::Off {
            // Shuffle + RepeatOff: exhaust when all tracks have been visited.
            let history = self.shuffle_history.read().await;
            let mut visited_set: std::collections::HashSet<usize> =
                history.iter().copied().collect();
            if let Some(ci) = current_idx {
                visited_set.insert(ci);
            }

            if visited_set.len() >= queue_len {
                return None;
            }

            let mut rng = rand::rng();
            let unvisited: Vec<usize> = (0..queue_len)
                .filter(|i| !visited_set.contains(i))
                .collect();

            if let Some(&idx) = unvisited.choose(&mut rng) {
                Some(idx)
            } else {
                let other_candidates: Vec<usize> =
                    (0..queue_len).filter(|&i| Some(i) != current_idx).collect();
                Some(
                    other_candidates
                        .choose(&mut rng)
                        .copied()
                        .unwrap_or_else(|| rng.random_range(0..queue_len)),
                )
            }
        } else {
            // Shuffle + RepeatAll (or RepeatOne manual): pick random != current
            let mut rng = rand::rng();
            let candidates: Vec<usize> =
                (0..queue_len).filter(|&i| Some(i) != current_idx).collect();
            Some(
                candidates
                    .choose(&mut rng)
                    .copied()
                    .unwrap_or_else(|| rng.random_range(0..queue_len)),
            )
        }
    }

    /// Maintains shuffle history stack (visited track indices) for previous and exhaustion checking.
    async fn record_shuffle_history(&self, current_idx: Option<usize>, next_idx: usize) {
        let mut hist = self.shuffle_history.write().await;
        if let Some(ci) = current_idx {
            if !hist.contains(&ci) {
                hist.push(ci);
            }
        }
        if hist.last().copied() != Some(next_idx) {
            hist.push(next_idx);
        }
    }

    async fn next_internal(&self, for_auto_advance: bool) -> Result<Option<Track>, PlayerError> {
        let queue = self.queue.read().await;
        if queue.is_empty() {
            return Ok(None);
        }

        let current_idx = *self.current_index.read().await;
        let repeat = *self.repeat_mode.read().await;
        let shuffle = *self.shuffle_enabled.read().await;

        let next_index = if for_auto_advance && repeat == RepeatMode::One {
            current_idx.or(Some(0))
        } else if shuffle {
            self.next_shuffle_index(queue.len(), current_idx, repeat)
                .await
        } else {
            match (current_idx, repeat) {
                (Some(idx), RepeatMode::All) if idx + 1 >= queue.len() => Some(0),
                (Some(idx), RepeatMode::Off) if idx + 1 >= queue.len() => None,
                (Some(idx), RepeatMode::One) if idx + 1 >= queue.len() => None,
                (Some(idx), _) => Some(idx + 1),
                (None, _) => Some(0),
            }
        };

        match next_index {
            Some(idx) => {
                let track = queue[idx].clone();
                drop(queue);
                // Only update current_index after play_track succeeds to avoid divergence
                self.play_track(track.clone()).await?;
                *self.current_index.write().await = Some(idx);
                // Maintain shuffle history (visited stack) for exhaust/previous
                if shuffle {
                    self.record_shuffle_history(current_idx, idx).await;
                }
                Ok(Some(track))
            }
            None => {
                drop(queue);
                // Clear shuffle history on exhaustion (session complete)
                if shuffle {
                    self.shuffle_history.write().await.clear();
                }
                self.stop().await?;
                Ok(None)
            }
        }
    }

    pub async fn previous(&self) -> Result<Option<Track>, PlayerError> {
        let queue = self.queue.read().await;
        if queue.is_empty() {
            return Ok(None);
        }

        let current_idx = *self.current_index.read().await;
        let repeat = *self.repeat_mode.read().await;
        let shuffle = *self.shuffle_enabled.read().await;

        let prev_index = if repeat == RepeatMode::One {
            current_idx
        } else if shuffle {
            if queue.len() == 1 {
                Some(0)
            } else {
                // Use history stack to go back: pop current, return previous.
                let mut hist = self.shuffle_history.write().await;
                if hist.len() > 1 {
                    // Last is current, pop it and return new last
                    hist.pop();
                    let prev = hist.last().copied();
                    // Keep hist pointing at prev (do not push again)
                    prev
                } else if hist.len() == 1 && hist[0] != current_idx.unwrap_or(usize::MAX) {
                    // History has single entry not matching current (edge), use it
                    let prev = hist[0];
                    // hist already at prev
                    Some(prev)
                } else {
                    // No history to pop — fallback to random distinct
                    drop(hist);
                    let mut rng = rand::rng();
                    let mut candidates: Vec<usize> = (0..queue.len()).collect();
                    candidates.shuffle(&mut rng);
                    let idx = candidates
                        .into_iter()
                        .find(|&i| Some(i) != current_idx)
                        .unwrap_or_else(|| rng.random_range(0..queue.len()));
                    Some(idx)
                }
            }
        } else {
            match (current_idx, repeat) {
                (Some(0), RepeatMode::All) => Some(queue.len() - 1),
                (Some(0), RepeatMode::Off) => Some(0),
                (Some(idx), _) => Some(idx - 1),
                (None, _) => Some(0),
            }
        };

        match prev_index {
            Some(idx) => {
                let track = queue[idx].clone();
                drop(queue);
                // Only update index after successful play
                self.play_track(track.clone()).await?;
                *self.current_index.write().await = Some(idx);
                Ok(Some(track))
            }
            None => Ok(None),
        }
    }

    pub async fn is_playing(&self) -> bool {
        self.sink
            .read()
            .await
            .as_ref()
            .map(|s| !s.is_paused() && !s.empty())
            .unwrap_or(false)
    }

    pub async fn current_position(&self) -> Duration {
        let played = *self.played.read().await;
        let live = self
            .play_anchor
            .read()
            .await
            .map(|a| a.elapsed())
            .unwrap_or_default();
        let total = played + live;
        let duration = *self.track_duration.read().await;
        if duration.is_zero() {
            total
        } else {
            total.min(duration)
        }
    }

    pub async fn is_sink_empty(&self) -> bool {
        self.sink
            .read()
            .await
            .as_ref()
            .map(|s| s.empty())
            .unwrap_or(true)
    }

    /// Atomically snapshot `(is_playing, is_empty)` under a single `RwLock`
    /// read guard to avoid the TOCTOU race between separate
    /// `is_playing()` + `is_sink_empty()` calls (the sink could transition
    /// between the two awaits).
    pub async fn sink_snapshot(&self) -> (bool, bool) {
        let guard = self.sink.read().await;
        match guard.as_ref() {
            Some(s) => (!s.is_paused() && !s.empty(), s.empty()),
            None => (false, true),
        }
    }

    pub async fn play_started_elapsed(&self) -> Option<Duration> {
        self.play_started_at.read().await.map(|s| s.elapsed())
    }

    async fn mark_playing(&self) {
        let now = Instant::now();
        *self.played.write().await = Duration::ZERO;
        *self.play_anchor.write().await = Some(now);
        *self.play_started_at.write().await = Some(now);
    }

    pub async fn duration(&self) -> Duration {
        *self.track_duration.read().await
    }
    pub async fn get_volume(&self) -> f32 {
        *self.volume.read().await
    }
    pub async fn get_current_track(&self) -> Option<Track> {
        self.current_track.read().await.clone()
    }
    pub async fn get_queue(&self) -> Vec<Track> {
        self.queue.read().await.clone()
    }
    pub async fn set_queue(&self, tracks: Vec<Track>) {
        *self.queue.write().await = tracks;
        // Shuffle session invalidated by queue change
        self.shuffle_history.write().await.clear();
    }
    pub async fn add_to_queue(&self, track: Track) {
        let new_len = {
            let mut q = self.queue.write().await;
            q.push(track);
            q.len()
        };
        // Retain shuffle session; only invalidate out-of-bounds indices.
        self.shuffle_history.write().await.retain(|i| *i < new_len);
    }

    pub async fn play_next(&self, track: Track) {
        let current_idx = *self.current_index.read().await;
        let new_len = {
            let mut q = self.queue.write().await;
            if q.is_empty() {
                q.push(track);
                1
            } else {
                let insert_idx = current_idx.map(|i| i + 1).unwrap_or(0).min(q.len());
                q.insert(insert_idx, track);
                q.len()
            }
        };
        self.shuffle_history.write().await.retain(|i| *i < new_len);
    }

    pub async fn remove_from_queue(&self, index: usize) -> Result<Track, PlayerError> {
        let mut queue = self.queue.write().await;
        if index >= queue.len() {
            return Err(PlayerError::StateError("Queue index out of bounds".into()));
        }
        let track = queue.remove(index);
        if let Some(current) = *self.current_index.read().await {
            if index < current {
                *self.current_index.write().await = Some(current - 1);
            } else if index == current {
                *self.current_index.write().await = if current < queue.len() {
                    Some(current)
                } else {
                    None
                };
            }
        }
        drop(queue);
        self.shuffle_history.write().await.clear();
        Ok(track)
    }

    pub async fn clear_queue(&self) {
        self.queue.write().await.clear();
        *self.current_index.write().await = None;
        self.shuffle_history.write().await.clear();
    }

    pub async fn get_current_index(&self) -> Option<usize> {
        *self.current_index.read().await
    }
    pub async fn set_current_index(&self, index: Option<usize>) {
        *self.current_index.write().await = index;
    }
    pub async fn get_repeat_mode(&self) -> RepeatMode {
        *self.repeat_mode.read().await
    }
    pub async fn set_repeat_mode(&self, mode: RepeatMode) {
        *self.repeat_mode.write().await = mode;
        self.shuffle_history.write().await.clear();
    }
    pub async fn get_shuffle(&self) -> bool {
        *self.shuffle_enabled.read().await
    }
    pub async fn set_shuffle(&self, enabled: bool) {
        *self.shuffle_enabled.write().await = enabled;
        let current = *self.current_index.read().await;
        let mut hist = self.shuffle_history.write().await;
        hist.clear();
        // Seed history with current index when shuffle is enabled so previous has a base
        if enabled {
            if let Some(idx) = current {
                hist.push(idx);
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("Initialization error: {0}")]
    InitError(String),
    #[error("Player error: {0}")]
    SinkError(String),
    #[error("File error: {0}")]
    FileError(String),
    #[error("Decode error: {0}")]
    DecodeError(String),
    #[error("State error: {0}")]
    StateError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

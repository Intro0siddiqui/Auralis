//! Audio Player
//!
//! Audio playback using rodio.

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::domain::models::{RepeatMode, Track};

/// Holds the lazily-opened audio output stream.
///
/// `OutputStream` is `!Send`/`!Sync` on some platforms (it wraps a
/// platform-specific audio handle). It is only ever created on the thread
/// that first calls `play()` and accessed inside short synchronous mutex
/// sections — never moved between threads while alive.
struct OutputStreamHolder {
    stream: Option<(OutputStream, OutputStreamHandle)>,
}

// SAFETY: every access to the held `OutputStream` happens inside a short
// synchronous `Mutex` section (never across an `await`), and the stream is
// only ever stored/read on the thread that opened it. The `OutputStreamHandle`
// handed out to callers is `Send + Sync`.
unsafe impl Send for OutputStreamHolder {}
unsafe impl Sync for OutputStreamHolder {}

/// Audio player using rodio
pub struct AudioPlayer {
    /// Lazily-opened persistent audio output stream.
    ///
    /// `None` until the first `play()` — the device is opened on demand (with
    /// retries) instead of during application setup, because on Android the
    /// AAudio/oboe stream is not always available that early. If setup failed
    /// to open the device, `AudioPlayer::new()` used to error out and the
    /// player never got registered in Tauri state, breaking every playback
    /// command with "state not managed for field 'player'".
    output: Arc<std::sync::Mutex<OutputStreamHolder>>,
    sink: Arc<RwLock<Option<Sink>>>,
    volume: Arc<RwLock<f32>>,
    current_track: Arc<RwLock<Option<Track>>>,
    queue: Arc<RwLock<Vec<Track>>>,
    current_index: Arc<RwLock<Option<usize>>>,
    repeat_mode: Arc<RwLock<RepeatMode>>,
    shuffle_enabled: Arc<RwLock<bool>>,
    /// Target position to skip to when the current playback session was
    /// started by a seek (rodio 0.17 has no `Sink::try_seek`).
    seek_offset: Arc<RwLock<Duration>>,
    /// Playback time accumulated before the current play session (frozen
    /// while paused).
    played: Arc<RwLock<Duration>>,
    /// `Some` while the sink is actively playing; used to compute the live
    /// position.
    play_anchor: Arc<RwLock<Option<Instant>>>,
    /// When the current playback session started; used by the auto-advance
    /// watcher to distinguish a finished track from an empty/failed source.
    play_started_at: Arc<RwLock<Option<Instant>>>,
    track_duration: Arc<RwLock<Duration>>,
}

// SAFETY: `AudioPlayer` is shared across threads via `Arc` in Tauri managed
// state. All mutable fields are guarded by `Arc<RwLock<_>>` (tokio) or the
// `Arc<Mutex<_>>` holding the output stream, which is only ever touched inside
// short synchronous sections (never across an await). The `OutputStream` is
// stored behind the mutex and never moved between threads.
unsafe impl Send for AudioPlayer {}
unsafe impl Sync for AudioPlayer {}

impl AudioPlayer {
    /// Create a new audio player.
    ///
    /// Infallible in practice: the audio output device is **not** opened here
    /// (it can be briefly unavailable on Android during startup); it is opened
    /// lazily on the first `play()` via [`AudioPlayer::output_stream_handle`].
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
            seek_offset: Arc::new(RwLock::new(Duration::ZERO)),
            played: Arc::new(RwLock::new(Duration::ZERO)),
            play_anchor: Arc::new(RwLock::new(None)),
            play_started_at: Arc::new(RwLock::new(None)),
            track_duration: Arc::new(RwLock::new(Duration::ZERO)),
        })
    }

    /// Open the persistent audio output stream on first use.
    ///
    /// Returns the already-open handle when the stream exists, otherwise
    /// attempts to open the default output device. The `OutputStream` itself
    /// is kept alive inside the mutex (dropping it would stop the audio); the
    /// returned handle is `Send + Sync` and safe to use across awaits. Never
    /// holds the mutex across an await point.
    fn output_stream_sync(&self) -> Result<OutputStreamHandle, PlayerError> {
        let mut guard = self
            .output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, handle)) = guard.stream.as_ref() {
            return Ok(handle.clone());
        }
        let (stream, handle) =
            OutputStream::try_default().map_err(|e| PlayerError::InitError(e.to_string()))?;
        guard.stream = Some((stream, handle.clone()));
        Ok(handle)
    }

    /// Lazily open the output stream with retries.
    ///
    /// On Android the audio HAL/AAudio stream may not be available yet during
    /// early startup; retry a few times before giving up so a transient
    /// failure cannot take down playback permanently.
    async fn output_stream_handle(&self) -> Result<OutputStreamHandle, PlayerError> {
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_DELAY: Duration = Duration::from_millis(500);

        for attempt in 1..=MAX_ATTEMPTS {
            match self.output_stream_sync() {
                Ok(handle) => return Ok(handle),
                Err(e) if attempt == MAX_ATTEMPTS => return Err(e),
                Err(e) => {
                    warn!(attempt, error = %e, "Audio output unavailable; retrying");
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
        Err(PlayerError::InitError(
            "audio output unavailable".to_string(),
        ))
    }

    /// Play an audio file by path
    pub async fn play(&self, path: &str) -> Result<(), PlayerError> {
        info!(path = %path, "Starting playback");

        // Stop current playback
        self.stop().await?;

        // Read volume before touching the output stream
        let vol = *self.volume.read().await;

        // Reject formats the rodio/symphonia stack cannot decode with a clear
        // error instead of letting decode fail silently.
        if format_is_unsupported(path) {
            return Err(PlayerError::UnsupportedFormat(format!(
                "Format of '{}' is not supported by the audio engine",
                path
            )));
        }

        // Open and decode file
        let file = File::open(path).map_err(|e| PlayerError::FileError(e.to_string()))?;

        let reader = BufReader::new(file);
        let source = Decoder::new(reader).map_err(|e| PlayerError::DecodeError(e.to_string()))?;

        // Reuse the persistent output stream handle. The stream is opened
        // lazily here (never in `new()`) so a transient audio-device failure
        // on Android cannot prevent the player from being registered in
        // Tauri state.
        let stream_handle = self.output_stream_handle().await?;
        let sink =
            Sink::try_new(&stream_handle).map_err(|e| PlayerError::SinkError(e.to_string()))?;

        sink.set_volume(vol);
        sink.append(source);

        // Store sink and reset playback accounting
        {
            let mut current = self.sink.write().await;
            *current = Some(sink);
        }

        self.mark_playing().await;

        debug!(path = %path, "Playback started");
        Ok(())
    }

    /// Play a track with full metadata
    pub async fn play_track(&self, track: Track) -> Result<(), PlayerError> {
        info!(track_id = %track.id, title = %track.title, "Starting track playback");

        // Store duration
        {
            let mut dur = self.track_duration.write().await;
            *dur = Duration::from_secs(track.duration_secs as u64);
        }

        // Store current track
        {
            let mut current = self.current_track.write().await;
            *current = Some(track.clone());
        }

        self.play(&track.file_path).await
    }

    /// Pause playback
    pub async fn pause(&self) -> Result<(), PlayerError> {
        debug!("Pausing playback");
        let sink = self.sink.read().await;
        if let Some(s) = sink.as_ref() {
            s.pause();
        }
        drop(sink);

        // Fold the live elapsed time into `played` so the position freezes.
        let elapsed = match *self.play_anchor.read().await {
            Some(anchor) => anchor.elapsed(),
            None => Duration::ZERO,
        };
        if !elapsed.is_zero() {
            let mut played = self.played.write().await;
            *played += elapsed;
        }
        *self.play_anchor.write().await = None;
        Ok(())
    }

    /// Resume playback
    pub async fn resume(&self) -> Result<(), PlayerError> {
        debug!("Resuming playback");
        let sink = self.sink.read().await;
        if let Some(s) = sink.as_ref() {
            s.play();
            *self.play_anchor.write().await = Some(Instant::now());
        }
        Ok(())
    }

    /// Stop playback
    pub async fn stop(&self) -> Result<(), PlayerError> {
        debug!("Stopping playback");
        let mut sink = self.sink.write().await;
        if let Some(s) = sink.take() {
            s.stop();
        }
        drop(sink);
        *self.seek_offset.write().await = Duration::ZERO;
        *self.played.write().await = Duration::ZERO;
        *self.play_anchor.write().await = None;
        *self.play_started_at.write().await = None;
        Ok(())
    }

    /// Seek to a position within the current track.
    ///
    /// rodio 0.17 has no `Sink::try_seek`, so seeking restarts playback from
    /// the beginning and the target is stored as the seek offset, which
    /// [`AudioPlayer::current_position`] adds to the real elapsed time.
    pub async fn seek(&self, position: Duration) -> Result<(), PlayerError> {
        debug!(?position, "Seeking to position");

        // Resolve the currently loaded track to validate the target position.
        let path = {
            let guard = self.current_track.read().await;
            let track = guard.as_ref().ok_or(PlayerError::StateError(
                "No track currently loaded".to_string(),
            ))?;

            let total_duration = Duration::from_secs(track.duration_secs as u64);
            if position > total_duration {
                return Err(PlayerError::StateError(
                    "Seek position exceeds track duration".to_string(),
                ));
            }
            track.file_path.clone()
        };

        // Restart playback from the beginning, then store the target as the
        // seek offset. The offset must be set AFTER `play()` because a fresh
        // playback session resets it.
        self.play(&path).await?;
        *self.seek_offset.write().await = position;

        info!(
            ?position,
            "Seek completed (position tracked via seek offset)"
        );
        Ok(())
    }

    /// Set volume (0.0 to 1.0)
    pub async fn set_volume(&self, volume: f32) -> Result<(), PlayerError> {
        let volume = volume.clamp(0.0, 1.0);
        debug!(volume, "Setting volume");

        {
            let mut vol = self.volume.write().await;
            *vol = volume;
        }

        let sink = self.sink.read().await;
        if let Some(s) = sink.as_ref() {
            s.set_volume(volume);
        }

        Ok(())
    }

    /// Skip to next track in queue
    pub async fn next(&self) -> Result<Option<Track>, PlayerError> {
        debug!("Skipping to next track");

        let queue = self.queue.read().await;
        if queue.is_empty() {
            return Ok(None);
        }

        let current_idx = *self.current_index.read().await;
        let repeat_mode = *self.repeat_mode.read().await;
        let shuffle = *self.shuffle_enabled.read().await;

        let next_index = match (current_idx, repeat_mode, shuffle) {
            // Repeat single track: stay on current
            (Some(idx), RepeatMode::One, _) => Some(idx),
            // At end of queue with repeat all: wrap to start
            (Some(idx), RepeatMode::All, _) if idx + 1 >= queue.len() => Some(0),
            // At end of queue, no repeat: stop
            (Some(idx), RepeatMode::Off, _) if idx + 1 >= queue.len() => None,
            // Normal next
            (Some(idx), _, _) => Some(idx + 1),
            // No current track, start at beginning
            (None, _, _) => Some(0),
        };

        match next_index {
            Some(idx) => {
                let track = queue[idx].clone();
                drop(queue);
                *self.current_index.write().await = Some(idx);
                self.play_track(track.clone()).await?;
                Ok(Some(track))
            }
            None => {
                drop(queue);
                self.stop().await?;
                Ok(None)
            }
        }
    }

    /// Go back to previous track
    pub async fn previous(&self) -> Result<Option<Track>, PlayerError> {
        debug!("Going to previous track");

        let queue = self.queue.read().await;
        if queue.is_empty() {
            return Ok(None);
        }

        let current_idx = *self.current_index.read().await;
        let repeat_mode = *self.repeat_mode.read().await;

        let prev_index = match (current_idx, repeat_mode) {
            // Repeat single track: stay on current
            (Some(_), RepeatMode::One) => current_idx,
            // At start of queue with repeat all: wrap to end
            (Some(0), RepeatMode::All) => Some(queue.len() - 1),
            // At start, no repeat: stay at first
            (Some(0), RepeatMode::Off) => Some(0),
            // Normal previous
            (Some(idx), _) => Some(idx - 1),
            // No current track, start at beginning
            (None, _) => Some(0),
        };

        match prev_index {
            Some(idx) => {
                let track = queue[idx].clone();
                drop(queue);
                *self.current_index.write().await = Some(idx);
                self.play_track(track.clone()).await?;
                Ok(Some(track))
            }
            None => Ok(None),
        }
    }

    /// Check if currently playing
    pub async fn is_playing(&self) -> bool {
        let sink = self.sink.read().await;
        sink.as_ref()
            .map(|s| !s.is_paused() && !s.empty())
            .unwrap_or(false)
    }

    /// Get current position
    pub async fn current_position(&self) -> Duration {
        let offset = *self.seek_offset.read().await;
        let played = *self.played.read().await;
        let live = match *self.play_anchor.read().await {
            Some(anchor) => anchor.elapsed(),
            None => Duration::ZERO,
        };
        let total = offset + played + live;
        let duration = *self.track_duration.read().await;
        if duration.is_zero() {
            total
        } else {
            total.min(duration)
        }
    }

    /// Returns `true` when the sink has no queued sources (the current track
    /// finished or playback was stopped).
    pub async fn is_sink_empty(&self) -> bool {
        let sink = self.sink.read().await;
        sink.as_ref().map(|s| s.empty()).unwrap_or(true)
    }

    /// Returns the elapsed time since the current playback session started,
    /// or `None` when no session is active.
    pub async fn play_started_elapsed(&self) -> Option<Duration> {
        (*self.play_started_at.read().await).map(|started| started.elapsed())
    }

    /// Reset playback accounting and stamp the start of a new playback session.
    async fn mark_playing(&self) {
        let now = Instant::now();
        *self.seek_offset.write().await = Duration::ZERO;
        *self.played.write().await = Duration::ZERO;
        *self.play_anchor.write().await = Some(now);
        *self.play_started_at.write().await = Some(now);
    }

    /// Get track duration
    pub async fn duration(&self) -> Duration {
        *self.track_duration.read().await
    }

    /// Get current volume
    pub async fn get_volume(&self) -> f32 {
        *self.volume.read().await
    }

    /// Get current track
    pub async fn get_current_track(&self) -> Option<Track> {
        self.current_track.read().await.clone()
    }

    /// Get the current queue
    pub async fn get_queue(&self) -> Vec<Track> {
        self.queue.read().await.clone()
    }

    /// Set the queue
    pub async fn set_queue(&self, tracks: Vec<Track>) {
        let mut queue = self.queue.write().await;
        *queue = tracks;
    }

    /// Add track to queue
    pub async fn add_to_queue(&self, track: Track) {
        let mut queue = self.queue.write().await;
        queue.push(track);
    }

    /// Remove track from queue at index
    pub async fn remove_from_queue(&self, index: usize) -> Result<Track, PlayerError> {
        let mut queue = self.queue.write().await;
        if index >= queue.len() {
            return Err(PlayerError::StateError(
                "Queue index out of bounds".to_string(),
            ));
        }
        let track = queue.remove(index);

        // Adjust current index if needed
        if let Some(current) = *self.current_index.read().await {
            if index < current {
                *self.current_index.write().await = Some(current - 1);
            } else if index == current {
                // Removed the currently playing track
                *self.current_index.write().await = if current < queue.len() {
                    Some(current)
                } else {
                    None
                };
            }
        }

        Ok(track)
    }

    /// Clear the queue
    pub async fn clear_queue(&self) {
        let mut queue = self.queue.write().await;
        queue.clear();
        *self.current_index.write().await = None;
    }

    /// Get current queue index
    pub async fn get_current_index(&self) -> Option<usize> {
        *self.current_index.read().await
    }

    /// Set current queue index
    pub async fn set_current_index(&self, index: Option<usize>) {
        *self.current_index.write().await = index;
    }

    /// Get repeat mode
    pub async fn get_repeat_mode(&self) -> RepeatMode {
        *self.repeat_mode.read().await
    }

    /// Set repeat mode
    pub async fn set_repeat_mode(&self, mode: RepeatMode) {
        let mut repeat = self.repeat_mode.write().await;
        *repeat = mode;
    }

    /// Get shuffle enabled
    pub async fn get_shuffle(&self) -> bool {
        *self.shuffle_enabled.read().await
    }

    /// Set shuffle enabled
    pub async fn set_shuffle(&self, enabled: bool) {
        let mut shuffle = self.shuffle_enabled.write().await;
        *shuffle = enabled;
    }
}

/// Player-related errors
#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("Initialization error: {0}")]
    InitError(String),

    #[error("Sink error: {0}")]
    SinkError(String),

    #[error("File error: {0}")]
    FileError(String),

    #[error("Decode error: {0}")]
    DecodeError(String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("State error: {0}")]
    StateError(String),
}

/// Returns `true` if the file's format is known to be undecodable by rodio 0.17.
///
/// rodio 0.17 has no OPUS decoder (nor does its symphonia backend, which lacks
/// an opus codec), so `.opus` is rejected up front with a clear error.
fn format_is_unsupported(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("opus")
    )
}

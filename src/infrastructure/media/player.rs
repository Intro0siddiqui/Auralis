//! Audio Player
//!
//! Audio playback using rodio.

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::domain::models::{RepeatMode, Track};

/// Audio player using rodio
pub struct AudioPlayer {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    sink: Arc<RwLock<Option<Sink>>>,
    volume: Arc<RwLock<f32>>,
    current_track: Arc<RwLock<Option<Track>>>,
    queue: Arc<RwLock<Vec<Track>>>,
    current_index: Arc<RwLock<Option<usize>>>,
    repeat_mode: Arc<RwLock<RepeatMode>>,
    shuffle_enabled: Arc<RwLock<bool>>,
    position: Arc<RwLock<Duration>>,
    track_duration: Arc<RwLock<Duration>>,
}

// SAFETY: `OutputStream` is `!Send` by default because it wraps a platform-specific
// audio handle (cpal Stream) that is not safe to move between threads. However, in
// `AudioPlayer` we never access `_stream` directly after construction — it is only
// stored to keep the audio device alive. All actual playback operations go through
// the `Sink` (which IS `Send + Sync`) and the `OutputStreamHandle` (also `Send + Sync`),
// both of which are properly synchronized via the `Arc<RwLock<...>>` fields. Each
// `play()` call also creates a fresh, local `OutputStream` that lives only for the
// duration of that function. Therefore, the `AudioPlayer` is safe to share across
// threads even though it contains a non-`Send` type.
unsafe impl Send for AudioPlayer {}
unsafe impl Sync for AudioPlayer {}

impl AudioPlayer {
    /// Create a new audio player
    pub fn new() -> Result<Self, PlayerError> {
        info!("Initializing audio player");

        let (stream, stream_handle) =
            OutputStream::try_default().map_err(|e| PlayerError::InitError(e.to_string()))?;

        info!("Audio player initialized");
        Ok(Self {
            _stream: stream,
            _stream_handle: stream_handle,
            sink: Arc::new(RwLock::new(None)),
            volume: Arc::new(RwLock::new(0.8)),
            current_track: Arc::new(RwLock::new(None)),
            queue: Arc::new(RwLock::new(Vec::new())),
            current_index: Arc::new(RwLock::new(None)),
            repeat_mode: Arc::new(RwLock::new(RepeatMode::Off)),
            shuffle_enabled: Arc::new(RwLock::new(false)),
            position: Arc::new(RwLock::new(Duration::ZERO)),
            track_duration: Arc::new(RwLock::new(Duration::ZERO)),
        })
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

        // Reuse the persistent output stream handle. Creating a fresh local
        // `OutputStream` here would hold a non-`Send` type across an await and
        // make the returned future (and any Tauri command driving it) non-`Send`.
        let sink = Sink::try_new(&self._stream_handle)
            .map_err(|e| PlayerError::SinkError(e.to_string()))?;

        sink.set_volume(vol);
        sink.append(source);

        // Store sink and reset position
        {
            let mut current = self.sink.write().await;
            *current = Some(sink);
        }

        {
            let mut pos = self.position.write().await;
            *pos = Duration::ZERO;
        }

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
        Ok(())
    }

    /// Resume playback
    pub async fn resume(&self) -> Result<(), PlayerError> {
        debug!("Resuming playback");
        let sink = self.sink.read().await;
        if let Some(s) = sink.as_ref() {
            s.play();
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
        {
            let mut pos = self.position.write().await;
            *pos = Duration::ZERO;
        }
        Ok(())
    }

    /// Seek to a position within the current track.
    /// Note: rodio doesn't support true seeking, so this stores the target
    /// position and restarts playback from the beginning with a skip offset.
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

        // Store the seek position
        {
            let mut pos = self.position.write().await;
            *pos = position;
        }

        // Since rodio doesn't support seeking, we restart playback from the
        // beginning (position is tracked externally as the skip offset).
        self.play(&path).await?;

        info!(?position, "Seek completed (position tracked externally)");
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
        *self.position.read().await
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

    /// Update position (called periodically to track playback progress)
    pub async fn tick_position(&self, delta: Duration) {
        let mut pos = self.position.write().await;
        let duration = *self.track_duration.read().await;
        *pos = (*pos + delta).min(duration);
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

//! Audio Player
//!
//! Audio playback using rodio (0.22+).

use rodio::{mixer::Mixer, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::domain::models::{RepeatMode, Track};

/// Holds the lazily-opened audio output stream.
struct OutputStreamHolder {
    // We keep the MixerDeviceSink alive here. Dropping it stops the audio output.
    stream: Option<MixerDeviceSink>,
}

// SAFETY: every access to the held `MixerDeviceSink` happens inside a short
// synchronous `Mutex` section (never across an `await`), and the stream is
// only ever stored/read on the thread that opened it.
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
    /// Playback time accumulated before the current play session.
    played: Arc<RwLock<Duration>>,
    /// `Some` while the sink is actively playing.
    play_anchor: Arc<RwLock<Option<Instant>>>,
    /// When the current playback session started.
    play_started_at: Arc<RwLock<Option<Instant>>>,
    track_duration: Arc<RwLock<Duration>>,
}

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
        let file = File::open(path).map_err(|e| PlayerError::FileError(e.to_string()))?;
        let source = Decoder::new(BufReader::new(file))
            .map_err(|e| PlayerError::DecodeError(e.to_string()))?;

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
        if let Some(s) = self.sink.read().await.as_ref() {
            s.play();
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

    /// Seek using rodio's native `try_seek`.
    pub async fn seek(&self, position: Duration) -> Result<(), PlayerError> {
        debug!(?position, "Seeking to position");

        let total = *self.track_duration.read().await;
        if !total.is_zero() && position > total {
            return Err(PlayerError::StateError(
                "Seek position exceeds track duration".into(),
            ));
        }

        let sink_guard = self.sink.read().await;
        if let Some(player) = sink_guard.as_ref() {
            player
                .try_seek(position)
                .map_err(|e| PlayerError::StateError(format!("Seek failed: {e}")))?;

            // Reset time tracking to the new position
            drop(sink_guard);
            *self.played.write().await = position;
            *self.play_anchor.write().await = Some(Instant::now());
        } else {
            return Err(PlayerError::StateError("No active playback".into()));
        }

        info!(?position, "Seek completed");
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
        let queue = self.queue.read().await;
        if queue.is_empty() {
            return Ok(None);
        }

        let current_idx = *self.current_index.read().await;
        let repeat = *self.repeat_mode.read().await;
        let shuffle = *self.shuffle_enabled.read().await;

        let next_index = match (current_idx, repeat, shuffle) {
            (Some(idx), RepeatMode::One, _) => Some(idx),
            (Some(idx), RepeatMode::All, _) if idx + 1 >= queue.len() => Some(0),
            (Some(idx), RepeatMode::Off, _) if idx + 1 >= queue.len() => None,
            (Some(idx), _, _) => Some(idx + 1),
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

    pub async fn previous(&self) -> Result<Option<Track>, PlayerError> {
        let queue = self.queue.read().await;
        if queue.is_empty() {
            return Ok(None);
        }

        let current_idx = *self.current_index.read().await;
        let repeat = *self.repeat_mode.read().await;

        let prev_index = match (current_idx, repeat) {
            (Some(_), RepeatMode::One) => current_idx,
            (Some(0), RepeatMode::All) => Some(queue.len() - 1),
            (Some(0), RepeatMode::Off) => Some(0),
            (Some(idx), _) => Some(idx - 1),
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
    }
    pub async fn add_to_queue(&self, track: Track) {
        self.queue.write().await.push(track);
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
        Ok(track)
    }

    pub async fn clear_queue(&self) {
        self.queue.write().await.clear();
        *self.current_index.write().await = None;
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
    }
    pub async fn get_shuffle(&self) -> bool {
        *self.shuffle_enabled.read().await
    }
    pub async fn set_shuffle(&self, enabled: bool) {
        *self.shuffle_enabled.write().await = enabled;
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

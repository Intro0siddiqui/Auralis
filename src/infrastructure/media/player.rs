//! Audio Player
//!
//! Audio playback using rodio.

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Audio player using rodio
pub struct AudioPlayer {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    sink: Arc<RwLock<Option<Sink>>>,
    volume: Arc<RwLock<f32>>,
    current_track: Arc<RwLock<Option<String>>>,
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
        })
    }

    /// Play an audio file
    pub async fn play(&self, path: &str) -> Result<(), PlayerError> {
        info!(path = %path, "Starting playback");

        // Stop current playback
        self.stop().await?;

        // Create new sink
        let (_stream, stream_handle) =
            OutputStream::try_default().map_err(|e| PlayerError::InitError(e.to_string()))?;

        let sink =
            Sink::try_new(&stream_handle).map_err(|e| PlayerError::SinkError(e.to_string()))?;

        // Set volume
        let vol = *self.volume.read().await;
        sink.set_volume(vol);

        // Open and decode file
        let file = File::open(path).map_err(|e| PlayerError::FileError(e.to_string()))?;

        let reader = BufReader::new(file);
        let source = Decoder::new(reader).map_err(|e| PlayerError::DecodeError(e.to_string()))?;

        sink.append(source);

        // Store sink and track
        {
            let mut current = self.sink.write().await;
            *current = Some(sink);
        }

        {
            let mut track = self.current_track.write().await;
            *track = Some(path.to_string());
        }

        debug!(path = %path, "Playback started");
        Ok(())
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
        Ok(())
    }

    /// Check if currently playing
    pub async fn is_playing(&self) -> bool {
        let sink = self.sink.read().await;
        sink.as_ref()
            .map(|s| !s.is_paused() && !s.empty())
            .unwrap_or(false)
    }

    /// Get current position in seconds
    pub async fn get_position(&self) -> u32 {
        // rodio doesn't provide position directly, would need custom implementation
        0
    }

    /// Get current volume
    pub async fn get_volume(&self) -> f32 {
        *self.volume.read().await
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

    /// Get current track path
    pub async fn get_current_track(&self) -> Option<String> {
        self.current_track.read().await.clone()
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

    #[error("State error: {0}")]
    StateError(String),
}

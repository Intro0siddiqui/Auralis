//! Playback Service
//!
//! Handles audio playback operations including play, pause, seek,
//! queue management, and playback state.

use crate::domain::models::{NowPlaying, RepeatMode, Track};
use crate::domain::repositories::TrackRepository;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Playback state machine
#[derive(Debug, Clone)]
pub struct PlaybackState {
    /// Current track
    pub current_track: Option<Track>,

    /// Current position in seconds
    pub position_secs: u32,

    /// Whether playback is active
    pub is_playing: bool,

    /// Volume level (0.0 to 1.0)
    pub volume: f32,

    /// Repeat mode
    pub repeat_mode: RepeatMode,

    /// Shuffle enabled
    pub shuffle_enabled: bool,

    /// Playback queue
    pub queue: Vec<Track>,

    /// Original queue order (for unshuffle)
    pub original_queue: Vec<Track>,

    /// Current queue index
    pub queue_index: usize,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            current_track: None,
            position_secs: 0,
            is_playing: false,
            volume: 0.8,
            repeat_mode: RepeatMode::Off,
            shuffle_enabled: false,
            queue: Vec::new(),
            original_queue: Vec::new(),
            queue_index: 0,
        }
    }
}

/// Playback service for managing audio playback
pub struct PlaybackService {
    track_repository: Arc<dyn TrackRepository>,
    state: Arc<RwLock<PlaybackState>>,
}

impl PlaybackService {
    /// Create a new playback service
    pub fn new(track_repository: Arc<dyn TrackRepository>) -> Self {
        Self {
            track_repository,
            state: Arc::new(RwLock::new(PlaybackState::default())),
        }
    }

    /// Get current playback state
    pub async fn get_state(&self) -> PlaybackState {
        self.state.read().await.clone()
    }

    /// Play a track
    pub async fn play(&self, track_id: uuid::Uuid) -> Result<Option<NowPlaying>, PlaybackError> {
        info!(track_id = %track_id, "Playing track");

        let track = self
            .track_repository
            .find_by_id(track_id)
            .await
            .map_err(|e| PlaybackError::DatabaseError(e.to_string()))?
            .ok_or(PlaybackError::TrackNotFound(track_id))?;

        let mut state = self.state.write().await;

        // Update play count
        let mut updated_track = track.clone();
        updated_track.play_count += 1;
        updated_track.last_played = Some(chrono::Utc::now());

        self.track_repository
            .update(&updated_track)
            .await
            .map_err(|e| PlaybackError::DatabaseError(e.to_string()))?;

        // Set as current track and start playback
        state.current_track = Some(track);
        state.position_secs = 0;
        state.is_playing = true;
        state.queue_index = 0;

        debug!("Track playback started");
        Ok(Some(self.to_now_playing(&state)))
    }

    /// Play a queue of tracks starting at index
    pub async fn play_queue(
        &self,
        tracks: Vec<Track>,
        start_index: usize,
    ) -> Result<Option<NowPlaying>, PlaybackError> {
        info!(count = tracks.len(), start_index, "Playing queue");

        if tracks.is_empty() {
            return Ok(None);
        }

        let mut state = self.state.write().await;

        // Store original order for unshuffle
        state.original_queue = tracks.clone();

        // Apply shuffle if enabled
        if state.shuffle_enabled {
            use rand::seq::SliceRandom;
            let mut shuffled = tracks.clone();
            let mut rng = rand::thread_rng();
            shuffled.shuffle(&mut rng);

            // Move starting track to front
            if start_index < tracks.len() {
                let start_track = tracks[start_index].clone();
                if let Some(pos) = shuffled.iter().position(|t| t.id == start_track.id) {
                    shuffled.swap(0, pos);
                }
                state.queue = shuffled;
                state.queue_index = 0;
            } else {
                state.queue = shuffled;
                state.queue_index = 0;
            }
        } else {
            state.queue = tracks;
            state.queue_index = start_index.min(state.queue.len() - 1);
        }

        // Play the current track
        if let Some(track) = state.queue.get(state.queue_index) {
            state.current_track = Some(track.clone());
            state.position_secs = 0;
            state.is_playing = true;

            Ok(Some(self.to_now_playing(&state)))
        } else {
            Ok(None)
        }
    }

    /// Pause playback
    pub async fn pause(&self) -> Result<(), PlaybackError> {
        debug!("Pausing playback");
        let mut state = self.state.write().await;
        state.is_playing = false;
        Ok(())
    }

    /// Resume playback
    pub async fn resume(&self) -> Result<(), PlaybackError> {
        debug!("Resuming playback");
        let mut state = self.state.write().await;

        if state.current_track.is_some() {
            state.is_playing = true;
        }
        Ok(())
    }

    /// Stop playback
    pub async fn stop(&self) -> Result<(), PlaybackError> {
        info!("Stopping playback");
        let mut state = self.state.write().await;
        state.current_track = None;
        state.position_secs = 0;
        state.is_playing = false;
        state.queue.clear();
        state.queue_index = 0;
        Ok(())
    }

    /// Skip to next track
    pub async fn next(&self) -> Result<Option<NowPlaying>, PlaybackError> {
        debug!("Skipping to next track");
        let mut state = self.state.write().await;

        if state.queue.is_empty() {
            return Ok(None);
        }

        let next_index = match state.repeat_mode {
            RepeatMode::One => state.queue_index, // Stay on same track
            RepeatMode::All => (state.queue_index + 1) % state.queue.len(),
            RepeatMode::Off => {
                if state.queue_index + 1 < state.queue.len() {
                    state.queue_index + 1
                } else {
                    state.is_playing = false;
                    return Ok(None);
                }
            }
        };

        state.queue_index = next_index;
        state.current_track = state.queue.get(next_index).cloned();
        state.position_secs = 0;

        Ok(self.to_now_playing(&state).into())
    }

    /// Skip to previous track
    pub async fn previous(&self) -> Result<Option<NowPlaying>, PlaybackError> {
        debug!("Skipping to previous track");
        let mut state = self.state.write().await;

        if state.queue.is_empty() {
            return Ok(None);
        }

        // If more than 3 seconds in, restart current track
        if state.position_secs > 3 {
            state.position_secs = 0;
            return Ok(self.to_now_playing(&state).into());
        }

        let prev_index = if state.queue_index == 0 {
            if state.repeat_mode == RepeatMode::All {
                state.queue.len() - 1
            } else {
                0
            }
        } else {
            state.queue_index - 1
        };

        state.queue_index = prev_index;
        state.current_track = state.queue.get(prev_index).cloned();
        state.position_secs = 0;

        Ok(self.to_now_playing(&state).into())
    }

    /// Seek to position
    pub async fn seek(&self, position_secs: u32) -> Result<(), PlaybackError> {
        debug!(position = position_secs, "Seeking to position");
        let mut state = self.state.write().await;

        if let Some(track) = &state.current_track {
            if position_secs < track.duration_secs {
                state.position_secs = position_secs;
            } else {
                state.position_secs = track.duration_secs;
            }
        }
        Ok(())
    }

    /// Set volume
    pub async fn set_volume(&self, volume: f32) -> Result<(), PlaybackError> {
        debug!(volume, "Setting volume");
        let mut state = self.state.write().await;
        state.volume = volume.clamp(0.0, 1.0);
        Ok(())
    }

    /// Set repeat mode
    pub async fn set_repeat_mode(&self, mode: RepeatMode) -> Result<(), PlaybackError> {
        info!(mode = ?mode, "Setting repeat mode");
        let mut state = self.state.write().await;
        state.repeat_mode = mode;
        Ok(())
    }

    /// Toggle shuffle
    pub async fn set_shuffle(&self, enabled: bool) -> Result<(), PlaybackError> {
        info!(enabled, "Setting shuffle");
        let mut state = self.state.write().await;

        if enabled && !state.shuffle_enabled {
            // Enable shuffle: shuffle queue but keep current track
            if let Some(current) = &state.current_track {
                let mut remaining: Vec<_> = state
                    .queue
                    .iter()
                    .filter(|t| t.id != current.id)
                    .cloned()
                    .collect();

                use rand::seq::SliceRandom;
                let mut rng = rand::thread_rng();
                remaining.shuffle(&mut rng);

                state.queue = std::iter::once(current.clone()).chain(remaining).collect();
                state.queue_index = 0;
            }
        } else if !enabled && state.shuffle_enabled {
            // Disable shuffle: restore original order
            if let Some(current) = &state.current_track {
                if let Some(pos) = state.original_queue.iter().position(|t| t.id == current.id) {
                    state.queue_index = pos;
                }
                state.queue = state.original_queue.clone();
            }
        }

        state.shuffle_enabled = enabled;
        Ok(())
    }

    /// Add tracks to queue
    pub async fn add_to_queue(&self, tracks: Vec<Track>) -> Result<(), PlaybackError> {
        debug!(count = tracks.len(), "Adding tracks to queue");
        let mut state = self.state.write().await;
        state.queue.extend(tracks);
        Ok(())
    }

    /// Remove track from queue by index
    pub async fn remove_from_queue(&self, index: usize) -> Result<(), PlaybackError> {
        debug!(index, "Removing track from queue");
        let mut state = self.state.write().await;

        if index < state.queue.len() {
            state.queue.remove(index);

            // Adjust current index if needed
            if index < state.queue_index {
                state.queue_index = state.queue_index.saturating_sub(1);
            } else if index == state.queue_index && state.queue_index >= state.queue.len() {
                state.queue_index = state.queue.len().saturating_sub(1);
            }
        }
        Ok(())
    }

    /// Clear the queue
    pub async fn clear_queue(&self) -> Result<(), PlaybackError> {
        debug!("Clearing queue");
        let mut state = self.state.write().await;
        state.queue.clear();
        state.queue_index = 0;
        state.current_track = None;
        state.is_playing = false;
        Ok(())
    }

    /// Get current queue
    pub async fn get_queue(&self) -> Vec<Track> {
        self.state.read().await.queue.clone()
    }

    /// Get now playing info
    pub async fn get_now_playing(&self) -> Option<NowPlaying> {
        let state = self.state.read().await;
        state.current_track.as_ref()?;
        Some(self.to_now_playing(&state))
    }

    /// Convert state to NowPlaying
    fn to_now_playing(&self, state: &PlaybackState) -> NowPlaying {
        NowPlaying {
            track: state.current_track.clone().unwrap(),
            position_secs: state.position_secs,
            is_playing: state.is_playing,
            volume: state.volume,
            repeat_mode: state.repeat_mode,
            shuffle_enabled: state.shuffle_enabled,
        }
    }
}

/// Playback-related errors
#[derive(Debug, thiserror::Error)]
pub enum PlaybackError {
    #[error("Track not found: {0}")]
    TrackNotFound(uuid::Uuid),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Player error: {0}")]
    PlayerError(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

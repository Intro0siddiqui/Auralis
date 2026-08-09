//! Playback Commands
//!
//! Tauri command handlers for the audio playback domain.

use crate::templates::render;
use crate::domain::models::{NowPlaying, RepeatMode, Track};
use crate::templates::{NowPlayingPartial, QueuePartial};
use serde::{Deserialize, Serialize};
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

/// Start playback of a track (optionally via a queue index).
#[tauri::command]
pub async fn play(_track_id: Uuid, _queue_index: Option<usize>) -> Result<NowPlaying, String> {
    // TODO: delegate to AudioPlayer::play
    Err("play not yet implemented".to_string())
}

/// Pause current playback.
#[tauri::command]
pub async fn pause() -> Result<(), String> {
    // TODO: AudioPlayer::pause
    Ok(())
}

/// Resume paused playback.
#[tauri::command]
pub async fn resume() -> Result<(), String> {
    // TODO: AudioPlayer::resume
    Ok(())
}

/// Stop playback and clear the queue.
#[tauri::command]
pub async fn stop() -> Result<(), String> {
    // TODO: AudioPlayer::stop
    Ok(())
}

/// Skip to next track in queue.
#[tauri::command]
pub async fn next_track() -> Result<Option<NowPlaying>, String> {
    // TODO: AudioPlayer::next
    Ok(None)
}

/// Go back to previous track.
#[tauri::command]
pub async fn previous_track() -> Result<Option<NowPlaying>, String> {
    // TODO: AudioPlayer::previous
    Ok(None)
}

/// Seek to a position within the current track.
#[tauri::command]
pub async fn seek(_request: SeekRequest) -> Result<(), String> {
    // TODO: AudioPlayer::seek
    Ok(())
}

/// Set the output volume (0.0..=1.0).
#[tauri::command]
pub async fn set_volume(volume: f32) -> Result<(), String> {
    if !(0.0..=1.0).contains(&volume) {
        return Err("Volume must be between 0.0 and 1.0".to_string());
    }
    // TODO: AudioPlayer::set_volume
    Ok(())
}

/// Set the repeat mode.
#[tauri::command]
pub async fn set_repeat_mode(_mode: RepeatMode) -> Result<(), String> {
    // TODO: store on PlaybackState
    Ok(())
}

/// Toggle shuffle mode.
#[tauri::command]
pub async fn set_shuffle(_enabled: bool) -> Result<(), String> {
    // TODO: shuffle queue
    Ok(())
}

/// Get current now-playing state.
#[tauri::command]
pub async fn get_now_playing() -> Result<Option<NowPlaying>, String> {
    // TODO: read from player state
    Ok(None)
}

/// Get the current playback queue.
#[tauri::command]
pub async fn get_queue() -> Result<PlaybackQueue, String> {
    // TODO: read queue from state
    Ok(PlaybackQueue::default())
}

/// Append a track to the queue.
#[tauri::command]
pub async fn add_to_queue(_track_id: Uuid) -> Result<PlaybackQueue, String> {
    // TODO: append to queue
    Ok(PlaybackQueue::default())
}

/// Remove the track at the given queue index.
#[tauri::command]
pub async fn remove_from_queue(_index: usize) -> Result<PlaybackQueue, String> {
    // TODO: remove from queue
    Ok(PlaybackQueue::default())
}

/// Clear the playback queue.
#[tauri::command]
pub async fn clear_queue() -> Result<PlaybackQueue, String> {
    // TODO: clear queue
    Ok(PlaybackQueue::default())
}

/// Render the now-playing bar as an HTML fragment.
#[tauri::command]
pub async fn render_now_playing() -> Result<String, String> {
    let now_playing: Option<NowPlaying> = None; // TODO: read state
    let tmpl = NowPlayingPartial {
        now_playing: &now_playing,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render the queue as an HTML fragment.
#[tauri::command]
pub async fn render_queue() -> Result<String, String> {
    let queue: Vec<Track> = Vec::new();
    let tmpl = QueuePartial {
        queue: &queue,
        current_index: None,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

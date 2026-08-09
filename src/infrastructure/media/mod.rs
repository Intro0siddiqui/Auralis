//! Media Infrastructure Module
//!
//! Handles audio playback and media downloading.

mod downloader;
mod player;

pub use downloader::Downloader;
pub use player::AudioPlayer;

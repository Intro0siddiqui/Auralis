//! Media Infrastructure Module
//!
//! Handles audio playback and media downloading.

pub mod android_downloads;
pub mod background_service;
pub mod downloader;
pub mod opus;
pub mod player;

pub use downloader::Downloader;
pub use opus::{extract_opus_metadata, OpusSource};
pub use player::AudioPlayer;

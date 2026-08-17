//! Media Infrastructure Module
//!
//! Handles audio playback and media downloading.

pub mod background_service;
pub mod downloader;
pub mod player;

pub use downloader::Downloader;
pub use player::AudioPlayer;

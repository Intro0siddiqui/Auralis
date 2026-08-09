//! Settings Model
//!
//! Application configuration and user preferences.

use crate::AudioFormat;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Audio output settings
    pub audio: AudioSettings,

    /// Download settings
    pub downloads: DownloadSettings,

    /// Library settings
    pub library: LibrarySettings,

    /// Appearance settings
    pub appearance: AppearanceSettings,

    /// Sync settings
    pub sync: SyncSettings,

    /// Language code (e.g., "en", "es", "ja")
    pub language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            audio: AudioSettings::default(),
            downloads: DownloadSettings::default(),
            library: LibrarySettings::default(),
            appearance: AppearanceSettings::default(),
            sync: SyncSettings::default(),
            language: "en".to_string(),
        }
    }
}

/// Audio playback settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSettings {
    /// Output device name (empty for system default)
    pub output_device: String,

    /// Volume level (0.0 to 1.0)
    pub volume: f32,

    /// Enable gapless playback
    pub gapless_playback: bool,

    /// Audio normalization
    pub normalize_audio: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_device: String::new(),
            volume: 0.8,
            gapless_playback: true,
            normalize_audio: false,
        }
    }
}

/// Download preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSettings {
    /// Download directory
    pub download_path: PathBuf,

    /// Default audio format
    pub default_format: AudioFormat,

    /// Default audio quality (bitrate in kbps)
    pub default_quality: u32,

    /// Concurrent download limit
    pub max_concurrent: u32,

    /// Auto-download thumbnail/cover art
    pub embed_artwork: bool,

    /// Embed metadata in files
    pub embed_metadata: bool,
}

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            download_path: dirs::audio_dir().unwrap_or_else(|| PathBuf::from(".")),
            default_format: AudioFormat::Mp3,
            default_quality: 320,
            max_concurrent: 3,
            embed_artwork: true,
            embed_metadata: true,
        }
    }
}

/// Library management settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySettings {
    /// Directories to scan for music
    pub scan_paths: Vec<PathBuf>,

    /// Watch directories for changes
    pub watch_for_changes: bool,

    /// Auto-import downloaded tracks to library
    pub auto_import: bool,

    /// File patterns to include in scan
    pub include_patterns: Vec<String>,

    /// File patterns to exclude from scan
    pub exclude_patterns: Vec<String>,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        let music_dir = dirs::audio_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            scan_paths: vec![music_dir],
            watch_for_changes: true,
            auto_import: true,
            include_patterns: vec![
                "*.mp3".to_string(),
                "*.flac".to_string(),
                "*.m4a".to_string(),
                "*.ogg".to_string(),
                "*.wav".to_string(),
                "*.aac".to_string(),
            ],
            exclude_patterns: vec![".*".to_string(), ".*/".to_string()],
        }
    }
}

/// Appearance and theme settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    /// Theme mode
    pub theme: ThemeMode,

    /// Accent color (hex without #)
    pub accent_color: String,

    /// Show album art in library
    pub show_album_art: bool,

    /// Grid or list view for library
    pub library_view: LibraryView,

    /// Compact track rows
    pub compact_rows: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            accent_color: "4F46E5".to_string(), // Indigo
            show_album_art: true,
            library_view: LibraryView::Grid,
            compact_rows: false,
        }
    }
}

/// Theme mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    System,
}

impl fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeMode::Light => write!(f, "light"),
            ThemeMode::Dark => write!(f, "dark"),
            ThemeMode::System => write!(f, "system"),
        }
    }
}

impl PartialEq<&str> for ThemeMode {
    fn eq(&self, other: &&str) -> bool {
        let self_str = match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
            ThemeMode::System => "system",
        };
        self_str == *other
    }
}

/// Library display mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryView {
    Grid,
    List,
}

/// P2P synchronization settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSettings {
    /// Enable P2P sync
    pub enabled: bool,

    /// Device name (for identification)
    pub device_name: String,

    /// Auto-accept pairing requests
    pub auto_accept_pairing: bool,

    /// Sync only on WiFi
    pub wifi_only: bool,

    /// Sync playback state
    pub sync_playback: bool,

    /// Sync library changes
    pub sync_library: bool,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            device_name: hostname::get()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|_| "Auralis Device".to_string()),
            auto_accept_pairing: false,
            wifi_only: true,
            sync_playback: true,
            sync_library: true,
        }
    }
}

impl Settings {
    /// Load settings from disk
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_path = Self::config_path()?;

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let settings: Settings = toml::from_str(&content)?;
            Ok(settings)
        } else {
            let settings = Settings::default();
            settings.save()?;
            Ok(settings)
        }
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config_path = Self::config_path()?;
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    /// Get the configuration file path
    fn config_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let config_dir = dirs::config_dir().ok_or("Failed to get config directory")?;
        Ok(config_dir.join("auralis").join("settings.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert_eq!(settings.language, "en");
        assert_eq!(settings.audio.volume, 0.8);
    }

    #[test]
    fn test_settings_serialization() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.language, settings.language);
    }
}

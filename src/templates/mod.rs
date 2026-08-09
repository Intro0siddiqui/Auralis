//! HTML Template Engine
//!
//! Renders server-side HTML fragments consumed by HTMX. Templates are
//! embedded at compile time so the binary is self-contained.

use crate::domain::models::{
    Album, Artist, DownloadProgress, NowPlaying, PairedDevice, Playlist, Settings, Track,
    TrackFilter,
};
use askama::Template;
use serde::Serialize;

/// Layout-level template shared across pages
#[derive(Template)]
#[template(path = "layout.html")]
pub struct LayoutTemplate<'a> {
    pub title: &'a str,
    pub active_page: &'a str,
    pub content: String,
    pub now_playing: Option<NowPlaying>,
    pub settings: Option<Settings>,
}

/// Library page
#[derive(Template)]
#[template(path = "library.html")]
pub struct LibraryTemplate<'a> {
    pub tracks: &'a [Track],
    pub filter: &'a TrackFilter,
    pub total_count: usize,
    pub show_album: bool,
}

/// Album grid view
#[derive(Template)]
#[template(path = "albums.html")]
pub struct AlbumsTemplate<'a> {
    pub albums: &'a [Album],
}

/// Artists list
#[derive(Template)]
#[template(path = "artists.html")]
pub struct ArtistsTemplate<'a> {
    pub artists: &'a [Artist],
}

/// Downloads page
#[derive(Template)]
#[template(path = "downloads.html")]
pub struct DownloadsTemplate<'a> {
    pub downloads: &'a [DownloadProgress],
}

/// Playlists page
#[derive(Template)]
#[template(path = "playlists.html")]
pub struct PlaylistsTemplate<'a> {
    pub playlists: &'a [Playlist],
}

/// Single playlist detail
#[derive(Template)]
#[template(path = "playlist_detail.html")]
pub struct PlaylistDetailTemplate<'a> {
    pub playlist: &'a Playlist,
    pub tracks: &'a [Track],
    pub show_album: bool,
}

/// Sync / devices page
#[derive(Template)]
#[template(path = "sync.html")]
pub struct SyncTemplate<'a> {
    pub devices: &'a [PairedDevice],
}

/// Settings page
#[derive(Template)]
#[template(path = "settings.html")]
pub struct SettingsTemplate<'a> {
    pub settings: &'a Settings,
}

/// Search results partial
#[derive(Template)]
#[template(path = "partials/search_results.html")]
pub struct SearchResultsTemplate<'a> {
    pub tracks: &'a [Track],
    pub query: &'a str,
    pub show_album: bool,
}

/// Track list partial (used for HTMX swaps)
#[derive(Template)]
#[template(path = "partials/track_list.html")]
pub struct TrackListPartial<'a> {
    pub tracks: &'a [Track],
    pub show_album: bool,
}

/// Single track row partial
#[derive(Template)]
#[template(path = "partials/track_row.html")]
pub struct TrackRowPartial<'a> {
    pub track: &'a Track,
    pub show_album: bool,
}

/// Now playing bar partial
#[derive(Template)]
#[template(path = "partials/now_playing.html")]
pub struct NowPlayingPartial<'a> {
    pub now_playing: &'a Option<NowPlaying>,
}

/// Queue partial
#[derive(Template)]
#[template(path = "partials/queue.html")]
pub struct QueuePartial<'a> {
    pub queue: &'a [Track],
    pub current_index: Option<&'a usize>,
}

/// Download item partial
#[derive(Template)]
#[template(path = "partials/download_item.html")]
pub struct DownloadItemPartial<'a> {
    pub download: &'a DownloadProgress,
}

/// Toast / notification partial
#[derive(Template)]
#[template(path = "partials/toast.html")]
pub struct ToastPartial<'a> {
    pub message: &'a str,
    pub level: &'a str, // "info" | "success" | "warn" | "error"
}

/// Helper to convert a value to JSON for inline data attributes
pub fn json_value<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

/// Helper to render any askama template to a String
pub fn render<T: Template>(template: &T) -> Result<String, askama::Error> {
    template.render()
}

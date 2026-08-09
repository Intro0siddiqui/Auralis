//! Template Commands
//!
//! Tauri command handlers for HTML template rendering. These are the
//! primary entry points used by the HTMX frontend — every navigation,
//! fragment swap, and SSE-style update is one of these commands.

use crate::templates::render;
use crate::templates::{
    AlbumsTemplate, ArtistsTemplate, LibraryTemplate, SettingsTemplate,
    SyncTemplate, DownloadsTemplate, PlaylistsTemplate,
};
use crate::domain::models::{Album, Artist, DownloadProgress, PairedDevice, Playlist, Settings, Track, TrackFilter};
use askama::Template;
use serde::{Deserialize, Serialize};

/// Template names that can be requested from the frontend.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateName {
    Layout,
    Library,
    Albums,
    Artists,
    Downloads,
    Playlists,
    Sync,
    Settings,
}

/// Generic template render request.
#[derive(Debug, Serialize, Deserialize)]
pub struct RenderTemplateRequest {
    pub template: TemplateName,
    pub data: serde_json::Value,
}

/// Render a full page template by name.
#[tauri::command]
pub async fn render_template(request: RenderTemplateRequest) -> Result<String, String> {
    match request.template {
        TemplateName::Library => {
            let filter: TrackFilter = serde_json::from_value(request.data)
                .unwrap_or_default();
            let tracks: Vec<Track> = Vec::new();
            let tmpl = LibraryTemplate {
                tracks: &tracks,
                filter: &filter,
                total_count: 0,
                show_album: true,
            };
            render(&tmpl).map_err(|e| e.to_string())
        }
        TemplateName::Albums => {
            let albums: Vec<Album> = Vec::new();
            let tmpl = AlbumsTemplate { albums: &albums };
            render(&tmpl).map_err(|e| e.to_string())
        }
        TemplateName::Artists => {
            let artists: Vec<Artist> = Vec::new();
            let tmpl = ArtistsTemplate { artists: &artists };
            render(&tmpl).map_err(|e| e.to_string())
        }
        TemplateName::Downloads => {
            let downloads: Vec<DownloadProgress> = Vec::new();
            let tmpl = DownloadsTemplate { downloads: &downloads };
            render(&tmpl).map_err(|e| e.to_string())
        }
        TemplateName::Playlists => {
            let playlists: Vec<Playlist> = Vec::new();
            let tmpl = PlaylistsTemplate { playlists: &playlists };
            render(&tmpl).map_err(|e| e.to_string())
        }
        TemplateName::Sync => {
            let devices: Vec<PairedDevice> = Vec::new();
            let tmpl = SyncTemplate { devices: &devices };
            render(&tmpl).map_err(|e| e.to_string())
        }
        TemplateName::Settings => {
            let settings = Settings::default();
            let tmpl = SettingsTemplate { settings: &settings };
            render(&tmpl).map_err(|e| e.to_string())
        }
        TemplateName::Layout => {
            // Layout is a wrapper — render a basic shell with placeholder content.
            Ok(render_layout_shell("Auralis", "library", None).map_err(|e| e.to_string())?)
        }
    }
}

/// Render a small partial by name (used for HTMX swaps).
#[tauri::command]
pub async fn render_partial(name: String, data: serde_json::Value) -> Result<String, String> {
    // TODO: dispatch to the right partial template by name.
    // For now, return a generic empty fragment so the UI never breaks.
    Ok(format!(
        r#"<div class="partial partial-{}" data-empty="true">{}</div>"#,
        name,
        serde_json::to_string(&data).unwrap_or_default()
    ))
}

/// Helper used by the Layout template and the render_template command.
pub fn render_layout_shell(
    title: &str,
    active_page: &str,
    content: Option<String>,
) -> Result<String, askama::Error> {
    use crate::templates::LayoutTemplate;
    let tmpl = LayoutTemplate {
        title,
        active_page,
        content: content.unwrap_or_default(),
        now_playing: None,
        settings: None,
    };
    tmpl.render()
}

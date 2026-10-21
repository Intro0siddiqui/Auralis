//! Library HTML View Generators
//!
//! HTMX view partial generators and HTML escaping / formatting helpers.

use crate::infrastructure::database::Database;
use std::sync::Arc;
use tauri::State;

/// Escape HTML special characters for safe insertion into HTML strings.
///
/// Covers the five characters that matter in double-quoted attribute and
/// text contexts (`& < > " '`), plus backtick and forward slash as
/// defense-in-depth for unquoted-attribute and inline-JS contexts. The
/// current call sites only use double-quoted attributes, where the first
/// five suffice — the last two are hardening, not a vulnerability fix.
pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('`', "&#96;")
        .replace('/', "&#x2F;")
}

/// Format duration in seconds as M:SS.
pub(crate) fn format_time(secs: u32) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

/// Render cover artwork image tag with fallback to Lucide icon.
pub(crate) fn render_art_tag(art_path: Option<&str>, alt_text: &str, icon_name: &str) -> String {
    match art_path {
        Some(path) if !path.trim().is_empty() => {
            let safe_alt = html_escape(alt_text);
            let safe_path = html_escape(path);
            let json_path =
                serde_json::to_string(path).unwrap_or_else(|_| format!("\"{}\"", safe_path));
            let safe_json_path = html_escape(&json_path);
            format!(
                r#"<img src="{safe_path}" alt="{safe_alt}" data-art-path="{safe_path}" onerror="if(!this.dataset.fb){{this.dataset.fb='1';window.Auralis&amp;&amp;window.Auralis.bridge&amp;&amp;window.Auralis.bridge.embedArt(this,{safe_json_path})}}">"#
            )
        }
        _ => format!(r#"<i data-lucide="{icon_name}"></i>"#),
    }
}

/// Render a single track row HTML.
pub(crate) fn render_track_row_html(
    id: &str,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    duration_secs: u32,
    album_art_path: Option<&str>,
    is_favorite: bool,
) -> String {
    let safe_id = html_escape(id);
    let safe_title = html_escape(title);
    let artist_str = artist.unwrap_or("Unknown Artist");
    let safe_artist = html_escape(artist_str);
    let album_str = album.unwrap_or("Single");
    let safe_album = html_escape(album_str);
    let duration_str = format_time(duration_secs);
    let is_fav_class = if is_favorite { " liked" } else { "" };
    let is_fav_style = if is_favorite {
        r#" style="color: var(--like);""#
    } else {
        ""
    };
    let art_html = render_art_tag(album_art_path, title, "music");

    format!(
        r#"<div class="track-row glass-weak neu-glass" data-track-id="{safe_id}" data-role="play-row" style="cursor: pointer; margin-bottom: var(--space-2); border-radius: var(--radius-md); touch-action: manipulation;">
    <div class="track-row-artwork">
        {art_html}
    </div>
    <div class="track-row-info">
        <div class="track-row-title">{safe_title}</div>
        <div class="track-row-subtitle">{safe_artist} — {safe_album}</div>
    </div>
    <span class="track-row-duration">{duration_str}</span>
    <div class="track-row-actions">
        <button type="button" class="btn btn-ghost btn-icon play-track-btn" 
                title="Play" data-role="play-btn" data-track-id="{safe_id}" 
                onclick="event.stopPropagation(); window._safePlayTrack ? window._safePlayTrack('{safe_id}') : (window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.playTrack('{safe_id}'))" 
                ontouchend="event.stopPropagation(); window._safePlayTrack ? window._safePlayTrack('{safe_id}') : (window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.playTrack('{safe_id}'))">
            <i data-lucide="play"></i>
        </button>
        <button type="button" class="btn btn-ghost btn-icon{is_fav_class}"{is_fav_style} title="Like" onclick="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.toggleTrackFavorite('{safe_id}', this)" ontouchend="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.toggleTrackFavorite('{safe_id}', this)">
            <i data-lucide="heart"></i>
        </button>
        <button type="button" class="btn btn-ghost btn-icon track-menu-btn" 
                title="More options" data-role="menu-btn" data-track-id="{safe_id}" 
                onclick="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.openTrackContextMenu(event, '{safe_id}')" 
                ontouchend="event.stopPropagation(); window.Auralis &amp;&amp; window.Auralis.bridge &amp;&amp; window.Auralis.bridge.openTrackContextMenu(event, '{safe_id}')">
            <i data-lucide="more-vertical"></i>
        </button>
    </div>
</div>"#
    )
}

/// Render HTML grid of albums from database.
#[tauri::command]
pub async fn get_albums_grid_html(db: State<'_, Database>) -> Result<String, String> {
    let repo = crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
        db.inner().clone(),
    ));
    let albums = repo.get_albums_summary().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch albums summary");
        format!("Failed to fetch albums summary: {e}")
    })?;

    if albums.is_empty() {
        return Ok(r#"<div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
    <i data-lucide="disc-3" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
    <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No albums found</h3>
    <p style="color: var(--text-3); font-size: var(--text-sm);">Scan your music library to populate your album catalog.</p>
</div>"#.to_string());
    }

    let mut html = String::new();
    for album in albums {
        let safe_name = html_escape(&album.album);
        let artist_str = album.artist.as_deref().unwrap_or("Unknown Artist");
        let safe_artist = html_escape(artist_str);
        let first_id = album.first_track_id.as_deref().unwrap_or("");
        let safe_first_id = html_escape(first_id);
        let art_html = render_art_tag(album.album_art_path.as_deref(), &album.album, "disc-3");
        let track_count = album.track_count;
        let track_word = if track_count == 1 { "track" } else { "tracks" };

        html.push_str(&format!(
            r#"<div class="card card--glass neu-glass album-card" data-album-name="{safe_name}" data-first-track-id="{safe_first_id}" data-track-id="{safe_first_id}" data-role="play-card" style="cursor: pointer; touch-action: manipulation;">
    <div class="card-artwork">
        {art_html}
    </div>
    <div class="card-body">
        <div class="card-title">{safe_name}</div>
        <div class="card-subtitle">{safe_artist} · {track_count} {track_word}</div>
    </div>
</div>"#
        ));
    }

    Ok(html)
}

/// Render HTML grid of artists from database.
#[tauri::command]
pub async fn get_artists_grid_html(db: State<'_, Database>) -> Result<String, String> {
    let repo = crate::infrastructure::database::repositories::SqliteTrackRepository::new(Arc::new(
        db.inner().clone(),
    ));
    let artists = repo.get_artists_summary().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to fetch artists summary");
        format!("Failed to fetch artists summary: {e}")
    })?;

    if artists.is_empty() {
        return Ok(r#"<div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
    <i data-lucide="users" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
    <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No artists found</h3>
    <p style="color: var(--text-3); font-size: var(--text-sm);">Scan your music library to discover artists.</p>
</div>"#.to_string());
    }

    let mut html = String::new();
    for artist in artists {
        let safe_name = html_escape(&artist.artist);
        let first_id = artist.first_track_id.as_deref().unwrap_or("");
        let safe_first_id = html_escape(first_id);
        let track_count = artist.track_count;
        let track_word = if track_count == 1 { "track" } else { "tracks" };

        html.push_str(&format!(
            r#"<div class="card card--glass neu-glass artist-card" data-artist-name="{safe_name}" data-first-track-id="{safe_first_id}" data-track-id="{safe_first_id}" data-role="play-card" style="cursor: pointer; touch-action: manipulation;">
    <div class="card-artwork">
        <i data-lucide="user"></i>
    </div>
    <div class="card-body">
        <div class="card-title">{safe_name}</div>
        <div class="card-subtitle">{track_count} {track_word}</div>
    </div>
</div>"#
        ));
    }

    Ok(html)
}

/// Render HTML track rows for library view.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_library_tracks_html(
    db: State<'_, Database>,
    sort_by: Option<String>,
    downloaded_only: Option<bool>,
    artist: Option<String>,
    album: Option<String>,
    search: Option<String>,
) -> Result<String, String> {
    let conn = db.connection().map_err(|e| e.to_string())?;

    let mut sql = "SELECT id, title, artist, album, duration_secs, album_art_path, is_favorite FROM tracks WHERE 1=1".to_string();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if downloaded_only == Some(true) {
        sql.push_str(
            " AND (is_downloaded = 1 OR (file_path IS NOT NULL AND file_path NOT LIKE 'http%'))",
        );
    }

    if let Some(ref a) = artist {
        if !a.trim().is_empty() {
            sql.push_str(" AND artist = ?");
            params.push(Box::new(a.clone()));
        }
    }

    if let Some(ref alb) = album {
        if !alb.trim().is_empty() {
            sql.push_str(" AND album = ?");
            params.push(Box::new(alb.clone()));
        }
    }

    if let Some(ref q) = search {
        let trimmed = q.trim();
        if !trimmed.is_empty() {
            sql.push_str(" AND (title LIKE ? OR artist LIKE ? OR album LIKE ?)");
            let pattern = format!("%{trimmed}%");
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern));
        }
    }

    let sort_field = sort_by.as_deref().unwrap_or("date_added");
    match sort_field {
        "title" => sql.push_str(" ORDER BY title COLLATE NOCASE ASC, artist COLLATE NOCASE ASC"),
        "artist" => sql.push_str(" ORDER BY artist COLLATE NOCASE ASC, album COLLATE NOCASE ASC, track_number ASC, title COLLATE NOCASE ASC"),
        "album" => sql.push_str(" ORDER BY album COLLATE NOCASE ASC, disc_number ASC, track_number ASC, title COLLATE NOCASE ASC"),
        _ => sql.push_str(" ORDER BY date_added DESC, id DESC"),
    }

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare tracks query: {e}"))?;

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)? != 0,
            ))
        })
        .map_err(|e| format!("Failed to query library tracks: {e}"))?;

    let mut html = String::new();
    let mut count = 0;

    for row_res in rows {
        let (id, title, artist, album, duration_secs, art_path, is_fav) =
            row_res.map_err(|e| format!("Row error: {e}"))?;
        count += 1;

        html.push_str(&render_track_row_html(
            &id,
            &title,
            artist.as_deref(),
            album.as_deref(),
            duration_secs,
            art_path.as_deref(),
            is_fav,
        ));
    }

    if count == 0 {
        return Ok(r#"<div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center;">
    <div class="empty-state-icon" style="color: var(--accent); margin-bottom: var(--space-4);">
        <i data-lucide="music" style="width: 48px; height: 48px;"></i>
    </div>
    <h2 class="empty-state-title" style="color: var(--text-1); font-size: var(--text-xl); margin-bottom: var(--space-2);">No tracks found in library</h2>
    <p class="empty-state-description" style="color: var(--text-2); margin-bottom: var(--space-6); max-width: 420px; margin-left: auto; margin-right: auto;">Scan your device storage or download music to start listening.</p>
    <div style="display: flex; gap: var(--space-3); flex-wrap: wrap; justify-content: center;">
        <label class="btn btn-primary neu" for="global-audio-import-input" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="file-plus-2"></i>
            Import Audio
        </label>
        <button type="button" class="btn btn-secondary neu" onclick="window.Auralis.bridge.triggerFolderScan()" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="folder-search"></i>
            Scan Storage
        </button>
    </div>
</div>"#.to_string());
    }

    Ok(html)
}

/// Render HTML shelves for home view.
#[tauri::command]
pub async fn get_home_shelves_html(db: State<'_, Database>) -> Result<String, String> {
    let conn = db.connection().map_err(|e| e.to_string())?;

    let total_tracks: i64 = conn
        .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
        .unwrap_or(0);

    if total_tracks == 0 {
        return Ok(r##"<div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center; margin-top: var(--space-4);">
    <div class="empty-state-icon" style="color: var(--accent); margin-bottom: var(--space-4);">
        <i data-lucide="music" style="width: 48px; height: 48px;"></i>
    </div>
    <h2 class="empty-state-title" style="color: var(--text-1); font-size: var(--text-xl); margin-bottom: var(--space-2);">Your library is empty</h2>
    <p class="empty-state-description" style="color: var(--text-2); margin-bottom: var(--space-6); max-width: 420px; margin-left: auto; margin-right: auto;">Import audio files from your device storage or download music to start playing.</p>
    <div style="display: flex; gap: var(--space-3); flex-wrap: wrap; justify-content: center;">
        <label class="btn btn-primary neu" for="global-audio-import-input" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="file-plus-2"></i>
            Import Audio
        </label>
        <button type="button" class="btn btn-secondary neu" onclick="window.Auralis.bridge.triggerFolderScan()" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="folder-search"></i>
            Scan Storage
        </button>
        <button type="button" class="btn btn-secondary neu" hx-get="/partials/download.html" hx-target="#content" hx-swap="innerHTML" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
            <i data-lucide="download"></i>
            Download Audio
        </button>
    </div>
</div>"##.to_string());
    }

    // Query recently added cards
    let mut stmt = conn
        .prepare("SELECT id, title, artist, album_art_path FROM tracks ORDER BY date_added DESC, id DESC LIMIT 6")
        .map_err(|e| format!("Failed to prepare recent query: {e}"))?;

    let cards = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| format!("Failed to query recent cards: {e}"))?;

    let mut shelf_cards_html = String::new();
    for card_res in cards {
        let (id, title, artist, art_path) = card_res.map_err(|e| format!("Card error: {e}"))?;
        let safe_id = html_escape(&id);
        let safe_title = html_escape(&title);
        let artist_str = artist.unwrap_or_else(|| "Unknown Artist".to_string());
        let safe_artist = html_escape(&artist_str);
        let art_html = render_art_tag(art_path.as_deref(), &title, "disc-3");

        shelf_cards_html.push_str(&format!(
            r#"<div class="card album-card neu-glass" data-track-id="{safe_id}" data-role="play-card" style="cursor: pointer; touch-action: manipulation;">
    <div class="card-artwork">
        {art_html}
    </div>
    <div class="card-body">
        <div class="card-title">{safe_title}</div>
        <div class="card-subtitle">{safe_artist}</div>
    </div>
</div>"#
        ));
    }

    // Query continue listening track rows
    let mut stmt_continue = conn
        .prepare("SELECT id, title, artist, album, duration_secs, album_art_path, is_favorite FROM tracks ORDER BY CASE WHEN last_played IS NOT NULL THEN 0 ELSE 1 END, last_played DESC, date_added DESC LIMIT 6")
        .map_err(|e| format!("Failed to prepare continue query: {e}"))?;

    let track_rows = stmt_continue
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)? != 0,
            ))
        })
        .map_err(|e| format!("Failed to query continue tracks: {e}"))?;

    let mut continue_rows_html = String::new();
    for row_res in track_rows {
        let (id, title, artist, album, duration_secs, art_path, is_fav) =
            row_res.map_err(|e| format!("Row error: {e}"))?;
        continue_rows_html.push_str(&render_track_row_html(
            &id,
            &title,
            artist.as_deref(),
            album.as_deref(),
            duration_secs,
            art_path.as_deref(),
            is_fav,
        ));
    }

    let mut html = String::new();
    html.push_str(
        r##"<section class="section-header">
    <h2 class="section-title">Recently Added</h2>
    <a href="#" class="section-link" hx-get="/partials/library.html" hx-target="#content" hx-swap="innerHTML">See all</a>
</section>

<div class="shelf" id="recently-added-shelf">"##,
    );
    html.push_str(&shelf_cards_html);
    html.push_str(
        r#"</div>

<section class="section-header" style="margin-top: var(--space-6);">
    <h2 class="section-title">Continue Listening</h2>
</section>

<div class="track-list" id="continue-listening-tracks">"#,
    );
    html.push_str(&continue_rows_html);
    html.push_str("</div>");

    Ok(html)
}

/// Representation of a lightweight track item for search result rendering.
pub(crate) struct SearchTrackItem {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: u32,
    pub album_art_path: Option<String>,
    pub is_favorite: bool,
}

/// Query matching tracks across title, artist, and album from the database.
pub(crate) fn query_search_tracks(
    conn: &rusqlite::Connection,
    query: &str,
) -> Result<Vec<SearchTrackItem>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, artist, album, duration_secs, album_art_path, is_favorite 
            FROM tracks 
            WHERE title LIKE ?1 OR artist LIKE ?1 OR album LIKE ?1 
            ORDER BY title COLLATE NOCASE ASC 
            LIMIT 100",
        )
        .map_err(|e| format!("Failed to prepare search query: {e}"))?;

    let pattern = format!("%{query}%");
    let rows = stmt
        .query_map([&pattern], |row| {
            Ok(SearchTrackItem {
                id: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                duration_secs: row.get(4)?,
                album_art_path: row.get(5)?,
                is_favorite: row.get::<_, i64>(6)? != 0,
            })
        })
        .map_err(|e| format!("Failed to query search results: {e}"))?;

    let mut results = Vec::new();
    for row_res in rows {
        results.push(row_res.map_err(|e| format!("Row error: {e}"))?);
    }
    Ok(results)
}

/// Render pre-formatted HTML track rows for a list of search result tracks.
pub(crate) fn render_search_results_html(tracks: &[SearchTrackItem]) -> String {
    let mut html = String::new();
    for track in tracks {
        html.push_str(&render_track_row_html(
            &track.id,
            &track.title,
            track.artist.as_deref(),
            track.album.as_deref(),
            track.duration_secs,
            track.album_art_path.as_deref(),
            track.is_favorite,
        ));
    }
    html
}

/// Render HTML track rows for search view.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_search_results_html(
    db: State<'_, Database>,
    query: Option<String>,
    q: Option<String>,
) -> Result<String, String> {
    let raw_query = query.or(q).unwrap_or_default();
    let trimmed = raw_query.trim();

    if trimmed.is_empty() {
        return Ok(r#"<div class="empty-state glass neu" style="padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
    <div class="empty-state-icon"><i data-lucide="search"></i></div>
    <h2 class="empty-state-title">Search your library</h2>
    <p class="empty-state-description">Enter a query above to find tracks, artists, or albums.</p>
</div>"#.to_string());
    }

    let conn = db.connection().map_err(|e| e.to_string())?;
    let tracks = query_search_tracks(&conn, trimmed)?;

    if tracks.is_empty() {
        return Ok(r#"<div class="empty-state glass neu" style="padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
    <div class="empty-state-icon"><i data-lucide="search"></i></div>
    <h2 class="empty-state-title">No matching tracks</h2>
    <p class="empty-state-description">Try searching for a different title, artist, or album.</p>
</div>"#.to_string());
    }

    Ok(render_search_results_html(&tracks))
}

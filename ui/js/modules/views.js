/**
 * Views Module
 * Handles loading, rendering, filtering and UI state transitions for all app views.
 */

let _lastPlayTriggerTime = 0;
let _lastPlayTrackId = null;
export function _safePlayTrack(trackId) {
    if (!trackId) return;
    const now = Date.now();
    if (_lastPlayTrackId === trackId && (now - _lastPlayTriggerTime < 350)) return;
    _lastPlayTriggerTime = now;
    _lastPlayTrackId = trackId;
    if (window.Auralis && window.Auralis.bridge && typeof window.Auralis.bridge.playTrack === 'function') {
        window.Auralis.bridge.playTrack(trackId);
    } else if (window.AuralisPlayer && typeof window.AuralisPlayer.playTrack === 'function') {
        window.AuralisPlayer.playTrack(trackId);
    }
}
try {
    window._safePlayTrack = _safePlayTrack;
} catch (_) {}

document.addEventListener('click', (evt) => {
    const playBtn = evt.target.closest && evt.target.closest('.play-shelf-btn, .play-track-btn');
    if (!playBtn) return;

    const trackId = playBtn.dataset.firstTrackId || playBtn.dataset.trackId;
    if (trackId) {
        _safePlayTrack(trackId);
    }
});

export const viewMethods = {
    refreshCurrentView() {
        const content = document.getElementById('content');
        if (!content) return;

        if (content.querySelector('.page-library')) {
            this.activeView = 'library';
            this.loadLibraryView();
        } else if (content.querySelector('.page-albums')) {
            this.activeView = 'albums';
            this.loadAlbumsView();
        } else if (content.querySelector('.page-artists')) {
            this.activeView = 'artists';
            this.loadArtistsView();
        } else if (content.querySelector('.page-downloads')) {
            this.activeView = 'downloads';
            this.loadDownloadView();
        } else if (content.querySelector('.page-search')) {
            this.activeView = 'search';
            this.loadSearchView();
        } else if (content.querySelector('.page-settings, #settings-view')) {
            this.activeView = 'settings';
            this.loadSettingsView();
        } else if (content.querySelector('.page-playlists')) {
            this.activeView = 'playlists';
            this.loadPlaylistsView();
        } else if (content.querySelector('.page-sync')) {
            this.activeView = 'sync';
            this.loadSyncView();
        } else {
            this.activeView = 'home';
            this.loadHomeView();
        }
    },

    async loadLibraryView() {
        const expected = 'library';
        if (this.activeView !== expected) return;

        try {
            const page = await this.invoke('get_tracks');
            if (this.activeView !== expected) return;
            const content = document.getElementById('content');
            if (!content || !content.querySelector('.page-library')) return;

            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
            } else {
                this.tracks = [];
            }
            this.bindLibraryFilterControls();
            this.renderLibraryTracks();
        } catch (err) {
            console.error('Error loading library:', err);
            this.bindLibraryFilterControls();
            this.renderLibraryTracks();
        }
    },

    bindLibraryFilterControls() {
        const sortSelect = document.querySelector('.page-library select[name="sort_by"]');
        const downloadedCheckbox = document.querySelector('.page-library input[name="downloaded_only"]');

        if (sortSelect && !sortSelect.dataset.bound) {
            sortSelect.dataset.bound = 'true';
            sortSelect.addEventListener('change', () => {
                this.renderLibraryTracks();
            });
        }

        if (downloadedCheckbox && !downloadedCheckbox.dataset.bound) {
            downloadedCheckbox.dataset.bound = 'true';
            downloadedCheckbox.addEventListener('change', () => {
                this.renderLibraryTracks();
            });
        }
    },

    renderLibraryTracks() {
        const trackList = document.querySelector('.page-library .track-list');
        if (!trackList) return;

        const sortSelect = document.querySelector('.page-library select[name="sort_by"]');
        const downloadedCheckbox = document.querySelector('.page-library input[name="downloaded_only"]');

        let filtered = [...this.tracks];

        if (downloadedCheckbox && downloadedCheckbox.checked) {
            filtered = filtered.filter(t => t.is_downloaded || (t.file_path && !t.file_path.startsWith('http')));
        }

        const sortBy = sortSelect ? sortSelect.value : 'date_added';
        if (sortBy === 'title') {
            filtered.sort((a, b) => (a.title || '').localeCompare(b.title || ''));
        } else if (sortBy === 'artist') {
            filtered.sort((a, b) => (a.artist || '').localeCompare(b.artist || ''));
        } else if (sortBy === 'album') {
            filtered.sort((a, b) => (a.album || '').localeCompare(b.album || ''));
        } else if (sortBy === 'date_added') {
            if (filtered.some(t => t.created_at || t.date_added)) {
                filtered.sort((a, b) => new Date(b.created_at || b.date_added || 0) - new Date(a.created_at || a.date_added || 0));
            }
        }

        if (filtered.length > 0) {
            this.renderTrackRows(trackList, filtered);
        } else {
            trackList.innerHTML = `
                <div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center;">
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
                </div>
            `;
            if (window.lucide) window.lucide.createIcons();
        }
    },

    _syncPlayerBar() {
        // Re-sync mini-player bar with Rust after view navigation (fixes 01:39-01:44 desync)
        try {
            if (window.Auralis && window.Auralis.player && typeof window.Auralis.player.hydrateState === 'function') {
                window.Auralis.player.hydrateState().catch(()=>{});
            }
        } catch (_) {}
    },

    async loadHomeView() {
        const expected = 'home';
        if (this.activeView !== expected) return;
        try {
            const page = await this.invoke('get_tracks', { filter: { limit: 12 } });
            if (this.activeView !== expected) return;
            const _content = document.getElementById('content');
            if (_content && _content.firstElementChild && !_content.querySelector('.page-home')) return;
            const shelf = document.getElementById('recently-added-shelf') || document.querySelector('.page-home .shelf') || document.querySelector('#content .shelf');
            const trackList = document.getElementById('continue-listening-tracks') || document.querySelector('.page-home .track-list');
            const container = document.getElementById('home-dynamic-content') || document.querySelector('.page-home');

            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
                if (shelf) {
                    shelf.innerHTML = page.tracks.slice(0, 6).map(track => `
                        <div class="card album-card neu-glass" data-track-id="${track.id}" data-role="play-card" style="cursor: pointer; touch-action: manipulation;">
                            <div class="card-artwork">
                                ${track.album_art_path ? this.artImgTag(track.album_art_path, track.title) : `<i data-lucide="disc-3"></i>`}
                            </div>
                            <div class="card-body">
                                <div class="card-title">${this.escapeHtml(track.title)}</div>
                                <div class="card-subtitle">${this.escapeHtml(track.artist || 'Unknown Artist')}</div>
                            </div>
                        </div>
                    `).join('');
                    // Delegate clicks/touches robustly for mobile WebView (fixes 01:40-01:42 swallowed taps)
                    if (shelf && !shelf.dataset.bound) {
                        shelf.dataset.bound = 'true';
                        const handler = (e) => {
                            const card = e.target.closest && e.target.closest('[data-role="play-card"]');
                            if (!card || !shelf.contains(card)) return;
                            e.preventDefault();
                            const tid = card.dataset.trackId;
                            if (tid) _safePlayTrack(tid);
                        };
                        shelf.addEventListener('click', handler);
                        shelf.addEventListener('touchend', handler, { passive: false });
                    }
                }
                if (trackList) {
                    this.renderTrackRows(trackList, page.tracks.slice(0, 6));
                }
                if (window.lucide) window.lucide.createIcons();
                this._syncPlayerBar();
            } else {
                if (container) {
                    container.innerHTML = `
                        <div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center; margin-top: var(--space-4);">
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
                        </div>
                    `;
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (err) {
            console.error('Error loading home view:', err);
        }
    },

    async loadAlbumsView() {
        const expected = 'albums';
        if (this.activeView !== expected) return;

        try {
            const page = await this.invoke('get_tracks');
            if (this.activeView !== expected) return;
            const _content = document.getElementById('content');
            if (!_content || !_content.querySelector('.page-albums')) return;
            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
                const albumMap = new Map();
                for (const t of page.tracks) {
                    const albumName = t.album || 'Unknown Album';
                    if (!albumMap.has(albumName)) {
                        albumMap.set(albumName, { name: albumName, artist: t.artist || 'Unknown Artist', art: t.album_art_path, tracks: [] });
                    }
                    albumMap.get(albumName).tracks.push(t);
                }
                this._albumMap = albumMap;

                grid.innerHTML = Array.from(albumMap.values()).map(album => `
                    <div class="card album-card neu-glass" data-album-name="${this.escapeHtml(album.name)}" data-role="play-album" style="cursor: pointer; touch-action: manipulation;">
                        <div class="card-artwork">
                            ${album.art ? this.artImgTag(album.art, album.name) : `<i data-lucide="disc-3"></i>`}
                        </div>
                        <div class="card-body">
                            <div class="card-title">${this.escapeHtml(album.name)}</div>
                            <div class="card-subtitle">${this.escapeHtml(album.artist)} · ${album.tracks.length} tracks</div>
                        </div>
                    </div>
                `).join('');
                if (grid && !grid.dataset.bound) {
                    grid.dataset.bound = 'true';
                    const handler = (e) => {
                        const card = e.target.closest && e.target.closest('[data-role="play-album"]');
                        if (!card || !grid.contains(card)) return;
                        e.preventDefault();
                        const albName = card.dataset.albumName;
                        const alb = albName && this._albumMap && this._albumMap.get(albName);
                        if (alb && alb.tracks && alb.tracks.length > 0) {
                            this.tracks = alb.tracks;
                            _safePlayTrack(alb.tracks[0].id);
                        }
                    };
                    grid.addEventListener('click', handler);
                    grid.addEventListener('touchend', handler, { passive: false });
                }
                if (window.lucide) window.lucide.createIcons();
            } else {
                grid.innerHTML = `
                    <div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
                        <i data-lucide="disc-3" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
                        <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No albums found</h3>
                        <p style="color: var(--text-3); font-size: var(--text-sm);">Scan your music library to populate your album catalog.</p>
                    </div>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading albums:', err);
        }
    },

    async loadArtistsView() {
        const expected = 'artists';
        if (this.activeView !== expected) return;

        try {
            const page = await this.invoke('get_tracks');
            if (this.activeView !== expected) return;
            const _content = document.getElementById('content');
            if (!_content || !_content.querySelector('.page-artists')) return;
            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
                const artistMap = new Map();
                for (const t of page.tracks) {
                    const artistName = t.artist || 'Unknown Artist';
                    if (!artistMap.has(artistName)) {
                        artistMap.set(artistName, { name: artistName, tracks: [] });
                    }
                    artistMap.get(artistName).tracks.push(t);
                }
                this._artistMap = artistMap;

                grid.innerHTML = Array.from(artistMap.values()).map(artist => `
                    <div class="card artist-card neu-glass" data-artist-name="${this.escapeHtml(artist.name)}" data-role="play-artist" style="cursor: pointer; touch-action: manipulation;">
                        <div class="card-artwork">
                            <i data-lucide="user"></i>
                        </div>
                        <div class="card-body">
                            <div class="card-title">${this.escapeHtml(artist.name)}</div>
                            <div class="card-subtitle">${artist.tracks.length} tracks</div>
                        </div>
                    </div>
                `).join('');
                if (grid && !grid.dataset.bound) {
                    grid.dataset.bound = 'true';
                    const handler = (e) => {
                        const card = e.target.closest && e.target.closest('[data-role="play-artist"]');
                        if (!card || !grid.contains(card)) return;
                        e.preventDefault();
                        const artName = card.dataset.artistName;
                        const art = artName && this._artistMap && this._artistMap.get(artName);
                        if (art && art.tracks && art.tracks.length > 0) {
                            this.tracks = art.tracks;
                            _safePlayTrack(art.tracks[0].id);
                        }
                    };
                    grid.addEventListener('click', handler);
                    grid.addEventListener('touchend', handler, { passive: false });
                }
                if (window.lucide) window.lucide.createIcons();
            } else {
                grid.innerHTML = `
                    <div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
                        <i data-lucide="users" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
                        <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No artists found</h3>
                        <p style="color: var(--text-3); font-size: var(--text-sm);">Scan your music library to discover artists.</p>
                    </div>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading artists:', err);
        }
    },

    async loadPlaylistsView() {
        const expected = 'playlists';
        if (this.activeView !== expected) return;
        const grid = document.getElementById('playlists-grid') || document.querySelector('.page-playlists .grid');
        const detail = document.getElementById('playlist-detail');
        if (detail && detail.style.display !== 'block') {
            detail.style.display = 'none';
            if (grid) grid.style.display = 'grid';
        }
        if (!grid && !detail) return;

        try {
            const playlists = await this.invoke('get_playlists');
            if (this.activeView !== expected) return;
            const _content = document.getElementById('content');
            if (!_content || !_content.querySelector('.page-playlists')) return;
            if (playlists && playlists.length > 0) {
                grid.innerHTML = playlists.map(pl => {
                    const isSmart = Boolean(pl.is_smart);
                    let iconName = 'list-music';
                    let badgeHtml = '';
                    if (isSmart) {
                        const idStr = String(pl.id);
                        if (idStr === 'smart_favorites' || pl.name === 'Favorites') {
                            iconName = 'heart';
                            badgeHtml = `<span class="badge" style="background: rgba(239, 68, 68, 0.2); color: #f87171; border: 1px solid rgba(239, 68, 68, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart</span>`;
                        } else if (idStr === 'smart_recent' || pl.name === 'Recently Added') {
                            iconName = 'clock';
                            badgeHtml = `<span class="badge" style="background: rgba(59, 130, 246, 0.2); color: #60a5fa; border: 1px solid rgba(59, 130, 246, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart</span>`;
                        } else if (idStr === 'smart_most_played' || pl.name === 'Most Played') {
                            iconName = 'flame';
                            badgeHtml = `<span class="badge" style="background: rgba(245, 158, 11, 0.2); color: #fbbf24; border: 1px solid rgba(245, 158, 11, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart</span>`;
                        } else {
                            iconName = 'sparkles';
                            badgeHtml = `<span class="badge" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart</span>`;
                        }
                    }

                    return `
                    <div class="card playlist-card neu-glass" onclick="window.Auralis.bridge.openPlaylist('${pl.id}')" style="cursor: pointer;">
                        <div class="card-artwork"><i data-lucide="${iconName}"></i></div>
                        <div class="card-body">
                            <div style="display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); margin-bottom: var(--space-1);">
                                <div class="card-title" style="margin-bottom: 0;">${this.escapeHtml(pl.name)}</div>
                                ${badgeHtml}
                            </div>
                            <div class="card-subtitle">${pl.track_ids ? pl.track_ids.length : 0} tracks</div>
                        </div>
                    </div>
                `;
                }).join('');
                if (window.lucide) window.lucide.createIcons();
            } else {
                grid.innerHTML = `
                    <div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
                        <i data-lucide="list-music" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
                        <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No playlists created</h3>
                        <p style="color: var(--text-3); font-size: var(--text-sm); margin-bottom: var(--space-4);">Create playlists to curate your favorite music collections.</p>
                        <button class="btn btn-primary neu" onclick="window.Auralis.bridge.promptCreatePlaylist()">
                            <i data-lucide="plus"></i>
                            Create First Playlist
                        </button>
                    </div>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading playlists:', err);
        }
    },

    async openPlaylist(playlistId) {
        if (!playlistId) return;
        try {
            const result = await this.invoke('get_playlist', { id: playlistId });
            if (!result) {
                this.showToast('Playlist not found', 'warning');
                return;
            }
            const [playlist, tracks] = result;
            this.tracks = tracks || [];
            const detailEl = document.getElementById('playlist-detail');
            const gridEl = document.getElementById('playlists-grid') || document.querySelector('.page-playlists .grid');

            if (detailEl && gridEl) {
                gridEl.style.display = 'none';
                detailEl.style.display = 'block';

                const isSmart = Boolean(playlist.is_smart);
                let badge = isSmart ? `<span class="badge" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart Playlist</span>` : '';

                detailEl.innerHTML = `
                    <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: var(--space-4);">
                        <button class="btn btn-secondary btn-sm neu" onclick="window.Auralis.bridge.closePlaylistDetail()">
                            <i data-lucide="arrow-left"></i>
                            Back to Playlists
                        </button>
                        ${tracks && tracks.length > 0 ? `
                            <button class="btn btn-primary btn-sm neu" onclick="window.Auralis.bridge.playTrack('${tracks[0].id}')">
                                <i data-lucide="play"></i>
                                Play All (${tracks.length})
                            </button>
                        ` : ''}
                    </div>
                    <div class="card neu-glass" style="margin-bottom: var(--space-4); padding: var(--space-4);">
                        <div style="display: flex; align-items: center; gap: var(--space-2); margin-bottom: var(--space-1);">
                            <h3 style="font-size: var(--text-xl); font-weight: var(--font-bold); color: var(--text-1); margin: 0;">${this.escapeHtml(playlist.name)}</h3>
                            ${badge}
                        </div>
                        ${playlist.description ? `<p style="color: var(--text-3); font-size: var(--text-sm); margin: 0 0 var(--space-2) 0;">${this.escapeHtml(playlist.description)}</p>` : ''}
                        <div style="color: var(--text-3); font-size: var(--text-xs);">${tracks ? tracks.length : 0} tracks</div>
                    </div>
                    <div class="track-list" id="playlist-track-list">
                        <!-- Tracks -->
                    </div>
                `;

                const trackListEl = document.getElementById('playlist-track-list');
                if (tracks && tracks.length > 0) {
                    this.renderTrackRows(trackListEl, tracks);
                } else {
                    trackListEl.innerHTML = `
                        <div class="empty-state glass neu" style="padding: var(--space-6); text-align: center; border-radius: var(--radius-md);">
                            <i data-lucide="music" style="width: 32px; height: 32px; color: var(--accent); margin-bottom: var(--space-2);"></i>
                            <h4 style="color: var(--text-1); font-size: var(--text-base); margin-bottom: var(--space-1);">No tracks in playlist</h4>
                            <p style="color: var(--text-3); font-size: var(--text-xs);">Add tracks to this playlist from your library.</p>
                        </div>
                    `;
                }
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Failed to open playlist:', err);
            this.showToast(`Failed to open playlist: ${err}`, 'error');
        }
    },

    closePlaylistDetail() {
        const detailEl = document.getElementById('playlist-detail');
        const gridEl = document.getElementById('playlists-grid') || document.querySelector('.page-playlists .grid');
        if (detailEl) detailEl.style.display = 'none';
        if (gridEl) gridEl.style.display = 'grid';
        if (window.lucide) window.lucide.createIcons();
    },

    async promptCreatePlaylist() {
        const name = prompt('Enter playlist name:');
        if (!name || !name.trim()) return;

        try {
            const created = await this.invoke('create_playlist', { request: { name: name.trim() } });
            if (created) {
                this.showToast(`Playlist "${created.name}" created!`, 'success');
                this.loadPlaylistsView();
            }
        } catch (err) {
            this.showToast(`Failed to create playlist: ${err}`, 'error');
        }
    },

    async loadSearchView() {
        const form = document.getElementById('search-form');
        const resultsContainer = document.getElementById('search-results');
        if (!form || !resultsContainer) return;

        const performSearch = async () => {
            const input = form.querySelector('input[name="q"]');
            if (!input || !input.value.trim()) return;

            try {
                const page = await this.invoke('get_tracks', { filter: { search: input.value.trim() } });
                if (!document.getElementById('search-results')) return;
                if (page && page.tracks && page.tracks.length > 0) {
                    this.tracks = page.tracks;
                    this.renderTrackRows(resultsContainer, page.tracks);
                } else {
                    resultsContainer.innerHTML = `
                        <div class="empty-state glass neu" style="padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
                            <div class="empty-state-icon"><i data-lucide="search"></i></div>
                            <h2 class="empty-state-title">No matching tracks</h2>
                            <p class="empty-state-description">Try searching for a different title, artist, or album.</p>
                        </div>
                    `;
                    if (window.lucide) window.lucide.createIcons();
                }
            } catch (err) {
                console.error('Search error:', err);
            }
        };

        if (form._searchHandler) form.removeEventListener('submit', form._searchHandler);
        form._searchHandler = (e) => { e.preventDefault(); performSearch(); };
        form.addEventListener('submit', form._searchHandler);
        form.dataset.bound = 'true';

        const searchInput = form.querySelector('input[name="q"]');
        if (searchInput) {
            if (searchInput._inputHandler) searchInput.removeEventListener('input', searchInput._inputHandler);
            let debounceTimer;
            searchInput._inputHandler = () => {
                clearTimeout(debounceTimer);
                debounceTimer = setTimeout(performSearch, 300);
            };
            searchInput.addEventListener('input', searchInput._inputHandler);
        }
    },

    async loadSettingsView() {
        const settingsView = document.querySelector('.page-settings, #settings-view');
        if (!settingsView) return;

        try {
            const settings = await this.invoke('get_settings');
            if (!settings) return;
            if (!document.querySelector('.page-settings, #settings-view')) return;
            this.currentSettings = settings;
            this.currentSettings.audio = this.currentSettings.audio || {};
            this.currentSettings.downloads = this.currentSettings.downloads || {};
            this.currentSettings.sync = this.currentSettings.sync || {};
            this.currentSettings.appearance = this.currentSettings.appearance || {};

            const volInput = settingsView.querySelector('input[name="volume"]');
            const volBadge = settingsView.querySelector('#settings-volume-val');
            if (volInput && settings.audio) {
                const volPct = Math.round(settings.audio.volume * 100);
                volInput.value = volPct;
                if (volBadge) volBadge.textContent = `${volPct}%`;
            }

            const downloadPathInput = settingsView.querySelector('input[name="download_path"]');
            if (downloadPathInput && settings.downloads) {
                downloadPathInput.value = settings.downloads.download_path || '';
            }

            const formatSelect = settingsView.querySelector('select[name="default_format"]');
            if (formatSelect && settings.downloads && settings.downloads.default_format) {
                formatSelect.value = String(settings.downloads.default_format).toLowerCase();
            }

            const ytCookieInput = settingsView.querySelector('input[name="youtube_cookie"]');
            if (ytCookieInput && settings.downloads) {
                ytCookieInput.value = settings.downloads.youtube_cookie || '';
            }

            const ytPoTokenInput = settingsView.querySelector('input[name="youtube_po_token"]');
            if (ytPoTokenInput && settings.downloads) {
                ytPoTokenInput.value = settings.downloads.youtube_po_token || '';
            }

            if (window.AuralisYouTube && settings.downloads) {
                window.AuralisYouTube.setCredentials({
                    cookie: settings.downloads.youtube_cookie,
                    po_token: settings.downloads.youtube_po_token,
                });
            }

            const currentTheme = (settings.appearance && settings.appearance.theme) ? String(settings.appearance.theme).toLowerCase() : 'dark';
            const themeOptions = settingsView.querySelectorAll('.theme-option[data-theme]');
            themeOptions.forEach(opt => {
                opt.classList.toggle('active', opt.dataset.theme === currentTheme);
            });

            const useSystemDownloadsToggle = settingsView.querySelector('[name="use_system_downloads"], [data-name="use_system_downloads"]');
            if (useSystemDownloadsToggle && settings.downloads) {
                const val = settings.downloads.use_system_downloads !== undefined ? Boolean(settings.downloads.use_system_downloads) : true;
                if (useSystemDownloadsToggle.type === 'checkbox') {
                    useSystemDownloadsToggle.checked = val;
                } else {
                    useSystemDownloadsToggle.classList.toggle('active', val);
                    useSystemDownloadsToggle.setAttribute('aria-checked', val.toString());
                }
            }

            const syncToggle = settingsView.querySelector('[name="sync_enabled"], [data-name="sync_enabled"]');
            if (syncToggle && settings.sync) {
                if (syncToggle.type === 'checkbox') {
                    syncToggle.checked = Boolean(settings.sync.enabled);
                } else {
                    syncToggle.classList.toggle('active', Boolean(settings.sync.enabled));
                    syncToggle.setAttribute('aria-checked', Boolean(settings.sync.enabled).toString());
                }
            }

            const wifiToggle = settingsView.querySelector('[name="sync_wifi_only"], [data-name="sync_wifi_only"]');
            if (wifiToggle && settings.sync) {
                if (wifiToggle.type === 'checkbox') {
                    wifiToggle.checked = Boolean(settings.sync.wifi_only);
                } else {
                    wifiToggle.classList.toggle('active', Boolean(settings.sync.wifi_only));
                    wifiToggle.setAttribute('aria-checked', Boolean(settings.sync.wifi_only).toString());
                }
            }

            if (settingsView.dataset.bound) {
                if (window.lucide) window.lucide.createIcons();
                return;
            }
            settingsView.dataset.bound = 'true';

            const saveSettings = async () => {
                if (!this.currentSettings) return;
                try {
                    await this.invoke('update_settings', { settings: this.currentSettings });
                    this.showToast('Settings saved', 'success');
                } catch (err) {
                    this.showToast(`Failed to save settings: ${err}`, 'error');
                }
            };

            if (volInput) {
                volInput.addEventListener('input', (e) => {
                    if (volBadge) volBadge.textContent = `${e.target.value}%`;
                });
                volInput.addEventListener('change', async (e) => {
                    if (this.currentSettings) {
                        this.currentSettings.audio = this.currentSettings.audio || {};
                        this.currentSettings.audio.volume = parseFloat(e.target.value) / 100;
                        await saveSettings();
                    }
                });
            }

            if (downloadPathInput) {
                downloadPathInput.addEventListener('change', async (e) => {
                    if (this.currentSettings) {
                        this.currentSettings.downloads = this.currentSettings.downloads || {};
                        this.currentSettings.downloads.download_path = e.target.value;
                        await saveSettings();
                    }
                });
            }

            if (formatSelect) {
                formatSelect.addEventListener('change', async (e) => {
                    if (this.currentSettings) {
                        this.currentSettings.downloads = this.currentSettings.downloads || {};
                        this.currentSettings.downloads.default_format = e.target.value;
                        await saveSettings();
                    }
                });
            }

            if (ytCookieInput) {
                ytCookieInput.addEventListener('change', async (e) => {
                    if (this.currentSettings) {
                        this.currentSettings.downloads = this.currentSettings.downloads || {};
                        this.currentSettings.downloads.youtube_cookie = e.target.value || null;
                        await saveSettings();
                    }
                });
            }

            if (ytPoTokenInput) {
                ytPoTokenInput.addEventListener('change', async (e) => {
                    if (this.currentSettings) {
                        this.currentSettings.downloads = this.currentSettings.downloads || {};
                        this.currentSettings.downloads.youtube_po_token = e.target.value || null;
                        await saveSettings();
                    }
                });
            }

            themeOptions.forEach(btn => {
                btn.addEventListener('click', async (e) => {
                    e.preventDefault();
                    const selectedTheme = btn.dataset.theme;
                    if (typeof this.setTheme === 'function') {
                        await this.setTheme(selectedTheme);
                    } else {
                        themeOptions.forEach(opt => opt.classList.toggle('active', opt === btn));
                        this.applyTheme(selectedTheme);
                        if (this.currentSettings) {
                            this.currentSettings.appearance = this.currentSettings.appearance || {};
                            this.currentSettings.appearance.theme = selectedTheme;
                            await saveSettings();
                        }
                    }
                });
            });

            if (useSystemDownloadsToggle) {
                useSystemDownloadsToggle.addEventListener('click', async () => {
                    if (this.currentSettings) {
                        this.currentSettings.downloads = this.currentSettings.downloads || {};
                        const newState = !useSystemDownloadsToggle.classList.contains('active');
                        useSystemDownloadsToggle.classList.toggle('active', newState);
                        useSystemDownloadsToggle.setAttribute('aria-checked', newState.toString());
                        this.currentSettings.downloads.use_system_downloads = newState;
                        await saveSettings();
                    }
                });
            }

            if (syncToggle) {
                syncToggle.addEventListener('click', async () => {
                    if (this.currentSettings) {
                        this.currentSettings.sync = this.currentSettings.sync || {};
                        const newState = !syncToggle.classList.contains('active');
                        syncToggle.classList.toggle('active', newState);
                        syncToggle.setAttribute('aria-checked', newState.toString());
                        this.currentSettings.sync.enabled = newState;
                        await saveSettings();
                    }
                });
            }

            if (wifiToggle) {
                wifiToggle.addEventListener('click', async () => {
                    if (this.currentSettings) {
                        this.currentSettings.sync = this.currentSettings.sync || {};
                        const newState = !wifiToggle.classList.contains('active');
                        wifiToggle.classList.toggle('active', newState);
                        wifiToggle.setAttribute('aria-checked', newState.toString());
                        this.currentSettings.sync.wifi_only = newState;
                        await saveSettings();
                    }
                });
            }

            if (window.lucide) window.lucide.createIcons();
        } catch (err) {
            console.error('Settings load error:', err);
        }
    },

    renderTrackRows(container, tracks) {
        container.innerHTML = tracks.map(track => {
            const isFav = Boolean(track.is_favorite);
            return `
            <div class="track-row neu-glass" data-track-id="${track.id}" data-role="play-row" style="cursor: pointer; margin-bottom: var(--space-2); border-radius: var(--radius-md); touch-action: manipulation;">
                <div class="track-row-artwork">
                    ${track.album_art_path ? this.artImgTag(track.album_art_path, track.title) : `<i data-lucide="music"></i>`}
                </div>
                <div class="track-row-info">
                    <div class="track-row-title">${this.escapeHtml(track.title)}</div>
                    <div class="track-row-subtitle">${this.escapeHtml(track.artist || 'Unknown Artist')} — ${this.escapeHtml(track.album || 'Single')}</div>
                </div>
                <span class="track-row-duration">${this.formatTime(track.duration_secs || 0)}</span>
                <div class="track-row-actions">
                    <button type="button" class="btn btn-ghost btn-icon play-track-btn" 
                            title="Play" data-role="play-btn" data-track-id="${track.id}" 
                            onclick="event.stopPropagation(); window._safePlayTrack ? window._safePlayTrack('${track.id}') : (window.Auralis && window.Auralis.bridge && window.Auralis.bridge.playTrack('${track.id}'))" 
                            ontouchend="event.stopPropagation(); window._safePlayTrack ? window._safePlayTrack('${track.id}') : (window.Auralis && window.Auralis.bridge && window.Auralis.bridge.playTrack('${track.id}'))">
                        <i data-lucide="play"></i>
                    </button>
                    <button type="button" class="btn btn-ghost btn-icon ${isFav ? 'liked' : ''}" style="${isFav ? 'color: var(--like);' : ''}" title="Like" onclick="event.stopPropagation(); window.Auralis.bridge.toggleTrackFavorite('${track.id}', this)" ontouchend="event.stopPropagation(); window.Auralis.bridge.toggleTrackFavorite('${track.id}', this)">
                        <i data-lucide="heart"></i>
                    </button>
                    <button type="button" class="btn btn-ghost btn-icon track-menu-btn" 
                            title="More options" data-role="menu-btn" data-track-id="${track.id}" 
                            onclick="event.stopPropagation(); window.Auralis && window.Auralis.bridge && window.Auralis.bridge.openTrackContextMenu(event, '${track.id}')" 
                            ontouchend="event.stopPropagation(); window.Auralis && window.Auralis.bridge && window.Auralis.bridge.openTrackContextMenu(event, '${track.id}')">
                        <i data-lucide="more-vertical"></i>
                    </button>
                </div>
            </div>
        `;
        }).join('');

        // Delegate row taps for mobile WebView reliability (fixes 01:40-01:44 swallowed taps)
        if (container && !container.dataset.bound) {
            container.dataset.bound = 'true';
            const rowHandler = (e) => {
                // Ignore clicks on action buttons handled separately
                if (e.target.closest && e.target.closest('.track-row-actions')) {
                    const playBtn = e.target.closest('[data-role="play-btn"]');
                    if (playBtn) {
                        e.preventDefault(); e.stopPropagation();
                        const tid = playBtn.dataset.trackId;
                        if (tid) _safePlayTrack(tid);
                    }
                    return;
                }
                const row = e.target.closest && e.target.closest('[data-role="play-row"]');
                if (!row || !container.contains(row)) return;
                e.preventDefault();
                const tid = row.dataset.trackId;
                if (tid) _safePlayTrack(tid);
            };
            container.addEventListener('click', rowHandler);
            container.addEventListener('touchend', rowHandler, { passive: false });
        }

        if (window.lucide) window.lucide.createIcons();
    },

    closeTrackContextMenu() {
        const existing = document.getElementById('floating-track-context-menu');
        if (existing) existing.remove();
        if (this._contextMenuOutsideListener) {
            document.removeEventListener('click', this._contextMenuOutsideListener);
            document.removeEventListener('touchend', this._contextMenuOutsideListener);
            document.removeEventListener('keydown', this._contextMenuKeyHandler);
            this._contextMenuOutsideListener = null;
            this._contextMenuKeyHandler = null;
        }
    },

    openTrackContextMenu(event, trackId) {
        if (event) {
            event.preventDefault();
            event.stopPropagation();
        }
        this.closeTrackContextMenu();
        if (!trackId) return;

        const track = (this.tracks && this.tracks.find(t => String(t.id) === String(trackId))) || { id: trackId };
        const menu = document.createElement('div');
        menu.id = 'floating-track-context-menu';
        menu.className = 'context-menu';

        menu.innerHTML = `
            <button type="button" class="context-menu-item" data-action="play-next">
                <i data-lucide="play-circle"></i>
                <span>Play Next</span>
            </button>
            <button type="button" class="context-menu-item" data-action="add-queue">
                <i data-lucide="list-plus"></i>
                <span>Add to Queue</span>
            </button>
            <button type="button" class="context-menu-item" data-action="add-playlist">
                <i data-lucide="folder-plus"></i>
                <span>Add to Playlist</span>
            </button>
            <div class="context-menu-divider"></div>
            ${track.artist ? `
            <button type="button" class="context-menu-item" data-action="go-artist">
                <i data-lucide="user"></i>
                <span>Go to Artist</span>
            </button>` : ''}
            ${track.album ? `
            <button type="button" class="context-menu-item" data-action="go-album">
                <i data-lucide="disc"></i>
                <span>Go to Album</span>
            </button>` : ''}
            <button type="button" class="context-menu-item" data-action="edit-metadata">
                <i data-lucide="pencil"></i>
                <span>Edit Metadata</span>
            </button>
            <div class="context-menu-divider"></div>
            <button type="button" class="context-menu-item danger" data-action="delete-track">
                <i data-lucide="trash-2"></i>
                <span>Delete Track</span>
            </button>
        `;

        document.body.appendChild(menu);
        if (window.lucide) window.lucide.createIcons();

        // Calculate positioning
        const triggerEl = event.currentTarget || event.target;
        const rect = triggerEl && triggerEl.getBoundingClientRect ? triggerEl.getBoundingClientRect() : null;
        const clientX = (event.touches && event.touches[0] ? event.touches[0].clientX : event.clientX) || (rect ? rect.left : 100);
        const clientY = (event.touches && event.touches[0] ? event.touches[0].clientY : event.clientY) || (rect ? rect.bottom : 100);

        const menuRect = menu.getBoundingClientRect();
        const menuWidth = menuRect.width || 190;
        const menuHeight = menuRect.height || 260;

        let left = clientX;
        let top = clientY + 6;

        if (rect) {
            left = rect.right - menuWidth;
            top = rect.bottom + 4;
        }

        if (left + menuWidth > window.innerWidth - 8) {
            left = window.innerWidth - menuWidth - 8;
        }
        if (left < 8) left = 8;

        if (top + menuHeight > window.innerHeight - 8) {
            if (rect && rect.top - menuHeight > 8) {
                top = rect.top - menuHeight - 4;
            } else {
                top = window.innerHeight - menuHeight - 8;
            }
        }
        if (top < 8) top = 8;

        menu.style.left = `${Math.round(left)}px`;
        menu.style.top = `${Math.round(top)}px`;

        // Wire item actions
        menu.querySelector('[data-action="play-next"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.playNextTrack(trackId);
        });

        menu.querySelector('[data-action="add-queue"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.addToQueue(trackId);
        });

        menu.querySelector('[data-action="add-playlist"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.openAddToPlaylistModal(trackId);
        });

        menu.querySelector('[data-action="go-artist"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.goToArtist(track.artist);
        });

        menu.querySelector('[data-action="go-album"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.goToAlbum(track.album);
        });

        menu.querySelector('[data-action="edit-metadata"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.openTagEditor(track);
        });

        menu.querySelector('[data-action="delete-track"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.deleteTracks([trackId]);
        });

        // Click outside & ESC listener
        this._contextMenuOutsideListener = (e) => {
            if (!menu.contains(e.target)) {
                this.closeTrackContextMenu();
            }
        };
        this._contextMenuKeyHandler = (e) => {
            if (e.key === 'Escape') {
                this.closeTrackContextMenu();
            }
        };

        setTimeout(() => {
            document.addEventListener('click', this._contextMenuOutsideListener);
            document.addEventListener('touchend', this._contextMenuOutsideListener);
            document.addEventListener('keydown', this._contextMenuKeyHandler);
        }, 20);
    },

    async openTagEditor(trackOrId) {
        let track = typeof trackOrId === 'object' ? trackOrId : null;
        const trackId = track ? track.id : trackOrId;
        if (!track && trackId) {
            track = (this.tracks && this.tracks.find(t => String(t.id) === String(trackId)));
            if (!track) {
                try {
                    track = await this.invoke('get_track', { id: trackId });
                } catch (_) {}
            }
        }
        if (!track) {
            this.showToast('Track not found', 'error');
            return;
        }

        const existingModal = document.getElementById('tag-editor-modal');
        if (existingModal) existingModal.remove();

        const backdrop = document.createElement('div');
        backdrop.id = 'tag-editor-modal';
        backdrop.className = 'modal-backdrop';

        backdrop.innerHTML = `
            <div class="modal-dialog glass neu" onclick="event.stopPropagation()">
                <div class="modal-header">
                    <h3 class="modal-title">Edit Metadata</h3>
                    <button type="button" class="modal-close" id="tag-editor-close" aria-label="Close">
                        <i data-lucide="x"></i>
                    </button>
                </div>
                <form id="tag-editor-form" class="modal-body" onsubmit="return false">
                    <div class="form-group">
                        <label class="form-label">Title</label>
                        <input type="text" name="title" class="input" value="${this.escapeHtml(track.title || '')}" required>
                    </div>
                    <div class="form-row">
                        <div class="form-group">
                            <label class="form-label">Artist</label>
                            <input type="text" name="artist" class="input" value="${this.escapeHtml(track.artist || '')}">
                        </div>
                        <div class="form-group">
                            <label class="form-label">Album Artist</label>
                            <input type="text" name="album_artist" class="input" value="${this.escapeHtml(track.album_artist || '')}">
                        </div>
                    </div>
                    <div class="form-group">
                        <label class="form-label">Album</label>
                        <input type="text" name="album" class="input" value="${this.escapeHtml(track.album || '')}">
                    </div>
                    <div class="form-row">
                        <div class="form-group">
                            <label class="form-label">Genre</label>
                            <input type="text" name="genre" class="input" value="${this.escapeHtml(track.genre || '')}">
                        </div>
                        <div class="form-group">
                            <label class="form-label">Year</label>
                            <input type="number" name="year" class="input" value="${track.year || ''}" placeholder="YYYY">
                        </div>
                    </div>
                    <div class="form-row">
                        <div class="form-group">
                            <label class="form-label">Track #</label>
                            <input type="number" name="track_number" class="input" value="${track.track_number || ''}">
                        </div>
                        <div class="form-group">
                            <label class="form-label">Disc #</label>
                            <input type="number" name="disc_number" class="input" value="${track.disc_number || ''}">
                        </div>
                    </div>
                    <div class="modal-footer">
                        <button type="button" class="btn btn-secondary neu" id="tag-editor-cancel">Cancel</button>
                        <button type="submit" class="btn btn-primary neu" id="tag-editor-save">
                            <i data-lucide="check"></i>
                            Save Changes
                        </button>
                    </div>
                </form>
            </div>
        `;

        document.body.appendChild(backdrop);
        if (window.lucide) window.lucide.createIcons();

        const closeModal = () => {
            backdrop.remove();
            document.removeEventListener('keydown', escListener);
        };

        const escListener = (e) => {
            if (e.key === 'Escape') closeModal();
        };
        document.addEventListener('keydown', escListener);

        backdrop.addEventListener('click', closeModal);
        backdrop.querySelector('#tag-editor-close')?.addEventListener('click', closeModal);
        backdrop.querySelector('#tag-editor-cancel')?.addEventListener('click', closeModal);

        const form = backdrop.querySelector('#tag-editor-form');
        form?.addEventListener('submit', async (e) => {
            e.preventDefault();
            const title = form.querySelector('input[name="title"]').value.trim();
            const artist = form.querySelector('input[name="artist"]').value.trim() || null;
            const albumArtist = form.querySelector('input[name="album_artist"]').value.trim() || null;
            const album = form.querySelector('input[name="album"]').value.trim() || null;
            const genre = form.querySelector('input[name="genre"]').value.trim() || null;
            const yearVal = form.querySelector('input[name="year"]').value.trim();
            const trackNumVal = form.querySelector('input[name="track_number"]').value.trim();
            const discNumVal = form.querySelector('input[name="disc_number"]').value.trim();

            const update = {
                title: title || track.title,
                artist,
                album,
                album_artist: albumArtist,
                genre,
                year: yearVal ? parseInt(yearVal, 10) : null,
                track_number: trackNumVal ? parseInt(trackNumVal, 10) : null,
                disc_number: discNumVal ? parseInt(discNumVal, 10) : null,
            };

            try {
                const updatedTrack = await this.invoke('update_track_metadata', { id: track.id, update });
                if (updatedTrack) {
                    const idx = this.tracks.findIndex(t => String(t.id) === String(track.id));
                    if (idx >= 0) this.tracks[idx] = updatedTrack;

                    if (window.Auralis && window.Auralis.player && window.Auralis.player.currentTrack && String(window.Auralis.player.currentTrack.id) === String(track.id)) {
                        Object.assign(window.Auralis.player.currentTrack, updatedTrack);
                        window.Auralis.player.updateFullScreenMetadata();
                        this.updatePlayerBar(updatedTrack);
                    }

                    this.showToast('Metadata updated successfully!', 'success');
                    closeModal();
                    this.refreshCurrentView();
                }
            } catch (err) {
                console.error('Failed to update track metadata:', err);
                this.showToast(`Failed to update metadata: ${err}`, 'error');
            }
        });
    },

    async openAddToPlaylistModal(trackId) {
        if (!trackId) return;

        let playlists = [];
        try {
            playlists = await this.invoke('get_playlists') || [];
        } catch (e) {
            console.warn('Failed to load playlists:', e);
        }

        const existingModal = document.getElementById('add-playlist-modal');
        if (existingModal) existingModal.remove();

        const backdrop = document.createElement('div');
        backdrop.id = 'add-playlist-modal';
        backdrop.className = 'modal-backdrop';

        backdrop.innerHTML = `
            <div class="modal-dialog glass neu" onclick="event.stopPropagation()">
                <div class="modal-header">
                    <h3 class="modal-title">Add to Playlist</h3>
                    <button type="button" class="modal-close" id="add-playlist-close" aria-label="Close">
                        <i data-lucide="x"></i>
                    </button>
                </div>
                <div class="modal-body">
                    <div style="display: flex; gap: var(--space-2); margin-bottom: var(--space-2);">
                        <input type="text" id="new-playlist-input" class="input" placeholder="Create new playlist..." style="flex: 1;">
                        <button type="button" class="btn btn-primary neu" id="new-playlist-create-btn">
                            <i data-lucide="plus"></i>
                            Create
                        </button>
                    </div>
                    <div class="form-label" style="margin-top: var(--space-2);">Your Playlists</div>
                    <div class="playlist-picker-list" id="playlist-picker-list">
                        ${playlists.length > 0 ? playlists.map(pl => `
                            <div class="playlist-picker-item" data-playlist-id="${pl.id}">
                                <div style="display: flex; align-items: center; gap: var(--space-2);">
                                    <i data-lucide="${pl.is_smart ? 'sparkles' : 'list-music'}" style="width: 18px; height: 18px; color: var(--accent);"></i>
                                    <span style="font-weight: var(--font-medium); color: var(--text-1);">${this.escapeHtml(pl.name)}</span>
                                </div>
                                <span style="font-size: var(--text-xs); color: var(--text-3);">${(pl.track_ids && pl.track_ids.length) || 0} tracks</span>
                            </div>
                        `).join('') : `
                            <div class="empty-state glass" style="padding: var(--space-4); text-align: center; border-radius: var(--radius-md);">
                                <p style="color: var(--text-3); font-size: var(--text-xs);">No custom playlists yet. Create one above!</p>
                            </div>
                        `}
                    </div>
                </div>
                <div class="modal-footer">
                    <button type="button" class="btn btn-secondary neu" id="add-playlist-cancel">Cancel</button>
                </div>
            </div>
        `;

        document.body.appendChild(backdrop);
        if (window.lucide) window.lucide.createIcons();

        const closeModal = () => {
            backdrop.remove();
            document.removeEventListener('keydown', escListener);
        };

        const escListener = (e) => {
            if (e.key === 'Escape') closeModal();
        };
        document.addEventListener('keydown', escListener);

        backdrop.addEventListener('click', closeModal);
        backdrop.querySelector('#add-playlist-close')?.addEventListener('click', closeModal);
        backdrop.querySelector('#add-playlist-cancel')?.addEventListener('click', closeModal);

        const addTrackToPl = async (playlistId, playlistName) => {
            try {
                await this.invoke('add_tracks_to_playlist', {
                    playlist_id: playlistId,
                    playlistId: playlistId,
                    track_ids: [trackId],
                    trackIds: [trackId],
                });
                this.showToast(`Added to "${playlistName}"`, 'success');
                closeModal();
            } catch (err) {
                console.error('Failed to add to playlist:', err);
                this.showToast(`Failed to add: ${err}`, 'error');
            }
        };

        backdrop.querySelectorAll('.playlist-picker-item').forEach(item => {
            item.addEventListener('click', () => {
                const plId = item.dataset.playlistId;
                const plName = item.querySelector('span')?.textContent || 'Playlist';
                addTrackToPl(plId, plName);
            });
        });

        const newPlInput = backdrop.querySelector('#new-playlist-input');
        const createBtn = backdrop.querySelector('#new-playlist-create-btn');
        const handleCreate = async () => {
            const name = newPlInput?.value.trim();
            if (!name) return;
            try {
                const created = await this.invoke('create_playlist', { request: { name } });
                if (created && created.id) {
                    await addTrackToPl(created.id, created.name);
                }
            } catch (err) {
                this.showToast(`Failed to create playlist: ${err}`, 'error');
            }
        };

        createBtn?.addEventListener('click', handleCreate);
        newPlInput?.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') handleCreate();
        });
    },

    async goToArtist(artistName) {
        if (!artistName || artistName === 'Unknown Artist') {
            this.showToast('No artist information available', 'info');
            return;
        }
        try {
            const page = await this.invoke('get_tracks', { filter: { artist: artistName } });
            if (page && page.tracks) {
                this.tracks = page.tracks;
                const content = document.getElementById('content');
                if (content) {
                    content.innerHTML = `
                        <section class="page-library">
                            <header class="section-header">
                                <div>
                                    <button class="btn btn-secondary btn-sm neu" onclick="window.Auralis.bridge.loadLibraryView()" style="margin-bottom: var(--space-2);">
                                        <i data-lucide="arrow-left"></i> All Tracks
                                    </button>
                                    <h2 class="section-title">Artist: ${this.escapeHtml(artistName)}</h2>
                                </div>
                            </header>
                            <div class="track-list"></div>
                        </section>
                    `;
                    const list = content.querySelector('.track-list');
                    if (list) this.renderTrackRows(list, page.tracks);
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (err) {
            console.error('Failed to filter by artist:', err);
        }
    },

    async goToAlbum(albumName) {
        if (!albumName || albumName === 'Unknown Album' || albumName === 'Single') {
            this.showToast('No album information available', 'info');
            return;
        }
        try {
            const page = await this.invoke('get_tracks', { filter: { album: albumName } });
            if (page && page.tracks) {
                this.tracks = page.tracks;
                const content = document.getElementById('content');
                if (content) {
                    content.innerHTML = `
                        <section class="page-library">
                            <header class="section-header">
                                <div>
                                    <button class="btn btn-secondary btn-sm neu" onclick="window.Auralis.bridge.loadLibraryView()" style="margin-bottom: var(--space-2);">
                                        <i data-lucide="arrow-left"></i> All Tracks
                                    </button>
                                    <h2 class="section-title">Album: ${this.escapeHtml(albumName)}</h2>
                                </div>
                            </header>
                            <div class="track-list"></div>
                        </section>
                    `;
                    const list = content.querySelector('.track-list');
                    if (list) this.renderTrackRows(list, page.tracks);
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (err) {
            console.error('Failed to filter by album:', err);
        }
    },

    async deleteTracks(trackIds) {
        if (!trackIds || !trackIds.length) return;
        const confirmMsg = trackIds.length === 1
            ? 'Are you sure you want to delete this track from your library?'
            : `Are you sure you want to delete these ${trackIds.length} tracks from your library?`;
        if (!confirm(confirmMsg)) return;

        try {
            await this.invoke('delete_tracks', { ids: trackIds });
            this.showToast('Track deleted from library', 'info');
            const idSet = new Set(trackIds.map(String));
            this.tracks = this.tracks.filter(t => !idSet.has(String(t.id)));
            this.refreshCurrentView();
        } catch (err) {
            console.error('Failed to delete tracks:', err);
            this.showToast(`Failed to delete track: ${err}`, 'error');
        }
    }
};
